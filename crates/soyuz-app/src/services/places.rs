//! Volumes (with free space) and cloud-folder detection, refreshed on a background
//! thread so slow or disconnected drives never stall the UI.

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
            let mut first = true;
            loop {
                if !postman.send(UiMsg::Volumes(volumes_with_space())) {
                    return;
                }
                if first {
                    first = false;
                    postman.send(UiMsg::CloudPlaces(detect_cloud(&soyuz_fs::home())));
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
    v
}

/// Cloud storage roots: OneDrive variables (Windows), `~/Library/CloudStorage/*`
/// (macOS) and well-known folders under home. No shell APIs involved.
pub fn detect_cloud(home: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
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
}
