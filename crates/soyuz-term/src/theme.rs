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
