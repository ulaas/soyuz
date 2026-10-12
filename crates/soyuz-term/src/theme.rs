//! Terminal color themes for the egui widget (derived from the app's Orbit palette).
//! The toolkit-free colors live in [`crate::color`]; this converts them to `Color32`.

use egui::Color32;

#[cfg(test)]
use crate::color::xterm_fixed;
use crate::color::{TermColor, TermPalette};

#[derive(Clone, Debug, PartialEq)]
pub struct TermTheme {
    pub fg: Color32,
    pub bg: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    /// ANSI 0-15 (normal 0-7, bright 8-15).
    pub ansi: [Color32; 16],
    /// Render bold text with the bright palette variant.
    pub bright_bold: bool,
}

/// Neutral color → egui (unmultiplied alpha in, premultiplied `Color32` out).
pub fn to_color32(c: TermColor) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// egui → neutral color (unmultiplied alpha).
pub fn from_color32(c: Color32) -> TermColor {
    TermColor::from_array(c.to_srgba_unmultiplied())
}

impl From<&TermPalette> for TermTheme {
    fn from(p: &TermPalette) -> Self {
        TermTheme {
            fg: to_color32(p.fg),
            bg: to_color32(p.bg),
            cursor: to_color32(p.cursor),
            selection: to_color32(p.selection),
            ansi: p.ansi.map(to_color32),
            bright_bold: true,
        }
    }
}

impl From<&TermTheme> for TermPalette {
    fn from(t: &TermTheme) -> Self {
        TermPalette {
            fg: from_color32(t.fg),
            bg: from_color32(t.bg),
            cursor: from_color32(t.cursor),
            selection: from_color32(t.selection),
            ansi: t.ansi.map(from_color32),
        }
    }
}

impl TermTheme {
    pub fn dark() -> Self {
        TermTheme::from(&TermPalette::dark())
    }

    pub fn light() -> Self {
        TermTheme::from(&TermPalette::light())
    }
}

impl Default for TermTheme {
    fn default() -> Self {
        TermTheme::dark()
    }
}

#[cfg(test)]
impl TermTheme {
    /// The xterm 256-color palette: ANSI 0-15 from the theme, 6×6×6 cube, 24-step gray ramp.
    fn palette256(&self) -> [Color32; 256] {
        let mut p = [Color32::BLACK; 256];
        p[..16].copy_from_slice(&self.ansi);
        for i in 16..=255u8 {
            if let Some(c) = xterm_fixed(i) {
                p[usize::from(i)] = to_color32(c);
            }
        }
        p
    }
}

/// Opaque RGB blend `a + (b - a) * t`.
#[cfg(test)]
fn lerp_rgb(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_layout() {
        let t = TermTheme::dark();
        let p = t.palette256();
        assert_eq!(p[1], t.ansi[1]);
        assert_eq!(p[16], Color32::from_rgb(0, 0, 0));
        assert_eq!(p[21], Color32::from_rgb(0, 0, 255));
        assert_eq!(p[196], Color32::from_rgb(255, 0, 0));
        assert_eq!(p[231], Color32::from_rgb(255, 255, 255));
        assert_eq!(p[232], Color32::from_rgb(8, 8, 8));
        assert_eq!(p[255], Color32::from_rgb(238, 238, 238));
        assert_eq!(lerp_rgb(Color32::BLACK, Color32::WHITE, 0.5), Color32::from_rgb(128, 128, 128));
    }

    /// The neutral palettes convert to exactly the `Color32` values the themes always had.
    #[test]
    fn builtin_themes_are_bit_identical() {
        let d = TermTheme::dark();
        assert_eq!(d.fg, Color32::from_rgb(0xE6, 0xE8, 0xEC));
        assert_eq!(d.bg, Color32::from_rgb(0x0B, 0x0D, 0x11));
        assert_eq!(d.cursor, Color32::from_rgb(0x5B, 0x8C, 0xFF));
        assert_eq!(d.selection, Color32::from_rgba_unmultiplied(91, 140, 255, 77));
        assert_eq!(d.ansi[0], Color32::from_rgb(0x1C, 0x20, 0x27));
        assert_eq!(d.ansi[15], Color32::from_rgb(0xE6, 0xE8, 0xEC));
        assert!(d.bright_bold);
        let l = TermTheme::light();
        assert_eq!(l.fg, Color32::from_rgb(0x17, 0x1A, 0x20));
        assert_eq!(l.bg, Color32::from_rgb(0xE9, 0xEB, 0xEF));
        assert_eq!(l.selection, Color32::from_rgba_unmultiplied(47, 107, 255, 56));
        assert_eq!(l.ansi[13], Color32::from_rgb(0x9D, 0x66, 0xE6));
        // Opaque colors round-trip exactly; the 256-color tables agree.
        assert_eq!(TermPalette::from(&d).ansi, TermPalette::dark().ansi);
        let neutral = TermPalette::dark().palette256().map(to_color32);
        assert_eq!(d.palette256(), neutral);
    }
}
