//! Small widgets shared by the palette and the settings window: keyboard chips, match
//! highlighting, icon tiles.

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::theme::fonts::SEMIBOLD;
use crate::theme::{Palette, icons};

/// Display form of one key name (`"Up"` → `"↑"`).
pub fn key_glyph(k: &str) -> String {
    match k {
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Enter" => "↵".into(),
        "Backspace" => "⌫".into(),
        "Tab" => "⇥".into(),
        "PageUp" => "PgUp".into(),
        "PageDown" => "PgDn".into(),
        "Cmd" if cfg!(target_os = "macos") => "⌘".into(),
        "Alt" if cfg!(target_os = "macos") => "⌥".into(),
        "Shift" if cfg!(target_os = "macos") => "⇧".into(),
        "Ctrl" if cfg!(target_os = "macos") => "⌃".into(),
        other => other.to_string(),
    }
}

/// Split `"Ctrl+Shift+P"` / `"G G"` / `"Ctrl++"` into key groups (one group per chord of
/// a sequence, one entry per key of a chord).
pub fn split_keys(binding: &str) -> Vec<Vec<String>> {
    binding
        .split_whitespace()
        .map(|chord| {
            let mut keys: Vec<String> = Vec::new();
            let (head, plus) = match chord.strip_suffix("++") {
                Some(h) => (h, true),
                None if chord == "+" => ("", true),
                None => (chord, false),
            };
            if !head.is_empty() {
                keys.extend(head.split('+').filter(|s| !s.is_empty()).map(key_glyph));
            }
            if plus {
                keys.push("+".into());
            }
            keys
        })
        .collect()
}

/// Paint keyboard chips for `binding` right-aligned ending at `right`, vertically centered
/// on `cy`. Returns the left edge.
pub fn paint_chips(ui: &Ui, p: &Palette, binding: &str, right: f32, cy: f32, strong: bool) -> f32 {
    paint_chips_scaled(ui, p, binding, right, cy, strong, 1.0)
}

/// [`paint_chips`] at `scale` (1.0 = 20 px tall chips).
pub fn paint_chips_scaled(ui: &Ui, p: &Palette, binding: &str, right: f32, cy: f32, strong: bool, scale: f32) -> f32 {
    let font = FontId::proportional(11.0 * scale);
    let groups = split_keys(binding);
    let mut x = right;
    let painter = ui.painter();
    for (gi, group) in groups.iter().enumerate().rev() {
        for k in group.iter().rev() {
            let g = painter.layout_no_wrap(k.clone(), font.clone(), p.text_secondary);
            let w = (g.size().x + 10.0 * scale).max(20.0 * scale);
            let r = Rect::from_min_size(pos2(x - w, cy - 10.0 * scale), vec2(w, 20.0 * scale));
            let fill = if strong { p.bg_surface1 } else { p.bg_surface2 };
            painter.rect_filled(
                r.translate(vec2(0.0, 1.0)),
                CornerRadius::same(5),
                p.border_strong.gamma_multiply(0.6),
            );
            painter.rect_filled(r, CornerRadius::same(5), fill);
            painter.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
            painter.galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, p.text_secondary);
            x = r.left() - 3.0 * scale;
        }
        if gi > 0 {
            x -= 4.0;
            painter.text(
                pos2(x, cy),
                Align2::RIGHT_CENTER,
                tr!("actions-palette-then"),
                FontId::proportional(10.0),
                p.text_tertiary,
            );
            let w = painter
                .layout_no_wrap(tr!("actions-palette-then").into(), FontId::proportional(10.0), p.text_tertiary)
                .size()
                .x;
            x -= w + 5.0;
        }
    }
    x
}

/// Width [`paint_chips`] would use.
pub fn chips_width(ui: &Ui, binding: &str) -> f32 {
    chips_width_scaled(ui, binding, 1.0)
}

pub fn chips_width_scaled(ui: &Ui, binding: &str, scale: f32) -> f32 {
    let font = FontId::proportional(11.0 * scale);
    let groups = split_keys(binding);
    let mut w = 0.0;
    for (gi, group) in groups.iter().enumerate() {
        for k in group {
            let g = ui.painter().layout_no_wrap(k.clone(), font.clone(), Color32::WHITE);
            w += (g.size().x + 10.0 * scale).max(20.0 * scale) + 3.0 * scale;
        }
        if gi > 0 {
            w += 4.0
                + ui.painter()
                    .layout_no_wrap(tr!("actions-palette-then").into(), FontId::proportional(10.0), Color32::WHITE)
                    .size()
                    .x
                + 5.0;
        }
    }
    w
}

/// Allocate and paint chips inline (for layouts).
pub fn chips(ui: &mut Ui, binding: &str) -> Response {
    let p = crate::theme::palette(ui.ctx());
    let w = chips_width(ui, binding);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    paint_chips(ui, &p, binding, rect.right(), rect.center().y, false);
    resp
}

/// Text with the chars at `hl` (char indices) emphasized.
pub fn highlighted(text: &str, hl: &[u32], size: f32, color: Color32, accent: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let normal = TextFormat { font_id: FontId::proportional(size), color, ..Default::default() };
    let strong = TextFormat {
        font_id: FontId::new(size, FontFamily::Name(SEMIBOLD.into())),
        color: accent,
        ..Default::default()
    };
    let mut run = String::new();
    let mut run_hl = false;
    for (i, ch) in text.chars().enumerate() {
        let h = hl.binary_search(&(i as u32)).is_ok();
        if h != run_hl && !run.is_empty() {
            job.append(&run, 0.0, if run_hl { strong.clone() } else { normal.clone() });
            run.clear();
        }
        run_hl = h;
        run.push(ch);
    }
    if !run.is_empty() {
        job.append(&run, 0.0, if run_hl { strong } else { normal });
    }
    job
}

/// Rounded square with a tinted fill and a centered glyph.
pub fn icon_tile(ui: &Ui, rect: Rect, glyph: &str, tint: Color32, size: f32) {
    let painter = ui.painter();
    let [r, g, b, _] = tint.to_array();
    painter.rect_filled(rect, CornerRadius::same(7), Color32::from_rgba_unmultiplied(r, g, b, 34));
    painter.text(rect.center(), Align2::CENTER_CENTER, glyph, icons::font(size), tint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_splitting() {
        assert_eq!(split_keys("Ctrl+Shift+P"), vec![vec!["Ctrl", "Shift", "P"]]);
        assert_eq!(split_keys("G G"), vec![vec!["G"], vec!["G"]]);
        assert_eq!(split_keys("Ctrl++"), vec![vec!["Ctrl", "+"]]);
        assert_eq!(split_keys("Alt+Up"), vec![vec!["Alt".to_string(), "↑".to_string()]]);
        assert!(split_keys("").is_empty());
    }

    #[test]
    fn highlight_runs() {
        let job = highlighted("New folder", &[0, 4], 13.0, Color32::WHITE, Color32::RED);
        assert_eq!(job.text, "New folder");
        assert_eq!(job.sections.len(), 4);
        let plain = highlighted("abc", &[], 13.0, Color32::WHITE, Color32::RED);
        assert_eq!(plain.sections.len(), 1);
    }
}
