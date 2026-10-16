//! "Open with…" (`FrontendCmd::OpenWith`): the applications registered for a file's type
//! (looked up off the UI thread, [`AppsLookup`]) plus any program by path. The choice
//! becomes `Cmd::OpenWith`; nothing is launched here.
//!
//! `[open]` keeps choices: "Always use this app for .ext files" makes the application the
//! extension's default (`default_apps`, used by Enter / double-click, [`crate::ops::launch`]),
//! and with `remember_choice` the dialog preselects the application last chosen for the
//! extension ([`preselect`]). Both go through `Cmd::RememberOpenWith` ([`remember`]).
//! Files without an extension (`README`, `Makefile`, `.bashrc`) are kept together under
//! the key `"."` ([`OpenSection::NO_EXTENSION`]), and a file no application is registered
//! for opens this dialog on Enter (`[open] choose_when_unknown`, [`crate::ops::launch`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use soyuz_core::config::{DefaultApp, OpenSection};
use soyuz_fs::AppInfo;

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::host::{Host, Wake};

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

/// The commands for a choice: keep it for the file's type (`always`: as its default;
/// off: no default), then open `path` with `app`.
pub fn choice_cmds(path: &Path, app: AppInfo, always: bool) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    if let Some(ext) = ext_of(path) {
        cmds.push(Cmd::RememberOpenWith { ext, app: app.clone(), always });
    }
    cmds.push(open_cmd(path, app));
    cmds
}

/// The key a choice for `path` is kept under: its extension, or
/// [`OpenSection::NO_EXTENSION`] for a file without one (`None`: no file name).
pub fn ext_of(path: &Path) -> Option<String> {
    path.file_name().and_then(|n| n.to_str()).map(OpenSection::key_of)
}

/// The "Always use this app" checkbox's label for `path` ("… for .ext files", or "… for
/// files without an extension").
pub fn always_label(path: &Path) -> Option<String> {
    ext_of(path).map(|ext| match ext.as_str() {
        OpenSection::NO_EXTENSION => tr!("dialogs-openwith-always-noext").to_string(),
        _ => tr!("dialogs-openwith-always", ext = ext).to_string(),
    })
}

/// The application of a `[[open.default_apps]]` rule.
pub fn rule_app(rule: &DefaultApp) -> AppInfo {
    AppInfo {
        id: rule.id.clone(),
        name: rule.name.clone(),
        exec: rule.program.clone(),
        args: rule.args.clone(),
        icon: None,
        launchable: true,
    }
}

/// A rule (without extensions) for `app`.
pub fn rule_of(app: &AppInfo) -> DefaultApp {
    DefaultApp {
        extensions: Vec::new(),
        name: app.name.clone(),
        program: app.exec.clone(),
        args: app.args.clone(),
        id: app.id.clone(),
    }
}

/// Whether `app` can be a default: one that opens through the system's handler
/// (`launchable == false`, e.g. a Store app) cannot be launched by Soyuz directly.
pub fn can_be_default(app: &AppInfo) -> bool {
    app.launchable
}

/// What the dialog starts with once the applications are known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preselect {
    /// The row to select.
    pub selected: Option<usize>,
    /// A program path for the "another program" field (the application is not listed).
    pub custom: String,
    /// The "Always use this app" checkbox (the extension has a default).
    pub always: bool,
}

/// The starting choice for `path`: the extension's default application, else (with
/// `remember_choice`) the one last chosen, else the first listed.
pub fn preselect(cfg: &OpenSection, path: &Path, apps: &[AppInfo]) -> Preselect {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let rule = cfg.app_for(name);
    let find = |key: &str| {
        apps.iter().position(|a| (!a.id.is_empty() && a.id == key) || a.exec.as_os_str() == std::ffi::OsStr::new(key))
    };
    let first = (!apps.is_empty()).then_some(0);
    let mut out = Preselect { always: rule.is_some(), ..Preselect::default() };
    let wanted = match rule {
        Some(r) => Some(r.key()),
        None if cfg.remember_choice => ext_of(path).and_then(|e| cfg.last_used.get(&e).cloned()),
        None => None,
    };
    match wanted.as_deref().map(|k| (k, find(k))) {
        Some((_, Some(i))) => out.selected = Some(i),
        // Not listed (a program chosen by path, or uninstalled): its path in the field.
        Some((key, None)) => match rule {
            Some(r) => out.custom = r.program.display().to_string(),
            None if Path::new(key).is_absolute() => out.custom = key.to_string(),
            None => out.selected = first,
        },
        None => out.selected = first,
    }
    out
}

/// Keep a choice (`Cmd::RememberOpenWith`): the last-used application for `ext` (with
/// `remember_choice`), and `app` as its default (`always`) or no default. Saved like any
/// settings change (nothing is written when nothing changed).
pub fn remember(core: &mut AppCore, host: &dyn Host, ext: &str, app: &AppInfo, always: bool) {
    let rule = rule_of(app);
    crate::models::settings::update_config(core, host, |c| {
        if c.open.remember_choice {
            c.open.last_used.insert(ext.to_string(), rule.key());
        }
        let current = c.open.rule_for(ext).cloned();
        match (always && can_be_default(app), current) {
            (true, Some(r)) if r.same_app(&rule) => {}
            (true, _) => c.open.set_default(ext, Some(rule.clone())),
            (false, Some(_)) => c.open.set_default(ext, None),
            (false, None) => {}
        }
    });
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

    fn app(id: &str, exec: &str) -> AppInfo {
        AppInfo { id: id.into(), ..custom_app(Path::new(exec)) }
    }

    #[test]
    fn preselects_the_default_then_the_last_used() {
        let tool = std::env::temp_dir().join("soyuz-tool");
        let tool_key = tool.display().to_string();
        let apps = vec![app("photos", "/bin/photos"), app("gimp.desktop", "/usr/bin/gimp")];
        let png = Path::new("a.png");
        let mut cfg = OpenSection::default();
        assert_eq!(preselect(&cfg, png, &apps), Preselect { selected: Some(0), ..Default::default() });
        cfg.last_used.insert("png".into(), "gimp.desktop".into());
        assert_eq!(preselect(&cfg, png, &apps).selected, Some(1), "the last used");
        cfg.remember_choice = false;
        assert_eq!(preselect(&cfg, png, &apps).selected, Some(0), "remembering is off");
        cfg.remember_choice = true;
        cfg.last_used.insert("png".into(), tool_key.clone());
        let p = preselect(&cfg, png, &apps);
        assert_eq!((p.selected, p.custom.as_str()), (None, tool_key.as_str()), "a program by path");
        cfg.set_default("png", Some(rule_of(&apps[1])));
        let p = preselect(&cfg, Path::new("B.PNG"), &apps);
        assert_eq!((p.selected, p.always), (Some(1), true), "the default wins");
        cfg.set_default("png", Some(rule_of(&custom_app(&tool))));
        let p = preselect(&cfg, png, &apps);
        assert!(p.always && p.selected.is_none() && p.custom == tool_key, "{p:?}");
        assert!(!preselect(&cfg, Path::new("README"), &apps).always);
        // Files without an extension share the "." key.
        cfg.last_used.insert(".".into(), "gimp.desktop".into());
        assert_eq!(preselect(&cfg, Path::new("Makefile"), &apps).selected, Some(1));
        cfg.set_default(".", Some(rule_of(&custom_app(&tool))));
        let p = preselect(&cfg, Path::new("README"), &[]);
        assert!(p.always && p.custom == tool_key, "an unknown type with no listed apps: {p:?}");
    }

    #[test]
    fn choices_keep_by_extension() {
        let a = app("vlc", "/usr/bin/vlc");
        let cmds = choice_cmds(Path::new("clip.MP4"), a.clone(), true);
        assert!(matches!(&cmds[0], Cmd::RememberOpenWith { ext, always: true, .. } if ext == "mp4"));
        assert!(matches!(&cmds[1], Cmd::OpenWith { .. }));
        let cmds = choice_cmds(Path::new("Makefile"), a.clone(), true);
        assert!(matches!(&cmds[0], Cmd::RememberOpenWith { ext, .. } if ext == "."), "no extension: the . key");
        assert_eq!(rule_app(&rule_of(&a)), a);
        assert!(always_label(Path::new("clip.mp4")).is_some_and(|l| l.contains("mp4")));
        assert!(always_label(Path::new("Makefile")).is_some_and(|l| !l.contains(".{")));
        assert_ne!(always_label(Path::new("Makefile")), always_label(Path::new("x.mk")));
    }
}
