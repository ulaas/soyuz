//! Toolkit-free terminal colors: [`TermColor`] (8-bit sRGBA, unmultiplied alpha) and
//! [`TermPalette`], the built-in dark / light schemes. The egui widget converts them to
//! `egui::Color32` (`TermTheme`); a text UI can map them to its own color type.

/// An 8-bit sRGB color with *unmultiplied* alpha (255 = opaque).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TermColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl TermColor {
    pub const BLACK: TermColor = TermColor::rgb(0, 0, 0);

    /// Opaque color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        TermColor { r, g, b, a: 255 }
    }

    /// Color with unmultiplied alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        TermColor { r, g, b, a }
    }

    /// Opaque color from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Self {
        TermColor::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// `[r, g, b, a]` (unmultiplied).
    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub const fn from_array([r, g, b, a]: [u8; 4]) -> Self {
        TermColor { r, g, b, a }
    }
}

impl From<[u8; 4]> for TermColor {
    fn from(c: [u8; 4]) -> Self {
        TermColor::from_array(c)
    }
}

impl From<TermColor> for [u8; 4] {
    fn from(c: TermColor) -> Self {
        c.to_array()
    }
}

/// The colors of a terminal scheme, without any toolkit types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermPalette {
    pub fg: TermColor,
    pub bg: TermColor,
    pub cursor: TermColor,
    pub selection: TermColor,
    /// ANSI 0-15 (normal 0-7, bright 8-15).
    pub ansi: [TermColor; 16],
}

const fn hex(rgb: u32) -> TermColor {
    TermColor::hex(rgb)
}

impl TermPalette {
    /// The built-in dark scheme (Orbit dark).
    pub const fn dark() -> Self {
        TermPalette {
            fg: hex(0xE6E8EC),
            bg: hex(0x0B0D11),
            cursor: hex(0x5B8CFF),
            selection: TermColor::rgba(91, 140, 255, 77),
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
        }
    }

    /// The built-in light scheme (Orbit light).
    pub const fn light() -> Self {
        TermPalette {
            fg: hex(0x171A20),
            bg: hex(0xE9EBEF),
            cursor: hex(0x2F6BFF),
            selection: TermColor::rgba(47, 107, 255, 56),
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
        }
    }

    /// The xterm 256-color palette: ANSI 0-15 from this scheme, then the fixed entries.
    pub fn palette256(&self) -> [TermColor; 256] {
        xterm256(&self.ansi)
    }
}

impl Default for TermPalette {
    fn default() -> Self {
        TermPalette::dark()
    }
}

/// The fixed part of the xterm palette for `index` in 16..=255: the 6×6×6 color cube
/// (16-231) and the 24-step gray ramp (232-255). `None` for 0-15 (theme colors).
pub const fn xterm_fixed(index: u8) -> Option<TermColor> {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => None,
        16..=231 => {
            let i = (index - 16) as usize;
            Some(TermColor::rgb(STEPS[i / 36], STEPS[(i / 6) % 6], STEPS[i % 6]))
        }
        _ => {
            let v = 8 + 10 * (index - 232);
            Some(TermColor::rgb(v, v, v))
        }
    }
}

/// The xterm 256-color palette: `ansi` for 0-15, then [`xterm_fixed`].
pub fn xterm256(ansi: &[TermColor; 16]) -> [TermColor; 256] {
    let mut p = [TermColor::BLACK; 256];
    p[..16].copy_from_slice(ansi);
    for i in 16..=255u8 {
        if let Some(c) = xterm_fixed(i) {
            p[usize::from(i)] = c;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_layout_without_a_toolkit() {
        let t = TermPalette::dark();
        let p = t.palette256();
        assert_eq!(p[1], t.ansi[1]);
        assert_eq!(p[16], TermColor::rgb(0, 0, 0));
        assert_eq!(p[21], TermColor::rgb(0, 0, 255));
        assert_eq!(p[196], TermColor::rgb(255, 0, 0));
        assert_eq!(p[231], TermColor::rgb(255, 255, 255));
        assert_eq!(p[232], TermColor::rgb(8, 8, 8));
        assert_eq!(p[255], TermColor::rgb(238, 238, 238));
        assert_eq!(TermColor::hex(0x5B8CFF).to_array(), [0x5B, 0x8C, 0xFF, 255]);
        assert_eq!(TermColor::from([1, 2, 3, 4]), TermColor::rgba(1, 2, 3, 4));
        assert_ne!(TermPalette::dark(), TermPalette::light());
        assert_eq!(TermPalette::default(), TermPalette::dark());
    }
}
