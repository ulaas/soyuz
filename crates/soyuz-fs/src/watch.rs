//! Directory watching: a thin `notify-debouncer-full` wrapper (one notify backend per
//! [`Watcher`], N watched directories). notify itself uses ReadDirectoryChangesW /
//! inotify / FSEvents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::RecursiveMode;
use notify::event::{EventKind, ModifyKind, RemoveKind};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use parking_lot::Mutex;
use soyuz_core::{FsError, FsErrorKind, FsResult};

/// Maximum number of paths in one [`WatchKind::Changed`]; more → [`WatchKind::Rescan`].
pub const MAX_CHANGED_PATHS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WatchId(pub u64);

#[derive(Clone, Debug)]
pub struct WatchEvent {
    pub id: WatchId,
    pub kind: WatchKind,
}

#[derive(Clone, Debug)]
pub enum WatchKind {
    /// Changed paths (≤ 64), deduplicated, expressed under the watched directory as given.
    Changed(Vec<PathBuf>),
    /// Too many events or backend overflow: relist.
    Rescan,
    /// The watched directory was removed / renamed away / unmounted.
    Lost,
}

#[derive(Debug)]
struct Watched {
    dir: PathBuf,
    /// Canonical form of `dir` when it differs (FSEvents reports `/private/var/...` for
    /// `/var/...`); reported paths under it are mapped back onto `dir`.
    canon: Option<PathBuf>,
    recursive: bool,
    /// `Lost` was already reported (by an event or the liveness check).
    lost: bool,
}

impl Watched {
    /// `p` expressed under `dir` (mapping a canonical prefix back), if it is inside.
    fn local(&self, p: &Path) -> Option<PathBuf> {
        if p.starts_with(&self.dir) {
            return Some(p.to_path_buf());
        }
        let c = self.canon.as_ref()?;
        p.strip_prefix(c)
            .ok()
            .map(|rest| if rest.as_os_str().is_empty() { self.dir.clone() } else { self.dir.join(rest) })
    }

    fn gone(&self) -> bool {
        !crate::list::exists_nofollow(&self.dir)
    }
}

type Registry = Arc<Mutex<HashMap<WatchId, Watched>>>;

/// Owns one notify backend; `Send`.
pub struct Watcher {
    debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    registry: Registry,
    next_id: u64,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").field("watched", &self.registry.lock().len()).finish()
    }
}

impl Watcher {
    /// Events are sent on `tx`, followed by a call to `wake` (e.g. `ctx.request_repaint`).
    /// `wake` is called without any internal lock held.
    pub fn new(
        tx: crossbeam_channel::Sender<WatchEvent>,
        debounce: Duration,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> FsResult<Self> {
        let registry: Registry = Arc::default();
        let reg = registry.clone();
        let liveness_tx = tx.clone();
        let liveness_wake = wake.clone();
        let handler = move |res: DebounceEventResult| {
            let events = dispatch(&reg, res);
            if events.is_empty() {
                return;
            }
            for ev in events {
                if tx.send(ev).is_err() {
                    return;
                }
            }
            wake();
        };
        let debouncer = new_debouncer(debounce, None, handler).map_err(|e| notify_err("watch", None, e))?;
        spawn_liveness(Arc::downgrade(&registry), liveness_tx, liveness_wake);
        Ok(Watcher { debouncer, registry, next_id: 1 })
    }

    pub fn watch(&mut self, dir: &Path, recursive: bool) -> FsResult<WatchId> {
        if !crate::list::exists_nofollow(dir) {
            return Err(FsError::with_message(
                FsErrorKind::NotFound,
                "watch",
                Some(dir),
                "the directory does not exist",
            ));
        }
        let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        self.debouncer.watch(dir, mode).map_err(|e| notify_err("watch", Some(dir), e))?;
        let id = WatchId(self.next_id);
        self.next_id += 1;
        let canon =
            std::fs::canonicalize(dir).ok().map(|c| crate::path::strip_verbatim(&c)).filter(|c| c.as_path() != dir);
        self.registry.lock().insert(id, Watched { dir: dir.to_path_buf(), canon, recursive, lost: false });
        Ok(id)
    }

    pub fn unwatch(&mut self, id: WatchId) {
        let removed = self.registry.lock().remove(&id);
        if let Some(w) = removed {
            let still_used = self.registry.lock().values().any(|o| o.dir == w.dir);
            if !still_used {
                let _ = self.debouncer.unwatch(&w.dir);
            }
        }
    }
}

/// Some backends (ReadDirectoryChangesW keeps a removed directory "delete pending" while
/// its handle is open) never report the removal of the watched directory itself; a light
/// existence check every second reports `Lost` for them. Exits when the [`Watcher`] drops.
///
/// The registry is only snapshotted under its lock; the stats run outside it, so a dead
/// network share (a stat can block for tens of seconds) never stalls event dispatch or
/// `watch`/`unwatch` on the UI thread. Network paths and directories whose stat was slow
/// are checked less often (exponential back-off up to [`LIVENESS_MAX_INTERVAL`]).
fn spawn_liveness(
    reg: std::sync::Weak<Mutex<HashMap<WatchId, Watched>>>,
    tx: crossbeam_channel::Sender<WatchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let spawned = std::thread::Builder::new().name("soyuz-watch-alive".into()).spawn(move || {
        // Per watch: (next check due, current interval).
        let mut schedule: HashMap<WatchId, (Instant, Duration)> = HashMap::new();
        loop {
            std::thread::sleep(LIVENESS_INTERVAL);
            let snapshot: Vec<(WatchId, PathBuf)> = {
                let Some(reg) = reg.upgrade() else { return };
                let reg = reg.lock();
                schedule.retain(|id, _| reg.contains_key(id));
                reg.iter().filter(|(_, w)| !w.lost).map(|(id, w)| (*id, w.dir.clone())).collect()
            };
            let mut gone = Vec::new();
            for (id, dir) in snapshot {
                let now = Instant::now();
                let (due, interval) = *schedule.entry(id).or_insert_with(|| {
                    (now, if is_network_path(&dir) { LIVENESS_REMOTE_INTERVAL } else { LIVENESS_INTERVAL })
                });
                if now < due {
                    continue;
                }
                let t = Instant::now();
                let is_gone = !crate::list::exists_nofollow(&dir);
                let took = t.elapsed();
                let next = if took > LIVENESS_SLOW_STAT { (interval * 2).min(LIVENESS_MAX_INTERVAL) } else { interval };
                schedule.insert(id, (Instant::now() + next, next));
                if is_gone {
                    gone.push(id);
                }
            }
            if gone.is_empty() {
                continue;
            }
            let Some(reg) = reg.upgrade() else { return };
            let mut lost = Vec::new();
            for id in gone {
                // Re-check under the lock: the watch may have been removed meanwhile.
                if let Some(w) = reg.lock().get_mut(&id)
                    && !w.lost
                {
                    w.lost = true;
                    lost.push(id);
                }
            }
            drop(reg);
            if lost.is_empty() {
                continue;
            }
            for id in lost {
                if tx.send(WatchEvent { id, kind: WatchKind::Lost }).is_err() {
                    return;
                }
            }
            wake();
        }
    });
    if spawned.is_err() {
        log::warn!("could not start the watcher liveness thread");
    }
}

const LIVENESS_INTERVAL: Duration = Duration::from_millis(1000);
/// Initial liveness interval for network paths.
const LIVENESS_REMOTE_INTERVAL: Duration = Duration::from_secs(5);
/// Back-off ceiling for slow / unreachable volumes.
const LIVENESS_MAX_INTERVAL: Duration = Duration::from_secs(60);
/// A stat slower than this doubles that watch's interval.
const LIVENESS_SLOW_STAT: Duration = Duration::from_millis(200);

/// UNC / network-looking path (`\\server\share`, `//server/share`); cheap, lexical.
fn is_network_path(p: &Path) -> bool {
    let s = crate::path::strip_verbatim(p);
    let s = s.to_string_lossy();
    s.starts_with("\\\\") || s.starts_with("//")
}

fn notify_err(op: &'static str, path: Option<&Path>, e: notify::Error) -> FsError {
    let kind = match &e.kind {
        notify::ErrorKind::PathNotFound => FsErrorKind::NotFound,
        notify::ErrorKind::Io(io) => return FsError::from_io(op, path, io),
        notify::ErrorKind::MaxFilesWatch => FsErrorKind::NoSpace,
        _ => FsErrorKind::Io,
    };
    let mut fe = FsError::with_message(kind, op, path, e.to_string());
    if kind == FsErrorKind::NoSpace {
        fe.message = "the system limit of watched directories is reached (raise fs.inotify.max_user_watches)".into();
    }
    fe
}

/// Map debounced notify events onto per-watch events.
fn dispatch(reg: &Registry, res: DebounceEventResult) -> Vec<WatchEvent> {
    let mut reg = reg.lock();
    let events = match res {
        Ok(ev) => ev,
        Err(errors) => {
            log::warn!("watcher errors: {errors:?}");
            let mut out = Vec::new();
            for (id, w) in reg.iter_mut() {
                if w.gone() {
                    if !w.lost {
                        w.lost = true;
                        out.push(WatchEvent { id: *id, kind: WatchKind::Lost });
                    }
                } else {
                    out.push(WatchEvent { id: *id, kind: WatchKind::Rescan });
                }
            }
            return out;
        }
    };
    let mut changed: HashMap<WatchId, Vec<PathBuf>> = HashMap::new();
    let mut rescan: Vec<WatchId> = Vec::new();
    let mut lost: Vec<WatchId> = Vec::new();
    for ev in &events {
        if ev.need_rescan() {
            rescan.extend(reg.keys().copied());
            continue;
        }
        for p in &ev.paths {
            for (id, w) in reg.iter() {
                let Some(local) = w.local(p) else {
                    // An ancestor of the watched directory was removed or renamed away.
                    if w.dir.starts_with(p) && w.gone() && !lost.contains(id) {
                        lost.push(*id);
                    }
                    continue;
                };
                if local == w.dir {
                    let removed = matches!(
                        ev.kind,
                        EventKind::Remove(RemoveKind::Folder | RemoveKind::Any)
                            | EventKind::Modify(ModifyKind::Name(_))
                    );
                    if removed && w.gone() && !lost.contains(id) {
                        lost.push(*id);
                    }
                    continue;
                }
                let inside = w.recursive || local.parent() == Some(w.dir.as_path());
                if inside {
                    let v = changed.entry(*id).or_default();
                    if !v.contains(&local) {
                        v.push(local);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for id in &lost {
        if let Some(w) = reg.get_mut(id)
            && !w.lost
        {
            w.lost = true;
            out.push(WatchEvent { id: *id, kind: WatchKind::Lost });
        }
    }
    rescan.sort_by_key(|id| id.0);
    rescan.dedup();
    for id in &rescan {
        if !lost.contains(id) {
            out.push(WatchEvent { id: *id, kind: WatchKind::Rescan });
        }
    }
    for (id, paths) in changed {
        if lost.contains(&id) || rescan.contains(&id) {
            continue;
        }
        let kind = if paths.len() > MAX_CHANGED_PATHS { WatchKind::Rescan } else { WatchKind::Changed(paths) };
        out.push(WatchEvent { id, kind });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_paths_are_detected_lexically() {
        assert!(is_network_path(Path::new(r"\\server\share\dir")));
        assert!(is_network_path(Path::new(r"\\?\UNC\server\share")));
        assert!(is_network_path(Path::new("//server/share")));
        assert!(!is_network_path(Path::new(r"C:\data")));
        assert!(!is_network_path(Path::new("/home/u")));
    }

    fn watcher() -> (Watcher, crossbeam_channel::Receiver<WatchEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (Watcher::new(tx, Duration::from_millis(50), Arc::new(|| {})).unwrap(), rx)
    }

    fn collect(rx: &crossbeam_channel::Receiver<WatchEvent>, until: impl Fn(&[WatchEvent]) -> bool) -> Vec<WatchEvent> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while std::time::Instant::now() < deadline && !until(&got) {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
                got.push(ev);
            }
        }
        got
    }

    fn mentions(evs: &[WatchEvent], p: &Path) -> bool {
        evs.iter().any(|e| match &e.kind {
            WatchKind::Changed(v) => v.iter().any(|x| x.file_name() == p.file_name()),
            WatchKind::Rescan => true,
            WatchKind::Lost => false,
        })
    }

    #[test]
    fn create_delete_and_lost() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("watched");
        std::fs::create_dir(&dir).unwrap();
        let (mut w, rx) = watcher();
        let id = w.watch(&dir, false).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let f = dir.join("a.txt");
        std::fs::write(&f, b"1").unwrap();
        let evs = collect(&rx, |e| mentions(e, &f));
        assert!(mentions(&evs, &f), "create not reported: {evs:?}");
        assert!(evs.iter().all(|e| e.id == id));
        if let Some(WatchKind::Changed(v)) = evs.iter().map(|e| &e.kind).find(|k| matches!(k, WatchKind::Changed(_))) {
            assert!(v.iter().all(|p| p.starts_with(&dir)), "paths are under the watched dir as given: {v:?}");
        }
        std::fs::remove_file(&f).unwrap();
        let evs = collect(&rx, |e| mentions(e, &f));
        assert!(mentions(&evs, &f), "delete not reported: {evs:?}");
        // Removing the watched directory itself → Lost (backends that only report an
        // error get a Lost/Rescan decided by an existence check).
        std::fs::remove_dir(&dir).unwrap();
        let evs = collect(&rx, |e| e.iter().any(|x| matches!(x.kind, WatchKind::Lost)));
        assert!(
            evs.iter().any(|x| matches!(x.kind, WatchKind::Lost | WatchKind::Rescan)),
            "removal of the watched dir not reported: {evs:?}"
        );
        w.unwatch(id);
    }

    #[test]
    fn canonical_paths_map_back() {
        let w = Watched {
            dir: PathBuf::from("/var/x"),
            canon: Some(PathBuf::from("/private/var/x")),
            recursive: false,
            lost: false,
        };
        assert_eq!(w.local(Path::new("/private/var/x/f")), Some(PathBuf::from("/var/x/f")));
        assert_eq!(w.local(Path::new("/private/var/x")), Some(PathBuf::from("/var/x")));
        assert_eq!(w.local(Path::new("/var/x/g")), Some(PathBuf::from("/var/x/g")));
        assert_eq!(w.local(Path::new("/elsewhere")), None);
    }

    #[test]
    fn watching_a_missing_dir_fails() {
        let d = tempfile::tempdir().unwrap();
        let (mut w, _rx) = watcher();
        assert_eq!(w.watch(&d.path().join("missing"), false).unwrap_err().kind, FsErrorKind::NotFound);
    }
}
