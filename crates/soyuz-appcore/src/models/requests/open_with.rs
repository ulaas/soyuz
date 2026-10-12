//! "Open with…" (`FrontendCmd::OpenWith`): the applications registered for a file's type
//! (looked up off the UI thread, [`AppsLookup`]) plus any program by path. The choice
//! becomes `Cmd::OpenWith`; nothing is launched here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use soyuz_fs::AppInfo;

use crate::cmd::Cmd;
use crate::host::Wake;

/// The dialog's title for `path`.
pub fn title(path: &Path) -> String {
    let name = soyuz_core::entry::display_name(path);
    tr!("dialogs-openwith-title", name = soyuz_core::naming::reveal_controls(&name).into_owned())
}

/// An [`AppInfo`] for a program chosen by path (the file is appended as its argument).
pub fn custom_app(exe: &Path) -> AppInfo {
    let name = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| exe.display().to_string());
    AppInfo {
        id: exe.display().to_string(),
        name,
        exec: exe.to_path_buf(),
        args: Vec::new(),
        icon: None,
        launchable: true,
    }
}

/// Parse the "another program" field: trims whitespace and surrounding quotes.
pub fn parse_program(text: &str) -> Option<PathBuf> {
    let t = text.trim().trim_matches('"').trim();
    (!t.is_empty()).then(|| PathBuf::from(t))
}

/// How an application is listed (one that is not launchable directly opens with the
/// default handler).
pub fn app_label(app: &AppInfo) -> String {
    if app.launchable { app.name.clone() } else { tr!("dialogs-openwith-default-handler", app = &app.name) }
}

/// The program path's placeholder.
pub fn program_hint() -> &'static str {
    if cfg!(windows) { r"C:\Program Files\App\app.exe" } else { "/usr/bin/app" }
}

/// The application to open with: a typed program (which must be an absolute path to a
/// file) wins over the list's `selected` row. `Err` is the message to show.
pub fn choose(custom: &str, selected: Option<usize>, apps: Option<&[AppInfo]>) -> Result<AppInfo, String> {
    if let Some(exe) = parse_program(custom) {
        if !exe.is_absolute() || !exe.is_file() {
            return Err(tr!("dialogs-openwith-not-program", path = exe.display().to_string()));
        }
        return Ok(custom_app(&exe));
    }
    selected.and_then(|i| apps.and_then(|l| l.get(i).cloned())).ok_or_else(|| tr!("dialogs-openwith-choose").into())
}

/// The command for the chosen application.
pub fn open_cmd(path: &Path, app: AppInfo) -> Cmd {
    Cmd::OpenWith { path: path.to_path_buf(), app }
}

/// A lookup of the applications for one file, on a worker thread.
#[derive(Clone, Debug, Default)]
pub struct AppsLookup {
    slot: Arc<Mutex<Option<Vec<AppInfo>>>>,
}

impl AppsLookup {
    /// Start looking up the applications for `path`; `wake` runs when they are known.
    pub fn start(path: &Path, wake: Wake) -> AppsLookup {
        let me = AppsLookup::default();
        let (slot, path) = (me.slot.clone(), path.to_path_buf());
        let spawned = std::thread::Builder::new().name("soyuz-apps-for".into()).spawn(move || {
            let apps = soyuz_fs::launch::apps_for(&path);
            *slot.lock() = Some(apps);
            wake();
        });
        if spawned.is_err() {
            *me.slot.lock() = Some(Vec::new());
        }
        me
    }

    /// Already known applications (a test or a cache).
    pub fn ready(apps: Vec<AppInfo>) -> AppsLookup {
        AppsLookup { slot: Arc::new(Mutex::new(Some(apps))) }
    }

    /// The applications, `None` while the lookup runs.
    pub fn get(&self) -> Option<Vec<AppInfo>> {
        self.slot.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_field() {
        assert_eq!(parse_program("  \"C:\\A B\\x.exe\" "), Some(PathBuf::from("C:\\A B\\x.exe")));
        assert_eq!(parse_program("   "), None);
        let a = custom_app(Path::new("/usr/bin/gimp"));
        assert_eq!(a.name, "gimp");
        assert!(a.args.is_empty() && a.launchable);
    }

    #[test]
    fn choosing_validates_without_launching() {
        let apps = vec![custom_app(Path::new("/usr/bin/one")), custom_app(Path::new("/usr/bin/two"))];
        assert_eq!(choose("", Some(1), Some(&apps)).unwrap().name, "two");
        assert!(choose("", None, Some(&apps)).is_err());
        assert!(choose("", Some(5), Some(&apps)).is_err());
        assert!(choose("relative.exe", Some(0), Some(&apps)).is_err(), "a typed program must be absolute");
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("tool.bin");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(choose(&exe.display().to_string(), Some(0), Some(&apps)).unwrap().exec, exe);
        assert!(choose(&dir.path().display().to_string(), None, None).is_err(), "a folder is not a program");
        let mut na = apps[0].clone();
        na.launchable = false;
        assert!(app_label(&na).contains("one") && app_label(&na) != "one");
        assert_eq!(AppsLookup::ready(apps.clone()).get(), Some(apps));
    }
}
