//! Root layout: custom title bar (logo + tabs + window controls, when enabled) →
//! status bar → bottom panel → sidebar → tab strip (native title bar only) → toolbar →
//! pane cards (single, dual with a draggable splitter, or a 2×2 quad grid), then dialogs
//! and toasts on top. Secondary in-process windows draw the same shell (search bar,
//! preview, activity / bottom panel with its terminal, Quick Look); the shared panels
//! (assistant, editors, shelf, tool windows) draw in the window that owns them
//! (`models::owners`); the overlays go to the focused window.

use egui::{CentralPanel, CornerRadius, CursorIcon, Frame, Id, Margin, Rect, Sense, Ui, pos2, vec2};
use soyuz_core::Location;
use soyuz_core::config::{Orientation, TabStripMode};

use crate::app::{Focus, SoyuzApp};
use crate::cmd::Cmd;
use crate::models::owners::Panel;
use crate::theme::palette;
use crate::ui::dialogs::DialogCtx;
use crate::ui::sidebar::{self, SidebarCtx};
use crate::ui::statusbar::{self, StatusInfo};
use crate::ui::views::ViewCtx;
use crate::ui::{pane_view, panels, tabstrip, toolbar};

const GAP: f32 = 6.0;

pub fn show(ui: &mut Ui, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);

    app.core.prepare_visible_panes();

    let titlebar = app.gui.chrome.has_titlebar();
    let here = |app: &SoyuzApp, p| crate::models::owners::here(&app.core, p);
    if titlebar {
        crate::ui::titlebar::show(ui, app);
    }
    if app.core.ui.status_visible {
        let info = status_info(app);
        let mut zoom = None;
        statusbar::show(ui, &info, &mut app.core.cmds, &mut zoom);
        if let Some(z) = zoom {
            let id = app.core.ws.pane().id;
            app.core.ws.pane_mut().view.zoom = z.clamp(16, 256);
            app.core.cmds.push(Cmd::ViewChanged(id));
        }
    }
    // The bottom panel in every window, each with its own terminal sessions.
    panels::show(ui, app);
    panels::terminal::apply_requests(&mut app.core);
    if app.core.ui.sidebar_visible {
        let current = app.core.ws.pane().location.clone();
        let pane = app.core.ws.pane().id;
        let saved_searches = app.core.ui.search.names();
        let drives = crate::services::places::drive_volumes(&app.core.ui.volumes, &app.core.ui.cloud);
        let mut sctx = SidebarCtx {
            saved_searches: &saved_searches,
            saved_config: app.core.ui.search.config_count(),
            bookmarks: &app.core.config.bookmarks,
            volumes: &drives,
            cloud: &app.core.ui.cloud,
            sections: &app.core.config.sidebar.sections,
            current: &current,
            pane,
            cmds: &mut app.core.cmds,
            width: app.core.config.appearance.sidebar_width,
            brand: !titlebar,
            settings_open: app.gui.settings.is_open(),
            settings_key: crate::keys::chord_label(&app.core.keymap, &soyuz_core::config::Action::Settings),
            portal: &app.core.config.sidebar.portal,
            tree_key: crate::keys::chord_label(&app.core.keymap, &soyuz_core::config::Action::ToggleTreeSidebar),
            config: &app.core.config.sidebar,
            tree: crate::ui::tree::TreeArgs {
                state: &mut app.core.ui.tree,
                mode: app.core.config.sidebar.mode,
                vis: crate::ui::tree::model::Visibility {
                    show_hidden: app.core.ui.show_hidden,
                    show_system: app.core.ui.show_system,
                },
                follow: app.core.config.sidebar.tree_follow,
                drag_expand: (app.core.config.sidebar.tree_drag_expand_ms > 0)
                    .then(|| std::time::Duration::from_millis(u64::from(app.core.config.sidebar.tree_drag_expand_ms))),
                drag_default: app.core.config.behaviour.drag_default,
                keyboard: app.core.ui.focus == crate::app::Focus::Sidebar
                    && !ctx.memory(|m| m.focused()).is_some_and(|id| egui::TextEdit::load_state(&ctx, id).is_some()),
                debounce: std::time::Duration::from_millis(app.core.config.behaviour.watch_debounce_ms.max(50)),
            },
        };
        sidebar::show(ui, &mut app.core.ui.sidebar, &mut sctx);
    }
    if here(app, Panel::Assistant) {
        crate::ui::ai::show(ui, app);
    }
    if !titlebar
        && (app.core.config.layout.tab_strip == TabStripMode::Always || app.core.ws.tabs.len() > 1)
        && !crate::models::focus_mode::hides(&app.core, "tabs")
    {
        tabstrip::show(ui, app);
    }
    toolbar::show(ui, app);
    crate::ui::search::bar(ui, app);
    crate::ui::preview::show(ui, app);
    panels::show_activity(ui, app);
    crate::ui::editors::show(ui, app);
    if here(app, Panel::Shelf) {
        crate::ui::tools::show_panels(ui, app);
    }

    CentralPanel::default()
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 0, right: 8, top: 2, bottom: 2 }))
        .show(ui, |ui| {
            let area = ui.available_rect_before_wrap();
            if here(app, Panel::Editors) && crate::editors::maximized(&app.core) {
                crate::ui::editors::show_central(ui, area, app);
            } else {
                panes(ui, area, app);
            }
            ui.allocate_rect(area, Sense::hover());
        });

    // The tool windows, in their own window (over the panes, under the dialogs).
    if here(app, Panel::Tools) {
        crate::ui::tools::show_windows(&ctx, app);
    }
    // Overlays (in the focused window only). Quick Look shows in the window it was
    // opened in (every window has its own), above the dialogs.
    if !crate::models::workspaces::overlays_here(&app.core) {
        crate::ui::quicklook::show(&ctx, app);
        return;
    }
    crate::ui::tools::drag_strip(&ctx, app);
    crate::ui::welcome::show(&ctx, app, if app.core.ui.status_visible { 24.0 } else { 0.0 });
    let mut dctx = DialogCtx { cmds: &mut app.core.cmds, ws: &mut app.core.ws };
    app.gui.dialogs.show(&ctx, &mut app.core.dialogs, &mut dctx);
    crate::ui::quicklook::show(&ctx, app);
    crate::ui::panels::terminal::clipboard_prompt(&ctx, app);
    let screen = ctx.content_rect();
    let bottom_offset = if app.core.ui.status_visible { 30.0 } else { 6.0 };
    // Toasts stay clear of the assistant panel.
    let ai = app.core.ai.panel.open && here(app, Panel::Assistant);
    let right = screen.right() - if ai { app.core.ai.panel.width } else { 0.0 };
    let anchor = Rect::from_min_max(screen.min, pos2(right, screen.bottom() - bottom_offset));
    for action in crate::ui::toasts::show(&mut app.core.ui.toasts, &ctx, anchor, app.core.ui.reduced_motion) {
        app.core.cmds.push(Cmd::Action(action));
    }
}

fn status_info(app: &SoyuzApp) -> StatusInfo {
    let pane = app.core.ws.pane();
    let (selected, selected_bytes) = pane.selection_stats();
    let free = match &pane.location {
        Location::Dir(path) => app
            .core
            .ui
            .volumes
            .iter()
            .filter(|v| path.starts_with(&v.mount))
            .max_by_key(|v| v.mount.as_os_str().len())
            .map(|v| (v.total, v.free)),
        _ => None,
    };
    StatusInfo {
        pane: pane.id,
        visible: pane.visible.len(),
        total: pane.entries.len(),
        selected,
        selected_bytes,
        loading: pane.is_loading() || pane.refreshing,
        filtered: pane.filter_active(),
        hidden: if pane.filter_active() || !pane.collapsed_groups.is_empty() {
            0
        } else {
            pane.entries.len().saturating_sub(pane.visible.len())
        },
        free,
        mode: pane.view.mode,
        zoom: pane.view.zoom,
        theme: app.core.ui.theme,
        size_style: app.core.config.general.size_style,
        activity: panels::activity::indicator(&app.core),
        git_dir: match &pane.location {
            Location::Dir(d) => Some(d.clone()),
            _ => None,
        },
    }
}

fn panes(ui: &mut Ui, area: Rect, app: &mut SoyuzApp) {
    let p = palette(ui.ctx());
    let ti = app.core.ws.active.min(app.core.ws.tabs.len() - 1);
    let (shown, focused, split, orientation) = {
        let t = &app.core.ws.tabs[ti];
        (t.shown(), t.focused, t.split, t.orientation)
    };
    let list_focus = app.core.ui.focus == Focus::List;
    if shown == 4 {
        quad(ui, area, app, ti, focused, list_focus);
        return;
    }
    if shown < 2 {
        // rclone mounts appear under Cloud only, also in the Computer view.
        let drives = crate::services::places::drive_volumes(&app.core.ui.volumes, &app.core.ui.cloud);
        let tab = &mut app.core.ws.tabs[ti];
        let idx = focused.min(tab.panes.len() - 1);
        let pane = &mut tab.panes[idx];
        let mut v = ViewCtx {
            config: &app.core.config,
            keymap: &app.core.keymap,
            services: &mut app.core.services,
            cmds: &mut app.core.cmds,
            focused: true,
            list_focus,
            checkbox_mode: app.core.ui.checkbox_mode,
            show_hidden: app.core.ui.show_hidden,
            show_system: app.core.ui.show_system,
            volumes: &drives,
            dual: false,
            fileops: &mut app.core.fileops,
        };
        pane_view::show(ui, area, pane, &mut v, false);
        return;
    }
    // Dual pane with splitter.
    let vertical = orientation == Orientation::Vertical;
    let (a, b, splitter) = if vertical {
        let w = area.width() - GAP;
        let x = area.left() + (w * split).round();
        (
            Rect::from_min_max(area.min, pos2(x, area.bottom())),
            Rect::from_min_max(pos2(x + GAP, area.top()), area.max),
            Rect::from_min_max(pos2(x, area.top()), pos2(x + GAP, area.bottom())),
        )
    } else {
        let h = area.height() - GAP;
        let y = area.top() + (h * split).round();
        (
            Rect::from_min_max(area.min, pos2(area.right(), y)),
            Rect::from_min_max(pos2(area.left(), y + GAP), area.max),
            Rect::from_min_max(pos2(area.left(), y), pos2(area.right(), y + GAP)),
        )
    };
    let sresp = ui.interact(splitter, Id::unique("soyuz.splitter"), Sense::click_and_drag());
    crate::ui::a11y::splitter(&sresp, tr!("app-a11y-resize-panes"));
    if sresp.hovered() || sresp.dragged() {
        ui.ctx().set_cursor_icon(if vertical { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
        let bar = if vertical {
            Rect::from_center_size(splitter.center(), vec2(2.0, 40.0))
        } else {
            Rect::from_center_size(splitter.center(), vec2(40.0, 2.0))
        };
        ui.painter().rect_filled(bar, CornerRadius::same(1), p.accent.gamma_multiply(0.8));
    }
    if sresp.dragged()
        && let Some(pos) = sresp.interact_pointer_pos()
    {
        let f = if vertical { (pos.x - area.left()) / area.width() } else { (pos.y - area.top()) / area.height() };
        app.core.ws.tabs[ti].split = f.clamp(0.15, 0.85);
    }
    if sresp.double_clicked() {
        app.core.ws.tabs[ti].split = 0.5;
    }
    // rclone mounts appear under Cloud only, also in the Computer view.
    let drives = crate::services::places::drive_volumes(&app.core.ui.volumes, &app.core.ui.cloud);
    for (i, rect) in [(0usize, a), (1usize, b)] {
        let tab = &mut app.core.ws.tabs[ti];
        let pane = &mut tab.panes[i];
        let mut v = ViewCtx {
            config: &app.core.config,
            keymap: &app.core.keymap,
            services: &mut app.core.services,
            cmds: &mut app.core.cmds,
            focused: focused == i,
            list_focus,
            checkbox_mode: app.core.ui.checkbox_mode,
            show_hidden: app.core.ui.show_hidden,
            show_system: app.core.ui.show_system,
            volumes: &drives,
            dual: true,
            fileops: &mut app.core.fileops,
        };
        pane_view::show(ui, rect, pane, &mut v, true);
    }
}

/// The four cells of a 2×2 grid split at `fx` (columns) / `fy` (rows), plus the vertical
/// and horizontal splitter rects.
pub fn quad_cells(area: Rect, fx: f32, fy: f32) -> ([Rect; 4], Rect, Rect) {
    let w = area.width() - GAP;
    let h = area.height() - GAP;
    let x = area.left() + (w * fx).round();
    let y = area.top() + (h * fy).round();
    let cells = [
        Rect::from_min_max(area.min, pos2(x, y)),
        Rect::from_min_max(pos2(x + GAP, area.top()), pos2(area.right(), y)),
        Rect::from_min_max(pos2(area.left(), y + GAP), pos2(x, area.bottom())),
        Rect::from_min_max(pos2(x + GAP, y + GAP), area.max),
    ];
    let vsplit = Rect::from_min_max(pos2(x, area.top()), pos2(x + GAP, area.bottom()));
    let hsplit = Rect::from_min_max(pos2(area.left(), y), pos2(area.right(), y + GAP));
    (cells, vsplit, hsplit)
}

/// Quad layout: four panes, a column splitter (`split`) and a row splitter (`split2`).
fn quad(ui: &mut Ui, area: Rect, app: &mut SoyuzApp, ti: usize, focused: usize, list_focus: bool) {
    let p = palette(ui.ctx());
    let (fx, fy) = (app.core.ws.tabs[ti].split, app.core.ws.tabs[ti].split2);
    let (cells, vsplit, hsplit) = quad_cells(area, fx, fy);
    for (id, rect, vertical) in [("soyuz.quad.v", vsplit, true), ("soyuz.quad.h", hsplit, false)] {
        let sresp = ui.interact(rect, Id::unique(id), Sense::click_and_drag());
        crate::ui::a11y::splitter(&sresp, tr!("app-a11y-resize-panes"));
        if sresp.hovered() || sresp.dragged() {
            ui.ctx().set_cursor_icon(if vertical { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
            let bar = if vertical {
                Rect::from_center_size(rect.center(), vec2(2.0, 40.0))
            } else {
                Rect::from_center_size(rect.center(), vec2(40.0, 2.0))
            };
            ui.painter().rect_filled(bar, CornerRadius::same(1), p.accent.gamma_multiply(0.8));
        }
        if sresp.dragged()
            && let Some(pos) = sresp.interact_pointer_pos()
        {
            let tab = &mut app.core.ws.tabs[ti];
            if vertical {
                tab.split = ((pos.x - area.left()) / area.width()).clamp(0.15, 0.85);
            } else {
                tab.split2 = ((pos.y - area.top()) / area.height()).clamp(0.15, 0.85);
            }
        }
        if sresp.double_clicked() {
            let tab = &mut app.core.ws.tabs[ti];
            if vertical {
                tab.split = 0.5;
            } else {
                tab.split2 = 0.5;
            }
        }
    }
    // rclone mounts appear under Cloud only, also in the Computer view.
    let drives = crate::services::places::drive_volumes(&app.core.ui.volumes, &app.core.ui.cloud);
    for (i, rect) in cells.into_iter().enumerate() {
        let tab = &mut app.core.ws.tabs[ti];
        let pane = &mut tab.panes[i];
        let mut v = ViewCtx {
            config: &app.core.config,
            keymap: &app.core.keymap,
            services: &mut app.core.services,
            cmds: &mut app.core.cmds,
            focused: focused == i,
            list_focus,
            checkbox_mode: app.core.ui.checkbox_mode,
            show_hidden: app.core.ui.show_hidden,
            show_system: app.core.ui.show_system,
            volumes: &drives,
            dual: true,
            fileops: &mut app.core.fileops,
        };
        pane_view::show(ui, rect, pane, &mut v, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_cells_tile_the_area() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1006.0, 606.0));
        let (c, v, h) = quad_cells(area, 0.5, 0.5);
        assert_eq!(c[0], Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 300.0)));
        assert_eq!(c[3].max, area.max);
        assert_eq!(c[1].left(), c[3].left());
        assert_eq!(c[2].top(), c[3].top());
        assert_eq!(v.width(), GAP);
        assert_eq!(h.height(), GAP);
        let (c, _, _) = quad_cells(area, 0.25, 0.75);
        assert!(c[0].width() < c[1].width() && c[0].height() > c[2].height());
    }
}
