//! Icon helpers: Phosphor glyphs (Regular for UI, Fill for the active sidebar item).

use egui::{FontFamily, FontId, RichText};

pub use egui_phosphor::fill;
pub use egui_phosphor::regular as ph;

use super::fonts::{ICONS, PHOSPHOR_FILL};

/// A regular-weight icon at `size` points.
pub fn icon(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).font(FontId::new(size, FontFamily::Name(ICONS.into())))
}

/// A filled icon (the `phosphor-fill` family) at `size` points.
pub fn icon_fill(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).font(FontId::new(size, FontFamily::Name(PHOSPHOR_FILL.into())))
}

/// Font id for painting a regular icon directly with `Painter::text`.
pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// Font id for painting a filled icon directly with `Painter::text`.
pub fn font_fill(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(PHOSPHOR_FILL.into()))
}
