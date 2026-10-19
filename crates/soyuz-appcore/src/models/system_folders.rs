//! System folders below the favorites (`[sidebar] system_folders`,
//! `system_folder_list`): AppData, ProgramData, Temp … on Windows; `~/.config`,
//! `~/.local/share`, `/etc` … on Linux. Both frontends draw them after a divider whose
//! hide button asks first ([`Cmd::AskHideSystemFolders`]) and says where to turn them
//! back on.
//!
//! The paths come from environment variables and the XDG base-directory rules (no shell
//! known-folder API); which of them exist is checked once on a worker thread, never on
//! the UI thread. Until that check finishes none are listed.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use soyuz_core::config::Sidebar;

use crate::cmd::Cmd;
use crate::icon::IconId;
use crate::models::frontend::FrontendCmd;
use crate::state::session::FavoriteEntry;

/// One system folder of this OS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemFolder {
    /// Stable id (`[sidebar] system_folder_list`).
    pub id: &'static str,
    pub icon: IconId,
    pub path: PathBuf,
}

impl SystemFolder {
    /// The label in the current UI language.
    pub fn label(&self) -> String {
        label(self.id)
    }
}

/// The Windows system folders, in sidebar order.
const WINDOWS: &[&str] = &[
    "appdata",
    "local_appdata",
    "locallow",
    "programdata",
    "program_files",
    "program_files_x86",
    "temp",
    "public",
    "start_menu",
    "startup",
    "fonts",
    "hosts",
    "windows",
    "soyuz_config",
];

/// The Linux system folders, in sidebar order.
const LINUX: &[&str] = &[
    "xdg_config",
    "xdg_data",
    "xdg_cache",
    "xdg_state",
    "local_bin",
    "applications",
    "autostart",
    "flatpak_apps",
    "temp",
    "etc",
    "opt",
    "var_log",
    "soyuz_config",
];

/// The ids of this OS's system folders, in sidebar order (none on macOS yet).
pub fn ids() -> &'static [&'static str] {
    if cfg!(windows) {
        WINDOWS
    } else if cfg!(target_os = "linux") {
        LINUX
    } else {
        &[]
    }
}

/// Label of system folder `id`.
pub fn label(id: &str) -> String {
    // i18n-dynamic: app-sysfolder-
    crate::i18n::t_dyn(&format!("app-sysfolder-{}", id.replace('_', "-"))).to_string()
}

fn icon(id: &str) -> IconId {
    use IconId as I;
    match id {
        "appdata" | "local_appdata" | "locallow" | "xdg_config" => I::GearSix,
        "programdata" | "xdg_data" => I::Stack,
        "program_files" | "program_files_x86" | "applications" => I::AppWindow,
        "flatpak_apps" | "opt" => I::Cube,
        "temp" | "xdg_cache" => I::Broom,
        "xdg_state" => I::Pulse,
        "public" => I::Briefcase,
        "start_menu" => I::List,
        "startup" | "autostart" => I::Power,
        "fonts" => I::TextAa,
        "hosts" => I::Globe,
        "windows" => I::DesktopTower,
        "local_bin" => I::Terminal,
        "var_log" => I::Scroll,
        "etc" | "soyuz_config" => I::Sliders,
        _ => I::FolderSimple,
    }
}

/// The path of `id` on this OS: `env` reads an environment variable (empty = unset),
/// `home` is the home folder, `config` Soyuz's config folder. `None`: unknown here.
fn resolve(id: &str, env: &dyn Fn(&str) -> Option<PathBuf>, home: &Path, config: &Path) -> Option<PathBuf> {
    // XDG base directories: an absolute `$XDG_*_HOME`, else the spec's default.
    let xdg = |var: &str, default: &str| env(var).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(default));
    let in_root = |sub: &str| env("SystemRoot").or_else(|| env("windir")).map(|r| r.join(sub));
    let start_menu = || env("APPDATA").map(|a| a.join(r"Microsoft\Windows\Start Menu\Programs"));
    if cfg!(windows) {
        match id {
            "appdata" => env("APPDATA"),
            "local_appdata" => env("LOCALAPPDATA"),
            "locallow" => env("USERPROFILE").map(|u| u.join(r"AppData\LocalLow")),
            "programdata" => env("ProgramData"),
            "program_files" => env("ProgramFiles"),
            "program_files_x86" => env("ProgramFiles(x86)"),
            "temp" => env("TEMP").or_else(|| env("TMP")),
            "public" => env("PUBLIC"),
            "start_menu" => start_menu(),
            "startup" => start_menu().map(|s| s.join("Startup")),
            "fonts" => in_root("Fonts"),
            "hosts" => in_root(r"System32\drivers\etc"),
            "windows" => in_root(""),
            "soyuz_config" => Some(config.to_path_buf()),
            _ => None,
        }
        // `in_root("")` ends in a separator: normalize.
        .map(|p| p.components().collect())
    } else if cfg!(target_os = "linux") {
        match id {
            "xdg_config" => Some(xdg("XDG_CONFIG_HOME", ".config")),
            "xdg_data" => Some(xdg("XDG_DATA_HOME", ".local/share")),
            "xdg_cache" => Some(xdg("XDG_CACHE_HOME", ".cache")),
            "xdg_state" => Some(xdg("XDG_STATE_HOME", ".local/state")),
            "local_bin" => Some(home.join(".local/bin")),
            "applications" => Some(xdg("XDG_DATA_HOME", ".local/share").join("applications")),
            "autostart" => Some(xdg("XDG_CONFIG_HOME", ".config").join("autostart")),
            "flatpak_apps" => Some(home.join(".var/app")),
            "temp" => Some(env("TMPDIR").filter(|p| p.is_absolute()).unwrap_or_else(|| "/tmp".into())),
            "etc" => Some("/etc".into()),
            "opt" => Some("/opt".into()),
            "var_log" => Some("/var/log".into()),
            "soyuz_config" => Some(config.to_path_buf()),
            _ => None,
        }
    } else {
        None
    }
}

/// This OS's system folders with their paths (existing or not).
fn catalogue(env: &dyn Fn(&str) -> Option<PathBuf>, home: &Path, config: &Path) -> Vec<SystemFolder> {
    ids()
        .iter()
        .filter_map(|&id| Some(SystemFolder { id, icon: icon(id), path: resolve(id, env, home, config)? }))
        .collect()
}

static PROBED: OnceLock<Vec<SystemFolder>> = OnceLock::new();
static PROBE_STARTED: std::sync::Once = std::sync::Once::new();

/// The system folders that exist on this machine (empty until the worker-thread check
/// has run; the first call starts it).
pub fn available() -> &'static [SystemFolder] {
    if let Some(v) = PROBED.get() {
        return v;
    }
    PROBE_STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("soyuz-system-folders".into()).spawn(|| {
            let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
            let config = soyuz_core::config::config_dir();
            let found = catalogue(&env, &soyuz_fs::home(), &config).into_iter().filter(|f| f.path.is_dir()).collect();
            let _ = PROBED.set(found);
        });
    });
    &[]
}

/// The existence check has finished.
pub fn probed() -> bool {
    PROBED.get().is_some()
}

/// The system folders the sidebar shows: `[sidebar] system_folders` on, listed in
/// `system_folder_list`, existing, and not a favorite already.
pub fn shown(cfg: &Sidebar, favorites: &[FavoriteEntry]) -> Vec<SystemFolder> {
    if !cfg.system_folders {
        return Vec::new();
    }
    pick(available(), cfg, favorites)
}

fn pick(all: &[SystemFolder], cfg: &Sidebar, favorites: &[FavoriteEntry]) -> Vec<SystemFolder> {
    all.iter()
        .filter(|f| cfg.system_folder_list.iter().any(|id| id == f.id))
        .filter(|f| !favorites.iter().any(|fav| fav.path == f.path))
        .cloned()
        .collect()
}

/// [`Cmd::AskHideSystemFolders`]: confirm hiding them, saying where to show them again.
pub fn ask_hide() -> FrontendCmd {
    FrontendCmd::Confirm {
        title: tr!("app-sysfolders-hide-title").into(),
        message: tr!("app-sysfolders-hide-message").into(),
        confirm: tr!("app-sysfolders-hide-confirm").into(),
        on_confirm: Cmd::ShowSystemFolders(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<PathBuf> {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| PathBuf::from(v))
    }

    #[test]
    fn every_id_has_a_label_and_an_icon() {
        for id in WINDOWS.iter().chain(LINUX) {
            assert!(!label(id).starts_with("app-sysfolder"), "{id} has no label");
            assert_ne!(icon(id), IconId::FolderSimple, "{id} has no icon");
        }
    }

    #[test]
    fn default_list_names_known_ids() {
        let cfg = Sidebar::default();
        assert!(cfg.system_folders);
        for id in &cfg.system_folder_list {
            assert!(WINDOWS.iter().chain(LINUX).any(|k| k == id), "unknown default {id}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_come_from_the_environment() {
        let env = env_of(&[
            ("APPDATA", r"C:\Users\me\AppData\Roaming"),
            ("ProgramData", r"C:\ProgramData"),
            ("SystemRoot", r"C:\Windows"),
        ]);
        let cat = catalogue(&env, Path::new(r"C:\Users\me"), Path::new(r"C:\cfg"));
        let path = |id: &str| cat.iter().find(|f| f.id == id).map(|f| f.path.clone());
        assert_eq!(path("appdata"), Some(PathBuf::from(r"C:\Users\me\AppData\Roaming")));
        assert_eq!(path("programdata"), Some(PathBuf::from(r"C:\ProgramData")));
        assert_eq!(path("hosts"), Some(PathBuf::from(r"C:\Windows\System32\drivers\etc")));
        assert_eq!(path("windows"), Some(PathBuf::from(r"C:\Windows")));
        assert_eq!(
            path("startup"),
            Some(PathBuf::from(r"C:\Users\me\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup"))
        );
        assert_eq!(path("local_appdata"), None, "unset variable: left out");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_paths_follow_xdg() {
        let env = env_of(&[("XDG_CONFIG_HOME", "/cfg"), ("XDG_CACHE_HOME", "relative/ignored")]);
        let cat = catalogue(&env, Path::new("/home/me"), Path::new("/home/me/.config/soyuz"));
        let path = |id: &str| cat.iter().find(|f| f.id == id).map(|f| f.path.clone());
        assert_eq!(path("xdg_config"), Some(PathBuf::from("/cfg")));
        assert_eq!(path("autostart"), Some(PathBuf::from("/cfg/autostart")));
        assert_eq!(path("xdg_cache"), Some(PathBuf::from("/home/me/.cache")), "relative values are ignored");
        assert_eq!(path("xdg_data"), Some(PathBuf::from("/home/me/.local/share")));
        assert_eq!(path("temp"), Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn hiding_asks_first_and_says_where_to_turn_them_back_on() {
        let FrontendCmd::Confirm { message, on_confirm, .. } = ask_hide() else { panic!("a confirm") };
        assert!(matches!(on_confirm, Cmd::ShowSystemFolders(false)));
        assert!(message.contains(tr!("settings-views-sysfolders")), "{message}");
    }

    #[test]
    fn shown_follows_the_list_and_skips_favorites() {
        let f = |id: &'static str, p: &str| SystemFolder { id, icon: icon(id), path: PathBuf::from(p) };
        let all = vec![f("appdata", "/a"), f("programdata", "/p"), f("temp", "/t")];
        let cfg = Sidebar { system_folder_list: vec!["temp".into(), "appdata".into()], ..Default::default() };
        let favs = vec![FavoriteEntry { name: "T".into(), path: "/t".into() }];
        let got: Vec<&str> = pick(&all, &cfg, &favs).iter().map(|f| f.id).collect();
        assert_eq!(got, ["appdata"], "catalogue order, listed only, favorites skipped");
    }
}
