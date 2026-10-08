//! Per-folder view state and its persistence (`views.toml`, features.md UX #11).
//!
//! Folders are keyed by a hash of their *normalized* path (separators unified, trailing
//! separators and the Windows verbatim prefix removed, case folded on Windows/macOS), so
//! `C:\Foo\` and `c:/foo` share one entry.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::config::Config;
use soyuz_core::{Column, ColumnSpec, GroupBy, Location, SortDir, SortSpec, ViewMode};

/// Grid zoom limits (icon edge in points).
pub const ZOOM_MIN: u16 = 16;
pub const ZOOM_MAX: u16 = 256;

/// Everything about how a pane presents a folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub mode: ViewMode,
    pub sort: SortSpec,
    pub group: GroupBy,
    /// Details-view columns in display order (hidden ones keep their position).
    pub columns: Vec<ColumnSpec>,
    /// Icon / thumbnail size for the grid views.
    pub zoom: u16,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState::from_config(&Config::default())
    }
}

impl ViewState {
    /// Defaults from `[general]`, `[columns]` and `[appearance]`.
    pub fn from_config(c: &Config) -> Self {
        let sort = SortSpec {
            column: Column::Name,
            dir: SortDir::Asc,
            folders_first: c.general.folders_first,
            natural: c.general.natural_sort,
            case_insensitive: c.general.case_insensitive_sort,
        };
        ViewState {
            mode: c.general.default_view,
            sort,
            group: GroupBy::None,
            columns: default_columns(c),
            zoom: (c.appearance.thumbnail_size.clamp(ZOOM_MIN as u32, ZOOM_MAX as u32)) as u16,
        }
    }

    /// Visible columns, `Name` always first-class (inserted if a config removed it).
    pub fn visible_columns(&self) -> Vec<ColumnSpec> {
        let mut v: Vec<ColumnSpec> = self.columns.iter().filter(|c| c.visible).cloned().collect();
        if !v.iter().any(|c| c.column == Column::Name) {
            v.insert(0, ColumnSpec::new(Column::Name));
        }
        v
    }

    /// Show or hide a column (appending it when it was never configured).
    pub fn set_column_visible(&mut self, column: Column, visible: bool) {
        if column == Column::Name {
            return;
        }
        match self.columns.iter_mut().find(|c| c.column == column) {
            Some(c) => c.visible = visible,
            None if visible => self.columns.push(ColumnSpec::new(column)),
            None => {}
        }
    }

    /// Move the visible column at `from` (index among visible columns) to `to`.
    pub fn move_visible_column(&mut self, from: usize, to: usize) {
        let vis: Vec<Column> = self.visible_columns().iter().map(|c| c.column).collect();
        let (Some(&a), Some(&b)) = (vis.get(from), vis.get(to)) else { return };
        let (Some(ia), Some(ib)) =
            (self.columns.iter().position(|c| c.column == a), self.columns.iter().position(|c| c.column == b))
        else {
            return;
        };
        let item = self.columns.remove(ia);
        let ib = if ia < ib { ib } else { ib.min(self.columns.len()) };
        self.columns.insert(ib.min(self.columns.len()), item);
    }

    pub fn set_column_width(&mut self, column: Column, width: f32) {
        if let Some(c) = self.columns.iter_mut().find(|c| c.column == column) {
            c.width = width;
        } else if column == Column::Name {
            let mut spec = ColumnSpec::new(Column::Name);
            spec.width = width;
            self.columns.insert(0, spec);
        }
    }
}

fn default_columns(c: &Config) -> Vec<ColumnSpec> {
    let mut cols: Vec<ColumnSpec> = c
        .columns
        .details
        .iter()
        .map(|col| {
            let mut s = ColumnSpec::new(*col);
            if let Some(w) = c.columns.widths.get(&col.to_string()) {
                s.width = w.clamp(40.0, 2000.0);
            }
            s
        })
        .collect();
    if !cols.iter().any(|s| s.column == Column::Name) {
        cols.insert(0, ColumnSpec::new(Column::Name));
    }
    cols
}

/// Normalize a path to its view-memory identity (see module docs).
pub fn normalize_key_path(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/UNC/") {
        s = format!("//{rest}");
    } else if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_string();
    }
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    // "C:" and "C:/" are the same root.
    if s.len() == 2 && s.ends_with(':') {
        s.push('/');
    }
    if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s }
}

/// FNV-1a 64-bit (stable across runs and platforms, unlike `DefaultHasher`).
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The `views.toml` key of a location; `None` for locations without view memory.
pub fn folder_key(loc: &Location) -> Option<String> {
    let ident = match loc {
        Location::Dir(p) => normalize_key_path(p),
        Location::Archive { archive, inner } => format!("{}!/{inner}", normalize_key_path(archive)),
        Location::Trash | Location::Computer | Location::Recent => loc.display(),
        Location::Tags(t) => format!("tag:{t}"),
        Location::Search(_) => return None,
        Location::Remote(u) => format!("remote:{u}"),
    };
    Some(format!("{:016x}", fnv1a(&ident)))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FolderView {
    /// Human-readable path (informational; the key is the hash).
    pub path: String,
    pub view: ViewState,
}

/// All remembered views plus an optional "applied to all folders" default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewMemory {
    pub default: Option<ViewState>,
    pub folders: BTreeMap<String, FolderView>,
    #[serde(skip)]
    pub dirty: bool,
}

impl ViewMemory {
    /// The view for `loc`: remembered folder view, else the applied-to-all default, else
    /// `fallback` (config defaults).
    pub fn lookup(&self, loc: &Location, fallback: &ViewState) -> ViewState {
        folder_key(loc)
            .and_then(|k| self.folders.get(&k))
            .map(|f| f.view.clone())
            .or_else(|| self.default.clone())
            .unwrap_or_else(|| fallback.clone())
    }

    pub fn has_custom(&self, loc: &Location) -> bool {
        folder_key(loc).is_some_and(|k| self.folders.contains_key(&k))
    }

    pub fn remember(&mut self, loc: &Location, view: &ViewState) {
        let Some(key) = folder_key(loc) else { return };
        let entry = FolderView { path: loc.display(), view: view.clone() };
        if self.folders.get(&key) != Some(&entry) {
            self.folders.insert(key, entry);
            self.dirty = true;
        }
    }

    /// Forget the folder's view (it falls back to the default).
    pub fn reset(&mut self, loc: &Location) {
        if let Some(k) = folder_key(loc)
            && self.folders.remove(&k).is_some()
        {
            self.dirty = true;
        }
    }

    /// Make `view` the default for every folder and forget all per-folder views.
    pub fn apply_to_all(&mut self, view: &ViewState) {
        self.default = Some(view.clone());
        self.folders.clear();
        self.dirty = true;
    }

    pub fn load(path: &Path) -> ViewMemory {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                log::warn!("{}: {e}; starting with empty view memory", path.display());
                ViewMemory::default()
            }),
            Err(_) => ViewMemory::default(),
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_normalize() {
        let a = folder_key(&Location::Dir("/home/u/src/".into()));
        let b = folder_key(&Location::Dir("/home/u/src".into()));
        assert_eq!(a, b);
        assert_ne!(a, folder_key(&Location::Dir("/home/u/src2".into())));
        assert_eq!(folder_key(&Location::Search(soyuz_core::SearchId(1))), None);
        assert!(folder_key(&Location::Trash).is_some());
        assert_eq!(a.unwrap().len(), 16);
    }

    #[test]
    fn windows_style_paths() {
        assert_eq!(normalize_key_path(Path::new(r"\\?\C:\Foo\")), normalize_key_path(Path::new("C:/Foo")));
        assert_eq!(normalize_key_path(Path::new("C:")), normalize_key_path(Path::new(r"C:\")));
        assert_eq!(normalize_key_path(Path::new(r"\\?\UNC\srv\share")), normalize_key_path(Path::new(r"\\srv\share\")));
        if cfg!(windows) {
            assert_eq!(normalize_key_path(Path::new(r"C:\FOO")), "c:/foo");
        }
    }

    #[test]
    fn memory_lookup_reset_apply_all() {
        let fallback = ViewState::default();
        let mut m = ViewMemory::default();
        let here = Location::Dir("/data/photos".into());
        let other = Location::Dir("/data/docs".into());
        let mut grid = fallback.clone();
        grid.mode = ViewMode::Grid;
        m.remember(&here, &grid);
        assert!(m.dirty);
        assert_eq!(m.lookup(&here, &fallback).mode, ViewMode::Grid);
        assert_eq!(m.lookup(&other, &fallback).mode, fallback.mode);
        m.reset(&here);
        assert_eq!(m.lookup(&here, &fallback).mode, fallback.mode);
        m.remember(&here, &grid);
        let mut list = fallback.clone();
        list.mode = ViewMode::List;
        m.apply_to_all(&list);
        assert_eq!(m.lookup(&here, &fallback).mode, ViewMode::List);
        assert_eq!(m.lookup(&other, &fallback).mode, ViewMode::List);
        assert!(!m.has_custom(&here));
    }

    #[test]
    fn toml_round_trip() {
        let mut m = ViewMemory::default();
        let mut v = ViewState { group: GroupBy::Kind, ..ViewState::default() };
        v.sort.dir = SortDir::Desc;
        m.remember(&Location::Dir("/x".into()), &v);
        m.default = Some(ViewState::default());
        let text = m.to_toml();
        let back: ViewMemory = toml::from_str(&text).unwrap();
        assert_eq!(back.folders, m.folders);
        assert_eq!(back.default, m.default);
    }

    #[test]
    fn column_editing() {
        let mut v = ViewState::default();
        let before: Vec<Column> = v.visible_columns().iter().map(|c| c.column).collect();
        assert_eq!(before[0], Column::Name);
        v.set_column_visible(Column::Created, true);
        assert!(v.visible_columns().iter().any(|c| c.column == Column::Created));
        v.set_column_visible(Column::Name, false);
        assert_eq!(v.visible_columns()[0].column, Column::Name, "name cannot be hidden");
        let n = v.visible_columns().len();
        v.move_visible_column(n - 1, 1);
        assert_eq!(v.visible_columns()[1].column, Column::Created);
        v.set_column_width(Column::Size, 133.0);
        assert_eq!(v.columns.iter().find(|c| c.column == Column::Size).unwrap().width, 133.0);
    }
}
