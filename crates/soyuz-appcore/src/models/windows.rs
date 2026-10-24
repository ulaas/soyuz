//! In-process windows, model side. Every extra window owns a [`Workspace`] (tabs, groups)
//! plus its per-window UI state ([`WindowUi`]: address bar, focus, preview pane, Quick
//! Look, search bar, bottom / activity panel). While a window is handled its workspace
//! and UI state are swapped into `app.ws` / `app.ui` ([`with_window`]), so the whole app
//! logic runs unchanged. Jobs, services, settings, the configuration, the terminal and
//! the media player are shared (the player shows in one window, [`player_here`]). The GUI
//! draws each extra window as a viewport (`ui::workspaces::windows`).
//!
//! Routing: mailbox / watcher messages for a pane run in the window holding it
//! ([`with_pane_window`]); requests from outside (FileManager1, `soyuz open`, a second
//! launch) run in the focused window ([`with_focused_window`], then [`raise`]); a new
//! window at a location is [`open_location`].
//!
//! Structural changes (open, close, move tabs between windows) are queued as [`WinOp`]s
//! and applied between frames ([`apply_ops`]), never while a window's workspace is
//! swapped in.
//!
//! Closing a window with several tabs asks (or follows `[workspace] close_window_tabs`)
//! whether to move its tabs to another window or close them. Closing the *main* window
//! while others are open hands the main window over to the next window. "Quit" closes
//! everything.

use soyuz_core::Location;
use soyuz_core::config::CloseWindowTabs;

use super::frontend::FrontendCmd;
use super::panels::BottomUi;
use super::preview::PreviewModel;
use super::quicklook::QuickLook;
use super::search::SearchUi;
use super::toasts::Toast;
use super::workspaces::{build_workspace, dispose_workspace, tab_session, window_session};
use crate::core::{AddressState, AppCore, Focus, UiState};
use crate::geom::{Point, Vec2, vec2};
use crate::host::{Host, WindowCmd};
use crate::state::pane::{Pane, PaneId};
use crate::state::session::{PanelSession, WindowSession};
use crate::state::workspace::{ClosedTab, Tab, TabGroup, TabId, Workspace};
use crate::state::workspace_windows::{DropTarget, WindowId};

/// Per-window UI state, swapped into `app.ui` while the window is handled ([`swap_ui`]).
/// The rest of `UiState` (toasts, volumes, cloud, theme, sidebar, tree, hidden / system
/// files, checkbox mode, welcome card, settings, focus mode, terminal, player) is shared.
pub struct WindowUi {
    pub address: AddressState,
    pub focus: Focus,
    pub title: String,
    pub filter_focus: bool,
    pub preview: PreviewModel,
    pub quicklook: QuickLook,
    /// The search bar (`open`, `form`, `focus`, `saving`, `error`).
    pub search: SearchUi,
    /// The bottom / activity panel (`open`, `tab`, `height`, `activity_open`).
    pub bottom: BottomUi,
}

impl WindowUi {
    /// A new window: the preview pane per `[layout] preview_pane` at the main window's
    /// width, a closed search bar and a closed bottom panel.
    pub fn new(app: &AppCore) -> Self {
        let mut preview = PreviewModel::new(app.config.layout.preview_pane);
        preview.width = app.ui.preview.width;
        WindowUi {
            address: AddressState::default(),
            focus: Focus::List,
            title: String::new(),
            filter_focus: false,
            preview,
            quicklook: QuickLook::default(),
            search: SearchUi::new(&app.config),
            bottom: BottomUi::closed(app.ui.bottom.height),
        }
    }
}

/// Exchange the per-window part of `ui` with `w` (its own inverse). The one place that
/// knows which `UiState` fields belong to a window: [`swap`], [`with_window`] and the
/// main window taking over another (`promote`) all go through it.
pub fn swap_ui(ui: &mut UiState, w: &mut WindowUi) {
    std::mem::swap(&mut ui.address, &mut w.address);
    std::mem::swap(&mut ui.focus, &mut w.focus);
    std::mem::swap(&mut ui.title, &mut w.title);
    std::mem::swap(&mut ui.filter_focus, &mut w.filter_focus);
    std::mem::swap(&mut ui.preview, &mut w.preview);
    std::mem::swap(&mut ui.quicklook, &mut w.quicklook);
    ui.search.swap_window(&mut w.search);
    ui.bottom.swap_window(&mut w.bottom);
}

pub struct SecondaryWindow {
    pub id: WindowId,
    pub ws: Workspace,
    pub ui: WindowUi,
    /// Where the window was created (the frontend creates it there; the user moves and
    /// resizes the window freely afterwards).
    pub start_pos: Option<Point>,
    pub start_size: Vec2,
    pub start_maximized: bool,
    /// Last known outer position / inner size (for the session).
    pub pos: Option<Point>,
    pub size: Vec2,
    pub maximized: bool,
}

/// Deferred window operations.
pub enum WinOp {
    OpenTabs {
        tabs: Vec<Tab>,
        group: Option<TabGroup>,
        pos: Option<Point>,
        size: Option<Vec2>,
    },
    OpenWorkspace {
        ws: Box<Workspace>,
        pos: Option<Point>,
        size: Option<Vec2>,
    },
    InsertTab {
        to: WindowId,
        tab: Box<Tab>,
        group: Option<TabGroup>,
        index: usize,
    },
    /// Move the only tab of window `from` into `to` (and close `from`).
    MoveLastTab {
        from: WindowId,
        to: WindowId,
        index: usize,
    },
    Close {
        id: WindowId,
        merge: bool,
    },
    MergeInto(WindowId),
    Focus(WindowId),
}

/// What the frontend knows about the screen when window operations are applied.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Screen {
    /// Top-left corner of the main window's outer frame (new windows cascade from it).
    pub main_outer: Option<Point>,
    /// Size of the main window's monitor (restored positions far outside are dropped).
    pub monitor: Option<Vec2>,
}

/// Position of window `id` in `app.workspaces.windows`.
pub fn index(app: &AppCore, id: WindowId) -> Option<usize> {
    app.workspaces.windows.iter().position(|w| w.id == id)
}

/// Window `id` is open (the main window always is).
pub fn exists(app: &AppCore, id: WindowId) -> bool {
    id.is_main() || index(app, id).is_some()
}

/// Swap window `k`'s workspace and UI state with `app.ws` / `app.ui` (its own inverse).
/// Callers keep `app.workspaces.current` in step; prefer [`with_window`].
pub fn swap(app: &mut AppCore, k: usize) {
    let w = &mut app.workspaces.windows[k];
    std::mem::swap(&mut app.ws, &mut w.ws);
    swap_ui(&mut app.ui, &mut w.ui);
}

/// Make `id` the window being handled (`app.workspaces.current`; new jobs are stamped
/// with it so their activity panel opens there).
fn set_current(app: &mut AppCore, id: WindowId) {
    app.workspaces.current = id;
    app.fileops.jobs.window = id;
}

/// What [`swap_to`] did, for [`swap_back`] (frontends that need more than `&mut AppCore`
/// for a window frame; everything else uses [`with_window`]).
pub struct Swapped {
    was: WindowId,
    out: Option<usize>,
    into: Option<usize>,
}

/// Swap window `id` (open) in, whichever window is swapped in now.
pub fn swap_to(app: &mut AppCore, id: WindowId) -> Swapped {
    let was = app.workspaces.current;
    if id == was {
        return Swapped { was, out: None, into: None };
    }
    let out = if was.is_main() { None } else { index(app, was) };
    if let Some(k) = out {
        swap(app, k);
    }
    let into = if id.is_main() { None } else { index(app, id) };
    if let Some(k) = into {
        swap(app, k);
    }
    set_current(app, id);
    Swapped { was, out, into }
}

pub fn swap_back(app: &mut AppCore, s: Swapped) {
    if let Some(k) = s.into {
        swap(app, k);
    }
    if let Some(k) = s.out {
        swap(app, k);
    }
    set_current(app, s.was);
}

/// Run `f` with window `id` swapped into `app.ws` / `app.ui` (and `current` set), from
/// any state: the main window or another window may be swapped in. Window structure
/// only changes in [`apply_ops`], so `f` may queue [`WinOp`]s but never sees a window
/// vanish. `None` when the window is gone.
pub fn with_window<R>(app: &mut AppCore, id: WindowId, f: impl FnOnce(&mut AppCore) -> R) -> Option<R> {
    if !exists(app, id) {
        return None;
    }
    let s = swap_to(app, id);
    let r = f(app);
    swap_back(app, s);
    Some(r)
}

/// The focused window (overlays, keyboard; the main window when the last focused one is
/// gone).
pub fn focused(app: &AppCore) -> WindowId {
    let o = app.workspaces.owner;
    if exists(app, o) { o } else { WindowId::MAIN }
}

/// Run `f` in the focused window: requests from outside the window (FileManager1, `soyuz
/// open`, a second launch handed over to this process) open their tabs there. Raise it
/// with [`raise`] when the request should bring it to the front.
pub fn with_focused_window<R>(app: &mut AppCore, f: impl FnOnce(&mut AppCore) -> R) -> R {
    let s = swap_to(app, focused(app));
    let r = f(app);
    swap_back(app, s);
    r
}

/// Run `f` once per window (the main window first), each swapped in: settings that are
/// per window (bottom panel, preview pane) applied to all of them.
pub fn for_each_window(app: &mut AppCore, mut f: impl FnMut(&mut AppCore)) {
    let ids: Vec<WindowId> =
        std::iter::once(WindowId::MAIN).chain(app.workspaces.windows.iter().map(|w| w.id)).collect();
    for id in ids {
        with_window(app, id, &mut f);
    }
}

/// The workspaces of every window, whichever is swapped in (pumps that look panes up by
/// id: search results, folder sizes).
pub fn all_workspaces_mut(app: &mut AppCore) -> Vec<&mut Workspace> {
    std::iter::once(&mut app.ws).chain(app.workspaces.windows.iter_mut().map(|w| &mut w.ws)).collect()
}

/// Un-minimize window `id` and bring it to the front. Wayland compositors may only flash
/// it; Windows only lets the foreground process (or one it allowed) take the focus.
pub fn raise(host: &dyn Host, id: WindowId) {
    let w = crate::host::WindowId::Window(id);
    host.window(w, WindowCmd::Minimized(false));
    host.window(w, WindowCmd::Focus);
}

/// The window whose workspace holds `pane`.
pub fn window_of_pane(app: &AppCore, pane: PaneId) -> Option<WindowId> {
    let cur = app.workspaces.current;
    if app.ws.find_pane(pane).is_some() {
        return Some(cur);
    }
    // The current window's slot holds the main workspace while another window is in.
    let w = app.workspaces.windows.iter().find(|w| w.ws.find_pane(pane).is_some())?;
    Some(if w.id == cur { WindowId::MAIN } else { w.id })
}

/// Run `f` with the window holding `pane` swapped in, its workspace and UI state
/// (mailbox / watcher routing). `None` when no window has that pane.
pub fn with_pane_window<R>(app: &mut AppCore, pane: PaneId, f: impl FnOnce(&mut AppCore) -> R) -> Option<R> {
    let id = window_of_pane(app, pane)?;
    with_window(app, id, f)
}

// ---------------------------------------------------------------------------
// Media player
// ---------------------------------------------------------------------------

/// Pick the window that shows the media player this frame (start of the main frame, no
/// window swapped in): the focused window when Quick Look is open there, else a window
/// that already plays in Quick Look, else any window with Quick Look open, else the
/// focused window when its preview pane is open; otherwise the player stays where it
/// is. Moving the player closes it (the new window opens its own media).
pub fn update_player_window(app: &mut AppCore) {
    debug_assert!(app.workspaces.current.is_main());
    let ui_of = |app: &AppCore, id: WindowId| -> Option<(bool, bool)> {
        if id.is_main() {
            return Some((app.ui.quicklook.open, app.ui.preview.open));
        }
        app.workspaces.windows.iter().find(|w| w.id == id).map(|w| (w.ui.quicklook.open, w.ui.preview.open))
    };
    let ql = |app: &AppCore, id: WindowId| ui_of(app, id).is_some_and(|(q, _)| q);
    let owner = focused(app);
    let prev = if exists(app, app.workspaces.player_window) { app.workspaces.player_window } else { WindowId::MAIN };
    let next = if ql(app, owner) {
        owner
    } else if ql(app, prev) {
        prev
    } else if let Some(id) =
        std::iter::once(WindowId::MAIN).chain(app.workspaces.windows.iter().map(|w| w.id)).find(|id| ql(app, *id))
    {
        id
    } else if ui_of(app, owner).is_some_and(|(_, p)| p) {
        owner
    } else {
        prev
    };
    if next != app.workspaces.player_window {
        app.ui.player.close();
        app.workspaces.player_window = next;
    }
}

/// This frame's window shows the media player (others show posters).
pub fn player_here(app: &AppCore) -> bool {
    app.workspaces.player_window == app.workspaces.current
}

/// Workspace of a window while no window is swapped in.
fn ws_mut(app: &mut AppCore, id: WindowId) -> Option<&mut Workspace> {
    if id.is_main() {
        return Some(&mut app.ws);
    }
    app.workspaces.windows.iter_mut().find(|w| w.id == id).map(|w| &mut w.ws)
}

/// `(window, title)` for every window (menus).
pub fn window_list(app: &AppCore) -> Vec<(WindowId, String)> {
    let title = |ws: &Workspace| {
        let n = ws.tabs.len();
        let t = crate::state::crumbs::location_title(&ws.pane().location);
        if n > 1 { tr!("workspaces-window-more-tabs", title = t, count = n - 1) } else { t }
    };
    // `app.ws` is the current window; the main workspace sits in the current window's
    // slot while a secondary window is swapped in.
    let mut out = Vec::new();
    let cur = app.workspaces.current;
    let main_ws = if cur.is_main() {
        &app.ws
    } else {
        match app.workspaces.windows.iter().find(|w| w.id == cur) {
            Some(w) => &w.ws,
            None => &app.ws,
        }
    };
    out.push((WindowId::MAIN, tr!("workspaces-main-window", title = title(main_ws))));
    for w in &app.workspaces.windows {
        let ws = if w.id == cur { &app.ws } else { &w.ws };
        out.push((w.id, title(ws)));
    }
    out
}

/// Create a window for `ws` (applied from [`apply_ops`]).
fn create_window(
    app: &mut AppCore,
    host: &dyn Host,
    screen: &Screen,
    ws: Workspace,
    pos: Option<Point>,
    size: Option<Vec2>,
    max: bool,
) -> WindowId {
    let size = size.unwrap_or(vec2(1100.0, 720.0));
    let size = vec2(size.x.max(560.0), size.y.max(360.0));
    // Default position: cascade from the main window.
    let pos = pos.or_else(|| {
        let n = app.workspaces.windows.len() as f32 + 1.0;
        screen.main_outer.map(|p| p + vec2(36.0 * n, 36.0 * n))
    });
    let id = WindowId::next();
    let ui = WindowUi::new(app);
    app.workspaces.windows.push(SecondaryWindow {
        id,
        ws,
        ui,
        start_pos: pos,
        start_size: size,
        start_maximized: max,
        pos,
        size,
        maximized: max,
    });
    app.workspaces.owner = id;
    host.redraw();
    id
}

/// Open a new window showing `loc` (Ctrl+N / "New window", a second launch handed over
/// to this process). The window is created between frames ([`apply_ops`]), becomes the
/// focused window (`owner`) and opens activated.
pub fn open_location(app: &mut AppCore, loc: Location) {
    let mut pane = Pane::new(loc.clone(), app.view_for(&loc));
    crate::ops::start_listing(&mut pane, &mut app.services, false);
    let size = app.workspaces.current_size();
    app.workspaces.ops.push(WinOp::OpenTabs { tabs: vec![Tab::new(pane)], group: None, pos: None, size });
}

/// Move every tab of `from` to the end of `into`.
fn move_all(from: &mut Workspace, into: &mut Workspace) {
    let groups = std::mem::take(&mut from.groups);
    let tabs = std::mem::take(&mut from.tabs);
    from.active = 0;
    let keep = into.active;
    for t in tabs {
        let g = t.group.and_then(|g| groups.iter().find(|x| x.id == g).cloned());
        let end = into.tabs.len();
        into.insert_tab(t, end, g);
    }
    into.active = keep.min(into.tabs.len().saturating_sub(1));
}

/// Remember a closing window's tabs in the main window's "reopen closed tab" list.
fn remember_closed(main: &mut Workspace, gone: &Workspace) {
    let groups: Vec<_> = gone.groups.iter().map(|g| g.id).collect();
    for t in &gone.tabs {
        main.closed.push(ClosedTab { index: main.tabs.len(), session: tab_session(t, &groups, false) });
    }
    let excess = main.closed.len().saturating_sub(Workspace::MAX_CLOSED);
    main.closed.drain(..excess);
}

/// The main window takes over window `k` (its workspace, place and size).
fn promote(app: &mut AppCore, host: &dyn Host, k: usize) {
    let mut w = app.workspaces.windows.remove(k);
    std::mem::swap(&mut app.ws, &mut w.ws);
    dispose_workspace(app, &mut w.ws);
    app.ws.closed.append(&mut w.ws.closed);
    // Its per-window UI too (preview, search bar, bottom panel…); the frontend moves
    // what it keeps per window when it sees `promoted`.
    swap_ui(&mut app.ui, &mut w.ui);
    app.ui.title.clear();
    app.workspaces.promoted = Some(w.id);
    if app.workspaces.player_window == w.id {
        app.workspaces.player_window = WindowId::MAIN;
    }
    let root = crate::host::WindowId::Root;
    if w.maximized {
        host.window(root, WindowCmd::Maximized(true));
    } else {
        host.window(root, WindowCmd::Maximized(false));
        if let Some(p) = w.pos {
            host.window(root, WindowCmd::OuterPosition(p));
        }
        host.window(root, WindowCmd::InnerSize(w.size));
    }
    host.window(root, WindowCmd::Focus);
}

fn close_window(app: &mut AppCore, host: &dyn Host, id: WindowId, merge: bool) {
    if id.is_main() {
        if app.workspaces.windows.is_empty() {
            app.workspaces.quitting = true;
            host.window(crate::host::WindowId::Root, WindowCmd::Close);
            return;
        }
        if merge {
            let (main, next) = (&mut app.ws, &mut app.workspaces.windows[0].ws);
            move_all(main, next);
        } else {
            let next = &mut app.workspaces.windows[0].ws;
            remember_closed(next, &app.ws);
        }
        promote(app, host, 0);
    } else if let Some(k) = index(app, id) {
        let mut w = app.workspaces.windows.remove(k);
        if merge {
            move_all(&mut w.ws, &mut app.ws);
        } else {
            remember_closed(&mut app.ws, &w.ws);
            dispose_workspace(app, &mut w.ws);
        }
    }
    app.workspaces.geoms.retain(|g| g.id.is_main() || g.id != id);
    if app.workspaces.owner == id || index(app, app.workspaces.owner).is_none() {
        app.workspaces.owner = WindowId::MAIN;
    }
}

/// Apply queued window operations (between frames: no window is swapped in).
pub fn apply_ops(app: &mut AppCore, host: &dyn Host, screen: &Screen) {
    debug_assert!(app.workspaces.current.is_main());
    for op in std::mem::take(&mut app.workspaces.ops) {
        match op {
            WinOp::OpenTabs { mut tabs, group, pos, size } => {
                if tabs.is_empty() {
                    continue;
                }
                let mut ws = Workspace::new(tabs.remove(0));
                ws.tabs.extend(tabs);
                if let Some(g) = group {
                    for t in &mut ws.tabs {
                        if !t.pinned {
                            t.group = Some(g.id);
                        }
                    }
                    ws.groups.push(g);
                }
                ws.normalize_groups();
                create_window(app, host, screen, ws, pos, size, false);
            }
            WinOp::OpenWorkspace { ws, pos, size } => {
                create_window(app, host, screen, *ws, pos, size, false);
            }
            WinOp::InsertTab { to, tab, group, index } => {
                let to = if ws_mut(app, to).is_some() { to } else { WindowId::MAIN };
                if let Some(ws) = ws_mut(app, to) {
                    let at = index.min(ws.tabs.len());
                    ws.insert_tab(*tab, at, group);
                }
                focus(app, host, to);
            }
            WinOp::MoveLastTab { from, to, index: at } => {
                let Some(k) = index(app, from) else { continue };
                if ws_mut(app, to).is_none() || from == to {
                    continue;
                }
                let mut w = app.workspaces.windows.remove(k);
                let groups = std::mem::take(&mut w.ws.groups);
                if let Some(tab) = w.ws.tabs.pop() {
                    let g = tab.group.and_then(|g| groups.iter().find(|x| x.id == g).cloned());
                    if let Some(ws) = ws_mut(app, to) {
                        let at = at.min(ws.tabs.len());
                        ws.insert_tab(tab, at, g);
                    }
                }
                if app.workspaces.owner == from {
                    app.workspaces.owner = WindowId::MAIN;
                }
                focus(app, host, to);
            }
            WinOp::Close { id, merge } => close_window(app, host, id, merge),
            WinOp::MergeInto(into) => merge_into(app, host, into),
            WinOp::Focus(id) => focus(app, host, id),
        }
    }
}

fn focus(app: &mut AppCore, host: &dyn Host, id: WindowId) {
    host.window(crate::host::WindowId::Window(id), WindowCmd::Focus);
    app.workspaces.owner = id;
}

fn merge_into(app: &mut AppCore, host: &dyn Host, into: WindowId) {
    if app.workspaces.windows.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("workspaces-no-other-windows")));
        return;
    }
    if let Some(k) = index(app, into) {
        // Into a secondary window: the main window takes it over first.
        let (main, w) = (&mut app.ws, &mut app.workspaces.windows[k].ws);
        move_all(main, w);
        promote(app, host, k);
    }
    let others = std::mem::take(&mut app.workspaces.windows);
    let n = others.len();
    for mut w in others {
        move_all(&mut w.ws, &mut app.ws);
    }
    app.workspaces.owner = WindowId::MAIN;
    app.workspaces.geoms.retain(|g| g.id.is_main());
    app.ui.toasts.push(Toast::success(tr!("workspaces-merged", count = n)));
}

/// A tab dragged out of the strip of the current window was released (or moved by a
/// menu command). `grab`: where the pointer held the tab.
pub fn drop_tab(app: &mut AppCore, tab: TabId, target: DropTarget, grab: Vec2) {
    let cur = app.workspaces.current;
    let Some(i) = app.ws.tab_index(tab) else { return };
    let size = app.workspaces.current_size();
    match target {
        DropTarget::Reorder | DropTarget::Nothing => {}
        DropTarget::TearOff { pos } => match app.ws.take_tab(i) {
            Some((t, group)) => {
                let pos = (pos != Point::ZERO).then(|| pos - grab - vec2(8.0, 8.0));
                app.workspaces.ops.push(WinOp::OpenTabs { tabs: vec![t], group, pos, size });
            }
            None => {
                app.ui.toasts.push(Toast::info(tr!("workspaces-keep-one-tab")));
            }
        },
        DropTarget::Window { id, index } => {
            if id == cur {
                return;
            }
            if app.ws.tabs.len() > 1 {
                if let Some((t, group)) = app.ws.take_tab(i) {
                    app.workspaces.ops.push(WinOp::InsertTab { to: id, tab: Box::new(t), group, index });
                }
            } else if !cur.is_main() {
                app.workspaces.ops.push(WinOp::MoveLastTab { from: cur, to: id, index });
            } else {
                app.ui.toasts.push(Toast::info(tr!("workspaces-main-keeps-tab")));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Closing
// ---------------------------------------------------------------------------

/// The user asked to close window `id`, the current window (title bar, Alt+F4, "Close
/// window").
pub fn request_close(app: &mut AppCore, id: WindowId) {
    let tabs = app.ws.tabs.len(); // `id` is the current window
    let policy = app.config.workspace.close_window_tabs;
    match policy {
        _ if tabs <= 1 => app.workspaces.ops.push(WinOp::Close { id, merge: false }),
        CloseWindowTabs::Merge => app.workspaces.ops.push(WinOp::Close { id, merge: true }),
        CloseWindowTabs::Close => app.workspaces.ops.push(WinOp::Close { id, merge: false }),
        CloseWindowTabs::Ask => app.frontend.push(FrontendCmd::CloseWindowPrompt { id, tabs }),
    }
}

/// Main window close requests while other windows are open (`close_requested`: the main
/// window was asked to close this frame). Returns `true` when handled (the close was
/// cancelled).
pub fn intercept_main_close(app: &mut AppCore, host: &dyn Host, close_requested: bool) -> bool {
    if !close_requested || app.workspaces.quitting || app.workspaces.windows.is_empty() {
        return false;
    }
    host.window(crate::host::WindowId::Current, WindowCmd::CancelClose);
    request_close(app, WindowId::MAIN);
    true
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// The other windows for `session.toml`.
pub fn session_windows(app: &AppCore) -> Vec<WindowSession> {
    if !app.config.workspace.restore_windows {
        return Vec::new();
    }
    app.workspaces
        .windows
        .iter()
        .map(|w| {
            let mut s = window_session(&w.ws, false);
            s.pos = w.pos.map(|p| [p.x, p.y]);
            s.size = Some([w.size.x, w.size.y]);
            s.maximized = w.maximized;
            s.preview = Some(PanelSession { open: w.ui.preview.open, size: w.ui.preview.width });
            s.bottom = Some(PanelSession { open: w.ui.bottom.open, size: w.ui.bottom.height });
            s
        })
        .collect()
}

/// Reopen the windows of the restored session (first frame).
pub fn restore_windows(app: &mut AppCore, host: &dyn Host, screen: &Screen) {
    if app.workspaces.restore.is_empty() {
        return;
    }
    for s in std::mem::take(&mut app.workspaces.restore) {
        restore_window(app, host, screen, &s);
    }
    app.workspaces.owner = WindowId::MAIN;
}

fn restore_window(app: &mut AppCore, host: &dyn Host, screen: &Screen, s: &WindowSession) {
    let Some(ws) = build_workspace(app, s, false) else { return };
    let pos = s.pos.map(|[x, y]| crate::geom::pt(x, y)).filter(|p| on_some_screen(screen, *p));
    let size = s.size.map(|[w, h]| vec2(w, h));
    let id = create_window(app, host, screen, ws, pos, size, s.maximized);
    if let Some(k) = index(app, id) {
        restore_panels(&mut app.workspaces.windows[k].ui, s);
    }
}

/// The preview pane and the bottom panel of a restored window.
fn restore_panels(ui: &mut WindowUi, s: &WindowSession) {
    if let Some(p) = s.preview {
        ui.preview.open = p.open;
        ui.preview.width = p.size.clamp(240.0, 760.0);
    }
    if let Some(b) = s.bottom {
        ui.bottom.open = b.open;
        ui.bottom.height = b.size.clamp(120.0, 720.0);
        ui.bottom.reset_height = true;
    }
}

/// Rough check that a saved window position is still on a monitor (the main window's
/// monitor size is all the frontend knows; anything far outside is dropped).
fn on_some_screen(screen: &Screen, p: Point) -> bool {
    let monitor = screen.monitor.unwrap_or(vec2(1920.0, 1080.0));
    let span = vec2(monitor.x * 3.0, monitor.y * 3.0);
    p.x > -span.x && p.y > -40.0 && p.x < span.x && p.y < span.y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_positions_far_off_screen_are_dropped() {
        let s = Screen { main_outer: None, monitor: Some(vec2(1000.0, 800.0)) };
        assert!(on_some_screen(&s, crate::geom::pt(100.0, 100.0)));
        assert!(on_some_screen(&s, crate::geom::pt(-2900.0, 0.0)));
        assert!(!on_some_screen(&s, crate::geom::pt(-3100.0, 0.0)));
        assert!(!on_some_screen(&s, crate::geom::pt(0.0, -50.0)));
        assert!(!on_some_screen(&Screen::default(), crate::geom::pt(6000.0, 0.0)));
    }

    #[test]
    fn move_all_keeps_groups_and_active_tab() {
        let tab = || Tab::new(Pane::new(Location::Timeline, Default::default()));
        let mut from = Workspace::new(tab());
        from.tabs.push(tab());
        let g = TabGroup::new("g".to_string(), 2);
        from.tabs[1].group = Some(g.id);
        from.groups.push(g);
        let mut into = Workspace::new(tab());
        move_all(&mut from, &mut into);
        assert!(from.tabs.is_empty());
        assert_eq!(into.tabs.len(), 3);
        assert_eq!(into.active, 0);
        assert_eq!(into.groups.len(), 1);
        assert_eq!(into.tabs[2].group, Some(into.groups[0].id));
    }

    /// Set in the re-run child (see [`secondary_windows_keep_their_state`]).
    const CHILD: &str = "SOYUZ_WINDOWS_MODEL_DATA";

    /// Secondary windows against a real `AppCore`: per-window UI swapping, a search in a
    /// secondary window, requests from outside routed to the focused window, the
    /// session and the main window taking over. The core keeps its state in the config
    /// directory, so the test re-runs itself from a copy of this test binary next to a
    /// `portable` marker in a temporary directory (like `tests/headless_core.rs`).
    #[test]
    fn secondary_windows_keep_their_state() {
        if let Some(data) = std::env::var_os(CHILD) {
            run_child(std::path::PathBuf::from(data));
            return;
        }
        let tmp = tempfile::tempdir().expect("temp dir");
        let exe = std::env::current_exe().expect("test binary");
        let copy = tmp.path().join(exe.file_name().expect("exe name"));
        std::fs::copy(&exe, &copy).expect("copy the test binary");
        std::fs::write(tmp.path().join(soyuz_core::config::PORTABLE_MARKER), b"").expect("portable marker");
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("sub")).unwrap();
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("sub").join("c.txt"), b"c").unwrap();
        let mut cmd = std::process::Command::new(&copy);
        cmd.args(["--exact", "models::windows::tests::secondary_windows_keep_their_state", "--nocapture"])
            .args(["--test-threads=1"])
            .env(CHILD, &data);
        // On Linux the copy may still be open for writing in a forking test thread
        // ("Text file busy"): retry for a moment.
        let begin = std::time::Instant::now();
        let out = loop {
            match cmd.output() {
                Err(e)
                    if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && begin.elapsed() < std::time::Duration::from_secs(10) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                r => break r.expect("run the copied test binary"),
            }
        };
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "child run failed:\n{log}");
        assert!(log.contains("windows child: ok"), "the child did not run the test:\n{log}");
    }

    /// One frame of a frontend without a window.
    fn tick(app: &mut AppCore, host: &dyn Host) {
        app.begin_frame();
        app.pump_services(host);
        app.pump_jobs(host);
        app.pump_background(host);
        app.apply_cmds(host);
        // Every window draws its panes (the GUI does this in each window's frame).
        for_each_window(app, |app| app.prepare_visible_panes());
        app.end_frame();
    }

    fn settle(app: &mut AppCore, host: &dyn Host, what: &str, done: impl Fn(&AppCore) -> bool) {
        let start = std::time::Instant::now();
        loop {
            tick(app, host);
            if done(app) {
                return;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(30), "timed out waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn win(app: &AppCore, id: WindowId) -> &SecondaryWindow {
        &app.workspaces.windows[index(app, id).expect("window open")]
    }

    fn run_child(data: std::path::PathBuf) {
        use crate::host::NullHost;
        let dir = soyuz_core::config::config_dir();
        assert!(dir.ends_with(soyuz_core::config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());
        let mut c = soyuz_core::config::Config::default();
        c.updates.weekly_check = false;
        c.cli.server_enabled = false;
        c.search.index.enabled = false;
        c.layout.preview_pane = false;
        let startup = crate::core::Startup { open: Some(data.clone()), secondary: true, ..Default::default() };
        let host = NullHost::default();
        let mut app = AppCore::new(c, startup, host.wake(), crate::services::preview_gfx::no_upload());
        let sub = data.join("sub");

        // A second window on `sub`, focused.
        open_location(&mut app, Location::Dir(sub.clone()));
        apply_ops(&mut app, &host, &Screen::default());
        assert_eq!(app.workspaces.windows.len(), 1);
        let w = app.workspaces.windows[0].id;
        assert_eq!(app.workspaces.owner, w, "a new window takes the focus");
        settle(&mut app, &host, "the listings", |a| !a.ws.pane().is_loading() && !win(a, w).ws.pane().is_loading());

        // Per-window UI: what the window changes stays in the window.
        with_window(&mut app, w, |app| {
            assert_eq!(app.workspaces.current, w);
            assert_eq!(app.ws.pane().location, Location::Dir(sub.clone()));
            app.ui.preview.open = true;
            app.ui.preview.width = 500.0;
            app.ui.bottom.open = true;
            app.ui.bottom.height = 333.0;
            app.ui.search.open = true;
            app.ui.quicklook.open = true;
        });
        assert!(app.workspaces.current.is_main());
        assert!(!app.ui.preview.open && app.ui.preview.width != 500.0);
        assert!(!app.ui.bottom.open && !app.ui.search.open && !app.ui.quicklook.open);
        let k = index(&app, w).unwrap();
        swap(&mut app, k);
        assert!(app.ui.preview.open && app.ui.bottom.height == 333.0 && app.ui.search.open);
        swap(&mut app, k);
        assert!(!app.ui.preview.open, "swap is its own inverse");
        // Nested: from inside the window, the main window is swapped in and back.
        with_window(&mut app, w, |app| {
            with_window(app, WindowId::MAIN, |app| {
                assert!(!app.ui.preview.open);
                assert_eq!(app.ws.pane().location, Location::Dir(data.clone()));
            });
            assert!(app.ui.preview.open);
            assert_eq!(app.workspaces.current, w);
        });

        // A search in the secondary window streams there (the pump sees every window).
        with_window(&mut app, w, |app| {
            app.ui.search.form.text = "c.txt".into();
            crate::models::search::run(app);
            app.apply_cmds(&host);
        });
        assert!(app.ui.search.form.text.is_empty(), "the main window's form is its own");
        settle(&mut app, &host, "the search in the second window", |a| {
            let p = win(a, w).ws.pane();
            matches!(p.location, Location::Search(_)) && p.entries.iter().any(|e| e.path.ends_with("c.txt"))
        });

        // The terminal (main window only) never takes a secondary window's keyboard.
        with_window(&mut app, w, |app| {
            app.ui.terminal.focused = true;
            app.ui.bottom.show_terminal();
            assert!(!crate::models::terminal::owns_keyboard(app));
            app.ui.terminal.focused = false;
            app.ui.bottom.tab = super::super::panels::BottomTab::Activity;
        });

        // `soyuz open --reveal` and FileManager1 go to the focused window.
        crate::automation::cli_open(&mut app, &host, sub.join("c.txt"), true);
        assert_eq!(win(&app, w).ws.pane().location, Location::Dir(sub.clone()));
        assert_eq!(app.ws.pane().location, Location::Dir(data.clone()), "the main window stays");
        let focus = (crate::host::WindowId::Window(w), WindowCmd::Focus);
        assert!(host.windows.lock().unwrap().contains(&focus), "the focused window comes to the front");
        let targets = vec![(Location::Dir(data.clone()), None)];
        crate::services::filemanager1::handle(
            &mut app,
            &host,
            crate::services::filemanager1::Request::Open { targets, focus: true },
        );
        assert_eq!(win(&app, w).ws.tabs.len(), 2, "the folder opened in the focused window");
        assert_eq!(app.ws.tabs.len(), 1);

        // The player goes to the window with Quick Look open.
        update_player_window(&mut app);
        assert_eq!(app.workspaces.player_window, w);
        assert!(!player_here(&app));

        // Session: the panels of secondary windows round-trip.
        let s = session_windows(&app);
        assert_eq!(s[0].preview, Some(PanelSession { open: true, size: 500.0 }));
        assert_eq!(s[0].bottom, Some(PanelSession { open: true, size: 333.0 }));
        let back: WindowSession = toml::from_str(&toml::to_string(&s[0]).unwrap()).unwrap();
        assert_eq!(back, s[0]);
        app.workspaces.restore = vec![back];
        restore_windows(&mut app, &host, &Screen::default());
        let w2 = app.workspaces.windows[1].id;
        let ui = &win(&app, w2).ui;
        assert!(ui.preview.open && ui.preview.width == 500.0);
        assert!(ui.bottom.open && ui.bottom.height == 333.0 && !ui.search.open);

        // Closing the main window: `w` takes over, with its per-window state.
        app.workspaces.ops.push(WinOp::Close { id: WindowId::MAIN, merge: false });
        apply_ops(&mut app, &host, &Screen::default());
        assert_eq!(app.workspaces.promoted, Some(w));
        assert_eq!(app.workspaces.windows.len(), 1);
        assert_eq!(app.ws.tabs.len(), 2);
        assert!(app.ui.preview.open && app.ui.preview.width == 500.0);
        assert!(app.ui.bottom.open && app.ui.bottom.height == 333.0);
        assert!(app.ui.search.open && app.ui.search.form.text == "c.txt" && app.ui.quicklook.open);
        assert_eq!(app.workspaces.player_window, WindowId::MAIN);
        println!("windows child: ok");
    }
}
