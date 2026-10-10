//! Per-folder color and emblem: right-click a folder → "Folder color". The
//! choice shows as a tinted folder icon (with an optional emblem glyph) in the views and
//! as a colored dot in the breadcrumb and on tabs.
//!
//! Stored in `folder_colors.toml` next to the config (portable-aware), like tags: loaded
//! on a worker at startup and written back debounced on the store worker.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use egui::Color32;
use serde::{Deserialize, Serialize};

use crate::theme::Palette;
use crate::theme::icons::ph;
use crate::theme::tokens::parse_hex;

/// A folder color: one of the eight tag colors (follows the theme) or a fixed color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderColor {
    Tag(u8),
    Rgb(Color32),
}

impl FolderColor {
    pub fn resolve(self, p: &Palette) -> Color32 {
        match self {
            FolderColor::Tag(i) => p.tags[(i as usize).min(7)],
            FolderColor::Rgb(c) => c,
        }
    }

    fn to_config(self) -> String {
        match self {
            FolderColor::Tag(i) => super::tags::slug(i),
            FolderColor::Rgb(c) => crate::theme::themes::hex_string(c),
        }
    }

    fn parse(s: &str) -> Option<FolderColor> {
        if s.trim().starts_with('#') {
            return parse_hex(s).map(|c| FolderColor::Rgb(c.to_opaque()));
        }
        super::tags::from_name(s).map(FolderColor::Tag)
    }
}

/// Emblems offered in the menu: `(id, label, glyph)`.
pub const EMBLEMS: [(&str, &str, &str); 12] = [
    ("star", "Star", ph::STAR),
    ("heart", "Heart", ph::HEART),
    ("code", "Code", ph::CODE),
    ("image", "Pictures", ph::IMAGE),
    ("music", "Music", ph::MUSIC_NOTE),
    ("film", "Video", ph::FILM_STRIP),
    ("briefcase", "Work", ph::BRIEFCASE),
    ("book", "Books", ph::BOOK_OPEN),
    ("cloud", "Cloud", ph::CLOUD),
    ("lock", "Private", ph::LOCK_SIMPLE),
    ("game", "Games", ph::GAME_CONTROLLER),
    ("archive", "Archive", ph::ARCHIVE),
];

/// Glyph of an emblem id.
pub fn emblem_glyph(id: &str) -> Option<&'static str> {
    EMBLEMS.iter().find(|(e, _, _)| *e == id).map(|(_, _, g)| *g)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderStyle {
    pub color: Option<FolderColor>,
    /// Emblem id (see [`EMBLEMS`]).
    pub emblem: Option<String>,
}

impl FolderStyle {
    pub fn is_empty(&self) -> bool {
        self.color.is_none() && self.emblem.is_none()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Entry {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    color: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    emblem: String,
}

#[derive(Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    items: BTreeMap<String, Entry>,
}

/// Path → style with TOML (de)serialization.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    map: HashMap<PathBuf, FolderStyle>,
}

fn norm(p: &Path) -> PathBuf {
    let s = soyuz_fs::strip_verbatim(p);
    if cfg!(windows) { PathBuf::from(s.to_string_lossy().to_lowercase()) } else { s }
}

impl Store {
    pub fn get(&self, path: &Path) -> Option<&FolderStyle> {
        if self.map.is_empty() {
            return None;
        }
        self.map.get(&norm(path))
    }

    pub fn set(&mut self, path: &Path, style: FolderStyle) {
        if style.is_empty() {
            self.map.remove(&norm(path));
        } else {
            self.map.insert(norm(path), style);
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn to_toml(&self) -> String {
        let mut f = StoreFile { version: 1, items: BTreeMap::new() };
        for (p, s) in &self.map {
            f.items.insert(
                p.to_string_lossy().into_owned(),
                Entry {
                    color: s.color.map(FolderColor::to_config).unwrap_or_default(),
                    emblem: s.emblem.clone().unwrap_or_default(),
                },
            );
        }
        toml::to_string_pretty(&f).unwrap_or_default()
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let f: StoreFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut s = Store::default();
        for (p, e) in f.items {
            let style = FolderStyle {
                color: FolderColor::parse(&e.color),
                emblem: emblem_glyph(&e.emblem).map(|_| e.emblem.clone()),
            };
            s.set(Path::new(&p), style);
        }
        Ok(s)
    }
}

// ---------------------------------------------------------------------------------------
// UI-thread registry.

struct Registry {
    store: Store,
    loaded: bool,
    started: bool,
    dirty: bool,
    last_change: Instant,
    rx: Option<Receiver<Store>>,
    path: PathBuf,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            store: Store::default(),
            loaded: false,
            started: false,
            dirty: false,
            last_change: Instant::now(),
            rx: None,
            path: soyuz_core::config::config_dir().join("folder_colors.toml"),
        }
    }
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Start loading (idempotent), merge the loaded store, write changes back debounced.
pub fn pump(ctx: &egui::Context) {
    REG.with_borrow_mut(|r| {
        if !r.started {
            r.started = true;
            let (tx, rx) = unbounded();
            r.rx = Some(rx);
            let path = r.path.clone();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new().name("soyuz-folder-colors-load".into()).spawn(move || {
                let store = super::store_file::load(&path, Store::from_toml).value();
                let _ = tx.send(store);
                ctx.request_repaint();
            });
        }
        if let Some(mut s) = r.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            for (p, st) in std::mem::take(&mut r.store.map) {
                s.map.insert(p, st);
            }
            r.store = s;
            r.loaded = true;
        }
        if r.dirty && r.loaded && r.last_change.elapsed() > Duration::from_millis(400) {
            r.dirty = false;
            super::store_file::save_async(r.path.clone(), r.store.to_toml().into_bytes());
        } else if r.dirty {
            ctx.request_repaint_after(Duration::from_millis(450));
        }
    });
}

/// Queue pending changes now (app exit).
pub fn flush() {
    REG.with_borrow_mut(|r| {
        if r.dirty && r.loaded {
            r.dirty = false;
            super::store_file::save_async(r.path.clone(), r.store.to_toml().into_bytes());
        }
    });
}

/// The style of a folder (cheap; call per row).
pub fn style_of(path: &Path) -> Option<FolderStyle> {
    REG.with_borrow(|r| r.store.get(path).cloned())
}

/// The folder's color, resolved against the palette.
pub fn color_of(path: &Path, p: &Palette) -> Option<Color32> {
    REG.with_borrow(|r| r.store.get(path).and_then(|s| s.color).map(|c| c.resolve(p)))
}

/// The folder's emblem glyph.
pub fn emblem_of(path: &Path) -> Option<&'static str> {
    REG.with_borrow(|r| r.store.get(path).and_then(|s| s.emblem.as_deref()).and_then(emblem_glyph))
}

fn edit(paths: &[PathBuf], f: impl Fn(&mut FolderStyle)) {
    REG.with_borrow_mut(|r| {
        for p in paths {
            let mut s = r.store.get(p).cloned().unwrap_or_default();
            f(&mut s);
            r.store.set(p, s);
        }
        r.dirty = true;
        r.last_change = Instant::now();
    });
}

pub fn set_color(paths: &[PathBuf], color: Option<FolderColor>) {
    edit(paths, |s| s.color = color);
}

pub fn set_emblem(paths: &[PathBuf], emblem: Option<&str>) {
    edit(paths, |s| s.emblem = emblem.map(str::to_string));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trip() {
        let mut s = Store::default();
        s.set(Path::new("/a"), FolderStyle { color: Some(FolderColor::Tag(5)), emblem: Some("star".into()) });
        s.set(Path::new("/b"), FolderStyle { color: Some(FolderColor::Rgb(Color32::from_rgb(1, 2, 3))), emblem: None });
        s.set(Path::new("/c"), FolderStyle::default());
        assert_eq!(s.len(), 2, "empty styles are not stored");
        let back = Store::from_toml(&s.to_toml()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.get(Path::new("/a")).unwrap().color, Some(FolderColor::Tag(5)));
    }

    #[test]
    fn hostile_files_are_sanitized() {
        let t =
            "[items.\"/x\"]\ncolor = \"chartreuse\"\nemblem = \"../../evil\"\n[items.\"/y\"]\ncolor = \"#00FF0080\"\n";
        let s = Store::from_toml(t).unwrap();
        assert!(s.get(Path::new("/x")).is_none(), "unknown color and emblem: nothing stored");
        assert_eq!(s.get(Path::new("/y")).unwrap().color, Some(FolderColor::Rgb(Color32::GREEN)), "opaque");
        assert!(Store::from_toml("items = 3").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_lookup_ignores_case() {
        let mut s = Store::default();
        s.set(Path::new(r"C:\Work"), FolderStyle { color: Some(FolderColor::Tag(0)), emblem: None });
        assert!(s.get(Path::new(r"c:\work")).is_some());
        assert!(s.get(Path::new(r"\\?\C:\WORK")).is_some());
    }

    #[test]
    fn colors_resolve_against_the_palette() {
        let p = crate::theme::ORBIT_DARK.clone();
        assert_eq!(FolderColor::Tag(3).resolve(&p), p.tags[3]);
        assert_eq!(FolderColor::parse("blue"), Some(FolderColor::Tag(5)));
        assert_eq!(FolderColor::parse("#FF0000"), Some(FolderColor::Rgb(Color32::RED)));
        assert_eq!(FolderColor::parse("nope"), None);
        assert!(EMBLEMS.iter().all(|(id, _, _)| emblem_glyph(id).is_some()));
    }
}
