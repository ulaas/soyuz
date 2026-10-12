//! Portal (frontend-agnostic): folders pinned to a sidebar section by dropping them on it.
//!
//! * Only existing local folders are pinned (files, duplicates and missing paths are
//!   skipped); the check runs on a worker thread, never on the UI thread.
//! * Pins live in `[[sidebar.portal]]` and are written through
//!   [`update_config`](crate::models::settings::update_config), like a Settings edit.
//! * Pins whose folder is gone are reported as missing (checked every few seconds on a
//!   worker while the window is focused).
//!
//! The GUI section (rows, drag to reorder, drops, context menu) is `ui::portal`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use soyuz_core::config::PortalPin;

use crate::core::AppCore;
use crate::geom::Rect;
use crate::host::Host;
use crate::models::toasts::Toast;

/// How often pinned folders are checked for existence (while the window is focused).
const CHECK_EVERY: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq)]
pub enum PortalCmd {
    /// Pin dropped paths (folders only; checked on a worker first).
    Pin(Vec<PathBuf>),
    Unpin(PathBuf),
    /// Move the pin at `from` before the pin at `to` (`to == len` = to the end).
    Move {
        from: usize,
        to: usize,
    },
    /// Set a pin's label (empty = the folder's name).
    Rename {
        path: PathBuf,
        name: String,
    },
}

/// Result of a pin check: the folders that may be pinned and how many paths were dropped.
struct Checked {
    dirs: Vec<PathBuf>,
    dropped: usize,
}

/// Per-window Portal state (lives in the sidebar state).
#[derive(Default)]
pub struct PortalState {
    /// Pin being dragged to a new position (index).
    pub drag: Option<usize>,
    /// The section's rect last frame: OS drops over it are pinned.
    pub rect: Option<Rect>,
    /// Inline label editor: the pin's path, the text, frames left to take the focus.
    pub renaming: Option<(PathBuf, String, u8)>,
    /// Pinned folders that were not found at the last check.
    missing: HashSet<PathBuf>,
    last_check: Option<Instant>,
    checking: Option<Receiver<HashSet<PathBuf>>>,
    pinning: Vec<Receiver<Checked>>,
}

impl PortalState {
    /// The pinned folder was not found at the last check.
    pub fn is_missing(&self, path: &Path) -> bool {
        self.missing.contains(path)
    }
}

/// Paths are compared case-insensitively on Windows (NTFS names are).
pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// The label shown for a pin.
pub fn label(pin: &PortalPin) -> String {
    if pin.name.trim().is_empty() { soyuz_core::entry::display_name(&pin.path) } else { pin.name.clone() }
}

/// New pins for `dirs`: duplicates (of `existing` or each other) are skipped.
pub fn new_pins(existing: &[PortalPin], dirs: &[PathBuf]) -> Vec<PortalPin> {
    let mut out: Vec<PortalPin> = Vec::new();
    for d in dirs {
        let dup = existing.iter().chain(out.iter()).any(|p| same_path(&p.path, d));
        if !dup {
            out.push(PortalPin { path: d.clone(), name: String::new() });
        }
    }
    out
}

/// Pins after moving `from` before `to` (a no-op move returns `None`).
pub fn moved(pins: &[PortalPin], from: usize, to: usize) -> Option<Vec<PortalPin>> {
    if from >= pins.len() || to == from || to == from + 1 {
        return None;
    }
    let mut v = pins.to_vec();
    let p = v.remove(from);
    let to = if to > from { to - 1 } else { to };
    v.insert(to.min(v.len()), p);
    Some(v)
}

/// Worker: keep the dropped paths that are existing local folders.
fn check_dirs(paths: Vec<PathBuf>) -> Checked {
    let dropped = paths.len();
    let dirs =
        paths.into_iter().map(|p| soyuz_fs::strip_verbatim(&p)).filter(|p| p.is_absolute() && p.is_dir()).collect();
    Checked { dirs, dropped }
}

/// Pin dropped `paths` once a worker checked them (OS drops, `PortalCmd::Pin`).
pub fn pin_dropped(app: &mut AppCore, host: &dyn Host, paths: Vec<PathBuf>) {
    let (tx, rx) = channel();
    let wake = host.wake();
    let spawned = std::thread::Builder::new().name("soyuz-portal-pin".into()).spawn(move || {
        let _ = tx.send(check_dirs(paths));
        wake();
    });
    if spawned.is_ok() {
        app.ui.sidebar.portal.pinning.push(rx);
    }
}

pub fn apply(app: &mut AppCore, host: &dyn Host, cmd: PortalCmd) {
    let pins = &app.config.sidebar.portal;
    match cmd {
        PortalCmd::Pin(paths) => pin_dropped(app, host, paths),
        PortalCmd::Unpin(path) => {
            let keep: Vec<PortalPin> = pins.iter().filter(|p| !same_path(&p.path, &path)).cloned().collect();
            crate::models::settings::update_config(app, host, |c| c.sidebar.portal = keep);
        }
        PortalCmd::Move { from, to } => {
            if let Some(v) = moved(pins, from, to) {
                crate::models::settings::update_config(app, host, |c| c.sidebar.portal = v);
            }
        }
        PortalCmd::Rename { path, name } => {
            let mut v = pins.clone();
            if let Some(p) = v.iter_mut().find(|p| same_path(&p.path, &path)) {
                let name = name.trim().to_string();
                p.name = if name == soyuz_core::entry::display_name(&p.path) { String::new() } else { name };
            }
            crate::models::settings::update_config(app, host, |c| c.sidebar.portal = v);
        }
    }
}

/// Per frame: finished pin checks and the periodic existence check (`focused`: the
/// window has the focus; later checks only run then).
pub fn pump(app: &mut AppCore, host: &dyn Host, focused: bool) {
    let st = &mut app.ui.sidebar.portal;
    let mut done: Vec<Checked> = Vec::new();
    st.pinning.retain(|rx| match rx.try_recv() {
        Ok(c) => {
            done.push(c);
            false
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => true,
        Err(_) => false,
    });
    if let Some(rx) = &st.checking {
        match rx.try_recv() {
            Ok(m) => {
                st.missing = m;
                st.checking = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => st.checking = None,
            Err(_) => {}
        }
    }
    let pins: Vec<PathBuf> = app.config.sidebar.portal.iter().map(|p| p.path.clone()).collect();
    let due = st.last_check.is_none_or(|t| t.elapsed() >= CHECK_EVERY);
    // The first check runs right away; later ones only while the window is focused.
    let may_check = st.last_check.is_none() || focused;
    if !pins.is_empty() && st.checking.is_none() && due && may_check {
        st.last_check = Some(Instant::now());
        let (tx, rx) = channel();
        let wake = host.wake();
        let spawned = std::thread::Builder::new().name("soyuz-portal-check".into()).spawn(move || {
            let missing: HashSet<PathBuf> = pins.into_iter().filter(|p| !p.is_dir()).collect();
            let _ = tx.send(missing);
            wake();
        });
        if spawned.is_ok() {
            st.checking = Some(rx);
        }
    }
    if !app.config.sidebar.portal.is_empty() && focused {
        host.redraw_after(CHECK_EVERY);
    }
    for c in done {
        finish_pin(app, host, c);
    }
}

fn finish_pin(app: &mut AppCore, host: &dyn Host, c: Checked) {
    if c.dirs.is_empty() {
        if c.dropped > 0 {
            app.ui.toasts.push(Toast::info(tr!("app-portal-folders-only")));
        }
        return;
    }
    let add = new_pins(&app.config.sidebar.portal, &c.dirs);
    if add.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("app-portal-already")));
        return;
    }
    for p in &add {
        app.ui.sidebar.portal.missing.remove(&p.path);
    }
    let msg = match add.as_slice() {
        [one] => tr!("app-portal-pinned", name = label(one)),
        many => tr!("app-portal-pinned-many", count = many.len()),
    };
    crate::models::settings::update_config(app, host, |cfg| cfg.sidebar.portal.extend(add));
    app.ui.toasts.push(Toast::success(msg));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(p: &str) -> PortalPin {
        PortalPin { path: PathBuf::from(p), name: String::new() }
    }

    #[test]
    fn duplicates_are_skipped() {
        let existing = vec![pin("/a"), pin("/b")];
        let add = new_pins(&existing, &[PathBuf::from("/b"), PathBuf::from("/c"), PathBuf::from("/c")]);
        assert_eq!(add, vec![pin("/c")]);
        if cfg!(windows) {
            assert!(new_pins(&[pin("C:\\Work")], &[PathBuf::from("c:\\work")]).is_empty());
        }
    }

    #[test]
    fn move_semantics_match_favorites() {
        let v = vec![pin("/a"), pin("/b"), pin("/c")];
        let names = |v: Vec<PortalPin>| v.into_iter().map(|p| p.path.display().to_string()).collect::<Vec<_>>();
        assert_eq!(names(moved(&v, 0, 3).unwrap()), ["/b", "/c", "/a"]);
        assert_eq!(names(moved(&v, 2, 0).unwrap()), ["/c", "/a", "/b"]);
        assert!(moved(&v, 1, 1).is_none() && moved(&v, 1, 2).is_none() && moved(&v, 5, 0).is_none());
    }

    #[test]
    fn only_existing_folders_pass_the_check() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("folder");
        std::fs::create_dir(&dir).unwrap();
        let file = d.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        let c = check_dirs(vec![dir.clone(), file, d.path().join("gone"), PathBuf::from("relative")]);
        assert_eq!(c.dropped, 4);
        assert_eq!(c.dirs, vec![soyuz_fs::strip_verbatim(&dir)]);
    }

    #[test]
    fn labels_fall_back_to_the_folder_name() {
        assert_eq!(label(&pin("/x/Projects")), "Projects");
        assert_eq!(label(&PortalPin { path: "/x/p".into(), name: "Work".into() }), "Work");
        assert_eq!(label(&PortalPin { path: "/x/p".into(), name: "  ".into() }), "p");
    }
}
