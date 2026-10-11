//! Draws the live AI highlights ([`crate::ai::touch`]): an animated outline and a badge
//! on rows / cells of marked paths.

use std::path::Path;

use egui::{Align2, Color32, CornerRadius, Painter, Rect, Stroke, StrokeKind, pos2};

use crate::ai::touch::{AiTouch, TouchKind, get};
use crate::theme::Palette;
use crate::theme::icons::{self, ph};

fn color(kind: TouchKind, p: &Palette) -> Color32 {
    match kind {
        TouchKind::Reading => p.info,
        TouchKind::Planned => p.warning,
        TouchKind::Changing => p.accent,
        TouchKind::Done => p.success,
    }
}

/// Outline + badge on a row / cell of `path` (no-op when untouched).
pub fn paint(painter: &Painter, p: &Palette, rect: Rect, path: &Path, reduced_motion: bool) {
    let Some(t) = get(path) else { return };
    paint_touch(painter, p, rect, t, reduced_motion);
}

/// Outline (+ badge when the rect is big enough) for a known mark.
pub fn paint_touch(painter: &Painter, p: &Palette, rect: Rect, t: AiTouch, reduced_motion: bool) {
    let age = t.age();
    let fade = t.fade(age);
    let pulse = if reduced_motion { 0.8 } else { 0.55 + 0.45 * (age * 3.2).sin().abs() };
    let c = color(t.kind, p);
    let r = rect.shrink(1.0);
    let small = r.width() < 26.0 || r.height() < 14.0;
    let round = CornerRadius::same(if small { 2 } else { 6 });
    painter.rect_filled(r, round, c.gamma_multiply(if small { 0.18 } else { 0.07 } * fade));
    painter.rect_stroke(r, round, Stroke::new(1.5, c.gamma_multiply(pulse * fade)), StrokeKind::Inside);
    if small {
        return;
    }
    // Badge at the right edge: a sparkle (assistant) or a plug (MCP client).
    let center = pos2(r.right() - 11.0, r.top() + 10.0_f32.min(r.height() / 2.0));
    painter.circle_filled(center, 7.5, c.gamma_multiply(0.9 * fade));
    let glyph = if t.mcp { ph::PLUGS_CONNECTED } else { ph::SPARKLE };
    let font = if t.mcp { icons::font(9.5) } else { icons::font_fill(9.5) };
    painter.text(center, Align2::CENTER_CENTER, glyph, font, Color32::WHITE.gamma_multiply(fade));
}
