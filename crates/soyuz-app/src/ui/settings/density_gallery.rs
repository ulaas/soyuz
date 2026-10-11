//! Density preview gallery (Settings → Appearance): one mini file list per density,
//! drawn at the real relative row heights; clicking a card selects that density.

use egui::accesskit::Role;
use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::config::Density;

use crate::theme::icons::ph;
use crate::theme::tokens::radius;
use crate::theme::{icons, palette};

fn options() -> [(Density, &'static str); 4] {
    [
        (Density::Compact, tr!("settings-density-compact")),
        (Density::Cozy, tr!("settings-density-cozy")),
        (Density::Comfortable, tr!("settings-density-comfortable")),
        (Density::Touch, tr!("settings-density-touch")),
    ]
}

/// Scale of the mini previews (row heights keep their real ratios).
const SCALE: f32 = 0.62;
const GAP: f32 = 10.0;

/// Card layout: how many columns fit and how wide each card is.
pub fn columns(width: f32) -> (usize, f32) {
    let cols = if width >= 4.0 * 130.0 + 3.0 * GAP { 4 } else { 2 };
    let w = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
    (cols, w.max(80.0))
}

/// Draw the gallery; returns whether the density changed.
pub fn gallery(ui: &mut Ui, density: &mut Density) -> bool {
    let p = palette(ui.ctx());
    let (cols, w) = columns(ui.available_width());
    let rows_shown = 3;
    let preview_h = Density::Touch.row_height() * SCALE * rows_shown as f32 + 12.0;
    let card_h = preview_h + 34.0;
    let mut changed = false;
    for chunk in options().chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for &(d, label) in chunk {
                let (rect, resp) = ui.allocate_exact_size(vec2(w, card_h), Sense::click());
                let selected = *density == d;
                crate::ui::a11y::with(ui.ctx(), resp.id, |n| {
                    n.set_role(Role::RadioButton);
                    n.set_label(tr!("settings-density-a11y", name = label));
                    // Radio buttons report "checked" through the toggled state (UIA and AT-SPI).
                    n.set_toggled(if selected {
                        egui::accesskit::Toggled::True
                    } else {
                        egui::accesskit::Toggled::False
                    });
                });
                if resp.clicked() && !selected {
                    *density = d;
                    changed = true;
                }
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let painter = ui.painter();
                let rc = CornerRadius::same(radius::CARD);
                let hover = ui.ctx().animate_bool_with_time(
                    resp.id.with("hover"),
                    resp.hovered(),
                    crate::theme::anim(ui.ctx(), 0.12),
                );
                painter.rect_filled(rect, rc, p.bg_surface2);
                let border = if selected {
                    Stroke::new(2.0, p.accent)
                } else {
                    Stroke::new(1.0, crate::ui::settings::widgets::lerp_color(p.border_subtle, p.border_strong, hover))
                };
                painter.rect_stroke(rect, rc, border, StrokeKind::Inside);
                // Mini list.
                let area = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(w - 16.0, preview_h - 8.0));
                painter.rect_filled(area, CornerRadius::same(radius::ROW), p.bg_window);
                let rh = d.row_height() * SCALE;
                let items = [
                    (ph::FOLDER_SIMPLE, p.glyph_folder, 0.62),
                    (ph::IMAGE, p.glyph_image, 0.78),
                    (ph::FILE_TEXT, p.glyph_doc, 0.5),
                ];
                for (i, (g, color, len)) in items.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(area.left() + 4.0, area.top() + 4.0 + i as f32 * rh),
                        vec2(area.width() - 8.0, rh - 1.0),
                    );
                    if i == 0 {
                        painter.rect_filled(r, CornerRadius::same(3), p.accent_subtle);
                    }
                    let icon = (rh * 0.55).clamp(7.0, 14.0);
                    painter.text(
                        pos2(r.left() + 4.0, r.center().y),
                        Align2::LEFT_CENTER,
                        *g,
                        icons::font(icon),
                        *color,
                    );
                    let bar_x = r.left() + 8.0 + icon;
                    let bar = Rect::from_min_size(
                        pos2(bar_x, r.center().y - 1.5),
                        vec2(((r.right() - bar_x - 6.0) * len).max(6.0), 3.0),
                    );
                    painter.rect_filled(bar, CornerRadius::same(2), p.text_tertiary.gamma_multiply(0.7));
                }
                // Caption.
                let ty = rect.bottom() - 15.0;
                painter.text(
                    pos2(rect.left() + 10.0, ty),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(12.5),
                    if selected { p.text_primary } else { p.text_secondary },
                );
                painter.text(
                    pos2(rect.right() - 10.0, ty),
                    Align2::RIGHT_CENTER,
                    if selected { ph::CHECK_CIRCLE.to_string() } else { format!("{} px", d.row_height() as u32) },
                    if selected { icons::font_fill(14.0) } else { FontId::proportional(11.0) },
                    if selected { p.accent } else { p.text_tertiary },
                );
                if resp.has_focus() {
                    crate::ui::widgets::focus_ring(ui, rect);
                }
            }
        });
        ui.add_space(GAP);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_wrap_on_narrow_pages() {
        assert_eq!(columns(700.0).0, 4);
        assert_eq!(columns(400.0).0, 2);
        let (n, w) = columns(700.0);
        assert!((w * n as f32 + GAP * (n as f32 - 1.0) - 700.0).abs() < 0.01);
    }
}
