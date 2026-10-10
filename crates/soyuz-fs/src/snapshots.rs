//! Snapshots and previous versions of files.
//!
//! A [`SnapshotProvider`] lists read-only point-in-time copies of a volume and maps a live
//! path into one of them. [`previous_versions`] asks every provider for the volume of a
//! path, stats the mapped paths and keeps the distinct versions (newest first).
//!
//! Providers:
//! * Windows — Volume Shadow Copies ([`VssProvider`]): the `HarddiskVolumeShadowCopyN`
//!   devices are found by listing the kernel object directory `\Device` (ntdll, no VSS
//!   COM and no shell), falling back to probing `N = 1..=probe_max`; each one is opened
//!   through `\\?\GLOBALROOT\Device\…\` and matched to its volume by serial number.
//!   Opening the devices needs administrator rights on a default installation; the
//!   report then says how many copies exist but could not be read ([`VersionsReport::denied`]).
//! * btrfs / ZFS snapshots and macOS Time Machine are deferred (Windows
//!   first). They plug in as further providers: btrfs `.snapshots/*/` subvolumes, ZFS
//!   `<mount>/.zfs/snapshot/*/`, Time Machine `/Volumes/<backup>/Backups.backupdb/…`.
//!
//! The mapping / filtering / de-duplication logic is pure and unit-tested with fake device
//! lists.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use soyuz_core::{FsError, FsResult};

/// Name prefix of shadow-copy devices in the kernel `\Device` directory.
pub const SHADOW_PREFIX: &str = "HarddiskVolumeShadowCopy";
/// Win32 path prefix of the kernel device namespace.
pub const GLOBALROOT: &str = r"\\?\GLOBALROOT\Device\";

/// One point-in-time copy of a volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Provider label shown in the UI, e.g. "Shadow copy".
    pub provider: &'static str,
    /// Provider-specific id (the device name for shadow copies).
    pub id: String,
    /// Root of the snapshot, ending with a separator (e.g.
    /// `\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3\`).
    pub root: PathBuf,
    /// Root of the live volume it was taken of (e.g. `C:\`).
    pub volume: PathBuf,
    /// When it was taken, when known.
    pub created: Option<SystemTime>,
    /// Ordering key when the time is unknown (higher = newer).
    pub seq: u32,
}

/// Snapshots of one volume plus how many exist but could not be opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotScan {
    pub snapshots: Vec<Snapshot>,
    /// Snapshot devices that exist but this process may not open (needs elevation).
    pub denied: usize,
}

pub trait SnapshotProvider: Send + Sync {
    /// Short label, e.g. "Shadow copy".
    fn name(&self) -> &'static str;
    /// Snapshots of the volume mounted at `volume_root`.
    fn snapshots(&self, volume_root: &Path) -> FsResult<SnapshotScan>;
    /// Where the live `path` (on `snap.volume`) lives inside `snap`.
    fn map(&self, snap: &Snapshot, path: &Path) -> Option<PathBuf> {
        map_into(path, &snap.volume, &snap.root)
    }
}

/// One earlier version of a path.
#[derive(Clone, Debug, PartialEq)]
pub struct PathVersion {
    pub snapshot: Snapshot,
    /// The path inside the snapshot (readable like any other path).
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub is_dir: bool,
}

/// Result of [`previous_versions`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VersionsReport {
    /// Distinct earlier versions, newest first.
    pub versions: Vec<PathVersion>,
    /// Snapshots of the volume that were looked at.
    pub snapshots_seen: usize,
    /// Snapshot devices that exist but could not be opened (not elevated).
    pub denied: usize,
    /// No provider exists for this platform / volume.
    pub unsupported: Option<String>,
}

/// Tunables for [`previous_versions`] (from `[sync]` in the config).
#[derive(Clone, Copy, Debug)]
pub struct VersionOptions {
    /// Shadow copies: probe `HarddiskVolumeShadowCopy1..=probe_max` when `\Device` cannot
    /// be listed.
    pub probe_max: u32,
}

impl Default for VersionOptions {
    fn default() -> Self {
        VersionOptions { probe_max: 64 }
    }
}

/// What opening one snapshot device found (pure mirror of the platform probe).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceProbe {
    Present { serial: u32, created: Option<SystemTime> },
    Denied,
    Missing,
}

// ---------------------------------------------------------------------------------------
// Pure helpers.

/// `N` of a `HarddiskVolumeShadowCopyN` device name.
pub fn shadow_index(name: &str) -> Option<u32> {
    name.strip_prefix(SHADOW_PREFIX)?.parse().ok()
}

/// Root path of a shadow copy device.
pub fn shadow_root(device: &str) -> PathBuf {
    PathBuf::from(format!("{GLOBALROOT}{device}\\"))
}

fn plain(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

/// Map `path` on the live volume `volume_root` into the snapshot rooted at `snap_root`.
/// Case-insensitive on the volume prefix (Windows volumes); `None` when `path` is not on
/// that volume.
pub fn map_into(path: &Path, volume_root: &Path, snap_root: &Path) -> Option<PathBuf> {
    let p = plain(path);
    let v = plain(volume_root);
    let v = v.trim_end_matches('\\');
    if v.is_empty() || p.len() < v.len() || !p.is_char_boundary(v.len()) {
        return None;
    }
    let (head, rest) = p.split_at(v.len());
    if !head.eq_ignore_ascii_case(v) || !(rest.is_empty() || rest.starts_with('\\')) {
        return None;
    }
    let rest = rest.trim_start_matches('\\');
    let mut root = snap_root.to_string_lossy().into_owned();
    if !root.ends_with('\\') && !root.ends_with('/') {
        root.push('\\');
    }
    Some(PathBuf::from(root + rest))
}

/// Snapshots of the volume with `serial` among probed shadow devices, plus the number of
/// devices that could not be opened. A creation time equal to the live volume's own
/// creation time (`live_created`) is not a snapshot time and is dropped.
pub fn select_for_volume(
    devices: &[(String, DeviceProbe)],
    volume_root: &Path,
    serial: u32,
    live_created: Option<SystemTime>,
) -> SnapshotScan {
    let mut scan = SnapshotScan::default();
    for (name, probe) in devices {
        let Some(seq) = shadow_index(name) else { continue };
        match *probe {
            DeviceProbe::Present { serial: s, created } if s == serial => {
                let created = created.filter(|c| Some(*c) != live_created);
                scan.snapshots.push(Snapshot {
                    provider: "Shadow copy",
                    id: name.clone(),
                    root: shadow_root(name),
                    volume: volume_root.to_path_buf(),
                    created,
                    seq,
                });
            }
            DeviceProbe::Present { .. } | DeviceProbe::Missing => {}
            DeviceProbe::Denied => scan.denied += 1,
        }
    }
    sort_newest_first(&mut scan.snapshots);
    scan
}

fn sort_newest_first(s: &mut [Snapshot]) {
    s.sort_by(|a, b| b.created.cmp(&a.created).then(b.seq.cmp(&a.seq)));
}

/// Newest first, keeping only versions that differ from the next newer one (or from the
/// current file, `current = (size, modified)`), like Explorer's "Previous versions" list.
pub fn distinct_versions(current: Option<(u64, Option<SystemTime>)>, mut v: Vec<PathVersion>) -> Vec<PathVersion> {
    v.sort_by(|a, b| b.snapshot.created.cmp(&a.snapshot.created).then(b.snapshot.seq.cmp(&a.snapshot.seq)));
    let mut out: Vec<PathVersion> = Vec::new();
    let mut newer = current;
    for x in v {
        let key = (x.size, x.modified);
        // Folders always differ in content we can't cheaply compare: keep them all.
        if !x.is_dir && newer == Some(key) {
            continue;
        }
        newer = Some(key);
        out.push(x);
    }
    out
}

// ---------------------------------------------------------------------------------------
// Providers.

/// Windows Volume Shadow Copies through the kernel device namespace.
pub struct VssProvider {
    pub probe_max: u32,
}

impl SnapshotProvider for VssProvider {
    fn name(&self) -> &'static str {
        "Shadow copy"
    }

    #[cfg(windows)]
    fn snapshots(&self, volume_root: &Path) -> FsResult<SnapshotScan> {
        use crate::sys::windows::shadow;
        let (serial, live_created) = shadow::volume_identity(volume_root)
            .map_err(|e| FsError::from_io("previous versions", Some(volume_root), &e))?;
        let names = match shadow::device_objects(SHADOW_PREFIX) {
            Ok(n) => n,
            Err(e) => {
                log::debug!("cannot list \\Device ({e}); probing shadow copies 1..={}", self.probe_max);
                (1..=self.probe_max).map(|n| format!("{SHADOW_PREFIX}{n}")).collect()
            }
        };
        let devices: Vec<(String, DeviceProbe)> = names
            .into_iter()
            .map(|n| {
                let p = match shadow::probe_device(&n) {
                    shadow::Probe::Present { serial, created } => DeviceProbe::Present { serial, created },
                    shadow::Probe::Denied => DeviceProbe::Denied,
                    shadow::Probe::Missing => DeviceProbe::Missing,
                };
                (n, p)
            })
            .collect();
        Ok(select_for_volume(&devices, volume_root, serial, live_created))
    }

    #[cfg(not(windows))]
    fn snapshots(&self, volume_root: &Path) -> FsResult<SnapshotScan> {
        Err(FsError::unsupported("previous versions", Some(volume_root)))
    }
}

/// Providers for this platform.
pub fn providers(opts: &VersionOptions) -> Vec<Box<dyn SnapshotProvider>> {
    if cfg!(windows) { vec![Box::new(VssProvider { probe_max: opts.probe_max.max(1) })] } else { Vec::new() }
}

/// Earlier versions of `path` in every snapshot of its volume. Blocking (opens every
/// snapshot device); run on a worker thread.
#[track_caller]
pub fn previous_versions(path: &Path, opts: &VersionOptions) -> FsResult<VersionsReport> {
    let _guard = crate::guard::check("previous_versions");
    let provs = providers(opts);
    if provs.is_empty() {
        return Ok(VersionsReport {
            unsupported: Some(
                "Snapshot browsing (btrfs, ZFS, Time Machine) isn't available on this platform yet.".into(),
            ),
            ..VersionsReport::default()
        });
    }
    let volume = crate::volumes::volume_root(path)?;
    let current = crate::list::stat(path, false).ok().map(|e| (e.size, e.modified));
    let mut report = VersionsReport::default();
    let mut found = Vec::new();
    let mut last_err = None;
    for prov in &provs {
        let scan = match prov.snapshots(&volume) {
            Ok(s) => s,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        report.denied += scan.denied;
        report.snapshots_seen += scan.snapshots.len();
        for snap in scan.snapshots {
            let Some(mapped) = prov.map(&snap, path) else { continue };
            if let Ok(e) = crate::list::stat(&mapped, false) {
                found.push(PathVersion {
                    snapshot: snap,
                    path: mapped,
                    size: e.size,
                    modified: e.modified,
                    is_dir: e.is_dir_like(),
                });
            }
        }
    }
    if report.snapshots_seen == 0
        && report.denied == 0
        && let Some(e) = last_err
    {
        return Err(e);
    }
    report.versions = distinct_versions(current, found);
    Ok(report)
}

/// Is `path` inside a snapshot (read-only)?
pub fn is_snapshot_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.len() > GLOBALROOT.len() && s[..GLOBALROOT.len()].eq_ignore_ascii_case(GLOBALROOT) && s.contains(SHADOW_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn shadow_names_and_roots() {
        assert_eq!(shadow_index("HarddiskVolumeShadowCopy12"), Some(12));
        assert_eq!(shadow_index("HarddiskVolume3"), None);
        assert_eq!(shadow_index("HarddiskVolumeShadowCopyX"), None);
        assert_eq!(
            shadow_root("HarddiskVolumeShadowCopy3"),
            PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3\")
        );
        assert!(is_snapshot_path(Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3\x.txt")));
        assert!(!is_snapshot_path(Path::new(r"C:\x.txt")));
    }

    #[test]
    fn maps_paths_into_snapshots() {
        let root = shadow_root("HarddiskVolumeShadowCopy2");
        let m = |p: &str, v: &str| map_into(Path::new(p), Path::new(v), &root);
        assert_eq!(
            m(r"C:\Users\me\doc.txt", r"C:\"),
            Some(PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy2\Users\me\doc.txt"))
        );
        // Verbatim prefixes, forward slashes and drive-letter case.
        assert_eq!(
            m(r"\\?\c:\Users/me", r"C:\"),
            Some(PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy2\Users\me"))
        );
        // The volume root itself.
        assert_eq!(m(r"C:\", r"C:\"), Some(root.clone()));
        // Mounted folder volume.
        assert_eq!(
            m(r"C:\mnt\data\a.bin", r"C:\mnt\data\"),
            Some(PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy2\a.bin"))
        );
        // Other volumes / sibling prefixes don't map.
        assert_eq!(m(r"D:\x", r"C:\"), None);
        assert_eq!(m(r"C:\mnt\database\x", r"C:\mnt\data"), None);
    }

    #[test]
    fn selects_snapshots_of_the_volume_from_a_fake_device_list() {
        let devices = vec![
            ("HarddiskVolumeShadowCopy1".to_string(), DeviceProbe::Present { serial: 0xAA, created: Some(t(100)) }),
            ("HarddiskVolumeShadowCopy2".to_string(), DeviceProbe::Present { serial: 0xBB, created: Some(t(200)) }),
            ("HarddiskVolumeShadowCopy3".to_string(), DeviceProbe::Present { serial: 0xAA, created: Some(t(300)) }),
            // Same creation time as the live volume: not a snapshot time.
            ("HarddiskVolumeShadowCopy4".to_string(), DeviceProbe::Present { serial: 0xAA, created: Some(t(5)) }),
            ("HarddiskVolumeShadowCopy5".to_string(), DeviceProbe::Denied),
            ("HarddiskVolumeShadowCopy6".to_string(), DeviceProbe::Missing),
            ("SomethingElse".to_string(), DeviceProbe::Denied),
        ];
        let scan = select_for_volume(&devices, Path::new(r"C:\"), 0xAA, Some(t(5)));
        assert_eq!(scan.denied, 1);
        let ids: Vec<&str> = scan.snapshots.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["HarddiskVolumeShadowCopy3", "HarddiskVolumeShadowCopy1", "HarddiskVolumeShadowCopy4"]);
        assert_eq!(scan.snapshots[2].created, None);
        assert_eq!(scan.snapshots[0].volume, PathBuf::from(r"C:\"));
        let mapped = VssProvider { probe_max: 4 }.map(&scan.snapshots[0], Path::new(r"C:\a\b.txt")).unwrap();
        assert_eq!(mapped, PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3\a\b.txt"));
    }

    #[test]
    fn keeps_only_distinct_versions_newest_first() {
        let snap = |n: u32, at: u64| Snapshot {
            provider: "Shadow copy",
            id: format!("{SHADOW_PREFIX}{n}"),
            root: shadow_root(&format!("{SHADOW_PREFIX}{n}")),
            volume: PathBuf::from(r"C:\"),
            created: Some(t(at)),
            seq: n,
        };
        let v = |n: u32, at: u64, size: u64, m: u64| PathVersion {
            snapshot: snap(n, at),
            path: PathBuf::from(format!("v{n}")),
            size,
            modified: Some(t(m)),
            is_dir: false,
        };
        // Current file = (30, t(3)); snapshots 4 and 3 hold the current content, 2 an older
        // one, 1 the same older one again.
        let all = vec![v(1, 100, 10, 1), v(2, 200, 10, 1), v(3, 300, 30, 3), v(4, 400, 30, 3)];
        let out = distinct_versions(Some((30, Some(t(3)))), all.clone());
        let ids: Vec<u32> = out.iter().map(|x| x.snapshot.seq).collect();
        assert_eq!(ids, [2], "unchanged copies collapse into the newest distinct one");
        // A deleted file (no current): every distinct version is listed.
        let out = distinct_versions(None, all);
        let ids: Vec<u32> = out.iter().map(|x| x.snapshot.seq).collect();
        assert_eq!(ids, [4, 2]);
    }

    #[cfg(windows)]
    #[test]
    fn kernel_device_directory_is_listable_without_elevation() {
        // Every Windows machine has `\Device\HarddiskVolumeN` objects.
        let names = crate::sys::windows::shadow::device_objects("HarddiskVolume").expect("list the Device directory");
        assert!(names.iter().any(|n| n.starts_with("HarddiskVolume")), "{names:?}");
        // A missing shadow device probes as missing (not denied).
        assert!(matches!(
            crate::sys::windows::shadow::probe_device("HarddiskVolumeShadowCopy999999"),
            crate::sys::windows::shadow::Probe::Missing
        ));
    }

    #[test]
    fn previous_versions_of_a_temp_file_reports_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        // No elevation / no shadow copies on CI: must not fail, just report nothing (or
        // the copies that exist when elevated).
        let r = previous_versions(&f, &VersionOptions { probe_max: 2 });
        if cfg!(windows) {
            let r = r.expect("report");
            assert!(r.unsupported.is_none());
            assert!(r.versions.iter().all(|v| v.path.to_string_lossy().ends_with("a.txt")));
            eprintln!("shadow copies: seen {} denied {}", r.snapshots_seen, r.denied);
        }
    }
}
