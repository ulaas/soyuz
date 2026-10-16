//! Small shared pieces of the editor UI: slim custom scrollbars (the editors virtualize
//! their content and keep their own scroll position), banners and inline input fields.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Stroke, TextEdit, Ui, pos2, vec2};

use crate::theme::tokens::radius;
use crate::theme::{Palette, icons};

/// Width of the slim scrollbars.
pub const SCROLLBAR: f32 = 10.0;

/// A vertical scrollbar over `rect`: `pos` (top, in content units) of `total` units
/// with `visible` units on screen. Returns `true` while it is being dragged.
pub fn vscroll(ui: &mut Ui, rect: Rect, id: Id, total: f64, visible: f64, pos: &mut f64, p: &Palette) -> bool {
    scrollbar(ui, rect, id, total, visible, pos, p, true)
}

/// A horizontal scrollbar (see [`vscroll`]).
pub fn hscroll(ui: &mut Ui, rect: Rect, id: Id, total: f64, visible: f64, pos: &mut f64, p: &Palette) -> bool {
    scrollbar(ui, rect, id, total, visible, pos, p, false)
}

#[allow(clippy::too_many_arguments)]
fn scrollbar(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    total: f64,
    visible: f64,
    pos: &mut f64,
    p: &Palette,
    vertical: bool,
) -> bool {
    if total <= visible || rect.width() < 2.0 || rect.height() < 2.0 {
        return false;
    }
    let len = if vertical { rect.height() } else { rect.width() } as f64;
    let thumb_len = (len * visible / total).clamp(24.0_f64.min(len), len);
    let max_pos = (total - visible).max(0.0);
    let t = if max_pos > 0.0 { (*pos / max_pos).clamp(0.0, 1.0) } else { 0.0 };
    let start = (len - thumb_len) * t;
    let thumb = if vertical {
        Rect::from_min_size(pos2(rect.left(), rect.top() + start as f32), vec2(rect.width(), thumb_len as f32))
    } else {
        Rect::from_min_size(pos2(rect.left() + start as f32, rect.top()), vec2(thumb_len as f32, rect.height()))
    };
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let grab = id.with("grab");
    if resp.drag_started()
        && let Some(pt) = resp.interact_pointer_pos()
    {
        let along = if vertical { pt.y - thumb.top() } else { pt.x - thumb.left() };
        let inside = if vertical { thumb.y_range().contains(pt.y) } else { thumb.x_range().contains(pt.x) };
        let off = if inside { along as f64 } else { thumb_len / 2.0 };
        ui.data_mut(|d| d.insert_temp(grab, off));
    }
    if (resp.dragged() || resp.clicked())
        && let Some(pt) = resp.interact_pointer_pos()
    {
        let off: f64 = ui.data(|d| d.get_temp(grab)).unwrap_or(thumb_len / 2.0);
        let along = if vertical { pt.y - rect.top() } else { pt.x - rect.left() } as f64 - off;
        let span = (len - thumb_len).max(1.0);
        *pos = (along / span).clamp(0.0, 1.0) * max_pos;
    }
    let active = resp.dragged() || resp.hovered();
    let shrink = if active { 1.5 } else { 2.5 };
    let r = if vertical { thumb.shrink2(vec2(shrink, 1.0)) } else { thumb.shrink2(vec2(1.0, shrink)) };
    let color = if active { p.text_tertiary } else { p.border_strong };
    ui.painter().rect_filled(r, CornerRadius::same(4), color);
    resp.dragged()
}

/// A slim notice bar (warning / info) with right-aligned buttons; returns the index of
/// the clicked button.
pub fn banner(
    ui: &mut Ui,
    rect: Rect,
    p: &Palette,
    glyph: &str,
    color: Color32,
    text: &str,
    buttons: &[&str],
) -> Option<usize> {
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), color.gamma_multiply(0.14));
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius::ROW),
        Stroke::new(1.0, color.gamma_multiply(0.45)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        icons::font(14.0),
        color,
    );
    let mut x = rect.right() - 6.0;
    let mut clicked = None;
    for (i, label) in buttons.iter().enumerate().rev() {
        let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), p.text_primary);
        let w = g.size().x + 18.0;
        let r = Rect::from_min_max(pos2(x - w, rect.top() + 4.0), pos2(x, rect.bottom() - 4.0));
        let resp = ui.interact(r, ui.scope_id().with(("banner", text, i)), Sense::click());
        crate::ui::a11y::button(&resp, label);
        let fill = if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 };
        ui.painter().rect_filled(r, CornerRadius::same(radius::CONTROL), fill);
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(radius::CONTROL),
            Stroke::new(1.0, p.border_subtle),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(pos2(r.left() + 9.0, r.center().y - g.size().y / 2.0), g, p.text_primary);
        if resp.clicked() {
            clicked = Some(i);
        }
        x -= w + 6.0;
    }
    let max_w = (x - rect.left() - 30.0).max(40.0);
    let g = crate::ui::widgets::trunc_galley(ui.painter(), text, FontId::proportional(12.0), p.text_primary, max_w);
    ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, p.text_primary);
    clicked
}

/// A compact single-line input placed at `rect` (themed by the global style).
pub fn field(ui: &mut Ui, rect: Rect, id: Id, text: &mut String, hint: &str, mono: bool) -> Response {
    let mut edit = TextEdit::singleline(text).id(id).hint_text(hint).desired_width(rect.width());
    if mono {
        edit = edit.font(egui::TextStyle::Monospace);
    }
    ui.put(rect, edit)
}

/// A small toggle chip ("Aa", "W", ".*") for search options.
pub fn chip(ui: &mut Ui, rect: Rect, id: Id, label: &str, tip: &str, on: bool, p: &Palette) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    crate::ui::a11y::toggle(&resp, tip, on);
    let fill = if on {
        p.accent_subtle
    } else if resp.hovered() {
        p.bg_surface2
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::CONTROL), fill);
    let color = if on { p.accent } else { p.text_secondary };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(11.5), color);
    resp.on_hover_text(tip).clicked()
}

/// A small icon button at `rect`.
pub fn icon_at(ui: &mut Ui, rect: Rect, id: Id, glyph: &str, tip: &str, enabled: bool, p: &Palette) -> bool {
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    crate::ui::a11y::button(&resp, tip);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(radius::CONTROL), p.bg_surface2);
    }
    let color = if !enabled {
        p.text_tertiary.gamma_multiply(0.6)
    } else if resp.hovered() {
        p.text_primary
    } else {
        p.text_secondary
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, icons::font(14.0), color);
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    enabled && resp.clicked()
}
