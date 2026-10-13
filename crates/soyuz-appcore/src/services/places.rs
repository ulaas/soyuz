//! Volumes (with free space) and cloud-folder detection, refreshed on a background
//! thread so slow or disconnected drives never stall the UI. On Linux, cloud places also
//! include gvfs cloud mounts (GNOME Online Accounts: Google Drive, OneDrive) and rclone
//! FUSE mounts, re-detected on every refresh because they come and go.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Sender, bounded};
use soyuz_fs::{VolumeInfo, VolumeKind};

use super::mailbox::{Postman, UiMsg};

const REFRESH: Duration = Duration::from_secs(20);

pub struct PlacesService {
    trigger: Sender<()>,
}

impl PlacesService {
    pub fn new(postman: Postman) -> Self {
        let (tx, rx) = bounded::<()>(1);
        let spawned = thread::Builder::new().name("soyuz-places".into()).spawn(move || {
            let mut last_cloud: Option<Vec<(String, PathBuf)>> = None;
            loop {
                if !postman.send(UiMsg::Volumes(volumes_with_space())) {
                    return;
                }
                // Home folders are detected once; Linux mounts on every refresh (sent
                // only when something changed).
                if last_cloud.is_none() || cfg!(target_os = "linux") {
                    let cloud = if demo_mask() { Vec::new() } else { detect_cloud(&soyuz_fs::home()) };
                    if last_cloud.as_ref() != Some(&cloud) {
                        postman.send(UiMsg::CloudPlaces(cloud.clone()));
                        last_cloud = Some(cloud);
                    }
                }
                match rx.recv_timeout(REFRESH) {
                    Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("cannot spawn places thread: {e}");
        }
        PlacesService { trigger: tx }
    }

    /// Ask for an immediate refresh (e.g. after navigating to a new volume).
    pub fn refresh(&self) {
        let _ = self.trigger.try_send(());
    }
}

/// What lists the volumes: `soyuz_fs::volumes` unless a test replaced it
/// ([`set_volume_source`]).
pub type VolumeSource = std::sync::Arc<dyn Fn() -> Vec<VolumeInfo> + Send + Sync>;

static VOLUME_SOURCE: std::sync::RwLock<Option<VolumeSource>> = std::sync::RwLock::new(None);

/// Test seam: the places thread lists volumes with `f` (`None`: the real ones), so
/// headless tests show made-up drives in temporary folders.
#[doc(hidden)]
pub fn set_volume_source(f: Option<VolumeSource>) {
    *VOLUME_SOURCE.write().unwrap_or_else(|e| e.into_inner()) = f;
}

fn volumes_with_space() -> Vec<VolumeInfo> {
    if let Some(f) = VOLUME_SOURCE.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return f();
    }
    let mut v = soyuz_fs::volumes();
    for vol in &mut v {
        if vol.total == 0
            && let Ok((total, free)) = soyuz_fs::space(&vol.mount)
        {
            vol.total = total;
            vol.free = free;
        }
    }
    if demo_mask() {
        mask_volumes(&mut v);
    }
    v
}

/// `SOYUZ_DEMO_MASK=1` (screenshots, `scripts/screenshots.ps1`): show volumes without
/// anything machine-specific — fixed local disks only, no labels (the localized kind name
/// is shown instead), and made-up sizes.
fn demo_mask() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SOYUZ_DEMO_MASK").is_ok_and(|v| v == "1"))
}

fn mask_volumes(v: &mut Vec<VolumeInfo>) {
    v.retain(|vol| vol.kind == soyuz_fs::VolumeKind::Fixed);
    for (i, vol) in v.iter_mut().enumerate() {
        vol.label.clear();
        vol.total = 1_000_000_000_000;
        vol.free = [412_000_000_000, 688_000_000_000, 231_000_000_000][i % 3];
    }
}

/// Volumes listed as drives (the sidebar's Drives section and the Computer view): rclone
/// FUSE mounts that the Cloud section already shows are left out, so they appear only under
/// Cloud. `app.ui.volumes` itself stays complete (breadcrumbs, per-volume rules, the
/// device list). Borrows when there is nothing to drop.
pub fn drive_volumes<'a>(vols: &'a [VolumeInfo], cloud: &[(String, PathBuf)]) -> std::borrow::Cow<'a, [VolumeInfo]> {
    let in_cloud = |v: &VolumeInfo| v.fs == "fuse.rclone" && cloud.iter().any(|(_, p)| *p == v.mount);
    if vols.iter().any(in_cloud) {
        std::borrow::Cow::Owned(vols.iter().filter(|v| !in_cloud(v)).cloned().collect())
    } else {
        std::borrow::Cow::Borrowed(vols)
    }
}

/// Cloud storage roots: OneDrive variables (Windows), `~/Library/CloudStorage/*`
/// (macOS), gvfs cloud mounts and rclone mounts (Linux) and well-known folders under
/// home. No shell APIs involved.
pub fn detect_cloud(home: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    // FUSE mount roots are not stat'ed (a hung network backend would block here); the
    // mount table / gvfs listing already says they exist.
    #[cfg(target_os = "linux")]
    {
        let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        out.extend(rclone_mounts(&mounts));
        if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            let gvfs = PathBuf::from(run).join("gvfs");
            out.extend(gvfs_names(&gvfs).into_iter().filter_map(|n| gvfs_cloud(&n).map(|l| (l, gvfs.join(&n)))));
        }
    }
    let mut push = |name: String, p: PathBuf| {
        if p.is_dir() && !out.iter().any(|(_, q)| q == &p) {
            out.push((name, p));
        }
    };
    for (var, name) in [
        ("OneDriveCommercial", tr!("services-places-onedrive-work")),
        ("OneDriveConsumer", "OneDrive"),
        ("OneDrive", "OneDrive"),
    ] {
        if let Some(v) = std::env::var_os(var)
            && !v.is_empty()
        {
            push(name.to_string(), PathBuf::from(v));
        }
    }
    if let Ok(rd) = std::fs::read_dir(home.join("Library").join("CloudStorage")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().replace('-', " ");
            push(name, e.path());
        }
    }
    for (dir, name) in [
        ("iCloudDrive", "iCloud Drive"),
        ("Dropbox", "Dropbox"),
        ("Google Drive", "Google Drive"),
        ("My Drive", "Google Drive"),
        ("pCloudDrive", "pCloud"),
        ("Nextcloud", "Nextcloud"),
        ("ownCloud", "ownCloud"),
        ("MEGA", "MEGA"),
        ("Box", "Box"),
    ] {
        push(name.to_string(), home.join(dir));
    }
    out
}

/// rclone FUSE mounts in `/proc/self/mountinfo` text: `(label, mount point)`. The label
/// is the remote name (`gdrive:` → `gdrive (rclone)`, `s3:bucket/x` → `s3:bucket/x (rclone)`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn rclone_mounts(mountinfo: &str) -> Vec<(String, PathBuf)> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (pre, post) = line.split_once(" - ")?;
            let mut post = post.split(' ');
            let fstype = post.next()?;
            let source = post.next().unwrap_or("");
            if fstype != "fuse.rclone" {
                return None;
            }
            let mount = unescape_mountinfo(pre.split(' ').nth(4)?);
            let remote = unescape_mountinfo(source);
            let remote = remote.strip_suffix(':').unwrap_or(&remote);
            let name = if remote.is_empty() {
                Path::new(&mount).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            } else {
                remote.to_string()
            };
            Some((format!("{name} (rclone)"), PathBuf::from(mount)))
        })
        .collect()
}

/// mountinfo escapes space, tab, newline and backslash as `\ooo` octal.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unescape_mountinfo(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && let Some(v) = s.get(i + 1..i + 4).and_then(|o| u8::from_str_radix(o, 8).ok())
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

/// Entry names of the gvfs FUSE directory, listed under a deadline: gvfsd-fuse answers
/// the listing and a hung backend must not stall the places thread. At most one listing
/// is in flight: while an earlier one is stuck, no new thread is started and the names
/// from the last listing that finished are used.
#[cfg(target_os = "linux")]
fn gvfs_names(dir: &Path) -> Vec<String> {
    use std::sync::atomic::AtomicBool;
    static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
    static LAST: parking_lot::Mutex<Vec<String>> = parking_lot::Mutex::new(Vec::new());
    gvfs_names_with(dir, &IN_FLIGHT, &LAST, Duration::from_secs(2), |d| {
        std::fs::read_dir(d)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    })
}

/// [`gvfs_names`] with its state and the listing passed in (tests).
#[cfg(target_os = "linux")]
fn gvfs_names_with(
    dir: &Path,
    in_flight: &'static std::sync::atomic::AtomicBool,
    last: &'static parking_lot::Mutex<Vec<String>>,
    timeout: Duration,
    list: impl FnOnce(&Path) -> Vec<String> + Send + 'static,
) -> Vec<String> {
    use std::sync::atomic::Ordering;
    if in_flight.swap(true, Ordering::SeqCst) {
        return last.lock().clone();
    }
    let (tx, rx) = bounded(1);
    let dir = dir.to_path_buf();
    let spawned = thread::Builder::new().name("soyuz-places-gvfs".into()).spawn(move || {
        let names = list(&dir);
        *last.lock() = names.clone();
        in_flight.store(false, Ordering::SeqCst);
        let _ = tx.send(names);
    });
    if spawned.is_err() {
        in_flight.store(false, Ordering::SeqCst);
        return last.lock().clone();
    }
    rx.recv_timeout(timeout).unwrap_or_else(|_| last.lock().clone())
}

/// Label of a gvfs mount that is cloud storage (`google-drive:host=gmail.com,user=me` →
/// `Google Drive (me@gmail.com)`); `None` for other mounts (SMB / SFTP / phones are
/// listed as drives).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn gvfs_cloud(name: &str) -> Option<String> {
    let (scheme, rest) = name.split_once(':')?;
    let what = match scheme {
        "google-drive" => "Google Drive",
        "onedrive" => "OneDrive",
        _ => return None,
    };
    let field = |k: &str| {
        rest.split(',').find_map(|p| p.split_once('=').filter(|(key, _)| *key == k).map(|(_, v)| v.replace("%40", "@")))
    };
    let account = match (field("user"), field("host")) {
        (Some(u), Some(h)) if !u.contains('@') => format!("{u}@{h}"),
        (Some(u), _) => u,
        (None, Some(h)) => h,
        (None, None) => return Some(what.to_string()),
    };
    Some(format!("{what} ({account})"))
}

/// Localized kind name for a volume without its own label ("Local Disk").
pub fn volume_kind_label(kind: VolumeKind) -> &'static str {
    match kind {
        VolumeKind::Fixed => tr!("views-volume-fixed"),
        VolumeKind::Removable => tr!("views-volume-removable"),
        VolumeKind::Network => tr!("views-volume-network"),
        VolumeKind::Optical => tr!("views-volume-optical"),
        VolumeKind::Ram => tr!("views-volume-ram"),
        VolumeKind::Unknown => tr!("views-computer-drive"),
    }
}

/// How full a volume is: the colour of its usage bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageLevel {
    Normal,
    /// Less than 15 % free.
    Warning,
    /// Less than 5 % free.
    Danger,
}

/// The level of a volume with `used` (0..=1) of its space taken.
pub fn usage_level(used: f32) -> UsageLevel {
    let free = 1.0 - used.clamp(0.0, 1.0);
    if free < 0.05 {
        UsageLevel::Danger
    } else if free < 0.15 {
        UsageLevel::Warning
    } else {
        UsageLevel::Normal
    }
}

/// The share of `v`'s space in use (`None` when its size is unknown).
pub fn used_share(v: &VolumeInfo) -> Option<f32> {
    (v.total > 0).then(|| 1.0 - v.free as f32 / v.total as f32)
}

/// Display name of a volume: its own label, else the localized kind name, followed by
/// the mount (`Local Disk (C:)`, `Yerel Disk (C:)`, `Backup (D:)`).
pub fn volume_label(v: &VolumeInfo) -> String {
    let mount = soyuz_fs::strip_verbatim(&v.mount).to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    let mount = if mount.is_empty() { "/".to_string() } else { mount };
    let base = if v.label.is_empty() { volume_kind_label(v.kind).to_string() } else { v.label.clone() };
    if base.contains(&mount) { base } else { format!("{base} ({mount})") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_levels_follow_the_free_share() {
        assert_eq!(usage_level(0.5), UsageLevel::Normal);
        assert_eq!(usage_level(0.86), UsageLevel::Warning);
        assert_eq!(usage_level(0.96), UsageLevel::Danger);
        assert_eq!(usage_level(7.0), UsageLevel::Danger);
        assert_eq!(usage_level(-1.0), UsageLevel::Normal);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_stuck_gvfs_listing_is_not_repeated() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
        static LAST: parking_lot::Mutex<Vec<String>> = parking_lot::Mutex::new(Vec::new());
        static STARTED: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new("/nonexistent");
        let t = Duration::from_millis(50);
        let quick = |names: &'static [&'static str]| {
            move |_: &Path| {
                STARTED.fetch_add(1, Ordering::SeqCst);
                names.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            }
        };
        assert_eq!(gvfs_names_with(dir, &IN_FLIGHT, &LAST, t, quick(&["a"])), ["a"]);
        // A hung listing: the call gives up with the last names...
        let (release_tx, release_rx) = bounded::<()>(1);
        let hung = move |_: &Path| {
            STARTED.fetch_add(1, Ordering::SeqCst);
            let _ = release_rx.recv();
            vec!["b".to_string()]
        };
        assert_eq!(gvfs_names_with(dir, &IN_FLIGHT, &LAST, t, hung), ["a"]);
        // ...and later refreshes start no new thread while it is stuck.
        let before = STARTED.load(Ordering::SeqCst);
        assert_eq!(gvfs_names_with(dir, &IN_FLIGHT, &LAST, t, quick(&["c"])), ["a"]);
        assert_eq!(STARTED.load(Ordering::SeqCst), before);
        // Once it returns, its names are used and probing resumes.
        release_tx.send(()).unwrap();
        let start = std::time::Instant::now();
        while IN_FLIGHT.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(LAST.lock().clone(), ["b"]);
        assert_eq!(gvfs_names_with(dir, &IN_FLIGHT, &LAST, t, quick(&["c"])), ["c"]);
    }

    #[test]
    fn detects_known_folders() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("Dropbox")).unwrap();
        std::fs::create_dir_all(home.path().join("Library/CloudStorage/GoogleDrive-me")).unwrap();
        let found = detect_cloud(home.path());
        assert!(found.iter().any(|(n, _)| n == "Dropbox"));
        assert!(found.iter().any(|(n, _)| n == "GoogleDrive me"));
        assert!(!found.iter().any(|(n, _)| n == "Nextcloud"));
    }

    #[test]
    fn rclone_cloud_mounts_leave_drives() {
        let vol = |mount: &str, fs: &str| VolumeInfo {
            mount: PathBuf::from(mount),
            label: String::new(),
            fs: fs.into(),
            kind: soyuz_fs::VolumeKind::Network,
            total: 0,
            free: 0,
            read_only: false,
            device: soyuz_core::DeviceId(0),
            supports: soyuz_fs::FsCaps::empty(),
        };
        let vols = vec![
            vol("/", "btrfs"),
            vol("/mnt/g", "fuse.rclone"),
            vol("/run/media/me/usb", "exfat"),
            vol("/mnt/h", "fuse.rclone"),
            vol("/mnt/gvfs-like", "fuse.gvfsd-fuse"),
        ];
        let cloud = vec![
            ("g (rclone)".to_string(), PathBuf::from("/mnt/g")),
            ("Google Drive".to_string(), PathBuf::from("/mnt/gvfs-like")),
        ];
        let d = drive_volumes(&vols, &cloud);
        let mounts: Vec<_> = d.iter().map(|v| v.mount.to_string_lossy().into_owned()).collect();
        assert_eq!(
            mounts,
            ["/", "/run/media/me/usb", "/mnt/h", "/mnt/gvfs-like"],
            "only rclone mounts shown under Cloud are dropped (sidebar Drives and Computer view), order kept"
        );
        assert_eq!(vols.len(), 5, "the full volume list is left untouched");
        assert!(matches!(drive_volumes(&vols, &[]), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn rclone_mounts_from_mountinfo() {
        let text = "\
808 242 0:84 / /run/user/1000/gvfs rw,nosuid shared:978 - fuse.gvfsd-fuse gvfsd-fuse rw,user_id=1000
900 242 0:99 / /home/me/Google\\040Drive rw,nosuid - fuse.rclone gdrive: rw,user_id=1000
901 242 0:100 / /mnt/b rw - fuse.rclone s3:bucket/x rw
902 242 0:101 / /mnt/c rw - fuse.sshfs me@host: rw
";
        assert_eq!(
            rclone_mounts(text),
            vec![
                ("gdrive (rclone)".to_string(), PathBuf::from("/home/me/Google Drive")),
                ("s3:bucket/x (rclone)".to_string(), PathBuf::from("/mnt/b")),
            ]
        );
        assert!(rclone_mounts("garbage").is_empty());
    }

    #[test]
    fn gvfs_cloud_labels() {
        assert_eq!(gvfs_cloud("google-drive:host=gmail.com,user=me").as_deref(), Some("Google Drive (me@gmail.com)"));
        assert_eq!(gvfs_cloud("onedrive:host=example.com,user=a%40b.c").as_deref(), Some("OneDrive (a@b.c)"));
        assert_eq!(gvfs_cloud("onedrive:").as_deref(), Some("OneDrive"));
        assert!(gvfs_cloud("smb-share:server=nas,share=x").is_none());
        assert!(gvfs_cloud("plain").is_none());
    }
}
