//! Draws the image editor ([`crate::editors::image::ImageDoc`]): toolbar, the zoom /
//! pan / crop view over a checkerboard, and the status bar.

use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use super::widgets;
use crate::editors::image::ImageDoc;
use crate::editors::image::ops::{NormRect, SaveMode};
use crate::theme::{Palette, icons::ph};
use crate::ui::gfx::TexExt;

const TOOLBAR_H: f32 = 34.0;
const STATUS_H: f32 = 24.0;

#[derive(Default)]
pub struct ImageOutput {
    pub save: bool,
}

pub fn show(
    doc: &mut ImageDoc,
    ui: &mut Ui,
    rect: Rect,
    p: &Palette,
    jpeg_quality: u8,
    focus: &mut Vec<Id>,
    id: Id,
) -> ImageOutput {
    doc.pump();
    let mut out = ImageOutput::default();
    let tb = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_H));
    toolbar(doc, ui, tb, p, id, &mut out);
    let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STATUS_H), rect.max);
    let mut top = tb.bottom();
    if doc.is_dirty() && matches!(doc.save_mode(), SaveMode::Lossy(_) | SaveMode::Unsupported) {
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
        let text = match doc.save_mode() {
            SaveMode::Lossy(_) => tr!("editors-image-lossy-warning", quality = jpeg_quality.to_string()),
            _ => tr!("editors-image-unsupported-warning").to_string(),
        };
        widgets::banner(ui, r, p, ph::WARNING, p.warning, &text, &[]);
        top += 36.0;
    }
    let area = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), status.top()));
    view(doc, ui, area, p, focus, id);
    self::status(doc, ui, status, p);
    out
}

fn toolbar(doc: &mut ImageDoc, ui: &mut Ui, r: Rect, p: &Palette, id: Id, out: &mut ImageOutput) {
    let mut x = r.left() + 6.0;
    let can_save = doc.is_dirty() && doc.save_mode() != SaveMode::Unsupported;
    let (can_undo, can_redo) = (doc.can_undo(), doc.can_redo());
    let crop_mode = doc.crop_mode;
    let fit = doc.zoom.is_none();
    let mut clicked: Vec<&'static str> = Vec::new();
    let items: [(&'static str, &'static str, &'static str, bool, bool); 12] = [
        ("save", ph::FLOPPY_DISK, tr!("editors-tb-save"), can_save, false),
        ("undo", ph::ARROW_COUNTER_CLOCKWISE, tr!("editors-tb-undo"), can_undo, false),
        ("redo", ph::ARROW_CLOCKWISE, tr!("editors-tb-redo"), can_redo, false),
        ("fit", ph::ARROWS_IN, tr!("editors-image-fit"), true, fit),
        ("100", ph::MAGNIFYING_GLASS, tr!("editors-image-actual-size"), true, false),
        ("out", ph::MAGNIFYING_GLASS_MINUS, tr!("editors-image-zoom-out"), true, false),
        ("in", ph::MAGNIFYING_GLASS_PLUS, tr!("editors-image-zoom-in"), true, false),
        ("rl", ph::ARROW_COUNTER_CLOCKWISE, tr!("editors-image-rotate-left"), true, false),
        ("rr", ph::ARROW_CLOCKWISE, tr!("editors-image-rotate-right"), true, false),
        ("fh", ph::FLIP_HORIZONTAL, tr!("editors-image-flip-h"), true, false),
        ("fv", ph::FLIP_VERTICAL, tr!("editors-image-flip-v"), true, false),
        ("crop", ph::CROP, tr!("editors-image-crop"), true, crop_mode),
    ];
    for (i, (key, glyph, tip, enabled, on)) in items.into_iter().enumerate() {
        if matches!(i, 3 | 7 | 11) {
            x += 8.0;
        }
        let b = Rect::from_min_size(pos2(x, r.top() + 4.0), vec2(26.0, 26.0));
        x += 28.0;
        if on {
            ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
        }
        if widgets::icon_at(ui, b, id.with(("im", key)), glyph, tip, enabled, p) {
            clicked.push(key);
        }
    }
    if doc.crop.is_some() {
        for (label, key) in [(tr!("editors-image-apply-crop"), "apply"), (tr!("common-cancel"), "cancel")] {
            let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), p.text_primary);
            let b = Rect::from_min_size(pos2(x + 4.0, r.top() + 5.0), vec2(g.size().x + 18.0, 24.0));
            x = b.right() + 2.0;
            let resp = ui.interact(b, id.with(("imb", key)), Sense::click());
            crate::ui::a11y::button(&resp, label);
            let primary = key == "apply";
            let fill = match (primary, resp.hovered()) {
                (true, true) => p.accent_hover,
                (true, false) => p.accent,
                (false, true) => p.bg_surface3,
                (false, false) => p.bg_surface2,
            };
            ui.painter().rect_filled(b, CornerRadius::same(4), fill);
            let fg = if primary { Color32::WHITE } else { p.text_primary };
            ui.painter().galley(pos2(b.left() + 9.0, b.center().y - g.size().y / 2.0), g, fg);
            if resp.clicked() {
                clicked.push(key);
            }
        }
    }
    out.save |= clicked.contains(&"save");
    for k in clicked {
        doc.command(k);
    }
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
}

fn status(doc: &ImageDoc, ui: &mut Ui, r: Rect, p: &Palette) {
    ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border_subtle));
    let (w, h) = doc.size();
    let fmt = doc.format.map(|f| format!("{f:?}").to_uppercase()).unwrap_or_else(|| tr!("editors-image-kind").into());
    let mut s = tr!(
        "editors-image-status",
        width = w.to_string(),
        height = h.to_string(),
        format = fmt,
        zoom = format!("{:.0}", doc.last_scale * 100.0)
    );
    if let Some(c) = doc.crop {
        let (_, _, cw, ch) = c.to_pixels(w, h);
        s.push_str("  ·  ");
        s.push_str(&tr!("editors-image-status-crop", width = cw.to_string(), height = ch.to_string()));
    }
    if !doc.ops.is_empty() {
        s.push_str("  ·  ");
        s.push_str(&tr!("editors-image-status-edits", count = doc.ops.len()));
    }
    ui.painter().text(
        pos2(r.left() + 10.0, r.center().y),
        Align2::LEFT_CENTER,
        s,
        FontId::proportional(11.5),
        p.text_secondary,
    );
    let mode = match doc.save_mode() {
        SaveMode::Lossless(_) => tr!("editors-image-mode-lossless"),
        SaveMode::Lossy(_) => tr!("editors-image-mode-lossy"),
        SaveMode::Unsupported => tr!("editors-image-mode-readonly"),
    };
    ui.painter().text(
        pos2(r.right() - 10.0, r.center().y),
        Align2::RIGHT_CENTER,
        mode,
        FontId::proportional(11.5),
        p.text_tertiary,
    );
}

fn view(doc: &mut ImageDoc, ui: &mut Ui, area: Rect, p: &Palette, focus: &mut Vec<Id>, id: Id) {
    let vid = id.with("image.view");
    focus.push(vid);
    let resp = ui.interact(area, vid, Sense::click_and_drag());
    let just_focused = std::mem::take(&mut doc.focus_request) && !resp.has_focus();
    if just_focused {
        resp.request_focus();
    }
    let (w, h) = doc.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let fit = ((area.width() - 24.0) / w).min((area.height() - 24.0) / h).clamp(0.01, 1.0);
    let mut scale = doc.zoom.unwrap_or(fit);
    // Keys.
    if resp.has_focus() && !just_focused {
        let keys: Vec<_> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|ev| match ev {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => {
                        Some((crate::ui::keys::key_name(*key), super::key_mods(*modifiers)))
                    }
                    _ => None,
                })
                .collect()
        });
        for (key, m) in keys {
            scale = doc.key(key, m, scale, fit);
        }
    }
    let pan = |doc: &ImageDoc| Vec2::from(doc.pan);
    // Wheel zoom around the pointer.
    if resp.hovered() {
        let (d, ptr) = ui.input_mut(|i| (std::mem::take(&mut i.smooth_scroll_delta), i.pointer.hover_pos()));
        if d.y != 0.0 {
            let factor = (d.y / 200.0).exp();
            let new = (scale * factor).clamp(0.02, 32.0);
            if let Some(ptr) = ptr {
                let c = area.center() + pan(doc);
                let rel = ptr - c;
                doc.pan = (pan(doc) + rel - rel * (new / scale)).into();
            }
            scale = new;
            doc.zoom = Some(new);
        }
    }
    doc.last_scale = scale;
    let size = vec2(w * scale, h * scale);
    let mut center = area.center() + pan(doc);
    // Pan (drag) or crop.
    let img_rect = Rect::from_center_size(center, size);
    let to_norm = |pt: egui::Pos2| ((pt.x - img_rect.left()) / size.x, (pt.y - img_rect.top()) / size.y);
    if doc.crop_mode {
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        if resp.drag_started()
            && let Some(pt) = resp.interact_pointer_pos()
        {
            doc.crop_drag = Some(pt.into());
        }
        if resp.dragged()
            && let (Some(a), Some(b)) = (doc.crop_drag, resp.interact_pointer_pos())
        {
            let (x0, y0) = to_norm(a.into());
            let (x1, y1) = to_norm(b);
            doc.crop = Some(NormRect { x0, y0, x1, y1 }.normalized());
        }
        if !resp.dragged() {
            doc.crop_drag = None;
        }
    } else if resp.dragged() {
        doc.pan = (pan(doc) + resp.drag_delta()).into();
        center = area.center() + pan(doc);
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if resp.hovered() && (size.x > area.width() || size.y > area.height()) {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if resp.double_clicked() && !doc.crop_mode {
        doc.toggle_fit();
    }
    let img_rect = Rect::from_center_size(center, size);
    // Paint.
    let painter = ui.painter_at(area);
    painter.rect_filled(area, CornerRadius::ZERO, p.bg_sunken.gamma_multiply(if p.dark { 0.55 } else { 0.35 }));
    let vis = img_rect.intersect(area);
    if vis.is_positive() {
        // Checkerboard under transparent pixels (visible part only).
        let cell = 12.0;
        let (c1, c2) = if p.dark {
            (Color32::from_gray(52), Color32::from_gray(64))
        } else {
            (Color32::from_gray(236), Color32::from_gray(250))
        };
        painter.rect_filled(vis, CornerRadius::ZERO, c1);
        let x0 = ((vis.left() - img_rect.left()) / cell).floor() as i64;
        let y0 = ((vis.top() - img_rect.top()) / cell).floor() as i64;
        let x1 = ((vis.right() - img_rect.left()) / cell).ceil() as i64;
        let y1 = ((vis.bottom() - img_rect.top()) / cell).ceil() as i64;
        if (x1 - x0) * (y1 - y0) < 20_000 {
            for gy in y0..y1 {
                for gx in x0..x1 {
                    if (gx + gy) % 2 == 0 {
                        let r = Rect::from_min_size(
                            pos2(img_rect.left() + gx as f32 * cell, img_rect.top() + gy as f32 * cell),
                            vec2(cell, cell),
                        )
                        .intersect(vis);
                        if r.is_positive() {
                            painter.rect_filled(r, CornerRadius::ZERO, c2);
                        }
                    }
                }
            }
        }
    }
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match doc.texture() {
        Some((t, true)) => {
            painter.image(t.id(), img_rect, uv, Color32::WHITE);
        }
        Some((t, false)) => {
            // The edited copy is on its way: keep the previous one, dimmed.
            painter.image(t.id(), img_rect, uv, Color32::from_gray(160));
            crate::ui::widgets::spinner(ui, area.center(), 10.0, p.accent);
        }
        None => crate::ui::widgets::spinner(ui, area.center(), 12.0, p.accent),
    }
    painter.rect_stroke(img_rect, CornerRadius::ZERO, Stroke::new(1.0, p.border_subtle), egui::StrokeKind::Outside);
    if let Some(c) = doc.crop {
        let r = Rect::from_min_max(
            pos2(img_rect.left() + c.x0 * size.x, img_rect.top() + c.y0 * size.y),
            pos2(img_rect.left() + c.x1 * size.x, img_rect.top() + c.y1 * size.y),
        );
        let shade = Color32::from_black_alpha(140);
        for s in [
            Rect::from_min_max(img_rect.min, pos2(img_rect.right(), r.top())),
            Rect::from_min_max(pos2(img_rect.left(), r.bottom()), img_rect.max),
            Rect::from_min_max(pos2(img_rect.left(), r.top()), pos2(r.left(), r.bottom())),
            Rect::from_min_max(pos2(r.right(), r.top()), pos2(img_rect.right(), r.bottom())),
        ] {
            painter.rect_filled(s, CornerRadius::ZERO, shade);
        }
        painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, p.accent), egui::StrokeKind::Outside);
        let (iw, ih) = doc.size();
        let (_, _, cw, ch) = c.to_pixels(iw, ih);
        painter.text(
            pos2(r.left() + 6.0, r.top() + 6.0),
            Align2::LEFT_TOP,
            format!("{cw} × {ch}"),
            FontId::proportional(11.5),
            Color32::WHITE,
        );
    } else if doc.crop_mode {
        painter.text(
            pos2(area.center().x, area.top() + 16.0),
            Align2::CENTER_CENTER,
            tr!("editors-image-crop-hint"),
            FontId::proportional(12.0),
            p.text_secondary,
        );
    }
}
