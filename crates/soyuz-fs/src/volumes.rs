//! Volumes, free space and the injectable [`FsProbe`].
//!
//! * Windows: `GetLogicalDrives` → `GetDriveTypeW` → `GetVolumeInformationW` →
//!   `GetDiskFreeSpaceExW`, plus mounted folders from `FindFirstVolumeW` /
//!   `GetVolumePathNamesForVolumeNameW`.
//! * Linux: `/proc/self/mountinfo` (pseudo filesystems filtered) + `statvfs`, labels from
//!   `/dev/disk/by-label`, removable media from sysfs (`/sys/class/block/<dev>`:
//!   `removable`, or a USB ancestor), GNOME gvfs mounts (`/run/user/$UID/gvfs/*`) as
//!   network volumes. One device shows up once: btrfs subvolumes and bind mounts of the
//!   same filesystem (same `major:minor`) collapse into the mount of the whole filesystem
//!   (mountinfo root `/`) or else the shortest mount path (`/` rather than `/home` for the
//!   usual Fedora / openSUSE btrfs layout; a separate `/home` partition is its own
//!   device and stays listed). tmpfs is listed (as `Ram`) only where users put files:
//!   `/tmp`, `/var/tmp` and the usual user mount places; `/run`, `/dev/shm` and friends
//!   stay hidden. [`volume_root`] is the longest mountinfo mount point containing the
//!   path (so a btrfs subvolume mount such as `/home` is its own root).
//! * macOS: `getfsstat(MNT_NOWAIT)`.
//!
//! Per-volume queries run on short-lived threads with a deadline so a dead network share
//! cannot stall the caller; such volumes are listed without sizes.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use soyuz_core::{DeviceId, FsError, FsErrorKind, FsResult};

use crate::trash::TrashSupport;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Ram,
    Unknown,
}

bitflags! {
    /// Filesystem capabilities.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct FsCaps: u16 {
        const SPARSE = 1;
        const REFLINK = 2;
        const HARDLINK = 4;
        const SYMLINK = 8;
        const ADS = 16;
        const CASE_SENSITIVE = 32;
        const COMPRESSION = 64;
        const TRASH = 128;
    }
}

#[derive(Clone, Debug)]
pub struct VolumeInfo {
    pub mount: PathBuf,
    /// The volume's own label. On Windows this is the raw OS label and is empty when the
    /// volume has none; the app then builds a localized name from [`kind`](Self::kind)
    /// and the mount (`Local Disk (C:)` / `Yerel Disk (C:)`), so this crate stays
    /// locale-agnostic.
    pub label: String,
    /// Filesystem name (`"NTFS"`, `"ext4"`, ...); empty when unknown.
    pub fs: String,
    pub kind: VolumeKind,
    /// Bytes; 0 when unknown (e.g. an unreachable network share).
    pub total: u64,
    pub free: u64,
    pub read_only: bool,
    pub device: DeviceId,
    pub supports: FsCaps,
}

/// Total time budget for per-volume queries in [`volumes`].
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) const QUERY_DEADLINE: Duration = Duration::from_millis(2500);

/// Mounted volumes (drives and mounted folders on Windows), sorted by mount path.
#[track_caller]
pub fn volumes() -> Vec<VolumeInfo> {
    let _guard = crate::guard::check("volumes");
    let mut v = imp::volumes();
    v.sort_by(|a, b| a.mount.cmp(&b.mount));
    v.dedup_by(|a, b| a.mount == b.mount);
    v
}

/// `(total, free)` bytes of the volume containing `path` (free = available to this user).
/// Non-existing paths use their nearest existing ancestor.
#[track_caller]
pub fn space(path: &Path) -> FsResult<(u64, u64)> {
    let _guard = crate::guard::check("space");
    let p = existing_ancestor(path)?;
    imp::space(&p)
}

/// Mount point / drive root containing `path`.
#[track_caller]
pub fn volume_root(path: &Path) -> FsResult<PathBuf> {
    let _guard = crate::guard::check("volume_root");
    let p = existing_ancestor(path)?;
    imp::volume_root(&p)
}

fn existing_ancestor(path: &Path) -> FsResult<PathBuf> {
    let abs = crate::path::normalize(path);
    let mut cur: &Path = &abs;
    loop {
        if crate::list::exists_nofollow(cur) {
            return Ok(cur.to_path_buf());
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return Err(FsError::new(FsErrorKind::NotFound, "volume", Some(path))),
        }
    }
}

/// Run `jobs` on worker threads and collect the results that finish before `deadline`
/// (`None` for the rest; their threads are left to finish on their own).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) fn parallel_with_deadline<T: Send + 'static>(
    jobs: Vec<Box<dyn FnOnce() -> T + Send>>,
    deadline: Duration,
) -> Vec<Option<T>> {
    let n = jobs.len();
    let (tx, rx) = mpsc::channel();
    for (i, job) in jobs.into_iter().enumerate() {
        let tx = tx.clone();
        let spawned = std::thread::Builder::new().name("soyuz-volume".into()).spawn(move || {
            let _ = tx.send((i, job()));
        });
        if spawned.is_err() {
            log::warn!("could not spawn a volume query thread");
        }
    }
    drop(tx);
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    let end = Instant::now() + deadline;
    let mut got = 0;
    while got < n {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((i, v)) => {
                out[i] = Some(v);
                got += 1;
            }
            Err(_) => break,
        }
    }
    out
}

/// Injectable probe so jobs can fake free space / device ids in tests.
pub trait FsProbe: Send + Sync {
    fn space(&self, p: &Path) -> FsResult<(u64, u64)>;
    fn device_of(&self, p: &Path) -> FsResult<DeviceId>;
    fn trash_support(&self, p: &Path) -> TrashSupport;
}

/// The real filesystem.
#[derive(Clone, Copy, Debug, Default)]
pub struct RealProbe;

impl FsProbe for RealProbe {
    fn space(&self, p: &Path) -> FsResult<(u64, u64)> {
        space(p)
    }

    fn device_of(&self, p: &Path) -> FsResult<DeviceId> {
        crate::copy::device_of(p)
    }

    fn trash_support(&self, p: &Path) -> TrashSupport {
        crate::trash::trash_support(p)
    }
}

// ---------------------------------------------------------------------------
// /proc/self/mountinfo parsing (pure; compiled everywhere for tests)
// ---------------------------------------------------------------------------

/// One line of `/proc/self/mountinfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) struct MountInfo {
    pub major: u32,
    pub minor: u32,
    /// Directory of the filesystem mounted here (`/` for a whole filesystem; the subvolume
    /// or source directory for btrfs subvolumes and bind mounts).
    pub root: String,
    pub mount: PathBuf,
    pub options: String,
    pub fstype: String,
    pub source: String,
}

/// `\040` style octal escapes used by the kernel for spaces, tabs, newlines, backslashes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn unescape_octal(s: &str) -> String {
    String::from_utf8_lossy(&unescape_octal_bytes(s)).into_owned()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unescape_octal_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = |d: &[u8]| -> Option<u32> {
            d.iter().try_fold(0u32, |v, c| (b'0'..=b'7').contains(c).then(|| v * 8 + u32::from(c - b'0')))
        };
        // `\400`..`\777` do not fit a byte: kept literally.
        if b[i] == b'\\'
            && let Some(v) = b.get(i + 1..i + 4).and_then(octal).and_then(|v| u8::try_from(v).ok())
        {
            out.push(v);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// A mount path from mountinfo, byte-exact (non-UTF-8 mount points stay intact on unix).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn path_unescaped(s: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(unescape_octal_bytes(s)))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(unescape_octal(s))
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_mountinfo(text: &str) -> Vec<MountInfo> {
    let mut out = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(sep) = fields.iter().position(|f| *f == "-") else { continue };
        if sep < 6 || fields.len() < sep + 3 {
            continue;
        }
        let Some((maj, min)) = fields[2].split_once(':') else { continue };
        let (Ok(major), Ok(minor)) = (maj.parse(), min.parse()) else { continue };
        out.push(MountInfo {
            major,
            minor,
            root: unescape_octal(fields[3]),
            mount: path_unescaped(fields[4]),
            options: fields[5].to_string(),
            fstype: fields[sep + 1].to_string(),
            source: unescape_octal(fields[sep + 2]),
        });
    }
    out
}

/// Filesystems that are never user-visible volumes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn is_pseudo_mount(m: &MountInfo) -> bool {
    const PSEUDO: &[&str] = &[
        "proc",
        "sysfs",
        "cgroup",
        "cgroup2",
        "devpts",
        "devtmpfs",
        "securityfs",
        "debugfs",
        "tracefs",
        "pstore",
        "bpf",
        "mqueue",
        "hugetlbfs",
        "configfs",
        "fusectl",
        "autofs",
        "binfmt_misc",
        "efivarfs",
        "nsfs",
        "ramfs",
        "rpc_pipefs",
        "selinuxfs",
        "squashfs",
        "fuse.gvfsd-fuse",
        "fuse.portal",
        "fuse.snapfuse",
        "nfsd",
        "overlay",
        "zram",
    ];
    let mount = m.mount.to_string_lossy();
    if m.mount == Path::new("/") {
        return false;
    }
    if PSEUDO.contains(&m.fstype.as_str()) {
        return true;
    }
    if m.fstype == "tmpfs" || m.fstype == "ramfs" {
        // RAM filesystems only where users keep files.
        let user_place = ["/tmp", "/var/tmp"].contains(&mount.as_ref())
            || ["/mnt/", "/media/", "/run/media/", "/home/"].iter().any(|p| mount.starts_with(p));
        return !user_place;
    }
    ["/proc", "/sys", "/dev", "/run", "/snap", "/var/lib/docker", "/var/snap", "/boot/efi"]
        .iter()
        .any(|p| mount == *p || mount.starts_with(&format!("{p}/")))
        && !mount.starts_with("/run/media/")
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn kind_for(fstype: &str, source: &str, mount: &Path) -> VolumeKind {
    let m = mount.to_string_lossy();
    match fstype {
        "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "fuse.sshfs" | "sshfs" | "9p" | "afs" | "ceph" | "glusterfs"
        | "davfs" | "fuse.rclone" => VolumeKind::Network,
        "iso9660" | "udf" => VolumeKind::Optical,
        "tmpfs" | "ramfs" => VolumeKind::Ram,
        _ if source.starts_with("//") => VolumeKind::Network,
        // udisks mounts hot-plugged media here; /mnt is for static mounts (sysfs decides).
        _ if m.starts_with("/media/") || m.starts_with("/run/media/") => VolumeKind::Removable,
        _ => VolumeKind::Fixed,
    }
}

/// The mount containing `path` (absolute, symlinks resolved): longest mount point that is a
/// path prefix; among stacked mounts on one point the last (topmost) one wins.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn mount_for<'a>(path: &Path, mounts: &'a [MountInfo]) -> Option<&'a MountInfo> {
    let mut best: Option<&MountInfo> = None;
    for m in mounts {
        if path.starts_with(&m.mount) && best.is_none_or(|b| m.mount.as_os_str().len() >= b.mount.as_os_str().len()) {
            best = Some(m);
        }
    }
    best
}

/// Inside Flatpak: drop the sandbox's own plumbing (its tmpfs `/`, the runtime at `/usr`,
/// the app at `/app`, the `/etc` files, Flatpak's private folders under home) and, for
/// the filesystem holding `home`, every mount but the one `home` is on (the host's
/// `/mnt`, `/opt`, `/srv` show up as bind mounts of it; without this [`dedupe_mounts`]
/// would name the system disk after one of them, or after the runtime's `/usr`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn flatpak_visible(mounts: Vec<MountInfo>, home: &Path) -> Vec<MountInfo> {
    const SANDBOX: &[&str] = &["/usr", "/app", "/etc", "/run", "/.hcwd", "/.profile", "/afs"];
    let plumbing = |m: &MountInfo| {
        m.mount == Path::new("/")
            || SANDBOX.iter().any(|p| m.mount.starts_with(p))
            || m.mount.starts_with(home.join(".local/share/flatpak"))
            || m.mount.starts_with(home.join(".var/app"))
    };
    let mounts: Vec<MountInfo> = mounts.into_iter().filter(|m| !plumbing(m)).collect();
    let home_mount = mounts
        .iter()
        .filter(|m| home.starts_with(&m.mount))
        .max_by_key(|m| m.mount.components().count())
        .map(|m| (m.major, m.minor, m.fstype.clone(), m.mount.clone()));
    match home_mount {
        Some((major, minor, fstype, at)) => mounts
            .into_iter()
            .filter(|m| !(m.major == major && m.minor == minor && m.fstype == fstype) || m.mount == at)
            .collect(),
        None => mounts,
    }
}

/// One entry per filesystem: mounts sharing `major:minor` (btrfs subvolumes, bind mounts)
/// collapse into the mount of the filesystem root (`root == "/"`), else the one with the
/// shortest mount path. Order is kept.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn dedupe_mounts(mounts: Vec<MountInfo>) -> Vec<MountInfo> {
    let rank = |m: &MountInfo| (m.root != "/", m.mount.components().count(), m.mount.as_os_str().len());
    let mut keep: Vec<MountInfo> = Vec::with_capacity(mounts.len());
    for m in mounts {
        // Network filesystems share anonymous device numbers per superblock: each export /
        // share stays separate when the sources differ.
        match keep.iter_mut().find(|k| k.major == m.major && k.minor == m.minor && k.fstype == m.fstype) {
            Some(k) if rank(&m) < rank(k) => *k = m,
            Some(_) => {}
            None => keep.push(m),
        }
    }
    keep
}

/// Label and kind of a gvfs FUSE mount directory name (`smb-share:server=nas,share=media`,
/// `sftp:host=example.org,user=me`, `mtp:host=Phone_1234`, ...).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn gvfs_label(name: &str) -> (String, VolumeKind) {
    let (scheme, rest) = name.split_once(':').unwrap_or((name, ""));
    let mut kv = std::collections::HashMap::new();
    for part in rest.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            kv.insert(k, percent_decode_lossy(v));
        }
    }
    let host = kv.get("host").or_else(|| kv.get("server")).cloned().unwrap_or_default();
    let kind = match scheme {
        "mtp" | "gphoto2" | "afc" => VolumeKind::Removable,
        _ => VolumeKind::Network,
    };
    let label = match scheme {
        "smb-share" => match kv.get("share") {
            Some(share) => format!("{share} on {host}"),
            None => host.clone(),
        },
        "afp-volume" => match kv.get("volume") {
            Some(v) => format!("{v} on {host}"),
            None => host.clone(),
        },
        "mtp" | "gphoto2" | "afc" => host.replace('_', " "),
        "sftp" | "ftp" | "ftps" | "dav" | "davs" | "google-drive" | "onedrive" | "nfs" => {
            let user = kv.get("user").map(|u| format!("{u}@")).unwrap_or_default();
            let what = match scheme {
                "sftp" => "SFTP",
                "ftp" | "ftps" => "FTP",
                "dav" | "davs" => "WebDAV",
                "google-drive" => "Google Drive",
                "onedrive" => "OneDrive",
                _ => "NFS",
            };
            format!("{user}{host} ({what})")
        }
        _ if !host.is_empty() => host.clone(),
        _ => name.to_string(),
    };
    (label, kind)
}

/// `%20`-style escapes used in gvfs mount names.
fn percent_decode_lossy(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn caps_for(fstype: &str) -> FsCaps {
    match fstype {
        "btrfs" | "xfs" | "bcachefs" => {
            FsCaps::REFLINK | FsCaps::SPARSE | FsCaps::HARDLINK | FsCaps::SYMLINK | FsCaps::CASE_SENSITIVE
        }
        "ext2" | "ext3" | "ext4" | "f2fs" | "zfs" | "jfs" | "reiserfs" | "nilfs2" => {
            FsCaps::SPARSE | FsCaps::HARDLINK | FsCaps::SYMLINK | FsCaps::CASE_SENSITIVE
        }
        "apfs" => FsCaps::REFLINK | FsCaps::SPARSE | FsCaps::HARDLINK | FsCaps::SYMLINK,
        "hfs" => FsCaps::HARDLINK | FsCaps::SYMLINK,
        "ntfs" | "ntfs3" | "fuseblk" => FsCaps::SPARSE | FsCaps::HARDLINK | FsCaps::SYMLINK | FsCaps::ADS,
        "nfs" | "nfs4" => FsCaps::HARDLINK | FsCaps::SYMLINK | FsCaps::CASE_SENSITIVE,
        // /tmp on Fedora / Arch / systemd: links and sparse files work like on ext4.
        "tmpfs" => FsCaps::SPARSE | FsCaps::HARDLINK | FsCaps::SYMLINK | FsCaps::CASE_SENSITIVE,
        "ramfs" => FsCaps::HARDLINK | FsCaps::SYMLINK | FsCaps::CASE_SENSITIVE,
        _ => FsCaps::empty(),
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{DeviceId, FsResult};

    use super::{FsCaps, QUERY_DEADLINE, VolumeInfo, VolumeKind, parallel_with_deadline};
    use crate::err;
    use crate::sys::windows::volume::{self as wv, VolInfo};

    const FILE_FILE_COMPRESSION: u32 = 0x10;
    const FILE_SUPPORTS_SPARSE_FILES: u32 = 0x40;
    const FILE_SUPPORTS_REPARSE_POINTS: u32 = 0x80;
    const FILE_NAMED_STREAMS: u32 = 0x0004_0000;
    const FILE_READ_ONLY_VOLUME: u32 = 0x0008_0000;
    const FILE_SUPPORTS_HARD_LINKS: u32 = 0x0040_0000;
    const FILE_SUPPORTS_BLOCK_REFCOUNTING: u32 = 0x0800_0000;

    fn kind_of(drive_type: u32) -> VolumeKind {
        match drive_type {
            2 => VolumeKind::Removable,
            3 => VolumeKind::Fixed,
            4 => VolumeKind::Network,
            5 => VolumeKind::Optical,
            6 => VolumeKind::Ram,
            _ => VolumeKind::Unknown,
        }
    }

    pub(super) fn caps(flags: u32, fs: &str) -> FsCaps {
        let mut c = FsCaps::empty();
        for (bit, cap) in [
            (FILE_SUPPORTS_SPARSE_FILES, FsCaps::SPARSE),
            (FILE_SUPPORTS_BLOCK_REFCOUNTING, FsCaps::REFLINK),
            (FILE_SUPPORTS_HARD_LINKS, FsCaps::HARDLINK),
            (FILE_SUPPORTS_REPARSE_POINTS, FsCaps::SYMLINK),
            (FILE_NAMED_STREAMS, FsCaps::ADS),
            (FILE_FILE_COMPRESSION, FsCaps::COMPRESSION),
        ] {
            if flags & bit != 0 {
                c |= cap;
            }
        }
        let fs = fs.to_ascii_uppercase();
        if fs == "NTFS" || fs == "REFS" {
            c |= FsCaps::TRASH;
        }
        c
    }

    struct Probe {
        drive_type: u32,
        info: Option<VolInfo>,
        space: Option<(u64, u64)>,
    }

    pub(super) fn volumes() -> Vec<VolumeInfo> {
        let mask = wv::logical_drives();
        let mut roots: Vec<PathBuf> = (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char)))
            .collect();
        for p in wv::volume_mount_paths() {
            if p.as_os_str().len() > 3 && !roots.contains(&p) {
                roots.push(p);
            }
        }
        let jobs: Vec<Box<dyn FnOnce() -> Probe + Send>> = roots
            .iter()
            .cloned()
            .map(|root| {
                Box::new(move || {
                    let drive_type = wv::drive_type(&root);
                    let info = wv::volume_info(&root).ok();
                    let space = if info.is_some() { wv::disk_free(&root).ok() } else { None };
                    Probe { drive_type, info, space }
                }) as Box<dyn FnOnce() -> Probe + Send>
            })
            .collect();
        let results = parallel_with_deadline(jobs, QUERY_DEADLINE);
        let mut out = Vec::new();
        for (root, res) in roots.into_iter().zip(results) {
            let (drive_type, info, space) = match res {
                Some(p) => (p.drive_type, p.info, p.space),
                // Timed out: most likely an unreachable network drive.
                None => (wv::drive_type(&root), None, None),
            };
            let kind = kind_of(drive_type);
            if info.is_none() && matches!(kind, VolumeKind::Removable | VolumeKind::Optical) {
                continue; // empty card reader / optical drive without media
            }
            let info = info.unwrap_or_default();
            let label = info.label.clone();
            let (total, free) = space.unwrap_or((0, 0));
            out.push(VolumeInfo {
                mount: root,
                label,
                fs: info.fs.clone(),
                kind,
                total,
                free,
                read_only: info.flags & FILE_READ_ONLY_VOLUME != 0,
                device: DeviceId(u64::from(info.serial)),
                supports: caps(info.flags, &info.fs)
                    & if kind == VolumeKind::Network { !FsCaps::TRASH } else { FsCaps::all() },
            });
        }
        out
    }

    pub(super) fn space(path: &Path) -> FsResult<(u64, u64)> {
        wv::disk_free(path).map_err(err::map("space", path))
    }

    pub(super) fn volume_root(path: &Path) -> FsResult<PathBuf> {
        match wv::volume_path_name(path) {
            Ok(p) => Ok(p),
            Err(e) => {
                // Prefix-only fallback (drive letter / UNC share).
                use std::path::Component;
                let abs = crate::path::strip_verbatim(&crate::path::normalize(path));
                let mut out = PathBuf::new();
                for c in abs.components() {
                    match c {
                        Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
                        _ => break,
                    }
                }
                if out.as_os_str().is_empty() {
                    return Err(err::io("volume_root", path, e));
                }
                if !out.has_root() {
                    out.push("\\");
                }
                Ok(out)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use soyuz_core::{DeviceId, FsResult};

    use super::{
        FsCaps, MountInfo, QUERY_DEADLINE, VolumeInfo, VolumeKind, caps_for, dedupe_mounts, flatpak_visible,
        gvfs_label, is_pseudo_mount, kind_for, mount_for, parallel_with_deadline, parse_mountinfo,
    };
    use crate::err;

    pub(crate) fn mounts() -> Vec<MountInfo> {
        std::fs::read_to_string("/proc/self/mountinfo").map(|t| parse_mountinfo(&t)).unwrap_or_default()
    }

    /// `/dev/sdb1` → label, from the `/dev/disk/by-label` symlinks.
    fn labels() -> HashMap<PathBuf, String> {
        let mut out = HashMap::new();
        if let Ok(rd) = std::fs::read_dir("/dev/disk/by-label") {
            for e in rd.flatten() {
                if let Ok(target) = std::fs::canonicalize(e.path()) {
                    let label = unescape_hex(&e.file_name().to_string_lossy());
                    out.insert(target, label);
                }
            }
        }
        out
    }

    /// udev escapes characters as `\x20`.
    fn unescape_hex(s: &str) -> String {
        let mut out = Vec::new();
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\'
                && b.get(i + 1) == Some(&b'x')
                && let Some(v) = s.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 4;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Is the block device `dev` (`/dev/sdb1`, `/dev/mapper/luks-…`) removable media? Its
    /// sysfs node (`/sys/class/block/<name>`, the whole disk for a partition) says
    /// `removable = 1` (card readers, optical drives, most sticks), or it hangs off a USB
    /// bus (USB disks report `removable = 0`). Device-mapper / md devices are judged by
    /// the devices under them (`slaves/`).
    pub(super) fn block_removable(dev: &Path) -> bool {
        let Some(name) = std::fs::canonicalize(dev).ok().and_then(|d| d.file_name().map(|n| n.to_owned())) else {
            return false;
        };
        sysfs_removable(&Path::new("/sys/class/block").join(name), 0)
    }

    fn sysfs_removable(node: &Path, depth: u32) -> bool {
        let Ok(real) = std::fs::canonicalize(node) else { return false };
        if real.components().any(|c| c.as_os_str().to_string_lossy().starts_with("usb")) {
            return true;
        }
        let disk =
            if real.join("partition").exists() { real.parent().map(Path::to_path_buf) } else { Some(real.clone()) };
        if disk.is_some_and(|d| std::fs::read_to_string(d.join("removable")).is_ok_and(|s| s.trim() == "1")) {
            return true;
        }
        if depth < 4
            && let Ok(rd) = std::fs::read_dir(real.join("slaves"))
        {
            return rd.flatten().any(|e| sysfs_removable(&e.path(), depth + 1));
        }
        false
    }

    /// One volume to query: the mount table entry plus label / kind overrides.
    struct Cand {
        m: MountInfo,
        label: Option<String>,
        kind: Option<VolumeKind>,
    }

    /// gvfs FUSE mounts (`/run/user/$UID/gvfs/<mount>`), listed under a deadline (the
    /// gvfs daemon answers the directory listing; a hung backend must not stall us).
    fn gvfs_mounts(all: &[MountInfo]) -> Vec<Cand> {
        let Some(root) = all.iter().find(|m| m.fstype == "fuse.gvfsd-fuse").cloned() else { return Vec::new() };
        let dir = root.mount.clone();
        let job: Box<dyn FnOnce() -> Vec<std::ffi::OsString> + Send> = Box::new(move || {
            std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.file_name()).collect()).unwrap_or_default()
        });
        let names = parallel_with_deadline(vec![job], std::time::Duration::from_millis(800))
            .pop()
            .flatten()
            .unwrap_or_default();
        names
            .into_iter()
            .map(|n| {
                let (label, kind) = gvfs_label(&n.to_string_lossy());
                Cand {
                    m: MountInfo {
                        mount: root.mount.join(&n),
                        root: "/".into(),
                        fstype: "fuse.gvfsd-fuse".into(),
                        source: n.to_string_lossy().into_owned(),
                        ..root.clone()
                    },
                    label: Some(label),
                    kind: Some(kind),
                }
            })
            .collect()
    }

    pub(super) fn volumes() -> Vec<VolumeInfo> {
        let table = mounts();
        let gvfs = gvfs_mounts(&table);
        let mut visible: Vec<MountInfo> = table.into_iter().filter(|m| !is_pseudo_mount(m)).collect();
        if crate::launch::linux::flatpak_id().is_some() {
            visible = flatpak_visible(visible, &crate::path::home());
        }
        let all: Vec<MountInfo> = dedupe_mounts(visible);
        let mut cands: Vec<Cand> = all.into_iter().map(|m| Cand { m, label: None, kind: None }).collect();
        cands.extend(gvfs);
        let labels = labels();
        type Job = Box<dyn FnOnce() -> Option<(u64, u64, u64)> + Send>;
        let jobs: Vec<Job> = cands
            .iter()
            .map(|c| {
                let mount = c.m.mount.clone();
                Box::new(move || {
                    let s = rustix::fs::statvfs(&mount).ok()?;
                    let dev = crate::sys::unix::ustat_at(rustix::fs::CWD, mount.as_os_str(), true).ok()?.dev;
                    let frsize = if s.f_frsize > 0 { s.f_frsize } else { s.f_bsize };
                    Some((s.f_blocks.saturating_mul(frsize), s.f_bavail.saturating_mul(frsize), dev))
                }) as Job
            })
            .collect();
        let results = parallel_with_deadline(jobs, QUERY_DEADLINE);
        let mut out = Vec::new();
        for (c, res) in cands.into_iter().zip(results) {
            let m = c.m;
            let (total, free, dev) = res.flatten().unwrap_or((0, 0, rustix::fs::makedev(m.major, m.minor)));
            let mut kind = c.kind.unwrap_or_else(|| kind_for(&m.fstype, &m.source, &m.mount));
            if kind == VolumeKind::Fixed && m.source.starts_with("/dev/") && block_removable(Path::new(&m.source)) {
                kind = VolumeKind::Removable;
            }
            let src = std::fs::canonicalize(&m.source).unwrap_or_else(|_| PathBuf::from(&m.source));
            let label = c.label.or_else(|| labels.get(&src).cloned()).unwrap_or_else(|| {
                if m.mount == Path::new("/") {
                    "File System".to_string()
                } else {
                    m.mount.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                }
            });
            let mut supports = caps_for(&m.fstype);
            if kind != VolumeKind::Network
                && crate::trash::trash_support(&m.mount) == crate::trash::TrashSupport::Native
            {
                supports |= FsCaps::TRASH;
            }
            out.push(VolumeInfo {
                read_only: m.options.split(',').any(|o| o == "ro"),
                mount: m.mount,
                label,
                fs: if m.fstype == "fuse.gvfsd-fuse" { "gvfs".into() } else { m.fstype },
                kind,
                total,
                free,
                device: DeviceId(dev),
                supports,
            });
        }
        out
    }

    pub(super) fn space(path: &Path) -> FsResult<(u64, u64)> {
        let s = rustix::fs::statvfs(path).map_err(|e| err::io("space", path, e.into()))?;
        let frsize = if s.f_frsize > 0 { s.f_frsize } else { s.f_bsize };
        Ok((s.f_blocks.saturating_mul(frsize), s.f_bavail.saturating_mul(frsize)))
    }

    /// Absolute path with symlinks resolved, except a final symlink component (a link
    /// lives on the filesystem of its parent directory).
    fn resolved(path: &Path) -> Option<PathBuf> {
        let link = crate::sys::unix::ustat_at(rustix::fs::CWD, path.as_os_str(), false)
            .is_ok_and(|st| st.mode & 0o170000 == 0o120000);
        match (link, path.parent(), path.file_name()) {
            (true, Some(parent), Some(name)) => {
                let parent = if parent.as_os_str().is_empty() { Path::new(".") } else { parent };
                std::fs::canonicalize(parent).ok().map(|p| p.join(name))
            }
            _ => std::fs::canonicalize(path).ok(),
        }
    }

    pub(super) fn volume_root(path: &Path) -> FsResult<PathBuf> {
        let table = mounts();
        if let Some(p) = resolved(path)
            && let Some(m) = mount_for(&p, &table)
        {
            return Ok(m.mount.clone());
        }
        super::unix_volume_root(path)
    }

    /// Mount points of every user-visible filesystem, before de-duplication (per-mount
    /// trash directories can live on any of them), each with "may hang" (network / FUSE).
    pub(crate) fn user_mounts() -> Vec<(PathBuf, bool)> {
        mounts()
            .into_iter()
            .filter(|m| !is_pseudo_mount(m))
            .map(|m| {
                let slow = super::may_hang(&m.fstype, &m.source, &m.mount);
                (m.mount, slow)
            })
            .collect()
    }
}

/// Mount points of every user-visible filesystem (btrfs subvolume and bind mounts
/// included), each with whether touching it may hang (network or FUSE: a stale hard NFS
/// mount or a dead sshfs blocks `stat` forever, so probe those under a deadline).
#[cfg(target_os = "linux")]
pub(crate) fn user_mounts() -> Vec<(PathBuf, bool)> {
    imp::user_mounts()
}

/// Network and FUSE filesystems: any syscall on them may block indefinitely.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn may_hang(fstype: &str, source: &str, mount: &Path) -> bool {
    fstype.starts_with("fuse") || kind_for(fstype, source, mount) == VolumeKind::Network
}

/// Mount points of every filesystem backed by a block device (`/dev/…` source), hidden
/// ones included (e.g. Timeshift's `/run/timeshift/…/backup`). Never a network mount, so
/// looking inside them cannot hang.
#[cfg(target_os = "linux")]
pub(crate) fn block_device_mounts() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> =
        imp::mounts().into_iter().filter(|m| m.source.starts_with("/dev/")).map(|m| m.mount).collect();
    v.sort();
    v.dedup();
    v
}

/// Walk up from `path` while the device id stays the same (fallback when the mount table
/// cannot be read).
#[cfg(unix)]
fn unix_volume_root(path: &Path) -> FsResult<PathBuf> {
    let dev_of = |p: &Path| crate::list::stat(p, true).ok().and_then(|e| e.dev);
    let mut cur = crate::path::normalize(path);
    let dev = dev_of(&cur).ok_or_else(|| FsError::new(FsErrorKind::NotFound, "volume_root", Some(path)))?;
    loop {
        let Some(parent) = cur.parent().map(Path::to_path_buf) else { return Ok(cur) };
        if dev_of(&parent) == Some(dev) {
            cur = parent;
        } else {
            return Ok(cur);
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{DeviceId, FsResult};

    use super::{FsCaps, VolumeInfo, VolumeKind, caps_for};
    use crate::err;
    use crate::sys::macos;

    pub(super) fn volumes() -> Vec<VolumeInfo> {
        let mut out = Vec::new();
        for m in macos::mounts() {
            let root = m.mount == Path::new("/");
            if !root && (m.flags & macos::MNT_DONTBROWSE != 0 || m.fstype == "devfs" || m.fstype == "autofs") {
                continue;
            }
            let kind = if m.flags & macos::MNT_LOCAL == 0 {
                VolumeKind::Network
            } else if m.fstype == "cd9660" || m.fstype == "udf" {
                VolumeKind::Optical
            } else if root {
                VolumeKind::Fixed
            } else if m.mount.starts_with("/Volumes") {
                VolumeKind::Removable
            } else {
                VolumeKind::Fixed
            };
            let label = if root {
                "Macintosh HD".to_string()
            } else {
                m.mount.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            };
            let mut supports = caps_for(&m.fstype);
            if crate::trash::trash_support(&m.mount) == crate::trash::TrashSupport::Native {
                supports |= FsCaps::TRASH;
            }
            let dev = crate::list::stat(&m.mount, true).ok().and_then(|e| e.dev).unwrap_or(DeviceId(0));
            out.push(VolumeInfo {
                label,
                fs: m.fstype.clone(),
                kind,
                total: m.total,
                free: m.free,
                read_only: m.flags & macos::MNT_RDONLY != 0,
                device: dev,
                supports,
                mount: m.mount,
            });
        }
        out
    }

    pub(super) fn space(path: &Path) -> FsResult<(u64, u64)> {
        let s = rustix::fs::statvfs(path).map_err(|e| err::io("space", path, e.into()))?;
        let frsize = if s.f_frsize > 0 { s.f_frsize } else { s.f_bsize };
        Ok((s.f_blocks.saturating_mul(frsize), s.f_bavail.saturating_mul(frsize)))
    }

    pub(super) fn volume_root(path: &Path) -> FsResult<PathBuf> {
        super::unix_volume_root(path)
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{DeviceId, FsResult};

    use super::{FsCaps, VolumeInfo, VolumeKind};
    use crate::err;

    pub(super) fn volumes() -> Vec<VolumeInfo> {
        let (total, free) = space(Path::new("/")).unwrap_or((0, 0));
        let device = crate::list::stat(Path::new("/"), true).ok().and_then(|e| e.dev).unwrap_or(DeviceId(0));
        vec![VolumeInfo {
            mount: PathBuf::from("/"),
            label: String::new(),
            fs: String::new(),
            kind: VolumeKind::Fixed,
            total,
            free,
            read_only: false,
            device,
            supports: FsCaps::HARDLINK | FsCaps::SYMLINK,
        }]
    }

    pub(super) fn space(path: &Path) -> FsResult<(u64, u64)> {
        let s = rustix::fs::statvfs(path).map_err(|e| err::io("space", path, e.into()))?;
        let frsize = if s.f_frsize > 0 { s.f_frsize } else { s.f_bsize };
        Ok((s.f_blocks.saturating_mul(frsize), s.f_bavail.saturating_mul(frsize)))
    }

    pub(super) fn volume_root(path: &Path) -> FsResult<PathBuf> {
        super::unix_volume_root(path)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{FsError, FsResult};

    use super::VolumeInfo;

    pub(super) fn volumes() -> Vec<VolumeInfo> {
        Vec::new()
    }
    pub(super) fn space(path: &Path) -> FsResult<(u64, u64)> {
        Err(FsError::unsupported("space", Some(path)))
    }
    pub(super) fn volume_root(path: &Path) -> FsResult<PathBuf> {
        Ok(path.ancestors().last().unwrap_or(path).to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw,errors=remount-ro
23 22 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw
24 22 0:22 / /sys rw,nosuid shared:7 - sysfs sysfs rw
40 22 8:17 / /media/user/My\\040Disk rw,nosuid,nodev,relatime shared:200 - vfat /dev/sdb1 rw,fmask=0022
41 22 0:50 / /mnt/share rw,relatime shared:210 - cifs //server/share rw,vers=3.0
42 22 0:51 / /run/user/1000 rw shared:220 - tmpfs tmpfs rw
43 22 7:3 / /snap/core/123 ro,nodev shared:230 - squashfs /dev/loop3 ro
44 22 8:3 / /home rw,relatime shared:2 - btrfs /dev/sda3 rw,space_cache
";

    #[test]
    fn mountinfo_parsing_and_filtering() {
        let all = parse_mountinfo(SAMPLE);
        assert_eq!(all.len(), 8);
        assert_eq!(all[3].mount, PathBuf::from("/media/user/My Disk"));
        assert_eq!(all[3].fstype, "vfat");
        assert_eq!((all[3].major, all[3].minor), (8, 17));
        let visible: Vec<_> = all.iter().filter(|m| !is_pseudo_mount(m)).map(|m| m.mount.clone()).collect();
        assert_eq!(
            visible,
            [
                PathBuf::from("/"),
                PathBuf::from("/media/user/My Disk"),
                PathBuf::from("/mnt/share"),
                PathBuf::from("/home")
            ]
        );
        assert_eq!(kind_for("cifs", "//server/share", Path::new("/mnt/share")), VolumeKind::Network);
        assert_eq!(kind_for("vfat", "/dev/sdb1", Path::new("/media/user/My Disk")), VolumeKind::Removable);
        assert_eq!(kind_for("ext4", "/dev/sda2", Path::new("/")), VolumeKind::Fixed);
        assert_eq!(
            kind_for("ext4", "/dev/sdc1", Path::new("/mnt/backup")),
            VolumeKind::Fixed,
            "sysfs decides for /mnt"
        );
        assert!(caps_for("btrfs").contains(FsCaps::REFLINK));
        assert!(!caps_for("vfat").contains(FsCaps::HARDLINK));
        assert!(caps_for("tmpfs").contains(FsCaps::HARDLINK) && !caps_for("tmpfs").contains(FsCaps::REFLINK));
        assert_eq!(unescape_octal(r"a\040b\134c"), "a b\\c");
        // Out of byte range (would overflow a u8): kept as written.
        assert_eq!(unescape_octal(r"x\777y\400"), r"x\777y\400");
        assert_eq!(unescape_octal(r"\377"), "\u{fffd}");
    }

    /// Fedora's btrfs layout: `/` and `/home` are subvolumes of one filesystem.
    const BTRFS: &str = "\
43 1 0:35 /root / rw,relatime shared:1 - btrfs /dev/sdd3 rw,subvolid=259,subvol=/root
61 43 0:54 / /tmp rw,nosuid,nodev shared:123 - tmpfs tmpfs rw,inode64
60 43 0:35 /home /home rw,relatime shared:169 - btrfs /dev/sdd3 rw,subvolid=257,subvol=/home
70 43 8:50 / /boot rw,relatime shared:174 - ext4 /dev/sdd2 rw
42 40 0:27 / /dev/shm rw,nosuid,nodev shared:3 - tmpfs tmpfs rw
53 43 0:29 / /run rw,nosuid,nodev shared:14 - tmpfs tmpfs rw,mode=755
242 53 0:83 / /run/user/1000 rw,nosuid,nodev,relatime shared:902 - tmpfs tmpfs rw,mode=700
808 242 0:84 / /run/user/1000/gvfs rw,nosuid,nodev,relatime shared:978 - fuse.gvfsd-fuse gvfsd-fuse rw
90 43 8:33 / /srv/data rw,relatime shared:300 - ext4 /dev/sdc1 rw
91 43 8:33 /exports /mnt/exports rw,relatime shared:300 - ext4 /dev/sdc1 rw
92 43 0:35 /root/var/lib/x /var/lib/x rw - btrfs /dev/sdd3 rw
";

    #[test]
    fn subvolumes_and_bind_mounts_collapse() {
        let all = parse_mountinfo(BTRFS);
        assert_eq!(all[2].root, "/home");
        let visible: Vec<_> = all.into_iter().filter(|m| !is_pseudo_mount(m)).collect();
        let mounts: Vec<PathBuf> = dedupe_mounts(visible).into_iter().map(|m| m.mount).collect();
        assert_eq!(
            mounts,
            ["/", "/tmp", "/boot", "/srv/data"].map(PathBuf::from),
            "btrfs subvolumes and the bind mount fold into one entry; /tmp tmpfs is listed, /run and /dev/shm not"
        );
        let tmp = parse_mountinfo(BTRFS).into_iter().find(|m| m.mount == Path::new("/tmp")).unwrap();
        assert_eq!(kind_for(&tmp.fstype, &tmp.source, &tmp.mount), VolumeKind::Ram);
    }

    /// A Flatpak sandbox with `--filesystem=host` on the layout above (trimmed).
    const FLATPAK: &str = "\
1 0 0:90 / / rw - tmpfs tmpfs rw
2 1 0:35 /root/var/lib/flatpak/runtime/org.freedesktop.Platform/files /usr ro - btrfs /dev/sdd3 rw
3 1 0:35 /root/var/lib/flatpak/app/io.github.ulaas.Soyuz/files /app ro - btrfs /dev/sdd3 rw
4 1 0:35 /root/etc/machine-id /etc/machine-id ro - btrfs /dev/sdd3 rw
5 1 0:35 /root/afs /afs rw - btrfs /dev/sdd3 rw
6 1 0:35 /home /home rw - btrfs /dev/sdd3 rw
7 6 0:96 / /home/u/.local/share/flatpak rw - tmpfs tmpfs rw
8 1 0:35 /root/mnt /mnt rw - btrfs /dev/sdd3 rw
9 1 0:35 /root/srv /srv rw - btrfs /dev/sdd3 rw
10 1 8:33 / /mnt/data rw - ext4 /dev/sdc1 rw
11 1 0:29 /media /run/media rw - tmpfs tmpfs rw
12 1 0:35 /root /run/host/root rw - btrfs /dev/sdd3 rw
13 1 0:91 / /tmp rw - tmpfs tmpfs rw
";

    #[test]
    fn flatpak_sandbox_mounts() {
        let all: Vec<_> = parse_mountinfo(FLATPAK).into_iter().filter(|m| !is_pseudo_mount(m)).collect();
        let vis = flatpak_visible(all, Path::new("/home/u"));
        let mounts: Vec<PathBuf> = dedupe_mounts(vis).into_iter().map(|m| m.mount).collect();
        assert_eq!(mounts, ["/home", "/mnt/data", "/tmp"].map(PathBuf::from));
    }

    #[test]
    fn volume_root_is_the_longest_mount_prefix() {
        let all = parse_mountinfo(BTRFS);
        let root = |p: &str| mount_for(Path::new(p), &all).map(|m| m.mount.clone());
        assert_eq!(root("/home/u/doc.txt"), Some(PathBuf::from("/home")));
        assert_eq!(root("/home"), Some(PathBuf::from("/home")));
        assert_eq!(root("/homework/x"), Some(PathBuf::from("/")), "component prefix, not string prefix");
        assert_eq!(root("/run/user/1000/gvfs/sftp:host=x/a"), Some(PathBuf::from("/run/user/1000/gvfs")));
        assert_eq!(root("/etc/fstab"), Some(PathBuf::from("/")));
    }

    #[test]
    fn gvfs_names() {
        assert_eq!(gvfs_label("smb-share:server=nas,share=media"), ("media on nas".into(), VolumeKind::Network));
        assert_eq!(gvfs_label("sftp:host=example.org,user=me"), ("me@example.org (SFTP)".into(), VolumeKind::Network));
        assert_eq!(gvfs_label("dav:host=cloud.example,ssl=true").0, "cloud.example (WebDAV)");
        assert_eq!(gvfs_label("mtp:host=Google_Pixel_7_ABC"), ("Google Pixel 7 ABC".into(), VolumeKind::Removable));
        assert_eq!(gvfs_label("smb-share:server=nas,share=my%20files").0, "my files on nas");
        assert_eq!(gvfs_label("odd").0, "odd");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn removable_lookup_does_not_panic() {
        // Real block devices vary; only check that the sysfs walk is total.
        let _ = imp::block_removable(Path::new("/dev/null"));
        let _ = imp::block_removable(Path::new("/dev/does-not-exist"));
        assert!(!imp::block_removable(Path::new("/nonexistent")));
    }

    #[test]
    fn real_volumes_and_space() {
        let v = volumes();
        assert!(!v.is_empty());
        let d = tempfile::tempdir().unwrap();
        let (total, free) = space(d.path()).unwrap();
        assert!(total > 0 && free <= total, "{total} {free}");
        // Non-existing paths use the nearest existing ancestor.
        let (t2, _) = space(&d.path().join("no").join("such")).unwrap();
        assert_eq!(t2, total);
        let root = volume_root(d.path()).unwrap();
        let dev = crate::copy::device_of(d.path()).unwrap();
        assert!(
            v.iter().any(|vol| vol.device == dev && (vol.mount == root || d.path().starts_with(&vol.mount))),
            "temp dir's volume {root:?} / {dev:?} not in {v:#?}"
        );
        assert!(d.path().starts_with(&root), "{root:?}");
        if cfg!(target_os = "linux") {
            // A symlink belongs to the filesystem of its directory, not of its target.
            #[cfg(unix)]
            {
                let link = d.path().join("to-root");
                std::os::unix::fs::symlink("/", &link).unwrap();
                assert_eq!(volume_root(&link).unwrap(), root);
            }
        }
    }

    #[test]
    fn deadline_collects_fast_jobs() {
        let jobs: Vec<Box<dyn FnOnce() -> u32 + Send>> = vec![
            Box::new(|| 1),
            Box::new(|| {
                std::thread::sleep(Duration::from_secs(5));
                2
            }),
        ];
        let start = Instant::now();
        let r = parallel_with_deadline(jobs, Duration::from_millis(200));
        assert_eq!(r[0], Some(1));
        assert_eq!(r[1], None);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
