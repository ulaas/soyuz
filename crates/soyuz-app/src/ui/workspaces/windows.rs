//! In-process windows. Every extra window is an egui *immediate* viewport drawn
//! right after the main window's frame. It owns a [`Workspace`] (tabs, groups) plus a
//! little per-window UI state; for its frame that workspace is swapped into `app.ws`, so
//! the whole shell (tab strip, toolbar, sidebar, panes, dialogs) runs unchanged. Jobs,
//! services, settings and the configuration are shared. Main-only panels (terminal,
//! assistant, editors, preview, tool panels) stay in the main window.
//!
//! Structural changes (open, close, move tabs between windows) are queued as [`WinOp`]s
//! and applied between frames, never while a window's workspace is swapped in.
//!
//! Closing a window with several tabs asks (or follows `[workspace] close_window_tabs`)
//! whether to move its tabs to another window or close them. Closing the *main* window
//! while others are open hands the main viewport over to the next window. "Quit" closes
//! everything.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Pos2, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, vec2};
use soyuz_core::Location;
use soyuz_core::config::CloseWindowTabs;

use super::{WsCmd, build_workspace, dispose_workspace, modal_open, overlays_here, window_session};
use crate::app::{AddressState, Focus, SoyuzApp};
use crate::state::pane::{Pane, PaneId};
use crate::state::session::WindowSession;
use crate::state::workspace::{ClosedTab, Tab, TabGroup, TabId, Workspace};
use crate::state::workspace_windows::{DropTarget, WindowGeom, WindowId};
use crate::ui::toasts::Toast;

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
    /// Where the window was created (the viewport builder keeps these; the user moves
    /// and resizes the window freely afterwards).
    start_pos: Option<Pos2>,
    start_size: Vec2,
    start_maximized: bool,
    /// Last known outer position / inner size (for the session).
    pub pos: Option<Pos2>,
    pub size: Vec2,
    pub maximized: bool,
}

/// Deferred window operations.
pub enum WinOp {
    OpenTabs {
        tabs: Vec<Tab>,
        group: Option<TabGroup>,
        pos: Option<Pos2>,
        size: Option<Vec2>,
    },
    OpenWorkspace {
        ws: Box<Workspace>,
        pos: Option<Pos2>,
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

pub fn viewport_id(id: WindowId) -> ViewportId {
    if id.is_main() { ViewportId::ROOT } else { ViewportId::from_hash_of(("soyuz.window", id.0)) }
}

fn index(app: &SoyuzApp, id: WindowId) -> Option<usize> {
    app.workspaces.windows.iter().position(|w| w.id == id)
}

/// Swap window `k`'s workspace and UI state with `app.ws` / `app.ui` (its own inverse).
fn swap(app: &mut SoyuzApp, k: usize) {
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

fn app_icon() -> Arc<egui::IconData> {
    static ICON: std::sync::OnceLock<Arc<egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| Arc::new(crate::app_icon::icon_data(128))).clone()
}

/// Create a window for `ws` (applied from [`apply_ops`]).
fn create_window(
    app: &mut SoyuzApp,
    ctx: &egui::Context,
    ws: Workspace,
    pos: Option<Pos2>,
    size: Option<Vec2>,
    max: bool,
) {
    let size = size.unwrap_or(vec2(1100.0, 720.0)).max(vec2(560.0, 360.0));
    // Default position: cascade from the main window.
    let pos = pos.or_else(|| {
        let main = ctx.input_for(ViewportId::ROOT, |i| i.viewport().outer_rect);
        let n = app.workspaces.windows.len() as f32 + 1.0;
        main.map(|r| r.min + vec2(36.0, 36.0) * n)
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
    ctx.request_repaint();
}

/// Open a new window showing `loc` (Ctrl+N / "New window").
pub fn open_location(app: &mut SoyuzApp, ctx: &egui::Context, loc: Location) {
    let mut pane = Pane::new(loc.clone(), app.view_for(&loc));
    crate::ops::start_listing(&mut pane, &mut app.services, false);
    let size = ctx.input(|i| i.viewport().inner_rect).map(|r| r.size());
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
        main.closed.push(ClosedTab { index: main.tabs.len(), session: super::tab_session(t, &groups, false) });
    }
    let excess = main.closed.len().saturating_sub(Workspace::MAX_CLOSED);
    main.closed.drain(..excess);
}

/// The main viewport takes over window `k` (its workspace, place and size).
fn promote(app: &mut SoyuzApp, ctx: &egui::Context, k: usize) {
    let mut w = app.workspaces.windows.remove(k);
    std::mem::swap(&mut app.ws, &mut w.ws);
    dispose_workspace(app, &mut w.ws);
    app.ws.closed.append(&mut w.ws.closed);
    app.ui.address = w.ui.address;
    app.ui.focus = w.ui.focus;
    app.ui.title.clear();
    let root = ViewportId::ROOT;
    if w.maximized {
        ctx.send_viewport_cmd_to(root, ViewportCommand::Maximized(true));
    } else {
        ctx.send_viewport_cmd_to(root, ViewportCommand::Maximized(false));
        if let Some(p) = w.pos {
            ctx.send_viewport_cmd_to(root, ViewportCommand::OuterPosition(p));
        }
        ctx.send_viewport_cmd_to(root, ViewportCommand::InnerSize(w.size));
    }
    ctx.send_viewport_cmd_to(root, ViewportCommand::Focus);
}

fn close_window(app: &mut SoyuzApp, ctx: &egui::Context, id: WindowId, merge: bool) {
    if id.is_main() {
        if app.workspaces.windows.is_empty() {
            app.workspaces.quitting = true;
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            return;
        }
        if merge {
            let (main, next) = (&mut app.ws, &mut app.workspaces.windows[0].ws);
            move_all(main, next);
        } else {
            let next = &mut app.workspaces.windows[0].ws;
            remember_closed(next, &app.ws);
        }
        promote(app, ctx, 0);
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
pub fn apply_ops(app: &mut SoyuzApp, ctx: &egui::Context) {
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
                create_window(app, ctx, ws, pos, size, false);
            }
            WinOp::OpenWorkspace { ws, pos, size } => create_window(app, ctx, *ws, pos, size, false),
            WinOp::InsertTab { to, tab, group, index } => {
                let to = if ws_mut(app, to).is_some() { to } else { WindowId::MAIN };
                if let Some(ws) = ws_mut(app, to) {
                    let at = index.min(ws.tabs.len());
                    ws.insert_tab(*tab, at, group);
                }
                focus(app, ctx, to);
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
                focus(app, ctx, to);
            }
            WinOp::Close { id, merge } => close_window(app, ctx, id, merge),
            WinOp::MergeInto(into) => merge_into(app, ctx, into),
            WinOp::Focus(id) => focus(app, ctx, id),
        }
    }
}

fn focus(app: &mut SoyuzApp, ctx: &egui::Context, id: WindowId) {
    ctx.send_viewport_cmd_to(viewport_id(id), ViewportCommand::Focus);
    app.workspaces.owner = id;
}

fn merge_into(app: &mut SoyuzApp, ctx: &egui::Context, into: WindowId) {
    if app.workspaces.windows.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("workspaces-no-other-windows")));
        return;
    }
    if let Some(k) = index(app, into) {
        // Into a secondary window: the main viewport takes it over first.
        let (main, w) = (&mut app.ws, &mut app.workspaces.windows[k].ws);
        move_all(main, w);
        promote(app, ctx, k);
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
/// menu command).
pub fn drop_tab(app: &mut SoyuzApp, ctx: &egui::Context, tab: TabId, target: DropTarget, grab: Vec2) {
    let cur = app.workspaces.current;
    let Some(i) = app.ws.tab_index(tab) else { return };
    let size = ctx.input(|i| i.viewport().inner_rect).map(|r| r.size());
    match target {
        DropTarget::Reorder | DropTarget::Nothing => {}
        DropTarget::TearOff { pos } => match app.ws.take_tab(i) {
            Some((t, group)) => {
                let pos = crate::ui::geom::pos(pos);
                let pos = (pos != Pos2::ZERO).then(|| pos - grab - vec2(8.0, 8.0));
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

fn prompt_flag() -> &'static Arc<AtomicBool> {
    static F: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    F.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

/// The user asked to close window `id` (title bar, Alt+F4, "Close window").
fn request_close(app: &mut SoyuzApp, id: WindowId) {
    let tabs = app.ws.tabs.len(); // `id` is the current window
    let policy = app.config.workspace.close_window_tabs;
    match policy {
        _ if tabs <= 1 => app.workspaces.ops.push(WinOp::Close { id, merge: false }),
        CloseWindowTabs::Merge => app.workspaces.ops.push(WinOp::Close { id, merge: true }),
        CloseWindowTabs::Close => app.workspaces.ops.push(WinOp::Close { id, merge: false }),
        CloseWindowTabs::Ask => {
            if !prompt_flag().swap(true, Ordering::SeqCst) {
                app.workspaces.owner = id;
                app.ui.dialogs.push(super::dialogs::CloseWindowDialog::new(id, tabs, prompt_flag().clone()));
            }
        }
    }
}

/// Main window close requests while other windows are open (from `App::logic`). Returns
/// `true` when handled (the close was cancelled).
pub fn intercept_main_close(app: &mut SoyuzApp, ctx: &egui::Context) -> bool {
    if !ctx.input(|i| i.viewport().close_requested()) || app.workspaces.quitting || app.workspaces.windows.is_empty() {
        return false;
    }
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    request_close(app, WindowId::MAIN);
    true
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// Start of the main window's frame: overlay owner and geometry.
pub fn begin_main_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    begin_frame(app, ctx, WindowId::MAIN);
}

fn begin_frame(app: &mut SoyuzApp, ctx: &egui::Context, id: WindowId) {
    let (focused, inner) = ctx.input(|i| (i.viewport().focused, i.viewport().inner_rect));
    if focused == Some(true) && !modal_open(app) {
        app.workspaces.owner = id;
    }
    if let Some(inner) = inner {
        app.workspaces.set_geom(WindowGeom {
            id,
            inner: crate::ui::geom::area(inner),
            strip: crate::geom::Rect::NOTHING,
            tabs: Vec::new(),
        });
    }
}

fn builder(app: &SoyuzApp, w: &SecondaryWindow) -> ViewportBuilder {
    let title = format!("{} — Soyuz", crate::state::crumbs::location_title(&w.ws.pane().location));
    let mut b = ViewportBuilder::default()
        .with_title(title)
        .with_app_id("soyuz")
        .with_inner_size(w.start_size)
        .with_min_inner_size([560.0, 360.0])
        .with_icon(app_icon());
    if let Some(p) = w.start_pos {
        b = b.with_position(p);
    }
    if w.start_maximized {
        b = b.with_maximized(true);
    }
    crate::ui::titlebar::configure_viewport(b, app.ui.chrome)
}

/// Draw every secondary window (end of the main frame) and apply window operations.
pub fn show(app: &mut SoyuzApp, ctx: &egui::Context) {
    if !app.workspaces.restore.is_empty() {
        for s in std::mem::take(&mut app.workspaces.restore) {
            restore_window(app, ctx, &s);
        }
        app.workspaces.owner = WindowId::MAIN;
    }
    apply_ops(app, ctx);
    let ids: Vec<WindowId> = app.workspaces.windows.iter().map(|w| w.id).collect();
    for id in ids {
        let Some(k) = index(app, id) else { continue };
        let b = builder(app, &app.workspaces.windows[k]);
        ctx.show_viewport_immediate(viewport_id(id), b, |ui, _class| run_window(ui, app, id));
    }
    apply_ops(app, ctx);
}

fn run_window(ui: &mut Ui, app: &mut SoyuzApp, id: WindowId) {
    let Some(k) = index(app, id) else { return };
    swap(app, k);
    app.workspaces.current = id;
    window_frame(ui, app, id);
    app.workspaces.current = WindowId::MAIN;
    // Window structure only changes in `apply_ops`, so `k` still names this window.
    swap(app, k);
}

fn window_frame(ui: &mut Ui, app: &mut SoyuzApp, id: WindowId) {
    let ctx = ui.ctx().clone();
    begin_frame(app, &ctx, id);
    let (outer, inner, maximized, close) = ctx.input(|i| {
        let v = i.viewport();
        (v.outer_rect, v.inner_rect, v.maximized, v.close_requested())
    });
    if let Some(k) = index(app, id) {
        let w = &mut app.workspaces.windows[k];
        w.maximized = maximized.unwrap_or(false);
        if !w.maximized {
            w.pos = outer.map(|r| r.min).or(w.pos);
            w.size = inner.map(|r| r.size()).unwrap_or(w.size);
        }
    }
    if close {
        request_close(app, id);
    }
    app.handle_input(&ctx);
    app.apply_cmds(&ctx);
    crate::ui::shell::show(ui, app);
    if overlays_here(app) {
        crate::palette::show(&ctx, app);
        crate::ui::settings::show(&ctx, app);
        crate::ops::fileops::end_frame(app, &ctx);
    }
    crate::ui::titlebar::frame(&ctx, app);
    app.apply_cmds(&ctx);
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

fn restore_window(app: &mut SoyuzApp, ctx: &egui::Context, s: &WindowSession) {
    let Some(ws) = build_workspace(app, s, false) else { return };
    let pos = s.pos.map(|[x, y]| Pos2::new(x, y)).filter(|p| on_some_screen(ctx, *p));
    let size = s.size.map(|[w, h]| vec2(w, h));
    create_window(app, ctx, ws, pos, size, s.maximized);
}

/// Rough check that a saved window position is still on a monitor (the main window's
/// monitor size is all egui knows; anything far outside is dropped).
fn on_some_screen(ctx: &egui::Context, p: Pos2) -> bool {
    let monitor = ctx.input_for(ViewportId::ROOT, |i| i.viewport().monitor_size).unwrap_or(vec2(1920.0, 1080.0));
    let span = monitor * 3.0;
    p.x > -span.x && p.y > -40.0 && p.x < span.x && p.y < span.y
}

/// Commands for the "close window" prompt.
pub fn close_cmd(id: WindowId, merge: bool) -> crate::cmd::Cmd {
    crate::cmd::Cmd::Workspace(WsCmd::CloseWindow { id, merge })
}
