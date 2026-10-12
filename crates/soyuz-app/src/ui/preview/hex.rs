//! Hex viewer: 16 bytes per row (offset · hex in two groups of 8 · ASCII), virtualized
//! rows over 64 KiB pages that load on demand from the preview worker — a 200 MB file
//! opens instantly because only the visible pages are ever read.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Rect, ScrollArea, Sense, Ui, pos2, vec2};
pub use soyuz_appcore::models::preview_hex::{
    BYTES_PER_ROW, HexDoc, PENDING_ROW, ascii_part, format_row, hex_part, offset_digits, row_count,
};

use crate::services::preview::PreviewClient;
use crate::theme::Palette;

/// Draw the hex view filling `rect`.
pub fn show(ui: &mut Ui, rect: Rect, doc: &mut HexDoc, client: &PreviewClient, p: &Palette, font_size: f32) {
    // Shrink the font so offset + hex + ASCII fit the pane (down to 8 px).
    let hex_family = crate::theme::fonts::family(soyuz_core::config::FontRoleKind::Hex);
    let font_size = (font_size + crate::theme::fonts::offset(ui.ctx(), soyuz_core::config::FontRoleKind::Hex)).max(6.0);
    let probe = FontId::new(font_size, hex_family.clone());
    let probe_w = ui.ctx().fonts_mut(|f| f.glyph_width(&probe, '0')).max(1.0);
    let cols = (offset_digits(doc.len) + 2 + 49 + 2 + 16) as f32;
    let fit = ((rect.width() - 30.0) / cols) / probe_w * font_size;
    let font_size = fit.clamp(8.0, font_size);
    let font = FontId::new(font_size, hex_family);
    let row_h = (font_size * 1.45).round();
    let digits = offset_digits(doc.len);
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    let width = char_w * (digits + 2 + 49 + 2 + 16) as f32 + 24.0;
    let rows = row_count(doc.len);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect);
    let painter_bg = p.bg_sunken;
    child.painter().rect_filled(rect, 0.0, painter_bg);
    ScrollArea::both().id_salt(("hex", doc.doc)).auto_shrink([false, false]).show_rows(
        &mut child,
        row_h,
        rows.min(usize::MAX as u64) as usize,
        |ui, range| {
            let (first_row, last_row) = (range.start as u64, range.end as u64);
            for row in range {
                let (r, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), row_h), Sense::hover());
                if row % 2 == 1 {
                    ui.painter().rect_filled(r, 0.0, p.bg_surface1.gamma_multiply(0.35));
                }
                let offset = row as u64 * BYTES_PER_ROW;
                let mut job = LayoutJob::default();
                let fmt = |c: Color32| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
                job.append(&format!("{offset:0digits$X}"), 0.0, fmt(p.text_tertiary));
                match doc.row(row as u64, client) {
                    Some(bytes) => {
                        job.append(&format!("  {}", hex_part(&bytes)), 0.0, fmt(p.text_primary));
                        job.append("  ", 0.0, fmt(p.text_tertiary));
                        for &b in &bytes {
                            let printable = (0x20..0x7F).contains(&b);
                            let s = if printable { (b as char).to_string() } else { "·".into() };
                            job.append(&s, 0.0, fmt(if printable { p.text_secondary } else { p.text_tertiary }));
                        }
                    }
                    None => job.append(&format!("  {PENDING_ROW}"), 0.0, fmt(p.text_tertiary)),
                }
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(
                    pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0),
                    galley,
                    p.text_primary,
                );
            }
            doc.trim_rows(first_row, last_row);
        },
    );
}
