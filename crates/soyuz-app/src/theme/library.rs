//! The theme library and the OS appearance watcher.
//!
//! One background thread (`soyuz-theme-watch`) polls every ~1.5 s:
//! - `<config dir>/themes/*.toml`: user themes are (re)parsed when a file's size or
//!   mtime changes, so editing a theme file updates the window live;
//! - the OS accent color and dark / light app mode (registry on Windows).
//!
//! Results are published behind a version counter; [`sync`] (once per frame, cheap)
//! reinstalls the palettes when something the current settings depend on changed and
//! makes `theme = "system"` follow the OS switch even when the windowing layer does
//! not report it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use egui::{Color32, Context, Theme, ThemePreference};
use parking_lot::RwLock;

use super::themes::{self, ThemeDef};

/// A user theme file that failed to load.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeError {
    pub file: PathBuf,
    pub error: String,
}

#[derive(Default)]
struct Library {
    /// User themes with their warnings, sorted by name.
    user: Vec<(ThemeDef, Vec<String>)>,
    errors: Vec<ThemeError>,
    /// File signatures of the last scan.
    sig: HashMap<PathBuf, (u64, Option<SystemTime>)>,
}

static LIB: OnceLock<RwLock<Library>> = OnceLock::new();
static VERSION: AtomicU64 = AtomicU64::new(1);
/// `0x01RRGGBB` when known, 0 otherwise.
static OS_ACCENT: AtomicU32 = AtomicU32::new(0);
/// 0 unknown, 1 dark, 2 light.
static OS_LIGHT: AtomicU8 = AtomicU8::new(0);
static STARTED: OnceLock<()> = OnceLock::new();

fn lib() -> &'static RwLock<Library> {
    LIB.get_or_init(|| RwLock::new(Library::default()))
}

/// `<config dir>/themes`.
pub fn themes_dir() -> PathBuf {
    // `config_dir` probes for the portable marker; resolve it once.
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| soyuz_core::config::config_dir().join("themes")).clone()
}

/// Bumped whenever user themes or the OS appearance change.
pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

/// The OS accent color, when known.
pub fn os_accent() -> Option<Color32> {
    let v = OS_ACCENT.load(Ordering::Relaxed);
    (v != 0).then(|| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// The OS app mode: `Some(Theme::Light)` / `Some(Theme::Dark)`, `None` when unknown.
pub fn os_theme() -> Option<Theme> {
    match OS_LIGHT.load(Ordering::Relaxed) {
        1 => Some(Theme::Dark),
        2 => Some(Theme::Light),
        _ => None,
    }
}

/// Built-in themes followed by user themes (a user theme with a built-in's id replaces it).
pub fn all() -> Vec<ThemeDef> {
    let l = lib().read();
    let mut out: Vec<ThemeDef> =
        themes::builtins().iter().filter(|b| !l.user.iter().any(|(u, _)| u.id == b.id)).cloned().collect();
    out.extend(l.user.iter().map(|(t, _)| t.clone()));
    out
}

/// A theme by id (user themes first).
pub fn find(id: &str) -> Option<ThemeDef> {
    let id = themes::slug(id);
    let l = lib().read();
    l.user
        .iter()
        .map(|(t, _)| t)
        .find(|t| t.id == id)
        .or_else(|| themes::builtins().iter().find(|t| t.id == id))
        .cloned()
}

/// Warnings of a user theme (empty for built-ins).
pub fn warnings_of(id: &str) -> Vec<String> {
    lib().read().user.iter().find(|(t, _)| t.id == id).map(|(_, w)| w.clone()).unwrap_or_default()
}

/// Theme files that failed to parse.
pub fn errors() -> Vec<ThemeError> {
    lib().read().errors.clone()
}

/// Load one user theme synchronously (startup, when the configured theme is a user
/// theme the watcher has not scanned yet). One small file read.
pub fn ensure_loaded(id: &str) {
    if find(id).is_some() {
        return;
    }
    let path = themes_dir().join(format!("{}.toml", themes::slug(id)));
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(t) = themes::parse(id, &text)
    {
        let mut l = lib().write();
        l.user.push(t);
        l.user.sort_by_key(|a| a.0.name.to_lowercase());
    }
}

/// Start the watcher thread (idempotent).
pub fn start(ctx: &Context) {
    if STARTED.set(()).is_err() {
        return;
    }
    // Two registry DWORD reads: cheap enough to have the OS accent for the first frame.
    poll_os();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("soyuz-theme-watch".into()).spawn(move || {
        let dir = themes_dir();
        loop {
            let mut changed = poll_os();
            changed |= scan(&dir);
            if changed {
                VERSION.fetch_add(1, Ordering::Relaxed);
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(1500));
        }
    });
    if let Err(e) = spawned {
        log::warn!("cannot start the theme watcher: {e}");
    }
}

/// Re-read the OS accent / app mode. Returns whether either changed.
fn poll_os() -> bool {
    let accent = soyuz_fs::os_theme::accent_color()
        .map_or(0, |[r, g, b]| 0x0100_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b));
    let light = match soyuz_fs::os_theme::apps_use_light_theme() {
        Some(true) => 2,
        Some(false) => 1,
        None => 0,
    };
    let a = OS_ACCENT.swap(accent, Ordering::Relaxed) != accent;
    let l = OS_LIGHT.swap(light, Ordering::Relaxed) != light;
    a || l
}

/// Rescan the themes folder; returns whether anything changed.
fn scan(dir: &Path) -> bool {
    let mut sig = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml"))
                && let Ok(m) = e.metadata()
                && m.is_file()
            {
                sig.insert(p, (m.len(), m.modified().ok()));
            }
        }
    }
    if lib().read().sig == sig {
        return false;
    }
    let mut user = Vec::new();
    let mut errors = Vec::new();
    for p in sig.keys() {
        let id = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let parsed = std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| themes::parse(&id, &t));
        match parsed {
            Ok(t) => user.push(t),
            Err(error) => errors.push(ThemeError { file: p.clone(), error }),
        }
    }
    user.sort_by_key(|a| a.0.name.to_lowercase());
    errors.sort_by(|a, b| a.file.cmp(&b.file));
    for e in &errors {
        log::warn!("theme {}: {}", e.file.display(), e.error);
    }
    let mut l = lib().write();
    *l = Library { user, errors, sig };
    true
}

/// Install a theme file's text into the themes folder as `<id>.toml` (import). Returns
/// the theme on success. Runs the write on the store worker.
pub fn import(text: &str, file_stem: &str) -> Result<(ThemeDef, Vec<String>), String> {
    let (t, w) = themes::parse(file_stem, text)?;
    let path = themes_dir().join(format!("{}.toml", t.id));
    crate::services::store_file::save_async(path, text.as_bytes().to_vec());
    // Visible right away; the watcher picks up the file itself on its next pass.
    {
        let mut l = lib().write();
        l.user.retain(|(u, _)| u.id != t.id);
        l.user.push((t.clone(), w.clone()));
        l.user.sort_by_key(|a| a.0.name.to_lowercase());
    }
    VERSION.fetch_add(1, Ordering::Relaxed);
    Ok((t, w))
}

#[derive(Clone, Copy, PartialEq)]
struct Seen(u64);

/// Per-frame: reinstall the palettes when user themes or the OS accent changed, and
/// follow the OS dark / light switch for `theme = "system"`.
pub fn sync(ctx: &Context, config: &soyuz_core::config::Config, preference: ThemePreference) {
    let id = egui::Id::new("soyuz.theme.library.seen");
    let v = version();
    let seen: Option<Seen> = ctx.data(|d| d.get_temp(id));
    if seen != Some(Seen(v)) {
        ctx.data_mut(|d| d.insert_temp(id, Seen(v)));
        if seen.is_some() && depends_on_library(&config.appearance) {
            for w in super::install(ctx, &config.appearance, &config.theme.colors) {
                log::warn!("{w}");
            }
            ctx.set_theme(preference);
        }
    }
    if preference == ThemePreference::System
        && let Some(os) = os_theme()
        && ctx.theme() != os
    {
        // The windowing layer reports no (or a stale) system theme: use ours. The app's
        // own preference stays "system", so this keeps following the OS.
        ctx.options_mut(|o| o.fallback_theme = os);
        if ctx.theme() != os {
            ctx.set_theme(os);
        }
    }
}

/// Whether the current appearance settings read anything the watcher updates.
fn depends_on_library(a: &soyuz_core::config::Appearance) -> bool {
    let id = themes::slug(&a.color_theme);
    let user = lib().read().user.iter().any(|(u, _)| u.id == id);
    let builtin = themes::builtins().iter().any(|t| t.id == id);
    user || !builtin || a.accent.trim().eq_ignore_ascii_case("system")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_picks_up_user_themes_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mine.toml"), "name = \"Mine\"\n[dark]\naccent = \"#FF0000\"\n").unwrap();
        std::fs::write(dir.path().join("broken.toml"), "name = 5").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        assert!(scan(dir.path()));
        assert!(!scan(dir.path()), "unchanged folder: no rescan");
        assert!(find("mine").is_some());
        assert!(find("Orbit").is_some(), "built-ins resolve too");
        assert!(errors().iter().any(|e| e.file.ends_with("broken.toml")));
        assert!(all().iter().any(|t| t.id == "mine" && !t.builtin));
        std::fs::remove_file(dir.path().join("mine.toml")).unwrap();
        assert!(scan(dir.path()));
        assert!(find("mine").is_none());
    }
}
