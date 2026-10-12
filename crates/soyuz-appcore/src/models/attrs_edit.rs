//! Attribute and permission editing shared by the Properties dialog and the bulk
//! "Attributes & timestamps" dialog, without drawing: tri-state attribute flags
//! (Windows attributes, immutable / hidden flags on unix) over a selection, the unix mode
//! bits (rwx × owner/group/other plus setuid / setgid / sticky) with their octal and
//! symbolic forms, and the conversion of the edits into `SetMeta` jobs (grouped by
//! resulting mode, since a job carries one mode).
//!
//! The GUI draws them in `ui::dialogs::properties_attrs`, the terminal UI in
//! `soyuz-tui`'s attributes dialog.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use soyuz_core::{Attributes, EntryInfo};
use soyuz_fs::{AttrSet, TimesSet};

pub use crate::models::properties::MetaJob;

/// Tri-state value for multi-selection editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    Off,
    On,
    Mixed,
}

impl Tri {
    pub fn from_counts(on: usize, total: usize) -> Tri {
        if on == 0 {
            Tri::Off
        } else if on == total {
            Tri::On
        } else {
            Tri::Mixed
        }
    }
}

/// Attribute flags editable on this platform, with labels and hints.
pub fn editable_attrs() -> Vec<(Attributes, &'static str, &'static str)> {
    if cfg!(windows) {
        vec![
            (Attributes::READONLY, tr!("dialogs-attr-readonly"), tr!("dialogs-attr-readonly-hint")),
            (Attributes::HIDDEN, tr!("dialogs-attr-hidden"), tr!("dialogs-attr-hidden-hint")),
            (Attributes::SYSTEM, tr!("dialogs-attr-system"), tr!("dialogs-attr-system-hint")),
            (Attributes::ARCHIVE, tr!("dialogs-attr-archive"), tr!("dialogs-attr-archive-hint")),
            (Attributes::NOT_CONTENT_INDEXED, tr!("dialogs-attr-not-indexed"), tr!("dialogs-attr-not-indexed-hint")),
            (Attributes::TEMPORARY, tr!("dialogs-attr-temporary"), tr!("dialogs-attr-temporary-hint")),
            (Attributes::OFFLINE, tr!("dialogs-attr-offline"), tr!("dialogs-attr-offline-hint")),
            (Attributes::COMPRESSED, tr!("dialogs-attr-compressed"), tr!("dialogs-attr-compressed-hint")),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            (Attributes::IMMUTABLE, tr!("dialogs-attr-locked"), tr!("dialogs-attr-locked-hint")),
            (Attributes::HIDDEN, tr!("dialogs-attr-hidden"), tr!("dialogs-attr-hidden-finder-hint")),
        ]
    } else {
        vec![(Attributes::IMMUTABLE, tr!("dialogs-attr-immutable"), tr!("dialogs-attr-immutable-hint"))]
    }
}

/// Read-only flags shown for information.
pub fn info_attrs() -> Vec<(Attributes, &'static str)> {
    vec![
        (Attributes::ENCRYPTED, tr!("dialogs-attr-encrypted")),
        (Attributes::SPARSE, tr!("dialogs-attr-sparse")),
        (Attributes::REPARSE, tr!("dialogs-attr-reparse")),
        (Attributes::CLOUD_PLACEHOLDER, tr!("dialogs-attr-cloud")),
    ]
}

/// Tri-state attribute editor over a set of entries: per [`editable_attrs`] row the
/// flag, its current value and its original value.
#[derive(Clone, Debug)]
pub struct AttrEdit {
    pub rows: Vec<(Attributes, Tri, Tri)>,
}

impl AttrEdit {
    pub fn new(entries: &[EntryInfo]) -> Self {
        let n = entries.len().max(1);
        let rows = editable_attrs()
            .into_iter()
            .map(|(a, _, _)| {
                let on = entries.iter().filter(|e| e.attrs.contains(a)).count();
                let t = Tri::from_counts(on, n);
                (a, t, t)
            })
            .collect();
        AttrEdit { rows }
    }

    /// Bits switched on / off relative to the original state (`None` = unchanged).
    pub fn changes(&self) -> Option<AttrSet> {
        let mut set = Attributes::empty();
        let mut clear = Attributes::empty();
        for (a, now, orig) in &self.rows {
            if now == orig {
                continue;
            }
            match now {
                Tri::On => set |= *a,
                Tri::Off => clear |= *a,
                Tri::Mixed => {}
            }
        }
        (!set.is_empty() || !clear.is_empty()).then_some(AttrSet { set, clear })
    }

    pub fn commit(&mut self) {
        for r in &mut self.rows {
            r.2 = r.1;
        }
    }
}

/// Mode bits in grid order: setuid, setgid, sticky, then r/w/x for owner, group, other.
pub const MODE_BITS: [u32; 12] = [0o4000, 0o2000, 0o1000, 0o400, 0o200, 0o100, 0o40, 0o20, 0o10, 0o4, 0o2, 0o1];

/// `0755` style octal text (4 digits when a special bit is set, else 3).
pub fn mode_to_octal(mode: u32) -> String {
    let m = mode & 0o7777;
    if m & 0o7000 != 0 { format!("{m:04o}") } else { format!("{m:03o}") }
}

/// Parse `755`, `0755`, `4755` (1–4 octal digits).
pub fn parse_octal(s: &str) -> Option<u32> {
    let s = s.trim();
    let s = s.strip_prefix("0o").unwrap_or(s);
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return None;
    }
    u32::from_str_radix(s, 8).ok()
}

/// `rwxr-sr-t` symbolic form (with `s`/`S`/`t`/`T` for the special bits).
pub fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    let trip = [(0o400, 0o200, 0o100, 0o4000, 's'), (0o40, 0o20, 0o10, 0o2000, 's'), (0o4, 0o2, 0o1, 0o1000, 't')];
    for (r, w, x, special, ch) in trip {
        s.push(if mode & r != 0 { 'r' } else { '-' });
        s.push(if mode & w != 0 { 'w' } else { '-' });
        s.push(match (mode & x != 0, mode & special != 0) {
            (true, true) => ch,
            (false, true) => ch.to_ascii_uppercase(),
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    s
}

/// Tri-state mode editor over a set of entries.
#[derive(Clone, Debug)]
pub struct ModeEdit {
    pub bits: [Tri; 12],
    orig: [Tri; 12],
    /// The octal field's text (empty while bits are mixed).
    pub octal: String,
    /// The octal field has the keyboard (its text is not rewritten from the bits then).
    pub octal_focus: bool,
}

impl ModeEdit {
    /// `None` when no entry has a mode (Windows).
    pub fn new(entries: &[EntryInfo]) -> Option<Self> {
        let modes: Vec<u32> = entries.iter().filter_map(|e| e.mode).collect();
        if modes.is_empty() {
            return None;
        }
        let mut bits = [Tri::Off; 12];
        for (i, b) in MODE_BITS.iter().enumerate() {
            bits[i] = Tri::from_counts(modes.iter().filter(|m| *m & b != 0).count(), modes.len());
        }
        let mut e = ModeEdit { bits, orig: bits, octal: String::new(), octal_focus: false };
        e.sync_octal();
        Some(e)
    }

    /// The mode if no bit is mixed.
    pub fn uniform(&self) -> Option<u32> {
        let mut m = 0;
        for (i, t) in self.bits.iter().enumerate() {
            match t {
                Tri::On => m |= MODE_BITS[i],
                Tri::Off => {}
                Tri::Mixed => return None,
            }
        }
        Some(m)
    }

    /// The octal text from the bits.
    pub fn sync_octal(&mut self) {
        self.octal = self.uniform().map(mode_to_octal).unwrap_or_default();
    }

    /// Set every bit from an octal text (`false`: not an octal mode, nothing changed).
    pub fn set_octal(&mut self, s: &str) -> bool {
        let Some(m) = parse_octal(s) else { return false };
        for (i, b) in MODE_BITS.iter().enumerate() {
            self.bits[i] = if m & b != 0 { Tri::On } else { Tri::Off };
        }
        true
    }

    pub fn changed(&self) -> bool {
        self.bits != self.orig
    }

    pub fn commit(&mut self) {
        self.orig = self.bits;
    }

    /// New mode for an entry whose current mode is `old` (`None` = unchanged).
    pub fn apply_to(&self, old: u32) -> Option<u32> {
        let mut m = old & 0o7777;
        for (i, (now, orig)) in self.bits.iter().zip(&self.orig).enumerate() {
            if now == orig {
                continue;
            }
            match now {
                Tri::On => m |= MODE_BITS[i],
                Tri::Off => m &= !MODE_BITS[i],
                Tri::Mixed => {}
            }
        }
        (m != old & 0o7777).then_some(m | (old & !0o7777))
    }
}

/// A time as typed in a text field: `2026-10-12 14:03:59`, local time.
pub fn time_text(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Parse a time typed in a text field (local time): `now`, `YYYY-MM-DD`,
/// `YYYY-MM-DD HH:MM` or `YYYY-MM-DD HH:MM:SS` (a `T` between date and time works too).
pub fn parse_time(s: &str, now: SystemTime) -> Option<SystemTime> {
    use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};
    let s = s.trim();
    if s.eq_ignore_ascii_case("now") {
        return Some(now);
    }
    let s = s.replacen('T', " ", 1);
    let naive = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(&s, f).ok())
        .or_else(|| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)))?;
    Local.from_local_datetime(&naive).earliest().map(SystemTime::from)
}

/// Turn edits into jobs: one per resulting mode (a job carries a single mode), plus one
/// for entries whose mode does not change. Empty when nothing changed.
pub fn plan_jobs(
    entries: &[EntryInfo],
    times: Option<TimesSet>,
    attrs: Option<AttrSet>,
    mode: Option<&ModeEdit>,
) -> Vec<MetaJob> {
    let mut by_mode: BTreeMap<Option<u32>, Vec<PathBuf>> = BTreeMap::new();
    for e in entries {
        let m = match (mode, e.mode) {
            (Some(me), Some(old)) if me.changed() => me.apply_to(old),
            _ => None,
        };
        by_mode.entry(m).or_default().push(e.path.clone());
    }
    by_mode
        .into_iter()
        .filter(|(m, _)| m.is_some() || times.is_some() || attrs.is_some())
        .map(|(mode, paths)| MetaJob { paths, times, attrs, mode })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::EntryKind;

    fn entry(p: &str, mode: Option<u32>, attrs: Attributes) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from(p), EntryKind::File);
        e.mode = mode;
        e.attrs = attrs;
        e
    }

    #[test]
    fn tri_from_counts() {
        assert_eq!(Tri::from_counts(0, 3), Tri::Off);
        assert_eq!(Tri::from_counts(3, 3), Tri::On);
        assert_eq!(Tri::from_counts(1, 3), Tri::Mixed);
    }

    #[test]
    fn octal_conversion() {
        assert_eq!(mode_to_octal(0o100755), "755");
        assert_eq!(mode_to_octal(0o4755), "4755");
        assert_eq!(mode_to_octal(0o1777), "1777");
        assert_eq!(parse_octal("755"), Some(0o755));
        assert_eq!(parse_octal("0755"), Some(0o755));
        assert_eq!(parse_octal("0o644"), Some(0o644));
        assert_eq!(parse_octal("4755"), Some(0o4755));
        assert_eq!(parse_octal("7"), Some(0o7));
        assert_eq!(parse_octal("789"), None);
        assert_eq!(parse_octal("17777"), None);
        assert_eq!(parse_octal(""), None);
        for m in [0o644, 0o755, 0o4711, 0o2750, 0o1777, 0o0] {
            assert_eq!(parse_octal(&mode_to_octal(m)), Some(m));
        }
    }

    #[test]
    fn symbolic_mode() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o644), "rw-r--r--");
        assert_eq!(mode_string(0o4755), "rwsr-xr-x");
        assert_eq!(mode_string(0o4644), "rwSr--r--");
        assert_eq!(mode_string(0o2750), "rwxr-s---");
        assert_eq!(mode_string(0o1777), "rwxrwxrwt");
        assert_eq!(mode_string(0o1776), "rwxrwxrwT");
    }

    #[test]
    fn mode_edit_tri_state() {
        let es = [entry("/a", Some(0o100644), Attributes::empty()), entry("/b", Some(0o100755), Attributes::empty())];
        let mut me = ModeEdit::new(&es).unwrap();
        assert_eq!(me.uniform(), None, "owner/group/other x differ");
        assert_eq!(me.octal, "");
        assert!(!me.changed());
        // Turn group write on for everything; the mixed execute bits stay per file.
        me.bits[7] = Tri::On;
        assert!(me.changed());
        assert_eq!(me.apply_to(0o100644), Some(0o100664));
        assert_eq!(me.apply_to(0o100755), Some(0o100775));
        let jobs = plan_jobs(&es, None, None, Some(&me));
        assert_eq!(jobs.len(), 2, "one job per resulting mode");
        assert!(jobs.iter().any(|j| j.mode == Some(0o100664) && j.paths == [PathBuf::from("/a")]));
        me.commit();
        assert!(!me.changed());
        assert_eq!(me.apply_to(0o644), None);
        assert!(me.set_octal("600"));
        assert!(!me.set_octal("9"));
        assert_eq!(me.uniform(), Some(0o600));
        assert!(ModeEdit::new(&[entry("/w", None, Attributes::empty())]).is_none());
    }

    #[test]
    fn time_fields() {
        let now = SystemTime::now();
        assert_eq!(parse_time(" now ", now), Some(now));
        let t = parse_time("2024-02-29 13:45:10", now).unwrap();
        assert_eq!(time_text(t), "2024-02-29 13:45:10");
        assert_eq!(parse_time("2024-02-29T13:45", now).map(time_text).as_deref(), Some("2024-02-29 13:45:00"));
        assert_eq!(parse_time("2024-02-29", now).map(time_text).as_deref(), Some("2024-02-29 00:00:00"));
        assert_eq!(parse_time("2024-02-30", now), None);
        assert_eq!(parse_time("yesterday", now), None);
    }

    #[test]
    fn attr_changes_and_plan() {
        let es = [entry("/a", None, Attributes::HIDDEN), entry("/b", None, Attributes::empty())];
        let mut ae = AttrEdit::new(&es);
        assert!(ae.changes().is_none());
        let first = ae.rows[0].0;
        for r in &mut ae.rows {
            if r.0 == first {
                r.1 = Tri::On;
            }
        }
        let c = ae.changes().unwrap();
        assert_eq!(c.set, first);
        assert!(c.clear.is_empty());
        let jobs = plan_jobs(&es, None, Some(c), None);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].paths.len(), 2);
        assert!(plan_jobs(&es, None, None, None).is_empty());
        let t = TimesSet { modified: Some(std::time::SystemTime::now()), ..TimesSet::default() };
        assert_eq!(plan_jobs(&es, Some(t), None, None)[0].times, Some(t));
        ae.commit();
        assert!(ae.changes().is_none());
    }
}
