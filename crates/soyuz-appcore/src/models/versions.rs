//! Previous versions (Phase T7, agent B), moved from the GUI's `ui/tools/versions.rs`:
//! the [`VersionsModel`] (enumeration on a worker, selection, compare), its labels and
//! empty-state texts, and the administrator-rights hint ([`elevate_hint`]) the terminal UI
//! shows instead of an elevated relaunch (owner decision: a message only, no restart
//! button). Both frontends draw the model: the GUI's `VersionsPanel` (Properties tab and
//! the "Restore previous version…" window) and the terminal UI's versions dialog. What the
//! user picks becomes a `models::tools::VersionCmd` for `models::tools::apply_version`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crossbeam_channel::{Receiver, bounded};
use soyuz_core::config::Config;
use soyuz_core::{DateFormat, SizeStyle, format_size, format_time};
use soyuz_fs::snapshots::{PathVersion, VersionOptions, VersionsReport};

use crate::host::Wake;
use crate::models::tools::VersionCmd;

/// Settings the model needs (`[sync] previous_versions`, `shadow_probe_max`, `[general]
/// date_format`, `size_style`).
#[derive(Clone, Debug)]
pub struct VersionsCfg {
    pub enabled: bool,
    pub probe_max: u32,
    pub date: DateFormat,
    pub size: SizeStyle,
}

impl Default for VersionsCfg {
    fn default() -> Self {
        VersionsCfg { enabled: true, probe_max: 64, date: DateFormat::default(), size: SizeStyle::Binary }
    }
}

impl VersionsCfg {
    pub fn from_config(c: &Config) -> VersionsCfg {
        VersionsCfg {
            enabled: c.sync.previous_versions,
            probe_max: c.sync.shadow_probe_max,
            date: c.general.date_format.clone(),
            size: c.general.size_style,
        }
    }

    /// Previous versions are on and this platform has a snapshot provider.
    pub fn enabled(&self) -> bool {
        supported() && self.enabled
    }
}

/// `soyuz_fs::snapshots::providers()` is non-empty on this platform (Windows: shadow
/// copies; Linux: snapper, ZFS and Timeshift snapshots; macOS: mounted Time Machine
/// backups). Whether the volume of a given file has snapshots is only known after the
/// (blocking) scan, which runs on a worker.
pub fn supported() -> bool {
    cfg!(any(windows, target_os = "linux", target_os = "macos"))
}

/// Linux wording (snapshots instead of shadow copies, no elevation).
const LINUX: bool = cfg!(target_os = "linux");

/// macOS wording (Time Machine backups, no elevation).
const MACOS: bool = cfg!(target_os = "macos");

/// The line above the list: where the versions come from.
pub fn intro_text() -> &'static str {
    if LINUX {
        tr!("tools-versions-intro-linux")
    } else if MACOS {
        tr!("tools-versions-intro-macos")
    } else {
        tr!("tools-versions-intro")
    }
}

/// Shown while the scan runs.
pub fn looking_text() -> &'static str {
    if LINUX {
        tr!("tools-versions-looking-linux")
    } else if MACOS {
        tr!("tools-versions-looking-macos")
    } else {
        tr!("tools-versions-looking")
    }
}

/// The warning above a non-empty list when `denied` more copies couldn't be read.
pub fn denied_more_text(denied: usize) -> String {
    if LINUX {
        tr!("tools-versions-denied-more-linux", count = denied)
    } else if MACOS {
        tr!("tools-versions-denied-more-macos", count = denied)
    } else {
        tr!("tools-versions-denied-more", count = denied)
    }
}

/// Title + body for an empty result, explaining why.
pub fn empty_text(r: &VersionsReport, is_dir: bool) -> (String, String) {
    let kind = if is_dir { "folder" } else { "file" };
    if LINUX {
        return if r.denied > 0 {
            (
                tr!("tools-versions-denied-linux").into(),
                tr!(
                    "tools-versions-denied-linux-body",
                    count = r.denied,
                    kind = kind,
                    user = std::env::var("USER").unwrap_or_else(|_| "<user>".into())
                ),
            )
        } else if r.snapshots_seen > 0 {
            (
                tr!("tools-versions-none").into(),
                tr!("tools-versions-none-body-linux", count = r.snapshots_seen, kind = kind),
            )
        } else {
            (tr!("tools-versions-no-copies-linux").into(), tr!("tools-versions-no-copies-body-linux").into())
        };
    }
    if MACOS {
        return if r.denied > 0 {
            (tr!("tools-versions-denied-macos").into(), tr!("tools-versions-denied-macos-body", count = r.denied))
        } else if r.snapshots_seen > 0 {
            (
                tr!("tools-versions-none").into(),
                tr!("tools-versions-none-body-macos", count = r.snapshots_seen, kind = kind),
            )
        } else {
            (tr!("tools-versions-no-copies-macos").into(), tr!("tools-versions-no-copies-body-macos").into())
        };
    }
    if r.denied > 0 {
        (
            tr!("tools-versions-admin-needed").into(),
            tr!("tools-versions-admin-needed-body", count = r.denied, kind = kind),
        )
    } else if r.snapshots_seen > 0 {
        (tr!("tools-versions-none").into(), tr!("tools-versions-none-body", count = r.snapshots_seen, kind = kind))
    } else {
        (tr!("tools-versions-no-copies").into(), tr!("tools-versions-no-copies-body").into())
    }
}

/// The terminal UI's note when copies were denied (`None` when none were): why, and how
/// to get them — a message only, the terminal UI never relaunches itself elevated. On
/// Linux and macOS it is the empty-state wording (snapper's `ALLOW_USERS`, Time Machine's
/// access); on Windows it says how to start the terminal UI as administrator.
pub fn elevate_hint(r: &VersionsReport, is_dir: bool) -> Option<(String, String)> {
    if r.denied == 0 {
        return None;
    }
    if LINUX || MACOS {
        let empty = VersionsReport { versions: Vec::new(), ..r.clone() };
        return Some(empty_text(&empty, is_dir));
    }
    let kind = if is_dir { "folder" } else { "file" };
    Some((tr!("tools-versions-admin-needed").into(), tr!("tui-versions-admin-hint", count = r.denied, kind = kind)))
}

/// "Shadow copy · 8 Oct 2026 14:00" (or the snapshot number when its time is unknown).
pub fn snapshot_label(v: &PathVersion, date: &DateFormat) -> String {
    match v.snapshot.created {
        Some(t) => tr!("tools-versions-snapshot-time", provider = provider_name(v), date = format_time(t, date)),
        None => tr!("tools-versions-snapshot-seq", provider = provider_name(v), seq = v.snapshot.seq.to_string()),
    }
}

/// The snapshot provider's display name (soyuz-fs names it in English).
pub fn provider_name(v: &PathVersion) -> &'static str {
    match v.snapshot.provider {
        "Shadow copy" => tr!("tools-versions-shadow-copy"),
        "ZFS snapshot" => tr!("tools-versions-zfs-snapshot"),
        other => other,
    }
}

/// "2.0 KiB · modified …" (a folder: only its modified time).
pub fn version_detail(v: &PathVersion, date: &DateFormat, size: SizeStyle) -> String {
    let sz = format_size(v.size, size);
    match (v.modified.map(|t| format_time(t, date)), v.is_dir) {
        (Some(modified), true) => tr!("tools-versions-detail-folder", modified = modified),
        (Some(modified), false) => tr!("tools-versions-detail-file", size = sz, modified = modified),
        (None, true) => tr!("tools-compare-meta-folder-only").to_string(),
        (None, false) => sz,
    }
}

/// Side-by-side facts of the current file and one version (computed on a worker).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comparison {
    pub same_content: Option<bool>,
    pub note: String,
}

impl Comparison {
    /// The verdict line: `None` while it runs.
    pub fn text(c: Option<&Comparison>) -> String {
        match c {
            None => tr!("tools-versions-comparing").to_string(),
            Some(Comparison { same_content: Some(true), .. }) => tr!("tools-versions-same").into(),
            Some(Comparison { same_content: Some(false), note }) => tr!("tools-versions-differ", note = note),
            Some(Comparison { note, .. }) => note.clone(),
        }
    }
}

/// Compare the current file `cur` with version `old`: size, then (up to 512 MB) BLAKE3.
/// Blocking: call it on a worker.
pub fn compare_files(cur: &Path, old: &Path) -> Comparison {
    const MAX: u64 = 512 * 1024 * 1024;
    let (Ok(a), Ok(b)) = (std::fs::metadata(cur), std::fs::metadata(old)) else {
        return Comparison { same_content: None, note: tr!("tools-versions-unreadable").into() };
    };
    if a.len() != b.len() {
        let d = a.len() as i64 - b.len() as i64;
        let note = tr!(
            "tools-versions-size-diff",
            size = format_size(d.unsigned_abs(), SizeStyle::Binary),
            dir = if d > 0 { "larger" } else { "smaller" }
        );
        return Comparison { same_content: Some(false), note };
    }
    if a.len() > MAX {
        return Comparison { same_content: None, note: tr!("tools-versions-too-large").into() };
    }
    let ctl = soyuz_core::OpControl::new();
    let h = |p: &Path| soyuz_fs::hash_file(p, soyuz_core::HashAlgo::Blake3, &ctl, &mut ()).ok();
    match (h(cur), h(old)) {
        (Some(x), Some(y)) => Comparison {
            same_content: Some(x == y),
            note: if x == y { String::new() } else { tr!("tools-versions-different-bytes").into() },
        },
        _ => Comparison { same_content: None, note: tr!("tools-versions-unreadable").into() },
    }
}

/// A compare of version `index`: its worker (until it answers) and the result.
#[derive(Debug)]
pub struct CompareState {
    pub index: usize,
    rx: Option<Receiver<Comparison>>,
    pub result: Option<Comparison>,
}

/// The previous versions of one file or folder: the scan, the selection and a compare.
#[derive(Debug)]
pub struct VersionsModel {
    pub path: PathBuf,
    pub is_dir: bool,
    /// The current size and modified time (the compare's "Current" side).
    pub current: Option<(u64, Option<SystemTime>)>,
    rx: Option<Receiver<Result<VersionsReport, String>>>,
    /// `None` while the scan runs.
    pub report: Option<Result<VersionsReport, String>>,
    pub selected: Option<usize>,
    pub compare: Option<CompareState>,
}

impl VersionsModel {
    /// A model for `path`, its scan started (or the "turned off" error when `cfg` says so).
    pub fn new(path: PathBuf, is_dir: bool, current: Option<(u64, Option<SystemTime>)>, cfg: &VersionsCfg) -> Self {
        let mut m = VersionsModel { path, is_dir, current, rx: None, report: None, selected: None, compare: None };
        m.refresh(cfg);
        m
    }

    /// A model showing `report` (tests, previews): no scan runs.
    pub fn with_report(path: PathBuf, is_dir: bool, report: Result<VersionsReport, String>) -> Self {
        VersionsModel { path, is_dir, current: None, rx: None, report: Some(report), selected: None, compare: None }
    }

    /// Look again: a new scan on a worker (it wakes the frontend when done).
    pub fn refresh(&mut self, cfg: &VersionsCfg) {
        if !cfg.enabled {
            self.report = Some(Err(tr!("tools-versions-off").into()));
            return;
        }
        let (tx, rx) = bounded(1);
        let path = self.path.clone();
        let wake: Wake = crate::models::tools::wake();
        let opts = VersionOptions { probe_max: cfg.probe_max };
        let spawned = std::thread::Builder::new().name("soyuz-versions".into()).spawn(move || {
            let r = soyuz_fs::snapshots::previous_versions(&path, &opts).map_err(|e| e.to_string());
            let _ = tx.send(r);
            wake();
        });
        match spawned {
            Ok(_) => {
                self.rx = Some(rx);
                self.report = None;
                self.selected = None;
                self.compare = None;
            }
            Err(e) => self.report = Some(Err(e.to_string())),
        }
    }

    /// The scan runs.
    pub fn looking(&self) -> bool {
        self.rx.is_some()
    }

    /// Take finished worker results; returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.rx
            && let Ok(r) = rx.try_recv()
        {
            self.report = Some(r);
            self.rx = None;
            changed = true;
        }
        if let Some(c) = &mut self.compare
            && let Some(Ok(r)) = c.rx.as_ref().map(|r| r.try_recv())
        {
            c.result = Some(r);
            c.rx = None;
            changed = true;
        }
        changed
    }

    /// The versions found (empty while looking or on an error).
    pub fn versions(&self) -> &[PathVersion] {
        match &self.report {
            Some(Ok(r)) => &r.versions,
            _ => &[],
        }
    }

    /// The selected version.
    pub fn selection(&self) -> Option<&PathVersion> {
        self.selected.and_then(|i| self.versions().get(i))
    }

    /// Select version `i` (ignored when out of range).
    pub fn select(&mut self, i: usize) {
        if i < self.versions().len() {
            self.selected = Some(i);
        }
    }

    /// Compare the selected version with the current file (files only), on a worker.
    pub fn start_compare(&mut self) {
        if self.is_dir {
            return;
        }
        let Some((i, old)) = self.selected.zip(self.selection().map(|v| v.path.clone())) else { return };
        let (tx, rx) = bounded(1);
        let cur = self.path.clone();
        let wake = crate::models::tools::wake();
        let _ = std::thread::Builder::new().name("soyuz-version-cmp".into()).spawn(move || {
            let _ = tx.send(compare_files(&cur, &old));
            wake();
        });
        self.compare = Some(CompareState { index: i, rx: Some(rx), result: None });
    }

    /// Open the selected version (a folder is browsed, a file opens from a temp copy).
    pub fn open_cmd(&self, date: &DateFormat) -> Option<VersionCmd> {
        let v = self.selection()?;
        Some(VersionCmd::Open { version: v.path.clone(), is_dir: v.is_dir, label: snapshot_label(v, date) })
    }

    /// Copy the selected version into a chosen folder (starting in the original's).
    pub fn copy_to_cmd(&self) -> Option<VersionCmd> {
        let v = self.selection()?;
        let start = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
        Some(VersionCmd::CopyTo { version: v.path.clone(), start })
    }

    /// Restore the selected version over the original (a Copy job: asks, undoable).
    pub fn restore_cmd(&self) -> Option<VersionCmd> {
        let v = self.selection()?;
        Some(VersionCmd::Restore { version: v.path.clone(), original: self.path.clone() })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use soyuz_fs::snapshots::{Snapshot, shadow_root};

    use super::*;

    fn version(seq: u32, created: Option<SystemTime>) -> PathVersion {
        let snap = Snapshot {
            provider: "Shadow copy",
            id: format!("HarddiskVolumeShadowCopy{seq}"),
            root: shadow_root(&format!("HarddiskVolumeShadowCopy{seq}")),
            volume: PathBuf::from(r"C:\"),
            created,
            seq,
        };
        PathVersion { snapshot: snap, path: format!("v{seq}").into(), size: 2048, modified: None, is_dir: false }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn empty_results_explain_why_linux() {
        let none = VersionsReport::default();
        assert_eq!(empty_text(&none, false).0, "No snapshots on this volume");
        let denied = VersionsReport { denied: 1, ..VersionsReport::default() };
        let (t, b) = empty_text(&denied, true);
        assert_eq!(t, "Snapshots aren't readable for your account");
        assert!(b.contains("a snapshot folder") && b.contains("ALLOW_USERS") && b.contains("folder"), "{b}");
        let seen = VersionsReport { snapshots_seen: 2, ..VersionsReport::default() };
        assert!(
            empty_text(&seen, false).1.contains("2 snapshots of this volume holds a different version of this file")
        );
        assert_eq!(elevate_hint(&denied, true), Some((t, b)), "the snapper wording");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn empty_results_explain_why_macos() {
        assert!(supported());
        let none = VersionsReport::default();
        assert_eq!(empty_text(&none, false).0, "No Time Machine backups found");
        let seen = VersionsReport { snapshots_seen: 2, ..VersionsReport::default() };
        assert!(empty_text(&seen, false).1.contains("2 Time Machine backups"));
        let denied = VersionsReport { denied: 2, ..VersionsReport::default() };
        assert_eq!(elevate_hint(&denied, false), Some(empty_text(&denied, false)));
    }

    #[test]
    #[cfg(windows)]
    fn empty_results_explain_why() {
        let none = VersionsReport::default();
        assert_eq!(empty_text(&none, false).0, "No shadow copies on this volume");
        let denied = VersionsReport { denied: 3, ..VersionsReport::default() };
        let (t, b) = empty_text(&denied, false);
        assert_eq!(t, "Administrator rights needed");
        assert!(b.contains("3 shadow copies") && b.contains("administrator"), "{b}");
        let seen = VersionsReport { snapshots_seen: 1, ..VersionsReport::default() };
        assert!(
            empty_text(&seen, true).1.contains("1 shadow copy of this volume holds a different version of this folder")
        );
        // The terminal UI's hint: how to start it elevated, never a relaunch.
        let (t, b) = elevate_hint(&denied, true).expect("a hint");
        assert_eq!(t, "Administrator rights needed");
        assert!(
            b.contains("3 shadow copies") && b.contains("Run as administrator") && b.contains("soyuz --tui"),
            "{b}"
        );
    }

    #[test]
    fn no_hint_without_denied_copies() {
        assert_eq!(elevate_hint(&VersionsReport::default(), false), None);
        let seen = VersionsReport { snapshots_seen: 4, ..VersionsReport::default() };
        assert_eq!(elevate_hint(&seen, true), None);
    }

    #[test]
    fn labels_use_time_or_number() {
        let v = version(7, None);
        assert_eq!(snapshot_label(&v, &DateFormat::default()), "Shadow copy #7");
        let v2 = version(7, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)));
        assert!(snapshot_label(&v2, &DateFormat::default()).starts_with("Shadow copy  ·  "));
        assert!(version_detail(&v2, &DateFormat::default(), SizeStyle::Binary).contains("KiB"));
    }

    #[test]
    fn compare_files_reports_same_and_different() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (d.path().join("a"), d.path().join("b"), d.path().join("c"));
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"hello").unwrap();
        std::fs::write(&c, b"hellO").unwrap();
        assert_eq!(compare_files(&a, &b).same_content, Some(true));
        assert_eq!(compare_files(&a, &c).same_content, Some(false));
        std::fs::write(&c, b"hello world").unwrap();
        let r = compare_files(&c, &a);
        assert_eq!(r.same_content, Some(false));
        assert!(r.note.contains("larger"), "{}", r.note);
        assert!(Comparison::text(Some(&r)).starts_with("Contents differ."));
        assert_eq!(Comparison::text(None), "Comparing contents…");
    }

    #[test]
    fn commands_need_a_selection() {
        let report = VersionsReport { versions: vec![version(1, None), version(2, None)], ..Default::default() };
        let original = PathBuf::from("dir").join("report.docx");
        let mut m = VersionsModel::with_report(original.clone(), false, Ok(report));
        assert!(m.restore_cmd().is_none() && m.open_cmd(&DateFormat::default()).is_none());
        m.select(5);
        assert_eq!(m.selected, None, "out of range");
        m.select(1);
        match m.restore_cmd() {
            Some(VersionCmd::Restore { version, original: o }) => {
                assert_eq!((version, o), (PathBuf::from("v2"), original.clone()));
            }
            other => panic!("{other:?}"),
        }
        match m.copy_to_cmd() {
            Some(VersionCmd::CopyTo { start, .. }) => assert_eq!(start, PathBuf::from("dir")),
            other => panic!("{other:?}"),
        }
        match m.open_cmd(&DateFormat::default()) {
            Some(VersionCmd::Open { is_dir: false, label, .. }) => assert_eq!(label, "Shadow copy #2"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn turned_off_needs_no_scan() {
        let cfg = VersionsCfg { enabled: false, ..VersionsCfg::default() };
        let m = VersionsModel::new(PathBuf::from("x"), false, None, &cfg);
        assert!(!m.looking());
        assert!(matches!(&m.report, Some(Err(e)) if e.contains("turned off")));
        assert!(!cfg.enabled());
    }
}
