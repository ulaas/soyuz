//! Root layout: custom title bar (logo + tabs + window controls, when enabled) →
//! status bar → bottom panel → sidebar → tab strip (native title bar only) → toolbar →
//! pane cards (single or dual with a draggable splitter), then dialogs and toasts on top.

use egui::{CentralPanel, CornerRadius, CursorIcon, Frame, Id, Margin, Rect, Sense, Ui, pos2, vec2};
use soyuz_core::Location;
use soyuz_core::config::{Orientation, TabStripMode};

use crate::app::{Focus, SoyuzApp};
use crate::cmd::Cmd;
use crate::ops;
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

    // Visible panes: start listings lazily and refresh their visible order.
    {
        let show_hidden = app.ui.show_hidden;
        let show_system = app.ui.show_system;
        let tab = app.ws.tab_mut();
        let shown = tab.shown();
        let focused = tab.focused;
        for (i, pane) in tab.panes.iter_mut().enumerate() {
            let visible = if shown == 2 { i < 2 } else { i == focused };
            if visible {
                ops::ensure_started(pane, &mut app.services);
            }
            pane.refresh_visible(show_hidden, show_system);
        }
    }

    let titlebar = app.ui.chrome.has_titlebar();
    if titlebar {
        crate::ui::titlebar::show(ui, app);
    }
    if app.ui.status_visible {
        let info = status_info(app);
        let mut zoom = None;
        statusbar::show(ui, &info, &mut app.cmds, &mut zoom);
        if let Some(z) = zoom {
            let id = app.ws.pane().id;
            app.ws.pane_mut().view.zoom = z.clamp(16, 256);
            app.cmds.push(Cmd::ViewChanged(id));
        }
    }
    panels::show(ui, app);
    panels::terminal::apply_requests(app);
    if app.ui.sidebar_visible {
        let current = app.ws.pane().location.clone();
        let pane = app.ws.pane().id;
        let saved_searches = app.ui.search.names();
        let mut sctx = SidebarCtx {
            saved_searches: &saved_searches,
            saved_config: app.ui.search.config_count(),
            bookmarks: &app.config.bookmarks,
            volumes: &app.ui.volumes,
            cloud: &app.ui.cloud,
            sections: &app.config.sidebar.sections,
            current: &current,
            pane,
            cmds: &mut app.cmds,
            width: app.config.appearance.sidebar_width,
            brand: !titlebar,
            settings_open: app.settings.is_open(),
            settings_key: crate::keys::chord_label(&app.keymap, &soyuz_core::config::Action::Settings),
            tree: crate::ui::tree::TreeArgs {
                state: &mut app.ui.tree,
                mode: app.config.sidebar.mode,
                vis: crate::ui::tree::model::Visibility {
                    show_hidden: app.ui.show_hidden,
                    show_system: app.ui.show_system,
                },
                follow: app.config.sidebar.tree_follow,
                drag_expand: (app.config.sidebar.tree_drag_expand_ms > 0)
                    .then(|| std::time::Duration::from_millis(u64::from(app.config.sidebar.tree_drag_expand_ms))),
                drag_default: app.config.behaviour.drag_default,
                keyboard: app.ui.focus == crate::app::Focus::Sidebar
                    && !ctx.memory(|m| m.focused()).is_some_and(|id| egui::TextEdit::load_state(&ctx, id).is_some()),
                debounce: std::time::Duration::from_millis(app.config.behaviour.watch_debounce_ms.max(50)),
            },
        };
        sidebar::show(ui, &mut app.ui.sidebar, &mut sctx);
    }
    if !titlebar && (app.config.layout.tab_strip == TabStripMode::Always || app.ws.tabs.len() > 1) {
        tabstrip::show(ui, app);
    }
    toolbar::show(ui, app);
    crate::ui::search::bar(ui, app);
    crate::ui::preview::show(ui, app);
    panels::show_activity(ui, app);
    crate::editors::show(ui, app);
    crate::ui::tools::show_panels(ui, app);

    CentralPanel::default()
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 0, right: 8, top: 2, bottom: 2 }))
        .show(ui, |ui| {
            let area = ui.available_rect_before_wrap();
            if crate::editors::maximized(app) {
                crate::editors::show_central(ui, area, app);
            } else {
                panes(ui, area, app);
            }
            ui.allocate_rect(area, Sense::hover());
        });

    // Overlays.
    crate::ui::tools::show_windows(&ctx, app);
    crate::ui::welcome::show(&ctx, app, if app.ui.status_visible { 24.0 } else { 0.0 });
    let mut dctx = DialogCtx { cmds: &mut app.cmds, ws: &mut app.ws };
    app.ui.dialogs.show(&ctx, &mut dctx);
    crate::ui::quicklook::show(&ctx, app);
    crate::ui::panels::terminal::clipboard_prompt(&ctx, app);
    let screen = ctx.content_rect();
    let bottom_offset = if app.ui.status_visible { 30.0 } else { 6.0 };
    let anchor = Rect::from_min_max(screen.min, pos2(screen.right(), screen.bottom() - bottom_offset));
    for action in app.ui.toasts.show(&ctx, anchor, app.ui.reduced_motion) {
        app.cmds.push(Cmd::Action(action));
    }
}

fn status_info(app: &SoyuzApp) -> StatusInfo {
    let pane = app.ws.pane();
    let (selected, selected_bytes) = pane.selection_stats();
    let free = match &pane.location {
        Location::Dir(path) => app
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
        theme: app.ui.theme,
        size_style: app.config.general.size_style,
        activity: panels::activity::indicator(app),
        git_dir: match &pane.location {
            Location::Dir(d) => Some(d.clone()),
            _ => None,
        },
    }
}

fn panes(ui: &mut Ui, area: Rect, app: &mut SoyuzApp) {
    let p = palette(ui.ctx());
    let ti = app.ws.active.min(app.ws.tabs.len() - 1);
    let (shown, focused, split, orientation) = {
        let t = &app.ws.tabs[ti];
        (t.shown(), t.focused, t.split, t.orientation)
    };
    let list_focus = app.ui.focus == Focus::List;
    if shown < 2 {
        let tab = &mut app.ws.tabs[ti];
        let idx = focused.min(tab.panes.len() - 1);
        let pane = &mut tab.panes[idx];
        let mut v = ViewCtx {
            config: &app.config,
            keymap: &app.keymap,
            services: &mut app.services,
            cmds: &mut app.cmds,
            focused: true,
            list_focus,
            checkbox_mode: app.ui.checkbox_mode,
            show_hidden: app.ui.show_hidden,
            show_system: app.ui.show_system,
            volumes: &app.ui.volumes,
            dual: false,
            fileops: &mut app.fileops,
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
    let sresp = ui.interact(splitter, Id::new("soyuz.splitter"), Sense::click_and_drag());
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
        app.ws.tabs[ti].split = f.clamp(0.15, 0.85);
    }
    if sresp.double_clicked() {
        app.ws.tabs[ti].split = 0.5;
    }
    for (i, rect) in [(0usize, a), (1usize, b)] {
        let tab = &mut app.ws.tabs[ti];
        let pane = &mut tab.panes[i];
        let mut v = ViewCtx {
            config: &app.config,
            keymap: &app.keymap,
            services: &mut app.services,
            cmds: &mut app.cmds,
            focused: focused == i,
            list_focus,
            checkbox_mode: app.ui.checkbox_mode,
            show_hidden: app.ui.show_hidden,
            show_system: app.ui.show_system,
            volumes: &app.ui.volumes,
            dual: true,
            fileops: &mut app.fileops,
        };
        pane_view::show(ui, rect, pane, &mut v, true);
    }
}
