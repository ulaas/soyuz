//! Toolbar above the panes: back / forward (right-click for history) / up, the
//! omnibar, the live filter, and the view / sort / dual-pane controls. Acts on the
//! focused pane.

use egui::{
    Align2, CornerRadius, FontId, Frame, Id, Key, Margin, Panel, Popup, Rect, RectAlign, Sense, Stroke, StrokeKind,
    TextEdit, Ui, UiBuilder, pos2, vec2,
};
use soyuz_core::config::Action;
use soyuz_core::{Location, ViewMode};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::breadcrumb::{self, OmnibarCtx};
use crate::ui::context_menu::{group_items, sort_items, view_mode_items};
use crate::ui::views::computer::volume_label;
use crate::ui::widgets::{icon_button, icon_toggle, menu_caption, menu_check, menu_item, menu_separator, submenu};

const FILTER_W: f32 = 210.0;

fn tip(app: &SoyuzApp, label: &str, action: &Action) -> String {
    match chord_label(&app.keymap, action) {
        Some(k) => format!("{label}  ({k})"),
        None => label.to_string(),
    }
}

pub fn show(root: &mut Ui, app: &mut SoyuzApp) {
    let p = palette(root.ctx());
    Panel::top("soyuz.toolbar")
        .exact_size(size::TOOLBAR)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 6, right: 8, top: 4, bottom: 4 }))
        .show(root, |ui| {
            let full = ui.available_rect_before_wrap();
            let pane_id = app.ws.pane().id;
            let (can_back, can_fwd, can_up) = {
                let pane = app.ws.pane();
                (pane.history.can_back(), pane.history.can_forward(), pane.location.parent().is_some())
            };
            // Navigation buttons (larger in touch mode).
            let bsz = (ui.spacing().interact_size.y + 6.0).max(30.0);
            let mut x = full.left();
            let nav = |ui: &mut Ui, x: &mut f32, glyph: &str, tip: String, enabled: bool| {
                let r = Rect::from_min_size(pos2(*x, full.center().y - bsz / 2.0), vec2(bsz, bsz));
                *x += bsz + 2.0;
                ui.scope_builder(UiBuilder::new().max_rect(r), |ui| icon_button(ui, glyph, &tip, enabled)).inner
            };
            let back = nav(ui, &mut x, ph::ARROW_LEFT, tip(app, tr!("app-toolbar-back"), &Action::Back), can_back);
            let fwd = nav(ui, &mut x, ph::ARROW_RIGHT, tip(app, tr!("app-toolbar-forward"), &Action::Forward), can_fwd);
            let up = nav(ui, &mut x, ph::ARROW_UP, tip(app, tr!("app-toolbar-up"), &Action::Up), can_up);
            if back.clicked() {
                app.cmds.push(Cmd::PaneAction(pane_id, Action::Back));
            }
            if fwd.clicked() {
                app.cmds.push(Cmd::PaneAction(pane_id, Action::Forward));
            }
            if up.clicked() {
                app.cmds.push(Cmd::PaneAction(pane_id, Action::Up));
            }
            history_menu(ui, &back, app, true);
            history_menu(ui, &fwd, app, false);
            x += 6.0;

            // Right side controls (laid out right to left).
            let mut rx = full.right();
            let mut right_btn = |ui: &mut Ui, glyph: &str, tip: &str, on: bool| {
                let r = Rect::from_min_size(pos2(rx - bsz, full.center().y - bsz / 2.0), vec2(bsz, bsz));
                rx -= bsz + 2.0;
                ui.scope_builder(UiBuilder::new().max_rect(r), |ui| icon_toggle(ui, glyph, tip, true, on)).inner
            };
            let settings_on = app.settings.is_open();
            if right_btn(ui, ph::GEAR_SIX, &tip(app, tr!("app-toolbar-settings"), &Action::Settings), settings_on)
                .clicked()
            {
                app.cmds.push(Cmd::Action(Action::Settings));
            }
            let dual_on = app.ws.tab().dual;
            let glyph = if app.ws.tab().shown() == 4 { ph::SQUARES_FOUR } else { ph::SQUARE_SPLIT_HORIZONTAL };
            let dual_tip = tip(app, tr!("app-toolbar-dual-pane"), &Action::ToggleDualPane);
            let dual = right_btn(ui, glyph, &dual_tip, dual_on);
            if dual.clicked() {
                app.cmds.push(Cmd::Action(Action::ToggleDualPane));
            }
            dual_menu(ui, &dual, app);
            let preview_on = app.ui.preview.open;
            if right_btn(
                ui,
                ph::SIDEBAR_SIMPLE,
                &tip(app, tr!("app-toolbar-preview-pane"), &Action::TogglePreviewPane),
                preview_on,
            )
            .clicked()
            {
                app.cmds.push(Cmd::Action(Action::TogglePreviewPane));
            }
            let search_on = app.ui.search.open;
            if right_btn(ui, ph::MAGNIFYING_GLASS, &tip(app, tr!("app-toolbar-search"), &Action::Search), search_on)
                .clicked()
            {
                if search_on {
                    app.ui.search.open = false;
                } else {
                    app.cmds.push(Cmd::Action(Action::Search));
                }
            }
            let palette_on = app.palette.open;
            if right_btn(
                ui,
                ph::COMMAND,
                &tip(app, tr!("app-toolbar-command-palette"), &Action::CommandPalette),
                palette_on,
            )
            .clicked()
            {
                app.cmds.push(Cmd::Action(Action::CommandPalette));
            }
            let sort = right_btn(ui, ph::SORT_ASCENDING, tr!("app-toolbar-sort"), false);
            let mode = app.ws.pane().view.mode;
            let mode_glyph = match mode {
                ViewMode::Details => ph::LIST,
                ViewMode::List => ph::LIST_DASHES,
                ViewMode::Grid => ph::SQUARES_FOUR,
                ViewMode::Gallery => ph::IMAGES,
                ViewMode::Miller => ph::COLUMNS,
            };
            let view = right_btn(ui, mode_glyph, tr!("app-toolbar-view"), false);
            view_menu(ui, &view, app);
            sort_menu(ui, &sort, app);
            rx -= 6.0;

            // Filter field.
            let fr = Rect::from_min_size(pos2(rx - FILTER_W, full.center().y - 16.0), vec2(FILTER_W, 32.0));
            filter(ui, fr, app);
            rx = fr.left() - 8.0;

            // Omnibar fills the middle.
            let omni =
                Rect::from_min_max(pos2(x, full.center().y - 16.0), pos2(rx.max(x + 120.0), full.center().y + 16.0));
            let root_label = {
                let pane = app.ws.pane();
                match &pane.location {
                    Location::Dir(path) => app
                        .ui
                        .volumes
                        .iter()
                        .filter(|v| path.starts_with(&v.mount))
                        .max_by_key(|v| v.mount.as_os_str().len())
                        .map(volume_label),
                    _ => None,
                }
            };
            let (location, loading) = {
                let pane = app.ws.pane();
                (pane.location.clone(), pane.is_loading() || pane.refreshing)
            };
            let mut ctx = OmnibarCtx {
                pane: pane_id,
                location: &location,
                services: &mut app.services,
                cmds: &mut app.cmds,
                address: &mut app.ui.address,
                root_label,
                loading,
            };
            breadcrumb::show(ui, omni, &mut ctx);
        });
}

fn history_menu(ui: &mut Ui, button: &egui::Response, app: &mut SoyuzApp, back: bool) {
    let id = Id::new(("soyuz.history", back));
    if button.secondary_clicked() {
        Popup::open_id(ui.ctx(), id);
    }
    let pane = app.ws.pane();
    let pane_id = pane.id;
    let entries: Vec<String> = if back {
        pane.history.back_entries().map(|e| crate::state::crumbs::location_title(&e.location)).collect()
    } else {
        pane.history.forward_entries().map(|e| crate::state::crumbs::location_title(&e.location)).collect()
    };
    let paths: Vec<String> = if back {
        pane.history.back_entries().map(|e| e.location.display()).collect()
    } else {
        pane.history.forward_entries().map(|e| e.location.display()).collect()
    };
    crate::ui::native::popup::Menu::new(id, ui.ctx().clone(), button.rect, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .open_memory(None)
        .align(RectAlign::BOTTOM_START)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_width(240.0);
            menu_caption(ui, if back { tr!("app-toolbar-back") } else { tr!("app-toolbar-forward") });
            if entries.is_empty() {
                menu_item(ui, None, tr!("app-toolbar-no-history"), None, false);
            }
            for (i, (title, path)) in entries.iter().zip(&paths).enumerate().take(20) {
                let r = menu_item(ui, Some(ph::CLOCK_COUNTER_CLOCKWISE), title, None, true).on_hover_text(path);
                if r.clicked() {
                    app.cmds.push(Cmd::HistoryJump { pane: pane_id, back, n: i + 1 });
                    ui.close();
                }
            }
        });
}

fn view_menu(_ui: &mut Ui, button: &egui::Response, app: &mut SoyuzApp) {
    crate::ui::native::popup::Menu::menu(button).align(RectAlign::BOTTOM_END).show(|ui| {
        ui.set_width(250.0);
        let km = app.keymap.clone();
        let pane_id = app.ws.pane().id;
        menu_caption(ui, tr!("app-toolbar-layout"));
        view_mode_items(ui, &km, &mut app.cmds, app.ws.pane());
        menu_separator(ui);
        let pane_ref = app.ws.pane();
        let cmds = &mut app.cmds;
        submenu(ui, Some(ph::ROWS), tr!("app-ctx-group-by"), |ui| {
            ui.set_width(200.0);
            group_items(ui, cmds, pane_ref);
        });
        menu_separator(ui);
        if menu_check(
            ui,
            app.ui.show_hidden,
            tr!("app-toolbar-hidden-files"),
            chord_label(&km, &Action::ToggleHidden).as_deref(),
        )
        .clicked()
        {
            app.cmds.push(Cmd::PaneAction(pane_id, Action::ToggleHidden));
            ui.close();
        }
        if menu_check(ui, app.ui.checkbox_mode, tr!("app-ctx-item-checkboxes"), None).clicked() {
            app.cmds.push(Cmd::PaneAction(pane_id, Action::ToggleCheckboxes));
            ui.close();
        }
        if menu_check(
            ui,
            app.ui.sidebar_visible,
            tr!("app-toolbar-sidebar"),
            chord_label(&km, &Action::ToggleSidebar).as_deref(),
        )
        .clicked()
        {
            app.cmds.push(Cmd::Action(Action::ToggleSidebar));
            ui.close();
        }
        menu_separator(ui);
        if menu_item(ui, Some(ph::COLUMNS_PLUS_RIGHT), tr!("app-toolbar-choose-columns"), None, true).clicked() {
            app.cmds.push(Cmd::PaneAction(pane_id, Action::ChooseColumns));
            ui.close();
        }
        let custom = app.views.has_custom(&app.ws.pane().location);
        if menu_item(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("app-toolbar-reset-folder-view"), None, custom)
            .clicked()
        {
            app.cmds.push(Cmd::PaneAction(pane_id, Action::ResetFolderView));
            ui.close();
        }
        if menu_item(ui, Some(ph::STACK), tr!("app-toolbar-apply-view-to-all"), None, true).clicked() {
            app.cmds.push(Cmd::PaneAction(pane_id, Action::ApplyViewToAll));
            ui.close();
        }
    });
}

fn sort_menu(_ui: &mut Ui, button: &egui::Response, app: &mut SoyuzApp) {
    crate::ui::native::popup::Menu::menu(button).align(RectAlign::BOTTOM_END).show(|ui| {
        ui.set_width(220.0);
        menu_caption(ui, tr!("app-ctx-sort-by"));
        sort_items(ui, &mut app.cmds, app.ws.pane());
        menu_separator(ui);
        let id = app.ws.pane().id;
        let pane = app.ws.pane_mut();
        if menu_check(ui, pane.view.sort.folders_first, tr!("app-toolbar-folders-first"), None).clicked() {
            pane.view.sort.folders_first = !pane.view.sort.folders_first;
            pane.resort();
            app.cmds.push(Cmd::ViewChanged(id));
            ui.close();
        }
        if menu_check(ui, pane.view.sort.natural, tr!("app-toolbar-natural-order"), None).clicked() {
            pane.view.sort.natural = !pane.view.sort.natural;
            pane.resort();
            app.cmds.push(Cmd::ViewChanged(id));
            ui.close();
        }
    });
}

fn dual_menu(ui: &mut Ui, button: &egui::Response, app: &mut SoyuzApp) {
    let id = Id::new("soyuz.dual.menu");
    if button.secondary_clicked() {
        Popup::open_id(ui.ctx(), id);
    }
    crate::ui::native::popup::Menu::new(id, ui.ctx().clone(), button.rect, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .open_memory(None)
        .align(RectAlign::BOTTOM_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_width(250.0);
            let km = app.keymap.clone();
            // Single / dual (side by side, stacked) / quad layouts (F7).
            let mut layout_cmds = Vec::new();
            crate::ui::workspaces::layout_menu(ui, app, &mut layout_cmds);
            app.cmds.extend(layout_cmds);
            let tab = app.ws.tab_mut();
            let dual = tab.dual;
            menu_separator(ui);
            let sync_key = chord_label(&km, &Action::SyncNavigation);
            if menu_check(ui, tab.sync_nav, tr!("app-toolbar-sync-browsing"), sync_key.as_deref()).clicked() {
                app.cmds.push(Cmd::Action(Action::SyncNavigation));
            }
            if menu_item(
                ui,
                Some(ph::ARROWS_LEFT_RIGHT),
                tr!("app-toolbar-swap-panes"),
                chord_label(&km, &Action::SwapPanes).as_deref(),
                dual,
            )
            .clicked()
            {
                app.cmds.push(Cmd::Action(Action::SwapPanes));
            }
            if menu_item(ui, Some(ph::COPY_SIMPLE), tr!("app-toolbar-mirror-pane"), None, true).clicked() {
                app.cmds.push(Cmd::Action(Action::MirrorPaneToOther));
            }
            if menu_item(
                ui,
                Some(ph::ARROW_SQUARE_RIGHT),
                tr!("app-toolbar-focus-other-pane"),
                chord_label(&km, &Action::FocusOtherPane).as_deref(),
                dual,
            )
            .clicked()
            {
                app.cmds.push(Cmd::Action(Action::FocusOtherPane));
            }
        });
}

/// Live filter for the focused pane.
fn filter(ui: &mut Ui, r: Rect, app: &mut SoyuzApp) {
    let p = palette(ui.ctx());
    let id = Id::new("soyuz.filter");
    let focused = ui.memory(|m| m.has_focus(id));
    let pane = app.ws.pane_mut();
    let active = pane.filter_active();
    ui.painter().rect_filled(r, CornerRadius::same(radius::CARD), p.bg_surface1);
    let stroke = if focused {
        Stroke::new(1.5, p.accent)
    } else if active {
        Stroke::new(1.0, p.accent.gamma_multiply(0.6))
    } else {
        Stroke::new(1.0, p.border_subtle)
    };
    ui.painter().rect_stroke(r, CornerRadius::same(radius::CARD), stroke, StrokeKind::Inside);
    ui.painter().text(
        pos2(r.left() + 16.0, r.center().y),
        Align2::CENTER_CENTER,
        ph::FUNNEL_SIMPLE,
        icons::font(14.0),
        if active { p.accent } else { p.text_tertiary },
    );
    let mut text = pane.filter.clone();
    let inner =
        Rect::from_min_max(pos2(r.left() + 30.0, r.center().y - 11.0), pos2(r.right() - 28.0, r.center().y + 11.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    let esc = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let resp = child.add(
        TextEdit::singleline(&mut text)
            .id(id)
            .hint_text(tr!("app-toolbar-filter"))
            .frame(Frame::NONE)
            .margin(Margin::symmetric(0, 2))
            .font(FontId::proportional(13.0))
            .vertical_align(egui::Align::Center)
            .desired_width(inner.width()),
    );
    crate::ui::a11y::name(&resp, tr!("app-toolbar-filter"));
    if app.ui.filter_focus {
        resp.request_focus();
        app.ui.filter_focus = false;
    }
    if resp.changed() {
        pane.set_filter(&text);
    }
    if esc {
        pane.set_filter("");
        resp.surrender_focus();
        app.ui.focus = crate::app::Focus::List;
    }
    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        app.ui.focus = crate::app::Focus::List;
        // Enter in the filter selects the first match.
        if pane.cursor_pos().is_none() && !pane.visible.is_empty() {
            pane.move_cursor(0, false, false);
        }
    }
    if active {
        let cr = Rect::from_center_size(pos2(r.right() - 15.0, r.center().y), vec2(20.0, 20.0));
        let cresp = ui.interact(cr, id.with("clear"), Sense::click());
        if cresp.hovered() {
            ui.painter().rect_filled(cr, CornerRadius::same(radius::CONTROL), p.bg_surface2);
        }
        ui.painter().text(cr.center(), Align2::CENTER_CENTER, ph::X, icons::font(12.0), p.text_tertiary);
        if cresp.clicked() {
            pane.set_filter("");
        }
    } else if !focused {
        let hint = chord_label(&app.keymap, &Action::Filter).unwrap_or_default();
        ui.painter().text(
            pos2(r.right() - 10.0, r.center().y),
            Align2::RIGHT_CENTER,
            hint,
            FontId::proportional(11.0),
            p.text_tertiary,
        );
    }
}
