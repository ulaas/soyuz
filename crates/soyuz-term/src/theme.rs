//! Terminal color themes (derived from the app's Orbit palette).

use egui::Color32;

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

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl TermTheme {
    pub fn dark() -> Self {
        TermTheme {
            fg: hex(0xE6E8EC),
            bg: hex(0x0B0D11),
            cursor: hex(0x5B8CFF),
            selection: Color32::from_rgba_unmultiplied(91, 140, 255, 77),
            ansi: [
                hex(0x1C2027),
                hex(0xF2555A),
                hex(0x3DD68C),
                hex(0xF5B544),
                hex(0x5B8CFF),
                hex(0xBB86FC),
                hex(0x4CC2FF),
                hex(0xA3A9B7),
                hex(0x6B7280),
                hex(0xFF7A7F),
                hex(0x6BE3A8),
                hex(0xF8CB72),
                hex(0x7BA3FF),
                hex(0xCFA6FD),
                hex(0x7DD3FF),
                hex(0xE6E8EC),
            ],
            bright_bold: true,
        }
    }

    pub fn light() -> Self {
        TermTheme {
            fg: hex(0x171A20),
            bg: hex(0xE9EBEF),
            cursor: hex(0x2F6BFF),
            selection: Color32::from_rgba_unmultiplied(47, 107, 255, 56),
            ansi: [
                hex(0x171A20),
                hex(0xD93A40),
                hex(0x1E9E5F),
                hex(0xD08A14),
                hex(0x2F6BFF),
                hex(0x8A4FD8),
                hex(0x1C8ED6),
                hex(0x4B5260),
                hex(0x8A91A0),
                hex(0xE5585D),
                hex(0x27B66F),
                hex(0xE39C22),
                hex(0x1F5BEF),
                hex(0x9D66E6),
                hex(0x2AA0EA),
                hex(0x171A20),
            ],
            bright_bold: true,
        }
    }
}

impl Default for TermTheme {
    fn default() -> Self {
        TermTheme::dark()
    }
}

impl TermTheme {
    /// The xterm 256-color palette: ANSI 0-15 from the theme, 6×6×6 cube, 24-step gray ramp.
    pub(crate) fn palette256(&self) -> [Color32; 256] {
        let mut p = [Color32::BLACK; 256];
        p[..16].copy_from_slice(&self.ansi);
        const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        for i in 0..216 {
            p[16 + i] = Color32::from_rgb(STEPS[i / 36], STEPS[(i / 6) % 6], STEPS[i % 6]);
        }
        for i in 0..24u8 {
            let v = 8 + 10 * i;
            p[232 + usize::from(i)] = Color32::from_rgb(v, v, v);
        }
        p
    }

    /// Dimmed variant of `c` (SGR 2): blended toward the background.
    pub(crate) fn dim(&self, c: Color32) -> Color32 {
        lerp_rgb(self.bg, c, 0.66)
    }
}

/// Opaque RGB blend `a + (b - a) * t`.
pub(crate) fn lerp_rgb(a: Color32, b: Color32, t: f32) -> Color32 {
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
}
