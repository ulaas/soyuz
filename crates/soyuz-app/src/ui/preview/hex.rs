//! Hex viewer: 16 bytes per row (offset · hex in two groups of 8 · ASCII), virtualized
//! rows over 64 KiB pages that load on demand from the preview worker — a 200 MB file
//! opens instantly because only the visible pages are ever read.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Rect, ScrollArea, Sense, Ui, pos2, vec2};

use crate::services::preview::{HEX_PAGE, HexInit, PreviewClient};
use crate::theme::Palette;

pub const BYTES_PER_ROW: u64 = 16;

/// Number of rows needed for `len` bytes (at least one).
pub fn row_count(len: u64) -> u64 {
    len.div_ceil(BYTES_PER_ROW).max(1)
}

/// Width of the offset column in hex digits (8, or more for files > 4 GiB).
pub fn offset_digits(len: u64) -> usize {
    let mut d = 8;
    while d < 16 && len > 1u64 << (d * 4) {
        d += 2;
    }
    d
}

/// Hex part of a row: `"48 65 6C 6C 6F 20 57 6F  72 6C 64 0A"`, padded to full width.
pub fn hex_part(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(16 * 3 + 1);
    for i in 0..BYTES_PER_ROW as usize {
        if i == 8 {
            s.push(' ');
        }
        match bytes.get(i) {
            Some(b) => s.push_str(&format!("{b:02X}")),
            None => s.push_str("  "),
        }
        if i + 1 < BYTES_PER_ROW as usize {
            s.push(' ');
        }
    }
    s
}

/// Printable ASCII with `·` for everything else.
pub fn ascii_part(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '·' }).collect()
}

/// One full row as text (used by tests and "copy row").
pub fn format_row(offset: u64, bytes: &[u8], digits: usize) -> String {
    format!("{offset:0digits$X}  {}  {}", hex_part(bytes), ascii_part(bytes))
}

/// Client-side state of an open hex document.
pub struct HexDoc {
    pub doc: u64,
    pub len: u64,
    path: Option<PathBuf>,
    pages: HashMap<u64, Arc<[u8]>>,
    requested: HashSet<u64>,
}

impl HexDoc {
    pub fn new(init: &HexInit) -> Self {
        HexDoc {
            doc: init.doc,
            len: init.len,
            path: init.path.clone(),
            pages: init.pages.clone(),
            requested: init.pages.keys().copied().collect(),
        }
    }

    pub fn on_page(&mut self, doc: u64, page: u64, bytes: Option<Arc<[u8]>>) {
        if doc == self.doc {
            self.pages.insert(page, bytes.unwrap_or_else(|| Arc::from(Vec::new())));
        }
    }

    /// Bytes of row `row`, requesting its page when missing.
    fn row(&mut self, row: u64, client: &PreviewClient) -> Option<Vec<u8>> {
        let start = row * BYTES_PER_ROW;
        let page = start / HEX_PAGE;
        let Some(data) = self.pages.get(&page) else {
            if let Some(p) = &self.path
                && self.requested.insert(page)
            {
                client.page(self.doc, p, page);
            }
            return None;
        };
        let off = (start - page * HEX_PAGE) as usize;
        let end = (off + BYTES_PER_ROW as usize).min(data.len());
        Some(data.get(off..end).map(<[u8]>::to_vec).unwrap_or_default())
    }

    /// Drop pages far from the viewport to bound memory on huge files.
    fn trim(&mut self, keep_from: u64, keep_to: u64) {
        if self.path.is_none() || self.pages.len() < 64 {
            return;
        }
        self.pages.retain(|&p, _| p + 8 >= keep_from && p <= keep_to + 8);
        let pages = &self.pages;
        self.requested.retain(|p| pages.contains_key(p));
    }
}

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
            let first_page = range.start as u64 * BYTES_PER_ROW / HEX_PAGE;
            let last_page = range.end as u64 * BYTES_PER_ROW / HEX_PAGE;
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
                    None => job.append("  ·· ·· ·· ·· ·· ·· ·· ··  ·· ·· ·· ·· ·· ·· ·· ··", 0.0, fmt(p.text_tertiary)),
                }
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(
                    pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0),
                    galley,
                    p.text_primary,
                );
            }
            doc.trim(first_page, last_page);
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_digits() {
        assert_eq!(row_count(0), 1);
        assert_eq!(row_count(16), 1);
        assert_eq!(row_count(17), 2);
        assert_eq!(offset_digits(1000), 8);
        assert_eq!(offset_digits(0xFFFF_FFFF), 8);
        assert_eq!(offset_digits(0x1_0000_0001), 10);
    }

    #[test]
    fn row_formatting() {
        let r = format_row(0x20, b"Hello, World!\n\x00\xFF", 8);
        assert_eq!(r, "00000020  48 65 6C 6C 6F 2C 20 57  6F 72 6C 64 21 0A 00 FF  Hello, World!···");
        // Short rows keep the ASCII column aligned.
        let short = format_row(0, b"ab", 8);
        assert_eq!(short.chars().count(), format_row(0, &[0u8; 16], 8).chars().count() - 14);
        assert!(hex_part(b"ab").starts_with("61 62 "));
        assert_eq!(hex_part(&[]).len(), hex_part(&[0; 16]).len());
    }

    #[test]
    fn doc_pages_resolve_rows() {
        let init = HexInit {
            doc: 1,
            len: 40,
            path: None,
            pages: HashMap::from([(0u64, Arc::<[u8]>::from((0u8..40).collect::<Vec<_>>()))]),
        };
        let mut d = HexDoc::new(&init);
        let ctx = egui::Context::default();
        let c = PreviewClient::new(&ctx);
        assert_eq!(d.row(0, &c).unwrap(), (0u8..16).collect::<Vec<_>>());
        assert_eq!(d.row(2, &c).unwrap(), (32u8..40).collect::<Vec<_>>());
        d.on_page(2, 0, None);
        assert_eq!(d.row(1, &c).unwrap().len(), 16, "pages of other docs are ignored");
    }
}
