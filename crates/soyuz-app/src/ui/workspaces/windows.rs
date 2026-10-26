//! In-process windows, GUI side. Every extra window is an egui *immediate* viewport drawn
//! right after the main window's frame. For its frame the window's workspace and UI state
//! are swapped into `app` (`models::windows::with_window`), plus the preview pane it
//! renders (`GuiState::window_previews`), so the whole shell (tab strip, toolbar,
//! sidebar, search bar, preview, activity / bottom panel with the window's terminal,
//! Quick Look, panes, dialogs) runs unchanged. The shared panels (assistant, editors,
//! shelf, tool windows) draw in the window that owns them (`models::owners`). The media
//! player (video and the audio card) plays in one window at a time
//! (`models::windows::update_player_window`); the others show posters.
//!
//! The bookkeeping (window list, queued `WinOp`s, closing, merging, session) is in
//! `models::windows`; this module creates the viewports, reads their per-frame input
//! and asks the close prompt.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Ui, ViewportBuilder, ViewportId};

use super::modal_open;
use crate::app::SoyuzApp;
use crate::models::windows::{self as model, Screen, SecondaryWindow};
use crate::state::workspace_windows::{WindowGeom, WindowId};
use crate::ui::geom;

pub fn viewport_id(id: WindowId) -> ViewportId {
    if id.is_main() { ViewportId::ROOT } else { ViewportId::from_hash_of(("soyuz.window", id.0)) }
}

fn app_icon() -> Arc<egui::IconData> {
    static ICON: std::sync::OnceLock<Arc<egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| Arc::new(crate::app_icon::icon_data(128))).clone()
}

/// What window operations need to know about the screen (from the main viewport).
fn screen(ctx: &egui::Context) -> Screen {
    ctx.input_for(ViewportId::ROOT, |i| Screen {
        main_outer: i.viewport().outer_rect.map(|r| geom::point(r.min)),
        monitor: i.viewport().monitor_size.map(geom::size),
    })
}

// ---------------------------------------------------------------------------
// Closing
// ---------------------------------------------------------------------------

fn prompt_flag() -> &'static Arc<AtomicBool> {
    static F: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    F.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

/// Ask whether to move or close the tabs of window `id` (`FrontendCmd::CloseWindowPrompt`;
/// one prompt at a time).
pub fn close_prompt(app: &mut SoyuzApp, id: WindowId, tabs: usize) {
    if !prompt_flag().swap(true, Ordering::SeqCst) {
        app.core.workspaces.owner = id;
        app.gui.dialogs.push(super::dialogs::CloseWindowDialog::new(id, tabs, prompt_flag().clone()));
    }
}

/// Commands for the "close window" prompt.
pub fn close_cmd(id: WindowId, merge: bool) -> crate::cmd::Cmd {
    crate::cmd::Cmd::Workspace(super::WsCmd::CloseWindow { id, merge })
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// Start of the main window's frame: overlay owner, geometry and the player's window.
pub fn begin_main_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    begin_frame(app, ctx, WindowId::MAIN);
    model::update_player_window(&mut app.core);
    crate::ui::preview::video::set_player_here(model::player_here(&app.core));
}

fn begin_frame(app: &mut SoyuzApp, ctx: &egui::Context, id: WindowId) {
    let (focused, inner) = ctx.input(|i| (i.viewport().focused, i.viewport().inner_rect));
    if focused == Some(true) && !modal_open(app) {
        app.core.workspaces.owner = id;
    }
    if let Some(inner) = inner {
        app.core.workspaces.set_geom(WindowGeom {
            id,
            inner: geom::area(inner),
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
        .with_inner_size(geom::vec(w.start_size))
        .with_min_inner_size([560.0, 360.0])
        .with_icon(app_icon());
    if let Some(p) = w.start_pos {
        b = b.with_position(geom::pos(p));
    }
    if w.start_maximized {
        b = b.with_maximized(true);
    }
    // Scripted runs (`SOYUZ_SCRIPT`, `SOYUZ_WINDOW_POS`): never take the focus from the
    // user's session, and open where the main window does (e.g. off-screen).
    if crate::ui::native::quiet() || std::env::var_os("SOYUZ_SCRIPT").is_some() {
        b = b.with_active(false);
        if let Some(p) = dev_pos() {
            b = b.with_position(p);
        }
    }
    crate::ui::titlebar::configure_viewport(b, app.gui.chrome)
}

/// `SOYUZ_WINDOW_POS=x,y` (debug hook, see `main.rs`).
fn dev_pos() -> Option<egui::Pos2> {
    let s = std::env::var("SOYUZ_WINDOW_POS").ok()?;
    let (x, y) = s.split_once(',')?;
    Some(egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Draw every secondary window (end of the main frame) and apply window operations.
pub fn show(app: &mut SoyuzApp, ctx: &egui::Context) {
    let screen = screen(ctx);
    model::restore_windows(&mut app.core, &crate::host::EguiHost::new(ctx), &screen);
    apply_ops(app, ctx, &screen);
    let ids: Vec<WindowId> = app.core.workspaces.windows.iter().map(|w| w.id).collect();
    for id in ids {
        let Some(k) = model::index(&app.core, id) else { continue };
        let b = builder(app, &app.core.workspaces.windows[k]);
        ctx.show_viewport_immediate(viewport_id(id), b, |ui, _class| run_window(ui, app, id));
    }
    apply_ops(app, ctx, &screen);
}

/// Apply window operations, then follow them with what the GUI keeps per window.
fn apply_ops(app: &mut SoyuzApp, ctx: &egui::Context, screen: &Screen) {
    model::apply_ops(&mut app.core, &crate::host::EguiHost::new(ctx), screen);
    if let Some(id) = app.core.workspaces.promoted.take() {
        // The main window took over window `id`: its rendered preview comes along.
        if let Some(p) = app.gui.window_previews.remove(&id) {
            app.gui.preview = p;
        }
        crate::ui::quicklook::window_promoted(id);
    }
    let open = &app.core.workspaces.windows;
    app.gui.window_previews.retain(|id, _| open.iter().any(|w| w.id == *id));
    crate::ui::quicklook::retain_windows(|id| id.is_main() || open.iter().any(|w| w.id == id));
}

fn run_window(ui: &mut Ui, app: &mut SoyuzApp, id: WindowId) {
    with_window(app, id, |app| window_frame(ui, app, id));
}

/// Run `f` with secondary window `id` swapped in, the GUI's per-window state (its rendered
/// preview) included (`models::windows::with_window` plus the GUI side). `None` when the
/// window is gone.
pub fn with_window<R>(app: &mut SoyuzApp, id: WindowId, f: impl FnOnce(&mut SoyuzApp) -> R) -> Option<R> {
    model::index(&app.core, id)?;
    // Window structure only changes in `apply_ops`, so the window stays put meanwhile.
    let swapped = model::swap_to(&mut app.core, id);
    let mut preview = app.gui.window_previews.remove(&id).unwrap_or_default();
    std::mem::swap(&mut app.gui.preview, &mut preview);
    let r = f(app);
    std::mem::swap(&mut app.gui.preview, &mut preview);
    app.gui.window_previews.insert(id, preview);
    model::swap_back(&mut app.core, swapped);
    Some(r)
}

fn window_frame(ui: &mut Ui, app: &mut SoyuzApp, id: WindowId) {
    let ctx = ui.ctx().clone();
    begin_frame(app, &ctx, id);
    crate::ui::preview::video::set_player_here(model::player_here(&app.core));
    crate::devscript::inject_window(app, &ctx, id);
    let (outer, inner, maximized, close) = ctx.input(|i| {
        let v = i.viewport();
        (v.outer_rect, v.inner_rect, v.maximized, v.close_requested())
    });
    if let Some(k) = model::index(&app.core, id) {
        let w = &mut app.core.workspaces.windows[k];
        w.maximized = maximized.unwrap_or(false);
        if !w.maximized {
            w.pos = outer.map(|r| geom::point(r.min)).or(w.pos);
            w.size = inner.map(|r| geom::size(r.size())).unwrap_or(w.size);
        }
    }
    if close {
        model::request_close(&mut app.core, id);
    }
    crate::ui::ai::escape_stops_task(app, &ctx);
    app.handle_input(&ctx);
    app.apply_cmds(&crate::host::EguiHost::new(&ctx));
    crate::ui::shell::show(ui, app);
    if super::overlays_here(&app.core) {
        crate::ui::palette::show(&ctx, app);
        crate::ui::settings::show(&ctx, app);
    }
    crate::ui::titlebar::frame(&ctx, app);
    if super::overlays_here(&app.core) {
        // Like the main window: drag-out to other apps, the drag badge, the "ask" menu.
        crate::ui::dnd::end_frame(app, &ctx);
        crate::ops::fileops::end_frame(&mut app.core, &crate::host::EguiHost::new(&ctx));
    }
    app.apply_cmds(&crate::host::EguiHost::new(&ctx));
}
