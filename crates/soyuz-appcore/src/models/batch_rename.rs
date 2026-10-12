//! The batch rename dialog's state (F2 on a multiple selection, Ctrl+Shift+R), without
//! drawing: the items, the rule [`Spec`] or the "one name per line" editor text, the
//! live before → after preview ([`Row`]s with invalid / duplicate / existing-name
//! status), the rename-cycle count, presets and the EXIF dates some tokens need (read
//! on a worker thread). The rename engine is `crate::rename`; [`BatchRenameModel::apply_cmd`]
//! turns the preview into one undoable `Rename` job (`MetaCmd::BatchRename`).
//!
//! Both frontends draw it: the GUI (`ui::dialogs::batch_rename`) and the terminal UI
//! (`soyuz-tui`, `input::dialogs::batch_rename`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::NaiveDateTime;
use parking_lot::Mutex;
use soyuz_core::Location;

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::host::Wake;
use crate::models::properties::MetaCmd;
use crate::ops::fileops::{names, rename as inline};
use crate::rename::presets::{self, Preset};
use crate::rename::{self, Item, Row, Rule, RuleEntry, Spec, Status};
use crate::state::pane::PaneId;

/// EXIF capture dates of the items, filled by a worker thread.
type ExifSlot = Arc<Mutex<Option<Vec<Option<NaiveDateTime>>>>>;

/// How the new names are made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The rule stack ([`Spec`]).
    Rules,
    /// A text with one new name per line.
    Editor,
}

/// The state of one batch rename dialog.
pub struct BatchRenameModel {
    pub title: String,
    pub pane: PaneId,
    pub items: Vec<Item>,
    known: HashSet<PathBuf>,
    /// Lowercased `known`, for O(1) case-insensitive collision checks.
    known_lower: HashSet<String>,
    listed_dir: Option<PathBuf>,
    pub windows_rules: bool,
    /// Names compare case-insensitively (Windows / macOS, or a Windows-family volume).
    pub case_insensitive: bool,
    pub spec: Spec,
    pub mode: Mode,
    /// The editor text (one name per line).
    pub editor: String,
    /// Cache key of the last preview.
    computed: Option<(Spec, Mode, String, bool)>,
    /// The preview, one row per item (after [`BatchRenameModel::recompute`]).
    pub rows: Vec<Row>,
    /// Why the rules (or the editor text) cannot be applied.
    pub compile_error: Option<String>,
    /// Rename cycles that need temporary names.
    pub cycles: usize,
    pub builtin: Vec<Preset>,
    pub user: Vec<Preset>,
    /// The name typed for a new preset.
    pub preset_name: String,
    exif: Option<ExifSlot>,
    exif_loaded: bool,
}

impl std::fmt::Debug for BatchRenameModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchRenameModel")
            .field("pane", &self.pane)
            .field("items", &self.items.len())
            .field("mode", &self.mode)
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

impl BatchRenameModel {
    /// The model for the focused pane: its selection, or every visible item when nothing
    /// is selected. `Err` is the message for an info toast (not on disk, nothing to rename).
    pub fn open(core: &AppCore) -> Result<BatchRenameModel, String> {
        let pane = core.ws.pane();
        let (Location::Dir(_) | Location::Search(_)) = &pane.location else {
            return Err(tr!("dialogs-rename-unavailable").into());
        };
        let entries: Vec<&soyuz_core::EntryInfo> = if pane.selection.is_empty() {
            pane.visible.iter().map(|&i| &pane.entries[i]).collect()
        } else {
            pane.selected_entries()
        };
        if entries.is_empty() {
            return Err(tr!("dialogs-rename-nothing").into());
        }
        let items: Vec<Item> = entries.iter().map(|e| Item::new(e.path.clone(), e.is_dir_like(), e.modified)).collect();
        let known: HashSet<PathBuf> = pane.entries.iter().map(|e| e.path.clone()).collect();
        let listed_dir = pane.location.as_fs_path().map(Path::to_path_buf);
        // Windows-family volumes (FAT, exFAT, NTFS, SMB) need the Windows rules and compare
        // names case-insensitively, also off Windows (search results: any item's folder).
        let windows_fs = items
            .iter()
            .any(|it| it.path.parent().is_some_and(|d| inline::volume_needs_windows_names(&core.ui.volumes, d)));
        let windows_rules = windows_fs
            || inline::windows_rules_for(
                core.config.general.strict_windows_names,
                &core.ui.volumes,
                listed_dir.as_deref(),
            );
        let mut m = BatchRenameModel::new(pane.id, items, known, listed_dir, windows_rules);
        m.case_insensitive = names::case_insensitive_fs() || windows_fs;
        Ok(m)
    }

    pub fn new(
        pane: PaneId,
        items: Vec<Item>,
        known: HashSet<PathBuf>,
        listed_dir: Option<PathBuf>,
        windows_rules: bool,
    ) -> Self {
        let spec = Spec {
            rules: vec![RuleEntry::new(Rule::Replace {
                find: String::new(),
                with: String::new(),
                regex: false,
                case_sensitive: false,
                first_only: false,
            })],
            ..Spec::default()
        };
        let editor = items.iter().map(Item::name).collect::<Vec<_>>().join("\n");
        BatchRenameModel {
            title: tr!("dialogs-rename-title", count = items.len()),
            pane,
            items,
            known_lower: known.iter().map(|k| k.to_string_lossy().to_lowercase()).collect(),
            known,
            listed_dir,
            windows_rules,
            case_insensitive: names::case_insensitive_fs(),
            spec,
            mode: Mode::Rules,
            editor,
            computed: None,
            rows: Vec::new(),
            compile_error: None,
            cycles: 0,
            builtin: presets::builtin(),
            user: presets::load(&presets::default_path()),
            preset_name: String::new(),
            exif: None,
            exif_loaded: false,
        }
    }

    fn exists(&self, p: &Path) -> bool {
        if self.known.contains(p) {
            return true;
        }
        // Outside the listed folder (search results) ask the file system.
        if self.listed_dir.as_deref() != p.parent() {
            return soyuz_fs::exists_nofollow(p);
        }
        // Case-insensitive file systems: compare names case-insensitively.
        self.case_insensitive && self.known_lower.contains(&p.to_string_lossy().to_lowercase())
    }

    /// Read the EXIF dates on a worker thread once a rule needs them (`wake` when done).
    fn ensure_exif(&mut self, wake: &Wake) {
        if self.exif.is_some() || !self.spec.uses_exif() {
            return;
        }
        let slot = Arc::new(Mutex::new(None));
        self.exif = Some(slot.clone());
        let paths: Vec<(PathBuf, bool)> = self.items.iter().map(|i| (i.path.clone(), i.is_dir)).collect();
        let wake = wake.clone();
        let _ = std::thread::Builder::new().name("soyuz-exif".into()).spawn(move || {
            let dates: Vec<Option<NaiveDateTime>> = paths
                .iter()
                .map(|(p, dir)| if *dir { None } else { rename::exif::read(p).and_then(|x| x.taken) })
                .collect();
            *slot.lock() = Some(dates);
            wake();
        });
    }

    /// Bring the preview up to date with the spec / editor text (cached: cheap when
    /// nothing changed). Call once per frame (or after each edit).
    pub fn recompute(&mut self, wake: &Wake) {
        self.ensure_exif(wake);
        if !self.exif_loaded
            && let Some(slot) = &self.exif
            && let Some(dates) = slot.lock().take()
        {
            for (it, d) in self.items.iter_mut().zip(dates) {
                it.exif_date = d;
            }
            self.exif_loaded = true;
            self.computed = None;
        }
        let key = (self.spec.clone(), self.mode, self.editor.clone(), self.exif_loaded);
        if self.computed.as_ref() == Some(&key) {
            return;
        }
        self.compile_error = None;
        let names: Vec<Result<String, String>> = match self.mode {
            Mode::Rules => match rename::compile(&self.spec) {
                Ok(c) => c.apply(&self.items),
                Err(e) => {
                    self.compile_error = Some(e.clone());
                    self.items.iter().map(|_| Err(e.clone())).collect()
                }
            },
            Mode::Editor => match rename::parse_editor(&self.editor, self.items.len()) {
                Ok(lines) => lines.into_iter().map(Ok).collect(),
                Err(e) => {
                    self.compile_error = Some(e.clone());
                    self.items.iter().map(|it| Ok(it.name())).collect()
                }
            },
        };
        let ci = self.case_insensitive;
        let rows = rename::validate(&self.items, &names, &|p| self.exists(p), self.windows_rules, ci);
        self.cycles = rename::count_cycles(&rename::pairs(&self.items, &rows), ci);
        self.rows = rows;
        self.computed = Some(key);
    }

    /// `(to rename, with a problem)`.
    pub fn counts(&self) -> (usize, usize) {
        let ok = self.rows.iter().filter(|r| r.status == Status::Ok).count();
        let bad = self.rows.iter().filter(|r| r.status.is_problem()).count();
        (ok, bad)
    }

    /// Something to rename and no problem.
    pub fn can_apply(&self) -> bool {
        let (ok, bad) = self.counts();
        ok > 0 && bad == 0 && self.compile_error.is_none()
    }

    /// The command that renames (one undoable `Rename` job).
    pub fn apply_cmd(&self) -> Cmd {
        Cmd::Meta(MetaCmd::BatchRename { pane: self.pane, pairs: rename::pairs(&self.items, &self.rows) })
    }

    /// Switch modes; entering the editor starts from the current preview, so rules and
    /// manual edits combine.
    pub fn set_mode(&mut self, m: Mode) {
        if self.mode == m {
            return;
        }
        if m == Mode::Editor {
            self.editor = self.rows.iter().map(|r| r.new_name.clone()).collect::<Vec<_>>().join("\n");
        }
        self.mode = m;
    }

    /// The editor text back to the current names.
    pub fn reset_editor(&mut self) {
        self.editor = self.items.iter().map(Item::name).collect::<Vec<_>>().join("\n");
    }

    /// Load a preset's rules.
    pub fn use_preset(&mut self, p: &Preset) {
        self.spec = p.spec.clone();
        self.mode = Mode::Rules;
    }

    /// Save the current rules as a user preset named [`BatchRenameModel::preset_name`].
    pub fn save_preset(&mut self) {
        let name = self.preset_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.user.retain(|p| p.name != name);
        self.user.push(Preset { name, spec: self.spec.clone() });
        self.preset_name.clear();
        if let Err(e) = presets::save(&presets::default_path(), &self.user) {
            log::warn!("cannot save rename presets: {e}");
        }
    }

    /// Delete user preset `i`.
    pub fn delete_preset(&mut self, i: usize) {
        if i >= self.user.len() {
            return;
        }
        self.user.remove(i);
        if let Err(e) = presets::save(&presets::default_path(), &self.user) {
            log::warn!("cannot save rename presets: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rename::{Counter, Position};

    fn model(dir: &Path, names: &[&str]) -> BatchRenameModel {
        let items: Vec<Item> = names.iter().map(|n| Item::new(dir.join(n), false, None)).collect();
        let known = items.iter().map(|i| i.path.clone()).chain([dir.join("taken.txt")]).collect();
        BatchRenameModel::new(PaneId(1), items, known, Some(dir.to_path_buf()), true)
    }

    #[test]
    fn numbering_preview_and_job() {
        let dir = std::env::temp_dir().join("soyuz-br-model");
        let mut m = model(&dir, &["a.txt", "b.txt"]);
        m.spec = Spec {
            counter: Counter { start: 1, step: 1, pad: 2, per_folder: false },
            rules: vec![RuleEntry::new(Rule::Number { at: Position::End, separator: "_".into() })],
            ..Spec::default()
        };
        let wake = crate::host::no_wake();
        m.recompute(&wake);
        let names: Vec<&str> = m.rows.iter().map(|r| r.new_name.as_str()).collect();
        assert_eq!(names, ["a_01.txt", "b_02.txt"]);
        assert_eq!(m.counts(), (2, 0));
        assert!(m.can_apply());
        match m.apply_cmd() {
            Cmd::Meta(MetaCmd::BatchRename { pairs, .. }) => {
                assert_eq!(
                    pairs,
                    vec![(dir.join("a.txt"), dir.join("a_01.txt")), (dir.join("b.txt"), dir.join("b_02.txt"))]
                )
            }
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn conflicts_block_the_apply() {
        let dir = std::env::temp_dir().join("soyuz-br-model2");
        let mut m = model(&dir, &["a.txt", "b.txt"]);
        m.spec.rules = vec![RuleEntry::new(Rule::Template { pattern: "taken".into() })];
        m.recompute(&crate::host::no_wake());
        assert!(m.rows.iter().all(|r| r.status == Status::Duplicate), "{:?}", m.rows);
        assert!(!m.can_apply());
        m.spec.rules = vec![RuleEntry::new(Rule::Replace {
            find: "a".into(),
            with: "taken".into(),
            regex: false,
            case_sensitive: false,
            first_only: false,
        })];
        m.recompute(&crate::host::no_wake());
        assert_eq!(m.rows[0].status, Status::Exists);
        assert_eq!(m.rows[1].status, Status::Unchanged);
        // The editor starts from the preview.
        m.set_mode(Mode::Editor);
        assert_eq!(m.editor, "taken.txt\nb.txt");
        m.editor = "c.txt".into();
        m.recompute(&crate::host::no_wake());
        assert!(m.compile_error.is_some(), "one line for two items");
        m.reset_editor();
        assert_eq!(m.editor, "a.txt\nb.txt");
    }
}
