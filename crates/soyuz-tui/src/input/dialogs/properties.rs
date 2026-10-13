//! Properties of the selection (`FrontendCmd::Properties`, Alt+Enter, and the checksum
//! actions) and of a folder from the tree (`FrontendCmd::FolderProperties`) — Phase T3.
//!
//! A read-only terminal view of what the GUI's General tab shows, as a scrolling list of
//! label / value rows: name, type, location, size and size on disk (for folders and
//! multiple items computed by a background walk, `models::folder_stats`, live while it
//! runs: "Contains 12 files, 3 folders"), link target, the created / modified / accessed
//! times, attributes and (unix) permissions and owner. Changes go through the Attributes
//! dialog (button "Attributes…", `Action::SetAttributes`) or inline rename, so they stay
//! undoable jobs. Esc / Enter close it; Up / Down / PageUp / PageDown and the wheel scroll
//! the rows.
//!
//! A single file has a checksum section (the GUI's Checksums tab, on the core's
//! `models::checksums::ChecksumSet`): "Checksums…" (or opening with `Action::Checksum` /
//! `Action::CompareChecksum`) shows a row per algorithm (not computed, the percentage
//! while it is hashed in the background, the digest), an "Algorithm" choice with
//! Compute / Compute all / Copy, and a "Compare" field: a pasted checksum (or a
//! `sha256sum` line) starts the algorithms of its length and shows match / mismatch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::checksums::{ALGOS, ChecksumSet, HashState};
use soyuz_appcore::models::folder_stats::{self, Stats};
use soyuz_appcore::models::properties::{Bg, Start, Tab};
use soyuz_appcore::models::tools::ToolCmd;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::config::Action;
use soyuz_core::{EntryInfo, EntryKind, HashAlgo, OpControl};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::view::dialogs::MAX_LIST_ROWS;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// A button of the dialog (the buttons follow the checksum section).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Attributes,
    Checksums,
    /// "Previous versions…" (Phase T7): the versions dialog opens over this one.
    Versions,
    Compute,
    ComputeAll,
    Copy,
    Close,
}

/// Field indexes while the checksum section shows.
pub const ALGO: usize = 0;
pub const COMPARE: usize = 1;

/// What the dialog keeps while it is open.
#[derive(Default)]
pub struct State {
    pub title: String,
    pub form: Form,
    /// What each of the form's buttons is.
    pub bmap: Vec<Btn>,
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
    /// The checksums of a single file (`None` otherwise).
    pub sums: Option<ChecksumSet>,
    /// The checksum section is shown.
    pub show_sums: bool,
    /// Cursor and scroll of the rows.
    pub list: SelectList,
    /// A single item with previous versions on (`[sync] previous_versions`): the
    /// "Previous versions…" button (the GUI's Properties tab).
    pub versions: bool,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("properties::State")
            .field("title", &self.title)
            .field("entries", &self.entries.len())
            .field("show_sums", &self.show_sums)
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
    let _ = tab; // one view: every part of the General tab, plus the checksums when asked
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
        let mut s = State::default();
        (s.title, s.pane) = (title, pane);
        if single {
            let path = entries[0].path.clone();
            s.fresh = Some(Bg::spawn(host, move || soyuz_fs::stat(&path, false).ok()));
            if matches!(entries[0].kind, EntryKind::Symlink { .. } | EntryKind::Junction) {
                let path = entries[0].path.clone();
                s.link = Some(Bg::spawn(host, move || {
                    soyuz_fs::read_link(&path).ok().map(|t| soyuz_fs::strip_verbatim(&t).display().to_string())
                }));
            }
            if entries[0].kind == EntryKind::File {
                s.sums = Some(ChecksumSet::new(entries[0].path.clone()));
            }
        }
        if entries.iter().any(|e| e.kind == EntryKind::Dir) || entries.len() > 1 {
            let ctl = OpControl::new();
            s.stats = Some(folder_stats::spawn(app.core.wake.clone(), &entries, ctl.clone()));
            s.ctl = Some(ctl);
        }
        s.versions = single && soyuz_appcore::models::versions::VersionsCfg::from_config(&app.core.config).enabled();
        s.entries = entries;
        s.show_sums = s.sums.is_some() && matches!(start, Some(Start::Hash(_) | Start::Compare));
        s.rebuild();
        match start {
            Some(Start::Hash(algo)) if s.sums.is_some() => {
                if let Some(i) = ALGOS.iter().position(|a| *a == algo)
                    && let Some(crate::widgets::FieldKind::Radio { selected, .. }) =
                        s.form.fields.get_mut(ALGO).map(|f| &mut f.kind)
                {
                    *selected = i;
                }
                if let Some(sums) = s.sums.as_mut() {
                    sums.start(algo, &app.core.wake);
                }
            }
            Some(Start::Compare) if s.show_sums => s.form.focus = COMPARE,
            _ => {}
        }
        s
    }

    /// The form for the section shown (the focus on Close, or on the checksums).
    fn rebuild(&mut self) {
        let mut bmap = vec![Btn::Attributes];
        let mut fields = Vec::new();
        if self.show_sums {
            fields.push(Field::radio(
                tr!("tui-ops-algorithm"),
                ALGOS.iter().map(|a| a.label().to_string()).collect(),
                0,
            ));
            fields.push(Field::text(tr!("dialogs-hash-compare"), ""));
            bmap = vec![Btn::Compute, Btn::ComputeAll, Btn::Copy, Btn::Attributes];
        } else if self.sums.is_some() {
            bmap.push(Btn::Checksums);
        }
        if !self.show_sums && self.versions {
            bmap.push(Btn::Versions);
        }
        bmap.push(Btn::Close);
        let buttons: Vec<Button> = bmap
            .iter()
            .map(|b| match b {
                Btn::Attributes => {
                    let mut x = Button::new(tr!("tui-ops-attributes")).accel('a');
                    // The tree's folder has no pane selection to edit.
                    x.enabled = self.pane.is_some();
                    x
                }
                Btn::Checksums => Button::new(tr!("tui-ops-checksums")).accel('k'),
                Btn::Versions => Button::new(tr!("tui-versions-button")).accel('v'),
                Btn::Compute => Button::new(tr!("dialogs-hash-compute")).accel('o'),
                Btn::ComputeAll => Button::new(tr!("dialogs-hash-compute-all")).accel('l'),
                Btn::Copy => Button::new(tr!("common-copy")).accel('y'),
                Btn::Close => Button::new(tr!("common-close")),
            })
            .collect();
        let close = bmap.len() - 1;
        let mut form = Form::new(fields, buttons).default_button(close);
        if self.show_sums {
            form.default_button = bmap.iter().position(|b| *b == Btn::Compute);
            form.focus = ALGO;
        } else {
            form.focus_button(close);
        }
        self.form = form;
        self.bmap = bmap;
    }

    /// The algorithm chosen in the checksum section.
    pub fn algo(&self) -> HashAlgo {
        let i = self.form.fields.get(ALGO).and_then(|f| f.selected()).unwrap_or(0);
        ALGOS.get(i).copied().unwrap_or(HashAlgo::Sha256)
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

    /// The compare field's text.
    pub fn compare_text(&self) -> &str {
        if self.show_sums { self.form.fields.get(COMPARE).and_then(|f| f.value()).unwrap_or("") } else { "" }
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

fn press(core: &mut AppCore, s: &mut State, b: usize) -> Outcome {
    let Some(&btn) = s.bmap.get(b) else { return Outcome::Close(None) };
    match (btn, s.pane) {
        (Btn::Attributes, Some(p)) => return Outcome::Close(Some(Cmd::PaneAction(p, Action::SetAttributes))),
        (Btn::Attributes, None) | (Btn::Close, _) => return Outcome::Close(None),
        (Btn::Checksums, _) => {
            s.show_sums = true;
            s.rebuild();
        }
        (Btn::Versions, _) => {
            // The versions dialog opens over this one (`crate::tools::drain`).
            if let Some(e) = s.current().first() {
                let current = Some((e.size, e.modified));
                let is_dir = e.is_dir_like();
                soyuz_appcore::models::tools::queue(ToolCmd::Versions(e.path.clone(), is_dir, current));
            }
        }
        (Btn::Compute, _) => {
            let algo = s.algo();
            if let Some(sums) = s.sums.as_mut() {
                sums.start(algo, &core.wake);
            }
        }
        (Btn::ComputeAll, _) => {
            if let Some(sums) = s.sums.as_mut() {
                sums.start_all(&core.wake);
            }
        }
        (Btn::Copy, _) => {
            if let Some(HashState::Done(h)) = s.sums.as_ref().map(|x| x.state(s.algo())) {
                core.cmds.push(Cmd::CopyText(h));
            }
        }
    }
    Outcome::Keep
}

/// The compare field changed: its verdict (starting the algorithms that could match).
fn compare(core: &AppCore, s: &mut State) {
    let text = s.compare_text().to_string();
    if let Some(sums) = s.sums.as_mut() {
        let _ = sums.compare(&text, &core.wake);
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let len = list_len(core, s);
    let page = usize::from(MAX_LIST_ROWS);
    for ev in &i.events {
        // The rows scroll with the keys unless a checksum field uses them.
        if s.form.focused_field().is_none() && s.list.apply(ev, len, page) {
            continue;
        }
        match s.form.apply(ev) {
            FormOutcome::Cancel => return Outcome::Close(None),
            FormOutcome::Pressed(b) => {
                let out = press(core, s, b);
                if matches!(out, Outcome::Close(_)) {
                    return out;
                }
            }
            FormOutcome::Changed(COMPARE) => compare(core, s),
            FormOutcome::Ignored => {
                let _ = s.list.apply(ev, len, page);
            }
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let len = list_len(core, s);
    match super::form_mouse(&mut s.form, Some((&mut s.list, len)), e) {
        FormOutcome::Pressed(b) => return press(core, s, b),
        FormOutcome::Changed(COMPARE) => compare(core, s),
        _ => {}
    }
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
