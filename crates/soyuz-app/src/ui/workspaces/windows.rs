//! In-process windows, GUI side. Every extra window is an egui *immediate* viewport drawn
//! right after the main window's frame. For its frame the window's workspace and UI state
//! are swapped into `app` (`models::windows::swap`), so the whole shell (tab strip,
//! toolbar, sidebar, panes, dialogs) runs unchanged. Main-only panels (terminal,
//! assistant, editors, preview, tool panels) stay in the main window.
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
        app.workspaces.owner = id;
        app.ui.dialogs.push(super::dialogs::CloseWindowDialog::new(id, tabs, prompt_flag().clone()));
    }
}

/// Commands for the "close window" prompt.
pub fn close_cmd(id: WindowId, merge: bool) -> crate::cmd::Cmd {
    crate::cmd::Cmd::Workspace(super::WsCmd::CloseWindow { id, merge })
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
    crate::ui::titlebar::configure_viewport(b, app.ui.chrome)
}

/// Draw every secondary window (end of the main frame) and apply window operations.
pub fn show(app: &mut SoyuzApp, ctx: &egui::Context) {
    let screen = screen(ctx);
    model::restore_windows(app, ctx, &screen);
    model::apply_ops(app, ctx, &screen);
    let ids: Vec<WindowId> = app.workspaces.windows.iter().map(|w| w.id).collect();
    for id in ids {
        let Some(k) = model::index(app, id) else { continue };
        let b = builder(app, &app.workspaces.windows[k]);
        ctx.show_viewport_immediate(viewport_id(id), b, |ui, _class| run_window(ui, app, id));
    }
    model::apply_ops(app, ctx, &screen);
}

fn run_window(ui: &mut Ui, app: &mut SoyuzApp, id: WindowId) {
    let Some(k) = model::index(app, id) else { return };
    model::swap(app, k);
    app.workspaces.current = id;
    window_frame(ui, app, id);
    app.workspaces.current = WindowId::MAIN;
    // Window structure only changes in `apply_ops`, so `k` still names this window.
    model::swap(app, k);
}

fn window_frame(ui: &mut Ui, app: &mut SoyuzApp, id: WindowId) {
    let ctx = ui.ctx().clone();
    begin_frame(app, &ctx, id);
    let (outer, inner, maximized, close) = ctx.input(|i| {
        let v = i.viewport();
        (v.outer_rect, v.inner_rect, v.maximized, v.close_requested())
    });
    if let Some(k) = model::index(app, id) {
        let w = &mut app.workspaces.windows[k];
        w.maximized = maximized.unwrap_or(false);
        if !w.maximized {
            w.pos = outer.map(|r| geom::point(r.min)).or(w.pos);
            w.size = inner.map(|r| geom::size(r.size())).unwrap_or(w.size);
        }
    }
    if close {
        model::request_close(app, id);
    }
    app.handle_input(&ctx);
    app.apply_cmds(&ctx);
    crate::ui::shell::show(ui, app);
    if super::overlays_here(app) {
        crate::palette::show(&ctx, app);
        crate::ui::settings::show(&ctx, app);
        crate::ops::fileops::end_frame(app, &ctx);
    }
    crate::ui::titlebar::frame(&ctx, app);
    app.apply_cmds(&ctx);
}
