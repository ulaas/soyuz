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
//! * Linux — snapper ([`SnapperProvider`]: `<mount>/.snapshots/<n>/snapshot`, the time from
//!   `<n>/info.xml`), ZFS ([`ZfsProvider`]: `<mount>/.zfs/snapshot/<name>`) and Timeshift
//!   ([`TimeshiftProvider`]: `timeshift/snapshots/<date>/localhost` in rsync mode,
//!   `timeshift-btrfs/snapshots/<date>/@` and `@home` in btrfs mode, on any mounted block
//!   device). Unmounted btrfs read-only subvolumes outside these layouts are not found.
//!   Linux snapshot paths map byte-exact ([`map_under`]); the `.snapshots` folder of a
//!   snapper config is usually root-only, which is reported as [`VersionsReport::denied`].
//! * macOS Time Machine is deferred; it would plug in as a further provider
//!   (`/Volumes/<backup>/Backups.backupdb/…`).
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

/// Map `path` on the live mount `volume_root` into the snapshot rooted at `snap_root`, by
/// path components (unix: case-sensitive and byte-exact). `None` when `path` is not under
/// `volume_root`.
pub fn map_under(path: &Path, volume_root: &Path, snap_root: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(volume_root).ok()?;
    Some(if rest.as_os_str().is_empty() { snap_root.to_path_buf() } else { snap_root.join(rest) })
}

/// `<date>2024-03-05 14:07:09</date>` (UTC) of a snapper `info.xml`.
pub fn snapper_date(info_xml: &str) -> Option<SystemTime> {
    let start = info_xml.find("<date>")? + "<date>".len();
    let end = start + info_xml[start..].find("</date>")?;
    let t = chrono::NaiveDateTime::parse_from_str(info_xml[start..end].trim(), "%Y-%m-%d %H:%M:%S").ok()?;
    Some(SystemTime::from(t.and_utc()))
}

/// Timeshift snapshot folder name `2024-03-05_14-07-09` (local time).
pub fn timeshift_date(name: &str) -> Option<SystemTime> {
    use chrono::TimeZone;
    let t = chrono::NaiveDateTime::parse_from_str(name, "%Y-%m-%d_%H-%M-%S").ok()?;
    chrono::Local.from_local_datetime(&t).earliest().map(SystemTime::from)
}

/// Creation time encoded in common ZFS snapshot names (zfs-auto-snapshot
/// `…-2024-03-05-1407`, sanoid `autosnap_2024-03-05_14:07:09_…`, `…_2024-03-05_14-07-09`,
/// plain `2024-03-05`); UTC is assumed.
pub fn zfs_name_date(name: &str) -> Option<SystemTime> {
    let b = name.as_bytes();
    for i in 0..b.len().saturating_sub(9) {
        let Some(date) = name.get(i..i + 10) else { continue };
        let Ok(d) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else { continue };
        let rest = name.get(i + 10..).unwrap_or("");
        let rest = rest.strip_prefix(['-', '_', 'T']).unwrap_or("");
        let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).take(6).collect();
        let (h, m, sec) = match digits.len() {
            n if n >= 6 => (&digits[0..2], &digits[2..4], &digits[4..6]),
            n if n >= 4 => (&digits[0..2], &digits[2..4], "00"),
            _ => ("00", "00", "00"),
        };
        let t =
            d.and_hms_opt(h.parse().ok()?, m.parse().ok()?, sec.parse().ok()?).or_else(|| d.and_hms_opt(0, 0, 0))?;
        return Some(SystemTime::from(t.and_utc()));
    }
    None
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

/// Read the entries of a snapshot directory: `Ok(None)` when it does not exist,
/// `Err(())` when it exists but may not be read (root-only `.snapshots`).
fn list_dir(dir: &Path) -> Result<Option<Vec<std::fs::DirEntry>>, ()> {
    match std::fs::read_dir(dir) {
        Ok(rd) => Ok(Some(rd.flatten().collect())),
        // Only when the folder is known to exist: a lookup below an unreadable mount
        // (a 0700 EFI partition) is not a snapshot store.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && crate::list::exists_nofollow(dir) => Err(()),
        Err(_) => Ok(None),
    }
}

/// snapper: `<volume>/.snapshots/<n>/snapshot`, created at `<n>/info.xml`'s `<date>`.
pub struct SnapperProvider;

impl SnapshotProvider for SnapperProvider {
    fn name(&self) -> &'static str {
        "Snapper"
    }

    fn snapshots(&self, volume_root: &Path) -> FsResult<SnapshotScan> {
        let mut scan = SnapshotScan::default();
        let base = volume_root.join(".snapshots");
        let entries = match list_dir(&base) {
            Ok(Some(e)) => e,
            Ok(None) => return Ok(scan),
            Err(()) => {
                scan.denied = 1;
                return Ok(scan);
            }
        };
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(seq) = name.parse::<u32>() else { continue };
            let root = e.path().join("snapshot");
            if seq == 0 || !crate::list::exists_nofollow(&root) {
                continue;
            }
            let created = std::fs::read_to_string(e.path().join("info.xml")).ok().and_then(|x| snapper_date(&x));
            scan.snapshots.push(Snapshot {
                provider: "Snapper",
                id: name,
                root,
                volume: volume_root.to_path_buf(),
                created,
                seq,
            });
        }
        sort_newest_first(&mut scan.snapshots);
        Ok(scan)
    }

    fn map(&self, snap: &Snapshot, path: &Path) -> Option<PathBuf> {
        map_under(path, &snap.volume, &snap.root)
    }
}

/// ZFS: `<dataset mount>/.zfs/snapshot/<name>` (the `.zfs` folder is reachable even when
/// hidden).
pub struct ZfsProvider;

impl SnapshotProvider for ZfsProvider {
    fn name(&self) -> &'static str {
        "ZFS snapshot"
    }

    fn snapshots(&self, volume_root: &Path) -> FsResult<SnapshotScan> {
        let mut scan = SnapshotScan::default();
        let entries = match list_dir(&volume_root.join(".zfs").join("snapshot")) {
            Ok(Some(e)) => e,
            Ok(None) => return Ok(scan),
            Err(()) => {
                scan.denied = 1;
                return Ok(scan);
            }
        };
        let mut names: Vec<(String, PathBuf)> =
            entries.into_iter().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
        names.sort();
        for (i, (name, root)) in names.into_iter().enumerate() {
            let created = zfs_name_date(&name);
            scan.snapshots.push(Snapshot {
                provider: "ZFS snapshot",
                id: name,
                root,
                volume: volume_root.to_path_buf(),
                created,
                seq: u32::try_from(i).unwrap_or(u32::MAX),
            });
        }
        sort_newest_first(&mut scan.snapshots);
        Ok(scan)
    }

    fn map(&self, snap: &Snapshot, path: &Path) -> Option<PathBuf> {
        map_under(path, &snap.volume, &snap.root)
    }
}

/// Timeshift snapshots on any mounted block device: rsync mode
/// (`<mount>/timeshift/snapshots/<date>/localhost` = `/`) and btrfs mode
/// (`<mount>/timeshift-btrfs/snapshots/<date>/@` = `/`, `@home` = `/home`; visible when the
/// top-level subvolume is mounted).
pub struct TimeshiftProvider {
    /// Mount points to look in (block-device mounts of the mount table).
    pub mounts: Vec<PathBuf>,
}

impl TimeshiftProvider {
    /// Snapshots below the given mount points (pure filesystem walk; testable with fake
    /// layouts).
    pub fn scan(mounts: &[PathBuf]) -> SnapshotScan {
        let mut scan = SnapshotScan::default();
        let mut seq = 0u32;
        for m in mounts {
            for (base, subs) in [
                (m.join("timeshift").join("snapshots"), &[("localhost", "/")][..]),
                (m.join("timeshift-btrfs").join("snapshots"), &[("@", "/"), ("@home", "/home")][..]),
            ] {
                let entries = match list_dir(&base) {
                    Ok(Some(e)) => e,
                    Ok(None) => continue,
                    Err(()) => {
                        scan.denied += 1;
                        continue;
                    }
                };
                for e in entries {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let Some(created) = timeshift_date(&name) else { continue };
                    for (sub, live) in subs {
                        let root = e.path().join(sub);
                        if !crate::list::exists_nofollow(&root) {
                            continue;
                        }
                        seq += 1;
                        scan.snapshots.push(Snapshot {
                            provider: "Timeshift",
                            id: format!("{name}/{sub}"),
                            root,
                            volume: PathBuf::from(live),
                            created: Some(created),
                            seq,
                        });
                    }
                }
            }
        }
        sort_newest_first(&mut scan.snapshots);
        scan
    }
}

impl SnapshotProvider for TimeshiftProvider {
    fn name(&self) -> &'static str {
        "Timeshift"
    }

    fn snapshots(&self, _volume_root: &Path) -> FsResult<SnapshotScan> {
        Ok(Self::scan(&self.mounts))
    }

    fn map(&self, snap: &Snapshot, path: &Path) -> Option<PathBuf> {
        map_under(path, &snap.volume, &snap.root)
    }
}

/// Providers for this platform.
pub fn providers(opts: &VersionOptions) -> Vec<Box<dyn SnapshotProvider>> {
    #[cfg(windows)]
    {
        vec![Box::new(VssProvider { probe_max: opts.probe_max.max(1) })]
    }
    #[cfg(target_os = "linux")]
    {
        let _ = opts;
        vec![
            Box::new(SnapperProvider),
            Box::new(ZfsProvider),
            Box::new(TimeshiftProvider { mounts: crate::volumes::block_device_mounts() }),
        ]
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = opts;
        Vec::new()
    }
}

/// Earlier versions of `path` in every snapshot of its volume. Blocking (opens every
/// snapshot device); run on a worker thread.
#[track_caller]
pub fn previous_versions(path: &Path, opts: &VersionOptions) -> FsResult<VersionsReport> {
    let _guard = crate::guard::check("previous_versions");
    let provs = providers(opts);
    if provs.is_empty() {
        return Ok(VersionsReport {
            unsupported: Some("Snapshot browsing (Time Machine) isn't available on this platform yet.".into()),
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

/// Is `path` inside a snapshot (read-only)? Shadow copies everywhere; snapper, ZFS and
/// Timeshift layouts on unix.
pub fn is_snapshot_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.len() > GLOBALROOT.len() && s[..GLOBALROOT.len()].eq_ignore_ascii_case(GLOBALROOT) && s.contains(SHADOW_PREFIX)
    {
        return true;
    }
    if cfg!(windows) {
        return false;
    }
    let parts: Vec<std::borrow::Cow<'_, str>> = path.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    parts.windows(3).any(|w| {
        (w[0] == ".snapshots" && w[1].parse::<u32>().is_ok() && w[2] == "snapshot")
            || ((w[0] == "timeshift" || w[0] == "timeshift-btrfs")
                && w[1] == "snapshots"
                && timeshift_date(&w[2]).is_some())
    }) || parts.windows(2).any(|w| w[0] == ".zfs" && w[1] == "snapshot")
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
    fn unix_mapping_dates_and_markers() {
        assert_eq!(
            map_under(Path::new("/home/u/a.txt"), Path::new("/home"), Path::new("/home/.snapshots/7/snapshot")),
            Some(PathBuf::from("/home/.snapshots/7/snapshot/u/a.txt"))
        );
        assert_eq!(map_under(Path::new("/homework/x"), Path::new("/home"), Path::new("/s")), None);
        assert_eq!(map_under(Path::new("/home"), Path::new("/home"), Path::new("/s")), Some(PathBuf::from("/s")));
        let xml =
            "<?xml version=\"1.0\"?>\n<snapshot>\n  <num>42</num>\n  <date>2024-03-05 14:07:09</date>\n</snapshot>\n";
        assert_eq!(snapper_date(xml), Some(t(1_709_647_629)));
        assert_eq!(snapper_date("<snapshot/>"), None);
        assert!(timeshift_date("2024-03-05_14-07-09").is_some());
        assert!(timeshift_date("latest").is_none());
        assert_eq!(zfs_name_date("zfs-auto-snap_hourly-2024-03-05-1407"), Some(t(1_709_647_620)));
        assert_eq!(zfs_name_date("autosnap_2024-03-05_14:07:09_daily"), Some(t(1_709_647_629)));
        assert_eq!(zfs_name_date("backup-2024-03-05"), Some(t(1_709_596_800)));
        assert_eq!(zfs_name_date("manual"), None);
        if !cfg!(windows) {
            assert!(is_snapshot_path(Path::new("/home/.snapshots/12/snapshot/u/a.txt")));
            assert!(is_snapshot_path(Path::new("/tank/data/.zfs/snapshot/daily/x")));
            assert!(is_snapshot_path(Path::new("/mnt/b/timeshift/snapshots/2024-03-05_14-07-09/localhost/etc")));
            assert!(!is_snapshot_path(Path::new("/home/u/.snapshots/notes.txt")));
        }
    }

    #[cfg(unix)]
    #[test]
    fn snapper_zfs_and_timeshift_fake_layouts() {
        let d = tempfile::tempdir().unwrap();
        let vol = d.path().join("vol");
        let live = vol.join("docs").join("a.txt");
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        std::fs::write(&live, b"current").unwrap();
        // snapper: 1 (old content), 2 (same as current), 0 and junk are ignored.
        for (n, date, content) in [(1, "2024-01-01 10:00:00", "old"), (2, "2024-02-01 10:00:00", "current")] {
            let s = vol.join(".snapshots").join(n.to_string());
            std::fs::create_dir_all(s.join("snapshot").join("docs")).unwrap();
            std::fs::write(s.join("info.xml"), format!("<snapshot><date>{date}</date></snapshot>")).unwrap();
            std::fs::write(s.join("snapshot").join("docs").join("a.txt"), content).unwrap();
        }
        std::fs::create_dir_all(vol.join(".snapshots").join("0").join("snapshot")).unwrap();
        std::fs::create_dir_all(vol.join(".snapshots").join("junk")).unwrap();
        let scan = SnapperProvider.snapshots(&vol).unwrap();
        let ids: Vec<&str> = scan.snapshots.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["2", "1"]);
        let mapped = SnapperProvider.map(&scan.snapshots[1], &live).unwrap();
        assert_eq!(std::fs::read(&mapped).unwrap(), b"old");

        // ZFS.
        let z = vol.join(".zfs").join("snapshot").join("zfs-auto-snap_daily-2024-03-01-0000");
        std::fs::create_dir_all(z.join("docs")).unwrap();
        std::fs::write(z.join("docs").join("a.txt"), b"zfs").unwrap();
        let scan = ZfsProvider.snapshots(&vol).unwrap();
        assert_eq!(scan.snapshots.len(), 1);
        assert!(scan.snapshots[0].created.is_some());
        assert_eq!(std::fs::read(ZfsProvider.map(&scan.snapshots[0], &live).unwrap()).unwrap(), b"zfs");

        // Timeshift (rsync and btrfs layouts) on another "mount".
        let backup = d.path().join("backup");
        let rs = backup.join("timeshift").join("snapshots").join("2024-03-05_14-07-09").join("localhost");
        std::fs::create_dir_all(&rs).unwrap();
        let bt = backup.join("timeshift-btrfs").join("snapshots").join("2024-03-06_08-00-00");
        std::fs::create_dir_all(bt.join("@")).unwrap();
        std::fs::create_dir_all(bt.join("@home")).unwrap();
        std::fs::create_dir_all(backup.join("timeshift").join("snapshots").join("not-a-date")).unwrap();
        let scan = TimeshiftProvider::scan(std::slice::from_ref(&backup));
        let ids: Vec<&str> = scan.snapshots.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "{ids:?}");
        assert!(ids[0].starts_with("2024-03-06_08-00-00/"), "newest first: {ids:?}");
        let home = scan.snapshots.iter().find(|s| s.id.ends_with("@home")).unwrap();
        assert_eq!(home.volume, PathBuf::from("/home"));
        let p = TimeshiftProvider { mounts: vec![] }.map(home, Path::new("/home/u/x")).unwrap();
        assert_eq!(p, bt.join("@home").join("u").join("x"));

        // The full pipeline over the snapper + ZFS providers: distinct versions only.
        let mut found = Vec::new();
        for prov in [&SnapperProvider as &dyn SnapshotProvider, &ZfsProvider] {
            for snap in prov.snapshots(&vol).unwrap().snapshots {
                let m = prov.map(&snap, &live).unwrap();
                let e = crate::list::stat(&m, false).unwrap();
                found.push(PathVersion { snapshot: snap, path: m, size: e.size, modified: e.modified, is_dir: false });
            }
        }
        assert_eq!(found.len(), 3);
        let cur = crate::list::stat(&live, false).map(|e| (e.size, e.modified)).ok();
        assert!(!distinct_versions(cur, found).is_empty());
    }

    #[test]
    fn previous_versions_of_a_temp_file_reports_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        // No elevation / no shadow copies on CI: must not fail, just report nothing (or
        // the copies that exist when elevated).
        let r = previous_versions(&f, &VersionOptions { probe_max: 2 });
        if cfg!(any(windows, target_os = "linux")) {
            let r = r.expect("report");
            assert!(r.unsupported.is_none());
            assert!(r.versions.iter().all(|v| v.path.to_string_lossy().ends_with("a.txt")));
            eprintln!("shadow copies: seen {} denied {}", r.snapshots_seen, r.denied);
        }
    }
}
