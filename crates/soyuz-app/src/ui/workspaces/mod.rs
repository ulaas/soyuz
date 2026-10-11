//! Workspace power-ups: tab groups, saved workspaces, layouts (single /
//! dual / quad) with synchronized browsing, in-process windows, the per-tab history graph
//! and the timeline.
//!
//! * Commands from the tab strip and menus arrive as [`WsCmd`] (`Cmd::Workspace`).
//! * Saved workspaces live in `workspaces.toml` ([`WorkspaceStore`]); the first nine are
//!   `open_workspace:<n>`, and each can carry its own hotkey.
//! * Windows: see [`windows`] — every window has its own [`Workspace`] while services,
//!   jobs and the configuration are shared.

pub mod dialogs;
pub mod history;
pub mod windows;

use std::path::PathBuf;

use egui::Ui;
use soyuz_core::Location;
use soyuz_core::config::{Action, KeyContext};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops;
use crate::state::pane::{Pane, PaneId};
use crate::state::session::{GroupSession, PaneSession, TabSession, WindowSession};
use crate::state::workspace::{GroupId, PaneLayout, Tab, TabGroup, TabId, Workspace};
use crate::state::workspace_store::WorkspaceStore;
use crate::state::workspace_windows::{DropTarget, WindowGeom, WindowId};
use crate::theme::icons::ph;
use crate::ui::toasts::Toast;
use crate::ui::widgets::{menu_item, menu_separator, submenu};

pub use windows::{SecondaryWindow, WinOp, WindowUi};

/// Workspace commands (pushed as `Cmd::Workspace`). Indices refer to the tabs of the
/// window whose frame pushed the command.
#[derive(Clone, Debug)]
pub enum WsCmd {
    NewGroup(usize),
    AddToGroup(usize, GroupId),
    RemoveFromGroup(usize),
    ToggleGroup(GroupId),
    StartRenameGroup(GroupId),
    RenameGroup(GroupId, String),
    SetGroupColor(GroupId, u8),
    Ungroup(GroupId),
    CloseGroup(GroupId),
    GroupToNewWindow(GroupId),
    /// Drag-and-drop inside the strip: move and join / leave a group.
    MoveTab {
        from: usize,
        to: usize,
        group: Option<GroupId>,
    },
    /// A tab dragged out of the strip was released.
    DropTab {
        tab: TabId,
        target: DropTarget,
        grab: egui::Vec2,
    },
    TabToNewWindow(usize),
    TabToWindow(usize, WindowId),
    SetLayout(PaneLayout),
    Open(usize),
    OpenInNewWindow(usize),
    Save(String),
    Delete(usize),
    Rename(usize, String),
    SetHotkey(usize, String),
    MoveUp(usize),
    CloseWindow {
        id: WindowId,
        merge: bool,
    },
    /// Open the "Manage workspaces" dialog.
    Manage,
    /// Go to `location` in `pane` from the history graph (revisits the node).
    Visit {
        pane: PaneId,
        location: Location,
    },
}

/// Per-process workspace state.
pub struct WorkspacesState {
    pub store: WorkspaceStore,
    path: PathBuf,
    /// The workspace last opened or saved ("Update current workspace").
    pub last: Option<String>,
    pub windows: Vec<SecondaryWindow>,
    /// The window whose workspace is in `app.ws` right now.
    pub current: WindowId,
    /// The window that shows dialogs, the palette, settings and toasts (the focused one).
    pub owner: WindowId,
    /// Last-frame geometry of every window (tab drops across windows).
    pub geoms: Vec<WindowGeom>,
    pub ops: Vec<WinOp>,
    /// "Quit" was chosen: closing the main window closes everything.
    pub quitting: bool,
    /// Inline rename of a group chip: `(group, text, focus requested)`.
    pub renaming: Option<(GroupId, String, bool)>,
    /// Windows to reopen on the first frame (session restore).
    pub restore: Vec<WindowSession>,
}

impl WorkspacesState {
    pub fn load() -> Self {
        let path = soyuz_core::config::config_dir().join("workspaces.toml");
        let store = WorkspaceStore::load(&path);
        mirror(&store);
        WorkspacesState {
            store,
            path,
            last: None,
            windows: Vec::new(),
            current: WindowId::MAIN,
            owner: WindowId::MAIN,
            geoms: Vec::new(),
            ops: Vec::new(),
            quitting: false,
            renaming: None,
            restore: Vec::new(),
        }
    }

    fn persist(&self) {
        crate::services::store_file::save_async(self.path.clone(), self.store.to_toml().into_bytes());
        mirror(&self.store);
    }

    /// Number of windows (main included).
    pub fn count(&self) -> usize {
        self.windows.len() + 1
    }

    pub fn set_geom(&mut self, g: WindowGeom) {
        match self.geoms.iter_mut().find(|x| x.id == g.id) {
            Some(x) => *x = g,
            None => self.geoms.push(g),
        }
    }
}

thread_local! {
    static SNAPSHOT: std::cell::RefCell<Vec<(String, String, usize)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn mirror(store: &WorkspaceStore) {
    let v = store.workspaces.iter().map(|w| (w.name.clone(), w.hotkey.clone(), w.layout.tabs.len())).collect();
    SNAPSHOT.with_borrow_mut(|s| *s = v);
}

/// `(name, hotkey, tab count)` of the saved workspaces (for dialogs, which can't see the
/// app state).
pub fn store_snapshot() -> Vec<(String, String, usize)> {
    SNAPSHOT.with_borrow(|s| s.clone())
}

impl Default for WorkspacesState {
    fn default() -> Self {
        Self::load()
    }
}

/// Overlays (dialogs, palette, settings, toasts, menus) belong to this frame's window.
pub fn overlays_here(app: &SoyuzApp) -> bool {
    app.workspaces.owner == app.workspaces.current
        || !app.workspaces.windows.iter().any(|w| w.id == app.workspaces.owner) && app.workspaces.current.is_main()
}

/// This frame draws the main window (main-only panels: terminal, assistant, editors…).
pub fn main_frame(app: &SoyuzApp) -> bool {
    app.workspaces.current.is_main()
}

/// A modal owns the keyboard / overlays (don't move the overlay owner then).
pub fn modal_open(app: &SoyuzApp) -> bool {
    app.ui.dialogs.is_open() || app.palette.open || app.settings.is_open()
}

// ---------------------------------------------------------------------------
// Session / saved-workspace conversion
// ---------------------------------------------------------------------------

pub fn pane_session(p: &Pane, with_view: bool) -> PaneSession {
    let mut selection: Vec<String> = p.selection.iter().map(|k| k.to_string()).collect();
    selection.truncate(500);
    PaneSession {
        location: p.location.display(),
        cursor: p.cursor.as_deref().map(str::to_string).or_else(|| p.pending_cursor.clone()),
        selection,
        scroll: p.scroll_y,
        view: with_view.then(|| p.view.clone()),
    }
}

pub fn tab_session(t: &Tab, groups: &[GroupId], with_view: bool) -> TabSession {
    TabSession {
        pinned: t.pinned,
        dual: t.dual,
        orientation: t.orientation,
        split: t.split,
        focused: t.focused,
        sync_nav: t.sync_nav,
        quad: t.quad,
        split2: t.split2,
        group: t.group.and_then(|g| groups.iter().position(|x| *x == g)),
        panes: t
            .panes
            .iter()
            .filter(|p| !matches!(p.location, Location::Search(_)))
            .map(|p| pane_session(p, with_view))
            .collect(),
    }
}

/// One window as a session / saved workspace (`with_view`: each pane's view too).
pub fn window_session(ws: &Workspace, with_view: bool) -> WindowSession {
    // Groups in strip order.
    let mut order: Vec<GroupId> = Vec::new();
    for t in &ws.tabs {
        if let Some(g) = t.group
            && !order.contains(&g)
        {
            order.push(g);
        }
    }
    let groups = order
        .iter()
        .filter_map(|g| ws.group(*g))
        .map(|g| GroupSession {
            name: g.name.clone(),
            color: crate::services::tags::slug(g.color),
            collapsed: g.collapsed,
        })
        .collect();
    WindowSession {
        active_tab: ws.active,
        tabs: ws.tabs.iter().map(|t| tab_session(t, &order, with_view)).collect(),
        groups,
        ..Default::default()
    }
}

/// Build a workspace from a session (`use_views`: apply stored pane views). Listings start
/// lazily when a pane is first shown. `None` when no tab could be restored.
pub fn build_workspace(app: &SoyuzApp, s: &WindowSession, use_views: bool) -> Option<Workspace> {
    let groups: Vec<TabGroup> = s
        .groups
        .iter()
        .map(|g| {
            let mut t = TabGroup::new(g.name.clone(), crate::services::tags::from_name(&g.color).unwrap_or(5));
            t.collapsed = g.collapsed;
            t
        })
        .collect();
    let limit = app.config.workspace.history_graph_limit;
    let mut tabs = Vec::new();
    let mut active_id = None;
    for (i, ts) in s.tabs.iter().enumerate() {
        let mut panes = Vec::new();
        for ps in ts.panes.iter().take(4) {
            let Some(loc) = Location::parse(&ps.location) else { continue };
            let view = match (&ps.view, use_views) {
                (Some(v), true) => v.clone(),
                _ => app.view_for(&loc),
            };
            let mut pane = Pane::new(loc, view);
            pane.pending_cursor = ps.cursor.clone();
            pane.pending_selection = ps.selection.clone();
            pane.restore_scroll = (ps.scroll > 0.0).then_some(ps.scroll);
            panes.push(pane);
        }
        if panes.is_empty() {
            continue;
        }
        let mut tab = Tab::new(panes.remove(0));
        tab.panes.extend(panes);
        tab.dual = ts.dual && tab.panes.len() > 1;
        tab.quad = ts.quad && tab.panes.len() >= 4;
        tab.focused = ts.focused.min(tab.shown().max(1) - 1);
        tab.orientation = ts.orientation;
        tab.split = ts.split.clamp(0.15, 0.85);
        tab.split2 = ts.split2.clamp(0.15, 0.85);
        tab.pinned = ts.pinned;
        tab.sync_nav = ts.sync_nav;
        tab.group = ts.group.and_then(|g| groups.get(g)).map(|g| g.id).filter(|_| !ts.pinned);
        tab.visits.set_limit(limit);
        if i == s.active_tab {
            active_id = Some(tab.id);
        }
        tabs.push(tab);
    }
    if tabs.is_empty() {
        return None;
    }
    tabs.sort_by_key(|t| !t.pinned);
    let mut ws = Workspace::new(tabs.remove(0));
    ws.tabs.extend(tabs);
    ws.groups = groups;
    ws.active = active_id.and_then(|id| ws.tab_index(id)).unwrap_or(0);
    ws.normalize_groups();
    Some(ws)
}

/// Stop the background work of every pane of a workspace that goes away.
pub fn dispose_workspace(app: &mut SoyuzApp, ws: &mut Workspace) {
    for t in &mut ws.tabs {
        for p in &mut t.panes {
            ops::dispose_pane(p, &mut app.services);
        }
        app.ui.terminal.drop_tab(t.id.0);
    }
}

// ---------------------------------------------------------------------------
// Actions and commands
// ---------------------------------------------------------------------------

/// Handle a workspace action. Returns `false` for actions this module doesn't own.
pub fn action(app: &mut SoyuzApp, a: &Action, ctx: &egui::Context) -> bool {
    let i = app.ws.active;
    match a {
        Action::ToggleQuadPane => {
            let next = if app.ws.tab().layout() == PaneLayout::Quad {
                PaneLayout::Dual(app.ws.tab().orientation)
            } else {
                PaneLayout::Quad
            };
            set_layout(app, next);
        }
        Action::NewTabGroup => apply(app, WsCmd::NewGroup(i), ctx),
        Action::UngroupTab => apply(app, WsCmd::RemoveFromGroup(i), ctx),
        Action::MergeWindows => app.workspaces.ops.push(WinOp::MergeInto(app.workspaces.current)),
        Action::Workspaces => crate::palette::open(app, crate::palette::query::Mode::Commands, "workspace "),
        Action::SaveWorkspace => {
            let name = app.workspaces.last.clone().unwrap_or_else(|| default_name(app));
            app.ui.dialogs.push(dialogs::SaveWorkspaceDialog::new(name));
        }
        Action::UpdateWorkspace => match app.workspaces.last.clone() {
            Some(name) => apply(app, WsCmd::Save(name), ctx),
            None => app.ui.dialogs.push(dialogs::SaveWorkspaceDialog::new(default_name(app))),
        },
        Action::OpenWorkspace(n) => {
            let idx = *n as usize - 1;
            if idx < app.workspaces.store.workspaces.len() {
                apply(app, WsCmd::Open(idx), ctx);
            } else {
                app.ui.toasts.push(
                    Toast::info(tr!("workspaces-no-slot", n = n.to_string())).body(tr!("workspaces-no-slot-body")),
                );
            }
        }
        Action::TabHistory => {
            let tab = app.ws.tab().id;
            let pane = app.ws.pane().id;
            app.ui.dialogs.push(history::HistoryDialog::new(tab, pane));
        }
        Action::OpenTimeline => {
            let id = app.ws.pane().id;
            ops::navigate(app, id, Location::Timeline, ops::NavOpts::user());
        }
        _ => return false,
    }
    true
}

fn default_name(app: &SoyuzApp) -> String {
    let base = crate::state::crumbs::location_title(&app.ws.pane().location);
    let mut name = base.clone();
    let mut n = 2;
    while app.workspaces.store.position(&name).is_some() {
        name = format!("{base} {n}");
        n += 1;
    }
    name
}

/// Switch the active tab's layout, creating panes as needed.
pub fn set_layout(app: &mut SoyuzApp, layout: PaneLayout) {
    let want = Tab::wanted_panes(layout);
    let loc = app.ws.pane().location.clone();
    let view = app.view_for(&loc);
    let tab = app.ws.tab_mut();
    while tab.panes.len() < want {
        tab.panes.push(Pane::new(loc.clone(), view.clone()));
    }
    tab.set_layout(layout);
    let shown = tab.shown();
    for p in tab.panes.iter_mut().take(shown) {
        ops::ensure_started(p, &mut app.services);
    }
}

pub fn apply(app: &mut SoyuzApp, cmd: WsCmd, ctx: &egui::Context) {
    let n = app.ws.tabs.len();
    match cmd {
        WsCmd::NewGroup(i) if i < n => {
            let name = crate::state::crumbs::location_title(&app.ws.tabs[i].active().location);
            let color = next_color(&app.ws);
            match app.ws.new_group(i, name, color) {
                Some(g) => {
                    app.workspaces.renaming =
                        Some((g, app.ws.group(g).map(|g| g.name.clone()).unwrap_or_default(), true))
                }
                None => {
                    app.ui.toasts.push(Toast::info(tr!("workspaces-pinned-cant-group")));
                }
            }
        }
        WsCmd::AddToGroup(i, g) if i < n => app.ws.add_to_group(i, g),
        WsCmd::RemoveFromGroup(i) if i < n => app.ws.remove_from_group(i),
        WsCmd::ToggleGroup(g) => app.ws.toggle_group_collapsed(g),
        WsCmd::StartRenameGroup(g) => {
            let name = app.ws.group(g).map(|x| x.name.clone()).unwrap_or_default();
            app.workspaces.renaming = Some((g, name, true));
        }
        WsCmd::RenameGroup(g, name) => {
            if let Some(x) = app.ws.group_mut(g) {
                x.name = name.trim().to_string();
            }
            app.workspaces.renaming = None;
        }
        WsCmd::SetGroupColor(g, c) => {
            if let Some(x) = app.ws.group_mut(g) {
                x.color = c.min(7);
            }
        }
        WsCmd::Ungroup(g) => app.ws.ungroup(g),
        WsCmd::CloseGroup(g) => {
            if app.ws.group_tab_count(g) >= app.ws.tabs.len() {
                // Keep one tab: closing every tab of the window isn't how a window closes.
                ops::new_tab(app);
                let last = app.ws.active;
                app.ws.tabs[last].group = None;
                app.ws.normalize_groups();
            }
            let mut i = app.ws.tabs.len();
            while i > 0 {
                i -= 1;
                if app.ws.tabs[i].group == Some(g) {
                    ops::close_tab(app, i);
                }
            }
            app.ws.groups.retain(|x| x.id != g);
        }
        WsCmd::GroupToNewWindow(g) => match app.ws.take_group(g) {
            Some((tabs, group)) => {
                let size = current_size(ctx);
                app.workspaces.ops.push(WinOp::OpenTabs { tabs, group: Some(group), pos: None, size });
            }
            None => {
                app.ui.toasts.push(Toast::info(tr!("workspaces-keep-one-tab")));
            }
        },
        WsCmd::MoveTab { from, to, group } if from < n => app.ws.move_tab_to_group(from, to, group),
        WsCmd::DropTab { tab, target, grab } => windows::drop_tab(app, ctx, tab, target, grab),
        WsCmd::TabToNewWindow(i) if i < n => {
            let id = app.ws.tabs[i].id;
            windows::drop_tab(app, ctx, id, DropTarget::TearOff { pos: crate::geom::Point::ZERO }, egui::Vec2::ZERO);
        }
        WsCmd::TabToWindow(i, w) if i < n => {
            let id = app.ws.tabs[i].id;
            let index = usize::MAX;
            windows::drop_tab(app, ctx, id, DropTarget::Window { id: w, index }, egui::Vec2::ZERO);
        }
        WsCmd::SetLayout(l) => set_layout(app, l),
        WsCmd::Open(i) => open_saved(app, i, false, ctx),
        WsCmd::OpenInNewWindow(i) => open_saved(app, i, true, ctx),
        WsCmd::Save(name) => {
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let mut layout = window_session(&app.ws, true);
            if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
                layout.size = Some([r.width(), r.height()]);
            }
            let now = crate::services::recent::unix_now();
            let existed = app.workspaces.store.position(&name).is_some();
            app.workspaces.store.upsert(&name, layout, now);
            app.workspaces.persist();
            app.workspaces.last = Some(name.clone());
            app.ui.toasts.push(Toast::success(if existed {
                tr!("workspaces-updated", name = &name)
            } else {
                tr!("workspaces-saved", name = &name)
            }));
        }
        WsCmd::Delete(i) => {
            if let Some(w) = app.workspaces.store.remove(i) {
                if app.workspaces.last.as_deref() == Some(w.name.as_str()) {
                    app.workspaces.last = None;
                }
                app.workspaces.persist();
                rebind(app);
                app.ui.toasts.push(Toast::info(tr!("workspaces-deleted", name = &w.name)));
            }
        }
        WsCmd::Rename(i, name) => {
            let old = app.workspaces.store.workspaces.get(i).map(|w| w.name.clone());
            if app.workspaces.store.rename(i, &name) {
                if app.workspaces.last == old {
                    app.workspaces.last = Some(name.trim().to_string());
                }
                app.workspaces.persist();
            } else {
                app.ui.toasts.push(Toast::warning(tr!("workspaces-name-exists")));
            }
        }
        WsCmd::SetHotkey(i, chord) => {
            let chord = chord.trim().to_string();
            if !chord.is_empty() && soyuz_core::config::parse_sequence(&chord).is_err() {
                app.ui.toasts.push(Toast::warning(tr!("workspaces-bad-chord", chord = &chord)));
                return;
            }
            if let Some(w) = app.workspaces.store.workspaces.get_mut(i) {
                w.hotkey = chord;
                app.workspaces.persist();
                rebind(app);
            }
        }
        WsCmd::MoveUp(i) if i > 0 => {
            app.workspaces.store.swap(i - 1, i);
            app.workspaces.persist();
            rebind(app);
        }
        WsCmd::CloseWindow { id, merge } => app.workspaces.ops.push(WinOp::Close { id, merge }),
        WsCmd::Manage => app.ui.dialogs.push(dialogs::ManageWorkspacesDialog::default()),
        WsCmd::Visit { pane, location } => {
            ops::navigate(app, pane, location, ops::NavOpts { revisit: true, ..ops::NavOpts::user() });
            ops::focus_pane(app, pane);
        }
        _ => {}
    }
}

/// The next group color: the first tag color no group uses yet (blue first).
fn next_color(ws: &Workspace) -> u8 {
    const ORDER: [u8; 8] = [5, 3, 1, 6, 4, 0, 2, 7];
    ORDER.iter().copied().find(|c| !ws.groups.iter().any(|g| g.color == *c)).unwrap_or(5)
}

fn current_size(ctx: &egui::Context) -> Option<egui::Vec2> {
    ctx.input(|i| i.viewport().inner_rect).map(|r| r.size())
}

fn open_saved(app: &mut SoyuzApp, i: usize, new_window: bool, ctx: &egui::Context) {
    let Some(saved) = app.workspaces.store.workspaces.get(i).cloned() else { return };
    let Some(mut ws) = build_workspace(app, &saved.layout, true) else {
        app.ui.toasts.push(Toast::warning(tr!("workspaces-nothing-to-open", name = &saved.name)));
        return;
    };
    app.workspaces.last = Some(saved.name.clone());
    if new_window {
        let size = saved.layout.size.map(|[w, h]| egui::vec2(w, h)).or_else(|| current_size(ctx));
        app.workspaces.ops.push(WinOp::OpenWorkspace { ws: Box::new(ws), pos: None, size });
    } else {
        std::mem::swap(&mut app.ws, &mut ws);
        app.ws.closed = std::mem::take(&mut ws.closed);
        dispose_workspace(app, &mut ws);
        app.ui.focus = crate::app::Focus::List;
    }
    app.ui.toasts.push(Toast::success(tr!("workspaces-opened", name = &saved.name)));
}

/// Rebuild the keymap from the config and add the workspace hotkeys.
pub fn rebind(app: &mut SoyuzApp) {
    let (km, _) = soyuz_core::config::keymap(&app.config);
    app.keymap = km;
    bind_hotkeys(app);
}

/// Add the saved workspaces' hotkeys to the keymap (after it was built from the config).
pub fn bind_hotkeys(app: &mut SoyuzApp) {
    bind_hotkeys_into(&mut app.keymap, &app.workspaces.store);
}

pub fn bind_hotkeys_into(keymap: &mut soyuz_core::config::Keymap, store: &WorkspaceStore) {
    let n = soyuz_core::config::actions::WORKSPACE_SLOTS as usize;
    for (i, w) in store.workspaces.iter().enumerate().take(n) {
        if w.hotkey.trim().is_empty() {
            continue;
        }
        let action = Action::OpenWorkspace(i as u8 + 1);
        match soyuz_core::config::parse_sequence(w.hotkey.trim()) {
            Ok(seq) if seq.len() == 1 => keymap.bind_stealing(KeyContext::Global, action, seq[0]),
            Ok(seq) => keymap.bind_sequence(KeyContext::Global, action, seq),
            Err(_) => log::warn!("workspaces.toml: “{}”: bad hotkey “{}”", w.name, w.hotkey),
        }
    }
}

/// Palette title for workspace slots (`None` for other actions).
pub fn palette_title(app: &SoyuzApp, a: &Action) -> Option<String> {
    match a {
        Action::OpenWorkspace(n) => app
            .workspaces
            .store
            .workspaces
            .get(*n as usize - 1)
            .map(|w| tr!("workspaces-palette-title", name = &w.name)),
        _ => None,
    }
}

/// Whether the palette should list `a` (workspace slots only when saved).
pub fn palette_shows(app: &SoyuzApp, a: &Action) -> bool {
    match a {
        Action::OpenWorkspace(n) => (*n as usize) <= app.workspaces.store.workspaces.len(),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

/// Group and window entries of a tab's context menu.
pub fn tab_menu_extra(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>, i: usize) {
    let Some(tab) = app.ws.tabs.get(i) else { return };
    let ws = &app.ws;
    menu_separator(ui);
    if !tab.pinned {
        if menu_item(ui, Some(ph::RECTANGLE_DASHED), tr!("workspaces-menu-add-new-group"), None, true).clicked() {
            cmds.push(Cmd::Workspace(WsCmd::NewGroup(i)));
            ui.close();
        }
        let others: Vec<&TabGroup> = ws.groups.iter().filter(|g| Some(g.id) != tab.group).collect();
        if !others.is_empty() {
            submenu(ui, Some(ph::STACK), tr!("workspaces-menu-add-to-group"), |ui| {
                for g in others {
                    let label =
                        if g.name.is_empty() { tr!("workspaces-unnamed-group").to_string() } else { g.name.clone() };
                    if dot_item(ui, g.color, &label).clicked() {
                        cmds.push(Cmd::Workspace(WsCmd::AddToGroup(i, g.id)));
                        ui.close();
                    }
                }
            });
        }
        if tab.group.is_some()
            && menu_item(ui, Some(ph::RECTANGLE), tr!("workspaces-menu-remove-from-group"), None, true).clicked()
        {
            cmds.push(Cmd::Workspace(WsCmd::RemoveFromGroup(i)));
            ui.close();
        }
    }
    menu_separator(ui);
    let can_move = ws.tabs.len() > 1;
    if menu_item(ui, Some(ph::APP_WINDOW), tr!("workspaces-menu-move-new-window"), None, can_move).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::TabToNewWindow(i)));
        ui.close();
    }
    let targets = windows::window_list(app);
    let here = app.workspaces.current;
    let others: Vec<&(WindowId, String)> = targets.iter().filter(|(id, _)| *id != here).collect();
    if !others.is_empty() {
        submenu(ui, Some(ph::BROWSERS), tr!("workspaces-menu-move-to-window"), |ui| {
            for (id, title) in others {
                if menu_item(ui, Some(ph::APP_WINDOW), title, None, can_move).clicked() {
                    cmds.push(Cmd::Workspace(WsCmd::TabToWindow(i, *id)));
                    ui.close();
                }
            }
        });
    }
    let km = &app.keymap;
    let sc = crate::keys::chord_label(km, &Action::TabHistory);
    if menu_item(ui, Some(ph::GIT_FORK), tr!("workspaces-history-title"), sc.as_deref(), true).clicked() {
        cmds.push(Cmd::Action(Action::TabHistory));
        ui.close();
    }
}

/// A menu row with a colored dot.
pub fn dot_item(ui: &mut Ui, color: u8, label: &str) -> egui::Response {
    let p = crate::theme::palette(ui.ctx());
    let r = menu_item(ui, Some(""), label, None, true);
    let c = p.tags[(color as usize).min(7)];
    ui.painter().circle_filled(egui::pos2(r.rect.left() + 17.0, r.rect.center().y), 5.0, c);
    r
}

/// Display name of tag color `c` (group colors).
fn color_name(c: usize) -> &'static str {
    match c {
        0 => tr!("workspaces-color-red"),
        1 => tr!("workspaces-color-orange"),
        2 => tr!("workspaces-color-yellow"),
        3 => tr!("workspaces-color-green"),
        4 => tr!("workspaces-color-cyan"),
        5 => tr!("workspaces-color-blue"),
        6 => tr!("workspaces-color-purple"),
        _ => tr!("workspaces-color-gray"),
    }
}

/// Context menu of a group chip.
pub fn group_menu(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>, g: GroupId) {
    ui.set_width(220.0);
    let Some(group) = app.ws.group(g) else { return };
    if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("workspaces-group-rename"), None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::StartRenameGroup(g)));
        ui.close();
    }
    // Color swatches.
    let p = crate::theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        for (c, color) in p.tags.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 22.0), egui::Sense::click());
            let center = r.center();
            ui.painter().circle_filled(center, 7.0, *color);
            if group.color as usize == c {
                ui.painter().circle_stroke(center, 9.5, egui::Stroke::new(1.5, p.text_primary));
            } else if resp.hovered() {
                ui.painter().circle_stroke(center, 9.0, egui::Stroke::new(1.0, p.border_strong));
            }
            let name = color_name(c);
            crate::ui::a11y::button(&resp, name);
            if resp.on_hover_text(name).clicked() {
                cmds.push(Cmd::Workspace(WsCmd::SetGroupColor(g, c as u8)));
                ui.close();
            }
        }
    });
    menu_separator(ui);
    let label = if group.collapsed { tr!("workspaces-group-expand") } else { tr!("workspaces-group-collapse") };
    let icon = if group.collapsed { ph::CARET_DOWN } else { ph::CARET_RIGHT };
    if menu_item(ui, Some(icon), label, None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::ToggleGroup(g)));
        ui.close();
    }
    if menu_item(ui, Some(ph::PLUS), tr!("workspaces-group-new-tab"), None, true).clicked() {
        if let Some(span) = app.ws.group_span(g) {
            cmds.push(Cmd::NewTabAfter(span.end - 1));
        }
        ui.close();
    }
    if menu_item(ui, Some(ph::RECTANGLE), tr!("workspaces-group-ungroup"), None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::Ungroup(g)));
        ui.close();
    }
    menu_separator(ui);
    let can_move = app.ws.group_tab_count(g) < app.ws.tabs.len();
    if menu_item(ui, Some(ph::APP_WINDOW), tr!("workspaces-group-to-new-window"), None, can_move).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::GroupToNewWindow(g)));
        ui.close();
    }
    if menu_item(ui, Some(ph::X), tr!("workspaces-group-close"), None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::CloseGroup(g)));
        ui.close();
    }
}

/// "Workspaces" submenu (app menu): open / save / manage.
pub fn workspaces_menu(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>) {
    submenu(ui, Some(ph::STACK), tr!("workspaces-menu"), |ui| {
        ui.set_width(240.0);
        let km = &app.keymap;
        for (i, w) in app.workspaces.store.workspaces.iter().enumerate() {
            let sc = (i < 9).then(|| crate::keys::chord_label(km, &Action::OpenWorkspace(i as u8 + 1))).flatten();
            let current = app.workspaces.last.as_deref() == Some(w.name.as_str());
            let icon = if current { ph::CHECK } else { ph::STACK };
            if menu_item(ui, Some(icon), &w.name, sc.as_deref(), true).clicked() {
                cmds.push(Cmd::Workspace(WsCmd::Open(i)));
                ui.close();
            }
        }
        if !app.workspaces.store.workspaces.is_empty() {
            menu_separator(ui);
        }
        if menu_item(ui, Some(ph::FLOPPY_DISK), tr!("workspaces-menu-save-as"), None, true).clicked() {
            cmds.push(Cmd::Action(Action::SaveWorkspace));
            ui.close();
        }
        let has_last = app.workspaces.last.is_some();
        if menu_item(ui, Some(ph::ARROWS_CLOCKWISE), tr!("workspaces-menu-update"), None, has_last).clicked() {
            cmds.push(Cmd::Action(Action::UpdateWorkspace));
            ui.close();
        }
        if menu_item(ui, Some(ph::GEAR_SIX), tr!("workspaces-menu-manage"), None, true).clicked() {
            cmds.push(Cmd::Workspace(WsCmd::Manage));
            ui.close();
        }
    });
}

/// The Timeline row (sidebar, under Recent).
pub fn timeline_row(ui: &mut Ui, current: &Location, pane: PaneId, cmds: &mut Vec<Cmd>) {
    let active = matches!(current, Location::Timeline);
    let r = crate::ui::widgets::NavRow {
        icon: ph::CALENDAR_DOTS,
        label: tr!("workspaces-timeline"),
        active,
        icon_color: None,
        usage: None,
    }
    .show(ui);
    let r = r.on_hover_text(tr!("workspaces-timeline-tip"));
    if r.clicked() {
        cmds.push(Cmd::FocusPane(pane));
        cmds.push(Cmd::Navigate { pane, location: Location::Timeline, select: None });
    }
    if r.middle_clicked() {
        cmds.push(Cmd::OpenInNewTab { location: Location::Timeline, activate: false });
    }
}

/// `SOYUZ_SCRIPT` steps `ws:<verb> [arg]` (see `devscript`).
pub fn devscript(app: &mut SoyuzApp, ctx: &egui::Context, arg: &str) {
    use soyuz_core::config::Orientation;
    let mut it = arg.split_whitespace();
    let verb = it.next().unwrap_or("");
    let rest: Vec<&str> = it.collect();
    let num = |k: usize| rest.get(k).and_then(|s| s.parse::<usize>().ok());
    let active = app.ws.active;
    let cmd = match verb {
        "group" => Some(WsCmd::NewGroup(active)),
        "add" => app.ws.tab().group.zip(num(0)).map(|(g, i)| WsCmd::AddToGroup(i, g)),
        "collapse" => app.ws.tabs.iter().find_map(|t| t.group).map(WsCmd::ToggleGroup),
        "color" => app.ws.tab().group.zip(num(0)).map(|(g, c)| WsCmd::SetGroupColor(g, c as u8)),
        "rename" => app.ws.tab().group.map(|g| WsCmd::RenameGroup(g, rest.join(" "))),
        "save" => Some(WsCmd::Save(rest.join(" "))),
        "open" => num(0).map(|n| WsCmd::Open(n.saturating_sub(1))),
        "layout" => Some(WsCmd::SetLayout(match rest.first().copied() {
            Some("single") => PaneLayout::Single,
            Some("stacked") => PaneLayout::Dual(Orientation::Horizontal),
            Some("quad") => PaneLayout::Quad,
            _ => PaneLayout::Dual(Orientation::Vertical),
        })),
        "sync" => {
            app.ws.tab_mut().sync_nav = true;
            None
        }
        "select" => {
            if let Some(i) = num(0).filter(|i| *i < app.ws.tabs.len()) {
                app.ws.active = i;
            }
            None
        }
        "tear" => num(0).map(WsCmd::TabToNewWindow),
        "move" => {
            let w = num(1).and_then(|k| windows::window_list(app).get(k).map(|(id, _)| *id));
            num(0).zip(w).map(|(i, w)| WsCmd::TabToWindow(i, w))
        }
        "close" => {
            let w = num(0).and_then(|k| windows::window_list(app).get(k).map(|(id, _)| *id));
            w.map(|id| WsCmd::CloseWindow { id, merge: rest.get(1) == Some(&"merge") })
        }
        "drop" => {
            // `drop <tab> <x> <y>`: release tab `tab` at screen point (x, y) as if dragged.
            let pt = rest.get(1).zip(rest.get(2)).and_then(|(x, y)| Some(egui::pos2(x.parse().ok()?, y.parse().ok()?)));
            num(0).filter(|i| *i < app.ws.tabs.len()).zip(pt).map(|(i, pt)| {
                let tab = app.ws.tabs[i].id;
                let dist = app.config.workspace.tear_off_distance;
                let target = crate::state::workspace_windows::resolve_drop(
                    app.workspaces.current,
                    crate::ui::geom::point(pt),
                    &app.workspaces.geoms,
                    dist,
                    app.ws.tabs.len() > 1,
                );
                log::info!("SOYUZ_SCRIPT: drop → {target:?}");
                WsCmd::DropTab { tab, target, grab: egui::Vec2::ZERO }
            })
        }
        "merge" => {
            app.workspaces.ops.push(WinOp::MergeInto(WindowId::MAIN));
            None
        }
        _ => {
            log::warn!("SOYUZ_SCRIPT: unknown ws step “{arg}”");
            None
        }
    };
    if let Some(c) = cmd {
        apply(app, c, ctx);
    }
}

/// Layout entries for the dual-pane button menu.
pub fn layout_menu(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>) {
    use crate::ui::widgets::menu_check;
    use soyuz_core::config::Orientation;
    let cur = app.ws.tab().layout();
    for (layout, label) in [
        (PaneLayout::Single, tr!("workspaces-layout-single")),
        (PaneLayout::Dual(Orientation::Vertical), tr!("workspaces-layout-dual")),
        (PaneLayout::Dual(Orientation::Horizontal), tr!("workspaces-layout-stacked")),
        (PaneLayout::Quad, tr!("workspaces-layout-quad")),
    ] {
        let sc = match layout {
            PaneLayout::Quad => crate::keys::chord_label(&app.keymap, &Action::ToggleQuadPane),
            _ => None,
        };
        if menu_check(ui, cur == layout, label, sc.as_deref()).clicked() {
            cmds.push(Cmd::Workspace(WsCmd::SetLayout(layout)));
            ui.close();
        }
    }
}
