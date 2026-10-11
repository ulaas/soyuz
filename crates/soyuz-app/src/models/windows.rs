//! In-process windows, model side. Every extra window owns a [`Workspace`] (tabs, groups)
//! plus a little per-window UI state; while a window is handled its workspace is swapped
//! into `app.ws`, so the whole app logic runs unchanged. Jobs, services, settings and the
//! configuration are shared. The GUI draws each extra window as a viewport
//! (`ui::workspaces::windows`).
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
use super::toasts::Toast;
use super::workspaces::{build_workspace, dispose_workspace, tab_session, window_session};
use crate::app::{AddressState, Focus, SoyuzApp};
use crate::geom::{Point, Vec2, vec2};
use crate::host::{Host, WindowCmd};
use crate::state::pane::{Pane, PaneId};
use crate::state::session::WindowSession;
use crate::state::workspace::{ClosedTab, Tab, TabGroup, TabId, Workspace};
use crate::state::workspace_windows::{DropTarget, WindowId};

/// Per-window UI state swapped with `app.ui` for the window's frame.
pub struct WindowUi {
    pub address: AddressState,
    pub focus: Focus,
    pub title: String,
    pub filter_focus: bool,
}

impl Default for WindowUi {
    fn default() -> Self {
        WindowUi { address: AddressState::default(), focus: Focus::List, title: String::new(), filter_focus: false }
    }
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
pub fn index(app: &SoyuzApp, id: WindowId) -> Option<usize> {
    app.workspaces.windows.iter().position(|w| w.id == id)
}

/// Swap window `k`'s workspace and UI state with `app.ws` / `app.ui` (its own inverse).
pub fn swap(app: &mut SoyuzApp, k: usize) {
    let w = &mut app.workspaces.windows[k];
    std::mem::swap(&mut app.ws, &mut w.ws);
    std::mem::swap(&mut app.ui.address, &mut w.ui.address);
    std::mem::swap(&mut app.ui.focus, &mut w.ui.focus);
    std::mem::swap(&mut app.ui.title, &mut w.ui.title);
    std::mem::swap(&mut app.ui.filter_focus, &mut w.ui.filter_focus);
}

/// Run `f` with the window holding `pane` swapped in (mailbox / watcher routing). `None`
/// when no window has that pane.
pub fn with_pane_window<R>(app: &mut SoyuzApp, pane: PaneId, f: impl FnOnce(&mut SoyuzApp) -> R) -> Option<R> {
    if app.ws.find_pane(pane).is_some() {
        return Some(f(app));
    }
    let k = app.workspaces.windows.iter().position(|w| w.ws.find_pane(pane).is_some())?;
    std::mem::swap(&mut app.ws, &mut app.workspaces.windows[k].ws);
    let r = f(app);
    std::mem::swap(&mut app.ws, &mut app.workspaces.windows[k].ws);
    Some(r)
}

/// Workspace of a window while no window is swapped in.
fn ws_mut(app: &mut SoyuzApp, id: WindowId) -> Option<&mut Workspace> {
    if id.is_main() {
        return Some(&mut app.ws);
    }
    app.workspaces.windows.iter_mut().find(|w| w.id == id).map(|w| &mut w.ws)
}

/// `(window, title)` for every window (menus).
pub fn window_list(app: &SoyuzApp) -> Vec<(WindowId, String)> {
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
    app: &mut SoyuzApp,
    host: &dyn Host,
    screen: &Screen,
    ws: Workspace,
    pos: Option<Point>,
    size: Option<Vec2>,
    max: bool,
) {
    let size = size.unwrap_or(vec2(1100.0, 720.0));
    let size = vec2(size.x.max(560.0), size.y.max(360.0));
    // Default position: cascade from the main window.
    let pos = pos.or_else(|| {
        let n = app.workspaces.windows.len() as f32 + 1.0;
        screen.main_outer.map(|p| p + vec2(36.0 * n, 36.0 * n))
    });
    let id = WindowId::next();
    app.workspaces.windows.push(SecondaryWindow {
        id,
        ws,
        ui: WindowUi::default(),
        start_pos: pos,
        start_size: size,
        start_maximized: max,
        pos,
        size,
        maximized: max,
    });
    app.workspaces.owner = id;
    host.redraw();
}

/// Open a new window showing `loc` (Ctrl+N / "New window").
pub fn open_location(app: &mut SoyuzApp, loc: Location) {
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
fn promote(app: &mut SoyuzApp, host: &dyn Host, k: usize) {
    let mut w = app.workspaces.windows.remove(k);
    std::mem::swap(&mut app.ws, &mut w.ws);
    dispose_workspace(app, &mut w.ws);
    app.ws.closed.append(&mut w.ws.closed);
    app.ui.address = w.ui.address;
    app.ui.focus = w.ui.focus;
    app.ui.title.clear();
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

fn close_window(app: &mut SoyuzApp, host: &dyn Host, id: WindowId, merge: bool) {
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
pub fn apply_ops(app: &mut SoyuzApp, host: &dyn Host, screen: &Screen) {
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
            WinOp::OpenWorkspace { ws, pos, size } => create_window(app, host, screen, *ws, pos, size, false),
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

fn focus(app: &mut SoyuzApp, host: &dyn Host, id: WindowId) {
    host.window(crate::host::WindowId::Window(id), WindowCmd::Focus);
    app.workspaces.owner = id;
}

fn merge_into(app: &mut SoyuzApp, host: &dyn Host, into: WindowId) {
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
pub fn drop_tab(app: &mut SoyuzApp, tab: TabId, target: DropTarget, grab: Vec2) {
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
pub fn request_close(app: &mut SoyuzApp, id: WindowId) {
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
pub fn intercept_main_close(app: &mut SoyuzApp, host: &dyn Host, close_requested: bool) -> bool {
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
pub fn session_windows(app: &SoyuzApp) -> Vec<WindowSession> {
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
            s
        })
        .collect()
}

/// Reopen the windows of the restored session (first frame).
pub fn restore_windows(app: &mut SoyuzApp, host: &dyn Host, screen: &Screen) {
    if app.workspaces.restore.is_empty() {
        return;
    }
    for s in std::mem::take(&mut app.workspaces.restore) {
        restore_window(app, host, screen, &s);
    }
    app.workspaces.owner = WindowId::MAIN;
}

fn restore_window(app: &mut SoyuzApp, host: &dyn Host, screen: &Screen, s: &WindowSession) {
    let Some(ws) = build_workspace(app, s, false) else { return };
    let pos = s.pos.map(|[x, y]| crate::geom::pt(x, y)).filter(|p| on_some_screen(screen, *p));
    let size = s.size.map(|[w, h]| vec2(w, h));
    create_window(app, host, screen, ws, pos, size, s.maximized);
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
}
