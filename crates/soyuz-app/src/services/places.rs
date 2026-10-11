//! Volumes (with free space) and cloud-folder detection, refreshed on a background
//! thread so slow or disconnected drives never stall the UI. On Linux, cloud places also
//! include gvfs cloud mounts (GNOME Online Accounts: Google Drive, OneDrive) and rclone
//! FUSE mounts, re-detected on every refresh because they come and go.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Sender, bounded};
use soyuz_fs::VolumeInfo;

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

fn volumes_with_space() -> Vec<VolumeInfo> {
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
/// the listing and a hung backend must not stall the places thread.
#[cfg(target_os = "linux")]
fn gvfs_names(dir: &Path) -> Vec<String> {
    let (tx, rx) = bounded(1);
    let dir = dir.to_path_buf();
    let spawned = thread::Builder::new().name("soyuz-places-gvfs".into()).spawn(move || {
        let names: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        let _ = tx.send(names);
    });
    if spawned.is_err() {
        return Vec::new();
    }
    rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

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
