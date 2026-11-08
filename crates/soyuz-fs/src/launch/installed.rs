//! The applications installed on this system, for the command palette's app launcher.
//!
//! * Windows: the shortcuts of the Start menu (the user's and all users' `Programs`
//!   folders). Uninstallers and shortcuts whose target is gone are left out; a shortcut
//!   starts as a double-click on it would ([`super::open_default`]).
//! * Linux / BSD: the desktop entries an "Open with" menu would list (launchable, not
//!   `NoDisplay`, shown in this desktop), started without files (D-Bus activation or
//!   `Exec=`, terminal entries in a terminal).
//! * macOS: the `.app` bundles in the Applications folders, opened through LaunchServices.
//!
//! Scanning reads the disk: call [`installed_apps`] off the UI thread.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::FsResult;

/// One application the palette can start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledApp {
    /// Stable id (the desktop file id, else the path of the shortcut or bundle).
    pub id: String,
    pub name: String,
    /// A short description (the shortcut's comment, the entry's generic name or comment),
    /// else where it was found.
    pub detail: String,
    /// More words to match (Start menu folder, program file name, `Keywords=`).
    pub keywords: String,
    /// The shortcut, desktop file or bundle.
    pub source: PathBuf,
}

/// Every application found, sorted by name (case-insensitive), duplicates by name removed.
pub fn installed_apps() -> Vec<InstalledApp> {
    let mut v = scan();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    v.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    v
}

/// Start `app` without files.
pub fn launch_app(app: &InstalledApp) -> FsResult<()> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let index = super::linux::app_index();
        match index.get(&app.id).filter(|a| a.entry.launchable()) {
            Some(entry) => super::linux::launch_entry(entry, &[], "launch", &app.source),
            None => Err(soyuz_core::FsError::with_message(
                soyuz_core::FsErrorKind::NotFound,
                "launch",
                Some(&app.source),
                "the application is no longer installed",
            )),
        }
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        super::open_default(&app.source)
    }
}

#[cfg(windows)]
fn scan() -> Vec<InstalledApp> {
    let mut out = Vec::new();
    for var in ["APPDATA", "ProgramData"] {
        let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()) else { continue };
        let root = PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs");
        for lnk in walk(&root, 4, &|p| ext_is(p, "lnk")) {
            if let Some(a) = start_menu_app(&root, &lnk) {
                out.push(a);
            }
        }
    }
    out
}

/// A Start menu shortcut, unless it is an uninstaller or its target is missing.
#[cfg(windows)]
fn start_menu_app(root: &Path, lnk: &Path) -> Option<InstalledApp> {
    let name = lnk.file_stem()?.to_string_lossy().into_owned();
    if is_uninstaller(&name) {
        return None;
    }
    let (target, sc) = super::lnk::resolve(lnk).ok()?;
    let target = target.filter(|t| crate::list::exists_nofollow(t))?;
    let folder = lnk
        .parent()
        .and_then(|d| d.strip_prefix(root).ok())
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let program = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if is_uninstaller(&program) {
        return None;
    }
    let comment =
        sc.name.filter(|c| !c.trim().is_empty() && !c.starts_with('@') && !c.trim().eq_ignore_ascii_case(&name));
    Some(InstalledApp {
        id: lnk.to_string_lossy().into_owned(),
        detail: comment.unwrap_or_else(|| if folder.is_empty() { program.clone() } else { folder.clone() }),
        keywords: format!("{folder} {program}"),
        name,
        source: lnk.to_path_buf(),
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn scan() -> Vec<InstalledApp> {
    let desktops = super::linux::desktop::current_desktops();
    super::linux::app_index()
        .entries
        .iter()
        .filter(|a| a.entry.listed(&desktops) && !a.entry.name.is_empty())
        .map(|a| {
            let e = &a.entry;
            InstalledApp {
                id: a.id.clone(),
                name: e.name.clone(),
                detail: e.generic_name.clone().or_else(|| e.comment.clone()).unwrap_or_default(),
                keywords: format!(
                    "{} {} {}",
                    e.keywords.join(" "),
                    e.comment.as_deref().unwrap_or_default(),
                    e.program().unwrap_or_default()
                ),
                source: a.file.clone(),
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn scan() -> Vec<InstalledApp> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Library/CoreServices/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    let mut out = Vec::new();
    for root in roots {
        for app in walk(&root, 2, &|p| ext_is(p, "app")) {
            let Some(name) = app.file_stem().map(|n| n.to_string_lossy().into_owned()) else { continue };
            let folder = app.parent().map(|d| d.display().to_string()).unwrap_or_default();
            out.push(InstalledApp {
                id: app.to_string_lossy().into_owned(),
                name,
                detail: folder.clone(),
                keywords: folder,
                source: app,
            });
        }
    }
    out
}

#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn ext_is(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The files under `root` that `hit` accepts, at most `depth` folders deep. A hit is not
/// descended into (an `.app` bundle is a folder).
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn walk(root: &Path, depth: usize, hit: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, d)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for de in rd.flatten() {
            let p = de.path();
            if hit(&p) {
                out.push(p);
            } else if d < depth && de.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((p, d + 1));
            }
        }
    }
    out
}

/// "Uninstall Foo", "Foo Uninstaller", `unins000.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_uninstaller(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("uninstall") || n.starts_with("unins0") || n.contains("kaldır")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstallers_are_recognized() {
        assert!(is_uninstaller("Uninstall Foo"));
        assert!(is_uninstaller("unins000.exe"));
        assert!(is_uninstaller("Foo Uninstaller"));
        assert!(!is_uninstaller("Visual Studio Code"));
        assert!(!is_uninstaller("Installer Helper"));
    }

    #[test]
    fn walk_finds_hits_without_descending_into_them() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir_all(r.join("Tools/Deep/Deeper")).unwrap();
        std::fs::create_dir_all(r.join("Bundle.app/Contents")).unwrap();
        std::fs::write(r.join("Bundle.app/Contents/Inner.app"), "").unwrap();
        std::fs::write(r.join("a.lnk"), "").unwrap();
        std::fs::write(r.join("Tools/b.LNK"), "").unwrap();
        std::fs::write(r.join("Tools/Deep/Deeper/c.lnk"), "").unwrap();
        std::fs::write(r.join("Tools/readme.txt"), "").unwrap();
        let mut lnks: Vec<String> = walk(r, 1, &|p| ext_is(p, "lnk"))
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        lnks.sort();
        assert_eq!(lnks, ["a.lnk", "b.LNK"]);
        let apps = walk(r, 3, &|p| ext_is(p, "app"));
        assert_eq!(apps, [r.join("Bundle.app")]);
    }

    /// The real scan runs and returns sorted names without duplicates (whatever is
    /// installed on the test machine; it only reads).
    #[test]
    fn installed_apps_are_sorted_and_unique() {
        let v = installed_apps();
        for w in v.windows(2) {
            assert!(w[0].name.to_lowercase() <= w[1].name.to_lowercase(), "{} / {}", w[0].name, w[1].name);
            assert!(!w[0].name.eq_ignore_ascii_case(&w[1].name));
        }
    }
}
