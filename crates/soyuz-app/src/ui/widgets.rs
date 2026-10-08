//! Small reusable widgets that follow the design tokens: nav rows, icon buttons,
//! section headers, keyboard badges, the focus ring and usage bars.

use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextStyle, Ui, vec2,
};

use crate::theme::apply::CAPTION;
use crate::theme::tokens::{radius, size, space};
use crate::theme::{Palette, icons, palette};

/// 2 px accent ring with a 1 px `bg.window` gap, drawn around `rect` (PLAN §1.2).
pub fn focus_ring(ui: &Ui, rect: Rect) {
    let p = palette(ui.ctx());
    let painter = ui.painter();
    painter.rect_stroke(
        rect.expand(1.0),
        CornerRadius::same(radius::ROW + 1),
        Stroke::new(1.0, p.bg_window),
        StrokeKind::Outside,
    );
    painter.rect_stroke(
        rect.expand(2.0),
        CornerRadius::same(radius::ROW + 2),
        Stroke::new(2.0, p.accent),
        StrokeKind::Outside,
    );
}

/// Uppercase section caption ("FAVORITES").
pub fn section_header(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.add_space(space::LG);
    ui.horizontal(|ui| {
        ui.add_space(space::MD);
        ui.label(RichText::new(text.to_uppercase()).text_style(TextStyle::Name(CAPTION.into())).color(p.text_tertiary));
    });
    ui.add_space(space::XS);
}

/// One sidebar navigation row: icon + label, hover / active fills, optional usage bar.
pub struct NavRow<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    pub active: bool,
    pub icon_color: Option<Color32>,
    /// `(used fraction 0..=1)` drawn as a thin bar under the label.
    pub usage: Option<f32>,
}

impl NavRow<'_> {
    pub fn show(self, ui: &mut Ui) -> Response {
        let p = palette(ui.ctx());
        let height = if self.usage.is_some() { size::ROW_COZY + 6.0 } else { size::ROW_COZY };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect.expand(3.0));
            let rounding = CornerRadius::same(radius::ROW);
            if self.active {
                painter.rect_filled(rect, rounding, p.accent_subtle);
            } else if response.hovered() {
                painter.rect_filled(rect, rounding, p.bg_surface2);
            }
            let icon_font = if self.active { icons::font_fill(size::ICON) } else { icons::font(size::ICON) };
            let icon_color = if self.active { p.accent } else { self.icon_color.unwrap_or(p.text_secondary) };
            let text_y = if self.usage.is_some() { rect.top() + size::ROW_COZY / 2.0 - 1.0 } else { rect.center().y };
            painter.text(egui::pos2(rect.left() + 10.0, text_y), Align2::LEFT_CENTER, self.icon, icon_font, icon_color);
            let label_color = if self.active { p.text_primary } else { p.text_primary.gamma_multiply(0.92) };
            let text_rect = Rect::from_min_max(
                egui::pos2(rect.left() + 34.0, rect.top()),
                egui::pos2(rect.right() - 8.0, rect.bottom()),
            );
            ui.painter_at(text_rect).text(
                egui::pos2(text_rect.left(), text_y),
                Align2::LEFT_CENTER,
                self.label,
                FontId::proportional(13.0),
                label_color,
            );
            if let Some(used) = self.usage {
                let bar = Rect::from_min_size(
                    egui::pos2(rect.left() + 34.0, rect.bottom() - 8.0),
                    vec2((rect.width() - 46.0).max(10.0), 3.0),
                );
                usage_bar(ui, &p, bar, used);
            }
            if response.has_focus() {
                focus_ring(ui, rect);
            }
        }
        response
    }
}

/// Thin free-space bar: accent normally, `warning` when < 15 % free, `danger` < 5 %.
pub fn usage_bar(ui: &Ui, p: &Palette, rect: Rect, used: f32) {
    let used = used.clamp(0.0, 1.0);
    let free = 1.0 - used;
    let color = if free < 0.05 {
        p.danger
    } else if free < 0.15 {
        p.warning
    } else {
        p.accent
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), p.border_subtle);
    let mut fill = rect;
    fill.set_width(rect.width() * used);
    painter.rect_filled(fill, CornerRadius::same(2), color);
}

/// Frameless square icon button (frame appears on hover).
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    let color = if enabled { p.text_secondary } else { p.text_tertiary.gamma_multiply(0.7) };
    let button = egui::Button::new(icons::icon(glyph, size::ICON_TOOLBAR - 2.0).color(color))
        .frame_when_inactive(false)
        .min_size(vec2(30.0, 30.0))
        .corner_radius(CornerRadius::same(radius::ROW));
    let r = ui.add_enabled(enabled, button);
    if r.has_focus() {
        focus_ring(ui, r.rect);
    }
    r.on_hover_text(tooltip).on_disabled_hover_text(tooltip)
}

/// Keyboard shortcut badge (`Ctrl+Shift+P`).
pub fn kbd(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui.ctx());
    egui::Frame::new()
        .fill(p.bg_surface2)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(radius::CONTROL))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).text_style(TextStyle::Small).color(p.text_secondary));
        })
        .response
}

/// A 1 px vertical divider of the given height.
pub fn vdivider(ui: &mut Ui, height: f32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    ui.painter().line_segment([rect.center_top(), rect.center_bottom()], Stroke::new(1.0, p.border_subtle));
}
