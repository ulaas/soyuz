//! Browser-style tab strip: pinned tabs (icon only) first, active tab as a raised pill,
//! close on hover, middle-click close, drag to reorder, double-click empty space or `+`
//! for a new tab, right-click for tab actions.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Margin, Panel, Rect, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use soyuz_core::config::Action;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::state::crumbs::location_title;
use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{menu_item, menu_separator, trunc_galley};

const PINNED_W: f32 = 40.0;
const MAX_W: f32 = 220.0;
const MIN_W: f32 = 96.0;

#[derive(Clone, Copy, Debug)]
struct Drag {
    from: usize,
    grab_dx: f32,
}

pub fn show(root: &mut Ui, app: &mut SoyuzApp) {
    let p = palette(root.ctx());
    Panel::top("soyuz.tabstrip")
        .exact_size(size::TAB_STRIP)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 6, right: 8, top: 4, bottom: 0 }))
        .show(root, |ui| {
            let full = ui.available_rect_before_wrap();
            let n = app.ws.tabs.len();
            let pinned = app.ws.tabs.iter().filter(|t| t.pinned).count();
            let plus_w = 32.0;
            let avail = full.width() - plus_w - 4.0;
            let unpinned = (n - pinned).max(1) as f32;
            let w = ((avail - pinned as f32 * (PINNED_W + 2.0)) / unpinned - 2.0).clamp(MIN_W, MAX_W);
            let drag_id = Id::new("soyuz.tabdrag");
            let drag: Option<Drag> = ui.ctx().data(|d| d.get_temp(drag_id)).filter(|d: &Drag| d.from < n);
            let pointer = ui.input(|i| i.pointer.latest_pos());

            // Background: double-click empty space → new tab.
            let bg = ui.interact(full, Id::new("soyuz.tabstrip.bg"), Sense::click());
            crate::ui::a11y::container(ui.ctx(), bg.id, egui::accesskit::Role::TabList, "Tabs");
            if bg.double_clicked() {
                app.cmds.push(Cmd::Action(Action::NewTab));
            }

            let mut rects: Vec<Rect> = Vec::with_capacity(n);
            let mut x = full.left();
            for t in &app.ws.tabs {
                let tw = if t.pinned { PINNED_W } else { w };
                rects.push(Rect::from_min_size(pos2(x, full.top()), vec2(tw, full.height() - 4.0)));
                x += tw + 2.0;
            }
            let clip = Rect::from_min_max(full.min, pos2(full.right() - plus_w, full.bottom()));
            let painter = ui.painter_at(clip);
            for (i, &r) in rects.iter().enumerate() {
                let tab = &app.ws.tabs[i];
                let active = i == app.ws.active;
                let resp = ui.interact(r, Id::new(("soyuz.tab", tab.id.0)), Sense::click_and_drag());
                let dragging = drag.is_some_and(|d| d.from == i);
                let r = if dragging {
                    let px = pointer.map_or(r.left(), |pp| pp.x - drag.map_or(0.0, |d| d.grab_dx));
                    Rect::from_min_size(pos2(px.clamp(full.left(), clip.right() - r.width()), r.top()), r.size())
                } else {
                    r
                };
                let hovered = resp.hovered() || dragging;
                let hover_t =
                    ui.ctx().animate_bool_with_time(resp.id.with("h"), hovered, crate::theme::anim(ui.ctx(), 0.1));
                if active || dragging {
                    let shadow = egui::epaint::Shadow {
                        offset: [0, 1],
                        blur: 6,
                        spread: 0,
                        color: Color32::from_black_alpha(if p.dark { 60 } else { 18 }),
                    };
                    painter.add(shadow.as_shape(r, CornerRadius::same(radius::CARD)));
                    painter.rect_filled(r, CornerRadius::same(radius::CARD), p.bg_surface1);
                    painter.rect_stroke(
                        r,
                        CornerRadius::same(radius::CARD),
                        Stroke::new(1.0, p.border_subtle),
                        StrokeKind::Inside,
                    );
                } else if hover_t > 0.0 {
                    painter.rect_filled(
                        r,
                        CornerRadius::same(radius::CARD),
                        p.bg_surface2.gamma_multiply(hover_t * 0.8),
                    );
                }
                let pane = tab.active();
                let title = location_title(&pane.location);
                crate::ui::a11y::tab(&resp, &title, active);
                let glyph = if tab.dual {
                    ph::SQUARE_SPLIT_HORIZONTAL
                } else if active {
                    ph::FOLDER_OPEN
                } else {
                    ph::FOLDER
                };
                let gcolor = if active { p.glyph_folder } else { p.text_tertiary };
                let loading = pane.is_loading();
                if tab.pinned {
                    painter.text(r.center(), Align2::CENTER_CENTER, glyph, icons::font_fill(15.0), gcolor);
                } else {
                    if loading && active {
                        crate::ui::widgets::spinner(ui, pos2(r.left() + 18.0, r.center().y), 5.5, p.accent);
                    } else {
                        painter.text(
                            pos2(r.left() + 18.0, r.center().y),
                            Align2::CENTER_CENTER,
                            glyph,
                            icons::font_fill(15.0),
                            gcolor,
                        );
                    }
                    let show_close = active || hovered;
                    let text_right = r.right() - if show_close { 30.0 } else { 12.0 };
                    let color = if active { p.text_primary } else { p.text_secondary };
                    let g = trunc_galley(
                        &painter,
                        &title,
                        FontId::proportional(12.5),
                        color,
                        text_right - (r.left() + 32.0),
                    );
                    painter.galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, color);
                    if show_close && n > 1 {
                        let cr = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(20.0, 20.0));
                        let cresp = ui.interact(cr, Id::new(("soyuz.tab.close", tab.id.0)), Sense::click());
                        crate::ui::a11y::button(&cresp, &format!("Close tab {title}"));
                        if cresp.hovered() {
                            painter.rect_filled(cr, CornerRadius::same(radius::CONTROL), p.bg_surface3);
                        }
                        painter.text(
                            cr.center(),
                            Align2::CENTER_CENTER,
                            ph::X,
                            icons::font(11.0),
                            if cresp.hovered() { p.text_primary } else { p.text_tertiary },
                        );
                        if cresp.clicked() {
                            app.cmds.push(Cmd::CloseTab(i));
                        }
                    }
                }
                let resp = resp.on_hover_text(pane.location.display());
                if let soyuz_core::Location::Dir(d) = &pane.location {
                    crate::ops::fileops::dnd::target(ui, &resp, d, &mut app.cmds);
                }
                if resp.clicked() {
                    app.cmds.push(Cmd::SelectTab(i));
                }
                if resp.middle_clicked() {
                    app.cmds.push(Cmd::CloseTab(i));
                }
                if resp.drag_started() {
                    let grab_dx = pointer.map_or(0.0, |pp| pp.x - rects[i].left());
                    ui.ctx().data_mut(|d| d.insert_temp(drag_id, Drag { from: i, grab_dx }));
                    app.cmds.push(Cmd::SelectTab(i));
                }
                let km = app.keymap.clone();
                let pinned_now = tab.pinned;
                let cmds = &mut app.cmds;
                resp.context_menu(|ui| tab_menu(ui, cmds, &km, i, pinned_now, n));
            }

            // Drag-to-reorder: drop position from the pointer.
            if let Some(d) = drag {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                let down = ui.input(|i| i.pointer.primary_down());
                if !down {
                    if let Some(pp) = pointer {
                        let x = pp.x - d.grab_dx + rects[d.from].width() / 2.0;
                        let to = rects.iter().position(|r| x < r.center().x).unwrap_or(n).min(n);
                        let to = if to > d.from { to - 1 } else { to };
                        if to != d.from {
                            app.cmds.push(Cmd::MoveTab { from: d.from, to });
                        }
                    }
                    ui.ctx().data_mut(|dm| dm.remove::<Drag>(drag_id));
                }
                ui.ctx().request_repaint();
            }

            // New tab button.
            let plus_x = rects.last().map_or(full.left(), |r| r.right() + 4.0).min(full.right() - plus_w);
            let pr = Rect::from_min_size(pos2(plus_x, full.top() + 2.0), vec2(28.0, 28.0));
            let presp = ui.interact(pr, Id::new("soyuz.tab.new"), Sense::click());
            crate::ui::a11y::button(&presp, "New tab");
            if presp.hovered() {
                ui.painter().rect_filled(pr, CornerRadius::same(radius::ROW), p.bg_surface2);
            }
            ui.painter().text(
                pr.center(),
                Align2::CENTER_CENTER,
                ph::PLUS,
                icons::font(15.0),
                if presp.hovered() { p.text_primary } else { p.text_secondary },
            );
            let tip = match chord_label(&app.keymap, &Action::NewTab) {
                Some(k) => format!("New tab ({k})"),
                None => "New tab".into(),
            };
            if presp.on_hover_text(tip).clicked() {
                app.cmds.push(Cmd::Action(Action::NewTab));
            }
        });
}

fn tab_menu(ui: &mut Ui, cmds: &mut Vec<Cmd>, km: &soyuz_core::config::Keymap, i: usize, pinned: bool, n: usize) {
    ui.set_width(230.0);
    let sc = |a: &Action| chord_label(km, a);
    if menu_item(ui, Some(ph::PLUS), "New tab", sc(&Action::NewTab).as_deref(), true).clicked() {
        cmds.push(Cmd::NewTabAfter(i));
        ui.close();
    }
    if menu_item(ui, Some(ph::COPY_SIMPLE), "Duplicate tab", sc(&Action::DuplicateTab).as_deref(), true).clicked() {
        cmds.push(Cmd::DuplicateTab(i));
        ui.close();
    }
    let (pin_icon, pin_label) = if pinned { (ph::PUSH_PIN_SLASH, "Unpin tab") } else { (ph::PUSH_PIN, "Pin tab") };
    if menu_item(ui, Some(pin_icon), pin_label, sc(&Action::PinTab).as_deref(), true).clicked() {
        cmds.push(Cmd::PinTab(i, !pinned));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::X), "Close tab", sc(&Action::CloseTab).as_deref(), n > 1).clicked() {
        cmds.push(Cmd::CloseTab(i));
        ui.close();
    }
    if menu_item(ui, Some(ph::X_CIRCLE), "Close other tabs", None, n > 1).clicked() {
        cmds.push(Cmd::CloseOtherTabs(i));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROW_LINE_RIGHT), "Close tabs to the right", None, i + 1 < n).clicked() {
        cmds.push(Cmd::CloseTabsToRight(i));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(
        ui,
        Some(ph::ARROW_COUNTER_CLOCKWISE),
        "Reopen closed tab",
        sc(&Action::ReopenClosedTab).as_deref(),
        true,
    )
    .clicked()
    {
        cmds.push(Cmd::Action(Action::ReopenClosedTab));
        ui.close();
    }
}
