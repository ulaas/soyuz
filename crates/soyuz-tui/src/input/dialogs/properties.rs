//! Properties of the selection (`FrontendCmd::Properties`, Alt+Enter, and the checksum
//! actions) and of a folder from the tree (`FrontendCmd::FolderProperties`) — Phase T3.
//!
//! A read-only terminal view of what the GUI's General tab shows, as a scrolling list of
//! label / value rows: name, type, location, size and size on disk (for folders and
//! multiple items computed by a background walk, `models::folder_stats`, live while it
//! runs: "Contains 12 files, 3 folders"), link target, the created / modified / accessed
//! times, attributes and (unix) permissions and owner, and — opened with
//! `Action::Checksum(algo)` — the file's checksum, hashed in the background. Changes go
//! through the Attributes dialog (button "Attributes…", `Action::SetAttributes`) or
//! inline rename, so they stay undoable jobs. Esc / Enter close it; Up / Down / PageUp /
//! PageDown and the wheel scroll the rows.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::folder_stats::{self, Stats};
use soyuz_appcore::models::properties::{Bg, Start, Tab};
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::config::Action;
use soyuz_core::{EntryInfo, EntryKind, HashAlgo, OpControl};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::view::dialogs::MAX_LIST_ROWS;
use crate::widgets::{Button, Form, FormOutcome, SelectList};

const ATTRIBUTES: usize = 0;

/// What the dialog keeps while it is open.
#[derive(Default)]
pub struct State {
    pub title: String,
    pub form: Form,
    /// The pane the items are in (`None`: a folder from the tree).
    pub pane: Option<PaneId>,
    pub entries: Vec<EntryInfo>,
    /// A fresh `stat` of a single item (the listing may lack sizes on disk, owners, …).
    pub fresh: Option<Bg<Option<EntryInfo>>>,
    /// Sizes and counts of folders / several items.
    pub stats: Option<Arc<Mutex<Stats>>>,
    /// Stops the walk when the dialog closes.
    pub ctl: Option<Arc<OpControl>>,
    /// The link target of a single link.
    pub link: Option<Bg<Option<String>>>,
    /// A checksum asked for (`Action::Checksum`).
    pub hash: Option<(HashAlgo, Bg<Result<String, String>>)>,
    /// Cursor and scroll of the rows.
    pub list: SelectList,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("properties::State")
            .field("title", &self.title)
            .field("entries", &self.entries.len())
            .field("list", &self.list)
            .finish_non_exhaustive()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(c) = &self.ctl {
            c.cancel();
        }
    }
}

/// Open the properties of `entries` (pane `pane`) on `tab`.
pub fn open(app: &mut App, pane: PaneId, entries: Vec<EntryInfo>, tab: Tab, start: Option<Start>) {
    let _ = tab; // one view: every part of the General tab, plus a checksum when asked
    let s = State::new(app, Some(pane), entries, start);
    app.push_dialog(TuiDialog::Properties(s));
}

/// Open the properties of folder `dir` (the folder tree).
pub fn open_folder(app: &mut App, dir: PathBuf) {
    let e = EntryInfo::minimal(dir, EntryKind::Dir);
    let s = State::new(app, None, vec![e], None);
    app.push_dialog(TuiDialog::Properties(s));
}

impl State {
    fn new(app: &App, pane: Option<PaneId>, entries: Vec<EntryInfo>, start: Option<Start>) -> State {
        let host = &app.host;
        let single = entries.len() == 1;
        let title = if single {
            tr!("dialogs-props-title-one", name = crate::dialog::safe(&entries[0].name))
        } else {
            tr!("common-items", count = entries.len())
        };
        let mut buttons = vec![Button::new(tr!("tui-ops-attributes")).accel('a'), Button::new(tr!("common-close"))];
        // The tree's folder has no pane selection to edit.
        buttons[ATTRIBUTES].enabled = pane.is_some();
        let mut form = Form::new(Vec::new(), buttons).default_button(1);
        form.focus_button(1);
        let mut s = State::default();
        (s.title, s.form, s.pane) = (title, form, pane);
        if single {
            let path = entries[0].path.clone();
            s.fresh = Some(Bg::spawn(host, move || soyuz_fs::stat(&path, false).ok()));
            if matches!(entries[0].kind, EntryKind::Symlink { .. } | EntryKind::Junction) {
                let path = entries[0].path.clone();
                s.link = Some(Bg::spawn(host, move || {
                    soyuz_fs::read_link(&path).ok().map(|t| soyuz_fs::strip_verbatim(&t).display().to_string())
                }));
            }
        }
        if entries.iter().any(|e| e.kind == EntryKind::Dir) || entries.len() > 1 {
            let ctl = OpControl::new();
            s.stats = Some(folder_stats::spawn(app.core.wake.clone(), &entries, ctl.clone()));
            s.ctl = Some(ctl);
        }
        if let Some(Start::Hash(algo)) = start
            && single
            && entries[0].kind == EntryKind::File
        {
            let path = entries[0].path.clone();
            let ctl = s.ctl.get_or_insert_with(OpControl::new).clone();
            s.hash = Some((
                algo,
                Bg::spawn(host, move || soyuz_fs::hash_file(&path, algo, &ctl, &mut ()).map_err(|e| e.to_string())),
            ));
        }
        s.entries = entries;
        s
    }

    /// The items as last read (a single item's fresh `stat` once it landed).
    pub fn current(&self) -> Vec<EntryInfo> {
        match self.fresh.as_ref().and_then(|b| b.with(|f| f.cloned().flatten())) {
            Some(f) if self.entries.len() == 1 => {
                let mut e = self.entries[0].clone();
                if e.modified.is_none() {
                    // A folder from the tree starts as a bare path.
                    return vec![EntryInfo { name: e.name, ..f }];
                }
                e.alloc_size = f.alloc_size.or(e.alloc_size);
                e.nlink = f.nlink.or(e.nlink);
                e.owner = e.owner.take().or(f.owner);
                e.group = e.group.take().or(f.group);
                vec![e]
            }
            _ => self.entries.clone(),
        }
    }
}

/// The folder all `entries` are in, if they share one.
pub fn common_parent(entries: &[EntryInfo]) -> Option<&Path> {
    let first = entries.first()?.path.parent()?;
    entries.iter().all(|e| e.path.parent() == Some(first)).then_some(first)
}

fn list_len(core: &AppCore, s: &State) -> usize {
    crate::view::dialogs::properties::rows(core, s).len()
}

fn press(s: &State, b: usize) -> Outcome {
    match (b, s.pane) {
        (ATTRIBUTES, Some(p)) => Outcome::Close(Some(Cmd::PaneAction(p, Action::SetAttributes))),
        _ => Outcome::Close(None),
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let len = list_len(core, s);
    let page = usize::from(MAX_LIST_ROWS);
    for ev in &i.events {
        if s.list.apply(ev, len, page) {
            continue;
        }
        match s.form.apply(ev) {
            FormOutcome::Cancel => return Outcome::Close(None),
            FormOutcome::Pressed(b) => return press(s, b),
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let len = list_len(core, s);
    if let FormOutcome::Pressed(b) = super::batch_rename::form_mouse(&mut s.form, Some((&mut s.list, len)), e) {
        return press(s, b);
    }
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
