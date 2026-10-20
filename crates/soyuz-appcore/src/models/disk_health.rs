//! Disk health on This PC's drive cards (`[sidebar] disk_health`): while a pane shows
//! This PC, the drives' SMART / NVMe health (`soyuz_fs::health`) is read on a worker,
//! cached per mount and read again every [`REFRESH`]. A drive that reports failing
//! raises one warning toast per session. Both frontends draw the cards from [`get`],
//! [`summary`] and [`details`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::{Location, SizeStyle, format_size};
use soyuz_fs::health::{Concern, DiskHealth, Verdict};
use soyuz_fs::{VolumeInfo, VolumeKind};

use crate::models::toasts::Toast;

/// How long a reading stays fresh.
pub const REFRESH: Duration = Duration::from_secs(300);

type Reading = Option<Arc<Vec<DiskHealth>>>;

struct State {
    cache: HashMap<PathBuf, (Instant, Reading)>,
    pending: HashSet<PathBuf>,
    tx: Sender<(PathBuf, Reading)>,
    rx: Receiver<(PathBuf, Reading)>,
    /// Drives already warned about (by model), once per session.
    warned: HashSet<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new({
        let (tx, rx) = unbounded();
        State { cache: HashMap::new(), pending: HashSet::new(), tx, rx, warned: HashSet::new() }
    });
}

/// The last reading of the drives under `mount` (`None`: not read yet, or nothing known).
pub fn get(mount: &Path) -> Reading {
    STATE.with_borrow(|s| s.cache.get(mount).and_then(|(_, r)| r.clone()))
}

/// Drives that may report health (not network, optical or RAM disks).
fn local(v: &VolumeInfo) -> bool {
    matches!(v.kind, VolumeKind::Fixed | VolumeKind::Removable | VolumeKind::Unknown)
}

/// Per frame (`AppCore::pump_services`): readings that arrived (toasts for failing
/// drives); new or stale ones asked for while a pane shows This PC.
pub fn pump(app: &mut crate::core::AppCore) {
    if !app.config.sidebar.disk_health {
        return;
    }
    let mut arrived = false;
    STATE.with_borrow_mut(|s| {
        while let Ok((mount, reading)) = s.rx.try_recv() {
            s.pending.remove(&mount);
            for h in reading.iter().flat_map(|r| r.iter()) {
                if h.verdict() == Verdict::Failing && s.warned.insert(h.model.clone()) {
                    let name = if h.model.is_empty() { mount.display().to_string() } else { h.model.clone() };
                    app.ui.toasts.push(Toast::warning(tr!("views-health-toast", drive = name)).body(summary_text(h)));
                }
            }
            s.cache.insert(mount, (Instant::now(), reading));
            arrived = true;
        }
    });
    if arrived {
        (app.wake)();
    }
    if !app.ws.tab().panes.iter().any(|p| matches!(p.location, Location::Computer)) {
        return;
    }
    let wake = app.wake.clone();
    STATE.with_borrow_mut(|s| {
        for v in app.ui.volumes.iter().filter(|v| local(v)) {
            let fresh = s.cache.get(&v.mount).is_some_and(|(at, _)| at.elapsed() < REFRESH);
            if fresh || !s.pending.insert(v.mount.clone()) {
                continue;
            }
            let (mount, tx, wake) = (v.mount.clone(), s.tx.clone(), wake.clone());
            let spawned = std::thread::Builder::new().name("soyuz-disk-health".into()).spawn(move || {
                let reading = soyuz_fs::health::volume_health(&mount).map(Arc::new);
                let _ = tx.send((mount, reading));
                wake();
            });
            if spawned.is_err() {
                s.pending.remove(&v.mount);
            }
        }
    });
}

/// The worst verdict of a volume's drives.
pub fn verdict(drives: &[DiskHealth]) -> Verdict {
    drives.iter().map(DiskHealth::verdict).max().unwrap_or(Verdict::Good)
}

/// "41 °C · 3 % worn" for one drive, or what is wrong with it.
fn summary_text(h: &DiskHealth) -> String {
    if let Some(c) = h.concerns.first() {
        return concern_text(c);
    }
    let mut parts = Vec::new();
    if let Some(t) = h.temperature_c {
        parts.push(tr!("views-health-temp", celsius = t));
    }
    if let Some(u) = h.percent_used {
        parts.push(tr!("views-health-worn", percent = u));
    }
    if parts.is_empty() {
        parts.push(tr!("views-health-good").to_string());
    }
    parts.join(" \u{b7} ")
}

/// The card's line: the verdict and a short reason / figures.
pub fn summary(drives: &[DiskHealth]) -> String {
    let worst = drives.iter().max_by_key(|h| h.verdict());
    let head = match verdict(drives) {
        Verdict::Good => tr!("views-health-good"),
        Verdict::Warning => tr!("views-health-warning"),
        Verdict::Failing => tr!("views-health-failing"),
    };
    match worst.map(summary_text) {
        Some(t) if t != head => format!("{head} \u{b7} {t}"),
        _ => head.to_string(),
    }
}

/// Why a drive is not good, in words.
pub fn concern_text(c: &Concern) -> String {
    match c {
        Concern::PredictsFailure => tr!("views-health-predicts-failure").to_string(),
        Concern::SpareLow => tr!("views-health-spare-low").to_string(),
        Concern::Degraded => tr!("views-health-degraded").to_string(),
        Concern::ReadOnly => tr!("views-health-read-only").to_string(),
        Concern::WornOut(p) => tr!("views-health-worn-out", percent = *p),
        Concern::BadSectors(n) => tr!("views-health-bad-sectors", count = *n),
        Concern::MediaErrors(n) => tr!("views-health-media-errors", count = *n),
        Concern::Hot(t) => tr!("views-health-hot", celsius = *t),
    }
}

/// Every figure, one per line (the card's tooltip, the terminal UI's detail).
pub fn details(drives: &[DiskHealth]) -> Vec<String> {
    let mut out = Vec::new();
    for h in drives {
        if !h.model.is_empty() {
            out.push(h.model.clone());
        }
        for c in &h.concerns {
            out.push(concern_text(c));
        }
        if let Some(t) = h.temperature_c {
            out.push(tr!("views-health-temp-line", celsius = t));
        }
        if let Some(u) = h.percent_used {
            out.push(tr!("views-health-worn-line", percent = u));
        }
        if let Some(s) = h.available_spare {
            out.push(tr!("views-health-spare-line", percent = s));
        }
        if let Some(hours) = h.power_on_hours {
            out.push(tr!("views-health-hours-line", hours = hours, days = hours / 24));
        }
        if let Some(c) = h.power_cycles {
            out.push(tr!("views-health-cycles-line", count = c));
        }
        if let Some(c) = h.unsafe_shutdowns {
            out.push(tr!("views-health-unsafe-line", count = c));
        }
        if let Some(b) = h.bytes_written {
            out.push(tr!("views-health-written-line", size = format_size(b, SizeStyle::Decimal)));
        }
        if let Some(b) = h.bytes_read {
            out.push(tr!("views-health-read-line", size = format_size(b, SizeStyle::Decimal)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts() {
        let good = DiskHealth {
            model: "Disk A".into(),
            temperature_c: Some(41),
            percent_used: Some(3),
            power_on_hours: Some(48),
            bytes_written: Some(2_000_000_000_000),
            ..DiskHealth::default()
        };
        assert_eq!(verdict(std::slice::from_ref(&good)), Verdict::Good);
        let s = summary(std::slice::from_ref(&good));
        assert!(s.starts_with(&tr!("views-health-good").to_string()) && s.contains("41"), "{s}");
        let d = details(std::slice::from_ref(&good));
        assert_eq!(d[0], "Disk A");
        assert!(d.iter().any(|l| l.contains("2.00 TB") || l.contains("2 TB")), "{d:?}");
        let bad = DiskHealth { concerns: vec![Concern::BadSectors(8)], ..good.clone() };
        let both = [good, bad];
        assert_eq!(verdict(&both), Verdict::Warning);
        assert!(summary(&both).contains(&concern_text(&Concern::BadSectors(8))));
    }
}
