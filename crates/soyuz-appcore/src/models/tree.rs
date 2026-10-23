//! Folder tree state and logic (frontend-agnostic): the lazily loaded tree of the drives
//! and the home folder, following the active pane, filtering, keyboard navigation
//! (arrows, Left/Right collapse/expand, Home/End, PageUp/PageDown, type-ahead), watching
//! the open folders and the app-level effects (`Cmd::Tree`, `[sidebar] mode`). Child
//! folders are listed by [`TreeService`] on worker threads; [`pump`] only drains results.
//! The GUI draws it in `ui::tree`.

pub mod model;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_core::config::{Action, DragDefault, SidebarMode};
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::core::{AppCore, Focus};
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::services::tree::{TreeMsg, TreeService};
use crate::state::pane::PaneId;
use model::{Flat, Nav, NavOutcome, Reveal, Root, RootKind, Row, RowKey, TreeModel, Visibility};

/// Type-ahead buffer lifetime.
const TYPE_AHEAD: Duration = Duration::from_millis(900);
/// Keyboard selection navigates the pane after this pause (arrowing through quickly
/// does not list every folder on the way).
const NAV_DELAY: Duration = Duration::from_millis(220);
/// At most this many expanded folders are remembered in the session.
pub const SESSION_LIMIT: usize = 256;

/// App-level effects of the tree (applied by [`apply`] through `Cmd::Tree`).
#[derive(Clone, Debug)]
pub enum TreeCmd {
    /// The tree was clicked: keyboard focus moves to the sidebar.
    Focus,
    NewFolder(PathBuf),
    Properties(PathBuf),
}

#[derive(Default)]
pub struct TreeState {
    pub model: TreeModel,
    service: Option<TreeService>,
    pub filter: String,
    /// Keyboard cursor / highlighted row.
    pub selected: Option<RowKey>,
    flat: Flat,
    flat_key: Option<(u64, Visibility, String)>,
    /// Reveal in progress: root index and target folder.
    reveal: Option<(usize, PathBuf)>,
    /// Last pane folder the tree followed.
    synced: Option<PathBuf>,
    /// The frontend should scroll the selected row into view (taken by [`Self::take_scroll`]).
    scroll_to_selected: bool,
    typed: String,
    typed_at: Option<Instant>,
    /// Folder a drag hovers (expands after `[sidebar] tree_drag_expand_ms`).
    drag_hover: Option<(PathBuf, Instant)>,
    pending_nav: Option<(PathBuf, Instant)>,
    /// Folders changed (watcher) while a listing of them was running.
    relist: HashSet<PathBuf>,
    watched: Vec<PathBuf>,
    watched_version: Option<u64>,
    /// Mode to return to when the tree is toggled back on.
    pub last_tree_mode: Option<SidebarMode>,
    /// Mode the sidebar icon bar's "Places" button returns to (`places` or `both`).
    pub last_places_mode: Option<SidebarMode>,
}

/// Per-frame inputs from the configuration and the window state.
pub struct TreeArgs<'a> {
    pub state: &'a mut TreeState,
    pub mode: SidebarMode,
    pub vis: Visibility,
    pub follow: bool,
    pub drag_expand: Option<Duration>,
    pub drag_default: DragDefault,
    /// The sidebar has keyboard focus (and no text field does).
    pub keyboard: bool,
    pub debounce: Duration,
}

/// What the tree needs from the rest of the sidebar.
pub struct TreeIo<'a> {
    pub volumes: &'a [VolumeInfo],
    pub current: &'a Location,
    pub pane: PaneId,
    pub cmds: &'a mut Vec<Cmd>,
}

/// One frame of keyboard input for the tree (collected by the frontend).
#[derive(Clone, Debug, Default)]
pub struct KeyInput {
    pub steps: Vec<Nav>,
    pub enter: bool,
    pub escape: bool,
    /// Typed text (type-ahead); ignored while the filter box has the focus.
    pub typed: String,
    /// The filter box has the keyboard focus.
    pub filter_focused: bool,
}

impl TreeState {
    /// Expanded folders restored from the session.
    pub fn restore_expanded(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            self.model.expand(&p);
        }
    }

    /// Expanded folders for the session (bounded).
    pub fn session_expanded(&self) -> Vec<PathBuf> {
        self.model.expanded.iter().take(SESSION_LIMIT).cloned().collect()
    }

    fn service(&mut self, host: &dyn Host, debounce: Duration) -> &TreeService {
        self.service.get_or_insert_with(|| TreeService::new(host.wake(), debounce))
    }

    fn list(&mut self, host: &dyn Host, debounce: Duration, dir: &Path) {
        self.model.mark_loading(dir);
        self.service(host, debounce).list(dir);
    }

    /// The visible rows (flattened by [`pump`]).
    pub fn rows(&self) -> &[Row] {
        &self.flat.rows
    }

    pub fn row_index(&self, key: &RowKey) -> Option<usize> {
        self.flat.rows.iter().position(|r| &r.key == key)
    }

    /// Whether the selected row should be scrolled into view (once).
    pub fn take_scroll(&mut self) -> bool {
        std::mem::take(&mut self.scroll_to_selected)
    }

    /// What a root row stands for (`None` for nested rows).
    pub fn root_kind(&self, row: &Row) -> Option<RootKind> {
        (row.depth == 0).then(|| self.model.roots.get(row.key.root).map(|r| r.kind.clone())).flatten()
    }

    /// A row was activated (click, menu): select it and open it in `pane` now.
    pub fn open_row(&mut self, key: RowKey, pane: PaneId, cmds: &mut Vec<Cmd>) {
        self.synced = Some(key.path.clone());
        self.pending_nav = None;
        navigate(pane, key.path.clone(), cmds);
        self.selected = Some(key);
    }

    /// A drag hovers the collapsed folder `path`: expand it after `delay`. Returns how
    /// long to wait before asking again (the frontend redraws then).
    pub fn drag_hover(&mut self, path: &Path, delay: Option<Duration>) -> Option<Duration> {
        match &self.drag_hover {
            Some((p, at)) if p == path => {
                let delay = delay?;
                if at.elapsed() >= delay {
                    self.model.expand(path);
                    self.drag_hover = None;
                    None
                } else {
                    Some(delay - at.elapsed())
                }
            }
            _ => {
                self.drag_hover = Some((path.to_path_buf(), Instant::now()));
                None
            }
        }
    }

    /// No drag hovers a folder any more.
    pub fn drag_left(&mut self) {
        self.drag_hover = None;
    }
}

/// Roots: Home first, then every volume (`label` names a volume).
fn roots(volumes: &[VolumeInfo], label: fn(&VolumeInfo) -> String) -> Vec<Root> {
    let mut v = vec![Root { path: soyuz_fs::home(), label: tr!("views-tree-home").into(), kind: RootKind::Home }];
    for (i, vol) in volumes.iter().enumerate() {
        v.push(Root { path: vol.mount.clone(), label: label(vol), kind: RootKind::Volume(i) });
    }
    v
}

/// Background results, roots, follow-the-pane and lazy loading. Cheap when idle.
/// `volume_label` names a volume's root row.
pub fn pump(host: &dyn Host, a: &mut TreeArgs<'_>, io: &mut TreeIo<'_>, volume_label: fn(&VolumeInfo) -> String) {
    let debounce = a.debounce;
    let s = &mut *a.state;
    if s.model.set_roots(roots(io.volumes, volume_label)) {
        // Follow the pane again: its folder may lie under a root that only just arrived (the
        // volume list comes in after the first frames; until then only the home folder is a root).
        s.synced = None;
    }
    let msgs = s.service.as_ref().map(TreeService::drain).unwrap_or_default();
    for m in msgs {
        match m {
            TreeMsg::Listed { dir, result } => match result {
                Ok(children) => {
                    for c in &children {
                        crate::host::note_text_shown(&c.name);
                    }
                    s.model.set_children(&dir, children);
                    if s.relist.remove(&dir) {
                        s.service(host, debounce).list(&dir);
                    }
                }
                Err(e) => s.model.set_failed(&dir, e),
            },
            TreeMsg::Probed { probes, .. } => {
                for (p, probe) in probes {
                    s.model.set_probe(&p, probe);
                }
            }
            TreeMsg::Changed(dir) => relist(s, host, debounce, &dir),
            TreeMsg::Lost(dir) => {
                if let Some(parent) = dir.parent() {
                    relist(s, host, debounce, parent);
                }
            }
        }
    }
    s.model.set_pinned(match io.current {
        Location::Dir(d) => Some(d.clone()),
        _ => None,
    });
    // Follow the active pane.
    if a.follow
        && let Location::Dir(cur) = io.current
        && s.synced.as_ref() != Some(cur)
    {
        s.synced = Some(cur.clone());
        let ci = model::case_insensitive();
        let already =
            s.selected.as_ref().is_some_and(|k| model::rel_components(cur, &k.path, ci).is_some_and(|r| r.is_empty()));
        if !already {
            s.reveal = model::best_root(&s.model.roots, cur, ci).map(|ri| (ri, cur.clone()));
        } else {
            s.reveal = None;
        }
    }
    if let Some((ri, target)) = s.reveal.clone() {
        match s.model.reveal(ri, &target, a.vis, model::case_insensitive()) {
            Reveal::Done(key) | Reveal::Partial(key) => {
                s.selected = Some(key);
                // As a section among the places, following must not yank the sidebar
                // down to the tree on every navigation; with the keyboard it may.
                s.scroll_to_selected = a.mode == SidebarMode::Tree || a.keyboard;
                s.reveal = None;
            }
            Reveal::Pending(dirs) => {
                for d in dirs {
                    if s.model.needs_load(&d) {
                        s.list(host, debounce, &d);
                    }
                }
            }
            Reveal::Outside => s.reveal = None,
        }
    }
    // Rows (cached until the model, the hidden rule or the filter change).
    let key = (s.model.version, a.vis, s.filter.clone());
    if s.flat_key.as_ref() != Some(&key) {
        s.flat = s.model.flatten(a.vis, &s.filter);
        s.flat_key = Some(key);
        for d in std::mem::take(&mut s.flat.to_load) {
            s.list(host, debounce, &d);
        }
    }
    // Watch the open folders.
    if s.watched_version != Some(s.model.version) {
        s.watched_version = Some(s.model.version);
        let mut open = s.model.open_dirs();
        open.truncate(crate::services::tree::WATCH_LIMIT);
        if open != s.watched {
            s.watched = open.clone();
            s.service(host, debounce).watch(open);
        }
    }
    // Debounced keyboard navigation.
    if let Some((path, at)) = s.pending_nav.clone() {
        let left = NAV_DELAY.saturating_sub(at.elapsed());
        if left.is_zero() {
            s.pending_nav = None;
            s.synced = Some(path.clone());
            navigate(io.pane, path, io.cmds);
        } else {
            host.redraw_after(left);
        }
    }
}

fn relist(s: &mut TreeState, host: &dyn Host, debounce: Duration, dir: &Path) {
    match s.model.node(dir).map(|n| n.load.clone()) {
        Some(model::Load::Loaded) => s.service(host, debounce).list(dir),
        Some(model::Load::Loading) => {
            s.relist.insert(dir.to_path_buf());
        }
        Some(model::Load::Failed(_)) => s.model.retry(dir),
        _ => {}
    }
}

fn navigate(pane: PaneId, path: PathBuf, cmds: &mut Vec<Cmd>) {
    cmds.push(Cmd::Navigate { pane, location: Location::Dir(path), select: None });
}

/// Arrow keys, Home/End, PageUp/PageDown, Enter, Escape and type-ahead. Returns `true`
/// when the filter box should give up the keyboard focus (Enter in the filter).
pub fn keyboard(s: &mut TreeState, k: KeyInput, io: &mut TreeIo<'_>) -> bool {
    let cur = s.selected.as_ref().and_then(|key| s.row_index(key));
    let mut moved = None;
    for step in k.steps {
        let cur = moved.or(cur);
        match model::nav(&s.flat.rows, cur, step) {
            NavOutcome::Select(i) => moved = Some(i),
            NavOutcome::Expand(i) => s.model.expand(&s.flat.rows[i].key.path),
            NavOutcome::Collapse(i) => s.model.collapse(&s.flat.rows[i].key.path),
            NavOutcome::None => {}
        }
    }
    if !k.filter_focused {
        let typed: String = k.typed.chars().filter(|c| !c.is_control()).collect();
        if !typed.trim().is_empty() || (!typed.is_empty() && !s.typed.is_empty()) {
            if s.typed_at.is_none_or(|t| t.elapsed() > TYPE_AHEAD) {
                s.typed.clear();
            }
            s.typed.push_str(&typed);
            s.typed_at = Some(Instant::now());
            if let Some(i) = model::type_ahead(&s.flat.rows, moved.or(cur), &s.typed) {
                moved = Some(i);
            }
        }
    }
    if let Some(i) = moved {
        let key = s.flat.rows[i].key.clone();
        s.pending_nav = Some((key.path.clone(), Instant::now()));
        s.selected = Some(key);
        s.scroll_to_selected = true;
    }
    let mut blur_filter = false;
    if k.enter
        && let Some(key) = s.selected.clone()
    {
        s.pending_nav = None;
        s.synced = Some(key.path.clone());
        navigate(io.pane, key.path, io.cmds);
        if k.filter_focused {
            blur_filter = true;
            io.cmds.push(Cmd::Tree(TreeCmd::Focus));
        }
    }
    if k.escape {
        if !s.filter.is_empty() {
            s.filter.clear();
        } else {
            io.cmds.push(Cmd::Action(Action::FocusList));
        }
    }
    blur_filter
}

/// Apply a [`TreeCmd`].
pub fn apply(app: &mut AppCore, cmd: TreeCmd) {
    match cmd {
        TreeCmd::Focus => app.ui.focus = Focus::Sidebar,
        TreeCmd::NewFolder(dir) => new_folder(app, dir),
        TreeCmd::Properties(dir) => app.frontend.push(FrontendCmd::FolderProperties(dir)),
    }
}

/// Create "New folder" inside `dir` (a job, so it is undoable), open `dir` in the
/// active pane and start renaming the new folder there.
fn new_folder(app: &mut AppCore, dir: PathBuf) {
    use crate::ops::fileops::names;
    use crate::services::jobs::{AfterJob, JobMeta, OpKind};
    let pane = app.ws.pane().id;
    let existing: Vec<String> = app
        .ui
        .tree
        .model
        .node(&dir)
        .map(|n| n.children.iter().map(|c| soyuz_core::entry::display_name(c)).collect())
        .unwrap_or_default();
    let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
    let name = names::unique_in(&refs, tr!("views-tree-new-folder-name"), names::case_insensitive_fs());
    let path = dir.join(&name);
    crate::ops::navigate(app, pane, Location::Dir(dir.clone()), crate::ops::NavOpts::user());
    let spec =
        app.fileops.jobs.spec(soyuz_jobs::JobKind::Create { path: path.clone(), kind: soyuz_jobs::CreateKind::Dir });
    let meta = JobMeta::new(OpKind::NewFolder, std::slice::from_ref(&path), Some(dir))
        .after(AfterJob::SelectAndRename { pane, path })
        .quiet();
    app.fileops.jobs.submit(spec, meta);
}

/// The sidebar icon bar: `Places` returns to the last places-like mode (`places` or
/// `both`), `Tree` shows the full-height disk tree. Writes `[sidebar] mode` like Settings.
pub fn set_mode(app: &mut AppCore, host: &dyn Host, want: SidebarMode) {
    let cur = app.config.sidebar.mode;
    let next = match want {
        SidebarMode::Tree => SidebarMode::Tree,
        _ if cur == SidebarMode::Tree => app.ui.tree.last_places_mode.unwrap_or(want),
        m => m,
    };
    if next == cur {
        return;
    }
    if cur != SidebarMode::Tree {
        app.ui.tree.last_places_mode = Some(cur);
    }
    if cur != SidebarMode::Places {
        app.ui.tree.last_tree_mode = Some(cur);
    }
    crate::models::settings::update_config(app, host, |c| c.sidebar.mode = next);
}

/// `Action::ToggleTreeSidebar`: places ↔ the last tree mode (the full-height disk tree by
/// default). Writes `[sidebar] mode` like the Settings window would.
pub fn toggle(app: &mut AppCore, host: &dyn Host) {
    let cur = app.config.sidebar.mode;
    let next = match cur {
        SidebarMode::Places => app.ui.tree.last_tree_mode.unwrap_or(SidebarMode::Tree),
        m => {
            app.ui.tree.last_tree_mode = Some(m);
            SidebarMode::Places
        }
    };
    if !app.ui.sidebar_visible {
        app.ui.sidebar_visible = true;
        if cur != SidebarMode::Places {
            return; // showing the sidebar already shows the tree
        }
    }
    crate::models::settings::update_config(app, host, |c| c.sidebar.mode = next);
    let msg = if next == SidebarMode::Places { tr!("views-tree-hidden") } else { tr!("views-tree-shown") };
    app.ui.toasts.push(Toast::info(msg));
}
