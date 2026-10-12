//! Toolkit-free terminal colors: [`TermColor`] (8-bit sRGBA, unmultiplied alpha) and
//! [`TermPalette`], the built-in dark / light schemes. The egui widget converts them to
//! `egui::Color32` (`TermTheme`); a text UI can map them to its own color type.
//! Also the cell colour resolution ([`CellStyle`]) shared by the egui widget and
//! [`crate::grid`].

use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// An 8-bit sRGB color with *unmultiplied* alpha (255 = opaque). The default is
/// transparent black.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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

/// The resolved paint style of one terminal cell: final foreground / background (bold
/// brightening, dim, inverse and hidden applied) and the text decorations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellStyle {
    pub fg: TermColor,
    pub bg: TermColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// Opaque blend `a + (b - a) * t` of the RGB channels (alpha ignored, result opaque).
pub fn lerp_rgb(a: TermColor, b: TermColor, t: f32) -> TermColor {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round().clamp(0.0, 255.0) as u8;
    TermColor::rgb(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b))
}

/// SGR 2 (dim): `c` blended toward the background.
pub fn dim(bg: TermColor, c: TermColor) -> TermColor {
    lerp_rgb(bg, c, 0.66)
}

fn from_rgb(c: Rgb) -> TermColor {
    TermColor::rgb(c.r, c.g, c.b)
}

/// Turns alacritty cell colours into [`TermColor`]s, shared by the egui widget and
/// [`crate::grid`]. Channels are used as given: the egui widget feeds premultiplied
/// `Color32` bytes so it keeps drawing exactly what it always drew; blends (dim) work on
/// the raw RGB channels and are opaque.
pub(crate) struct Resolver<'a> {
    pal: &'a TermPalette,
    p256: [TermColor; 256],
    colors: &'a Colors,
    bright_bold: bool,
}

impl<'a> Resolver<'a> {
    /// `colors` are the program's OSC 4 / 10 / 11 / 12 overrides; `bright_bold` draws bold
    /// text in ANSI 0-7 with the bright variant.
    pub(crate) fn new(pal: &'a TermPalette, colors: &'a Colors, bright_bold: bool) -> Self {
        Resolver { pal, p256: pal.palette256(), colors, bright_bold }
    }

    /// The background of cells without an explicit colour.
    pub(crate) fn default_bg(&self) -> TermColor {
        self.colors[NamedColor::Background].map_or(self.pal.bg, from_rgb)
    }

    /// The cursor colour (OSC 12 override, else the palette's).
    pub(crate) fn cursor(&self) -> TermColor {
        self.colors[NamedColor::Cursor].map_or(self.pal.cursor, from_rgb)
    }

    pub(crate) fn color(&self, c: Color) -> TermColor {
        match c {
            Color::Spec(rgb) => from_rgb(rgb),
            Color::Indexed(i) => self.colors[usize::from(i)].map_or(self.p256[usize::from(i)], from_rgb),
            Color::Named(n) => {
                if let Some(rgb) = self.colors[n] {
                    return from_rgb(rgb);
                }
                let idx = n as usize;
                if idx < 16 {
                    return self.pal.ansi[idx];
                }
                match n {
                    NamedColor::Foreground | NamedColor::BrightForeground => self.pal.fg,
                    NamedColor::Background => self.pal.bg,
                    NamedColor::Cursor => self.pal.cursor,
                    NamedColor::DimForeground => dim(self.pal.bg, self.pal.fg),
                    d => {
                        let base = (d as usize).saturating_sub(NamedColor::DimBlack as usize).min(7);
                        dim(self.pal.bg, self.pal.ansi[base])
                    }
                }
            }
        }
    }

    pub(crate) fn style(&self, cell: &Cell) -> CellStyle {
        resolve(cell, self)
    }
}

/// The paint style of `cell`: bold brightens ANSI 0-7 (when enabled), dim blends the
/// foreground toward the background, inverse swaps them, hidden paints the foreground in
/// the background colour.
pub(crate) fn resolve(cell: &Cell, r: &Resolver<'_>) -> CellStyle {
    let f = cell.flags;
    let bold = f.contains(Flags::BOLD);
    let mut fg_spec = cell.fg;
    if bold && r.bright_bold {
        fg_spec = match fg_spec {
            Color::Named(n) if (n as usize) < 8 => Color::Named(n.to_bright()),
            Color::Indexed(i) if i < 8 => Color::Indexed(i + 8),
            other => other,
        };
    }
    let mut fg = r.color(fg_spec);
    let mut bg = r.color(cell.bg);
    if f.contains(Flags::DIM) {
        fg = lerp_rgb(bg, fg, 0.66);
    }
    if f.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if f.contains(Flags::HIDDEN) {
        fg = bg;
    }
    CellStyle {
        fg,
        bg,
        bold,
        italic: f.contains(Flags::ITALIC),
        underline: f.intersects(Flags::ALL_UNDERLINES),
        strike: f.contains(Flags::STRIKEOUT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_without_a_toolkit() {
        let pal = TermPalette::dark();
        let colors = Colors::default();
        let r = Resolver::new(&pal, &colors, true);
        let cell = |fg, bg, flags| Cell { fg, bg, flags, ..Cell::default() };
        let plain =
            r.style(&cell(Color::Named(NamedColor::Foreground), Color::Named(NamedColor::Background), Flags::empty()));
        assert_eq!((plain.fg, plain.bg), (pal.fg, pal.bg));
        // Bold red is bright red; without bright_bold it stays red.
        let bold = cell(Color::Named(NamedColor::Red), Color::Named(NamedColor::Background), Flags::BOLD);
        assert_eq!(r.style(&bold).fg, pal.ansi[9]);
        assert!(r.style(&bold).bold);
        assert_eq!(Resolver::new(&pal, &colors, false).style(&bold).fg, pal.ansi[1]);
        assert_eq!(r.style(&cell(Color::Indexed(2), Color::Indexed(0), Flags::BOLD)).fg, pal.ansi[10]);
        // Inverse swaps; hidden paints fg = bg; dim blends toward the background.
        let inv = r.style(&cell(Color::Indexed(1), Color::Indexed(4), Flags::INVERSE));
        assert_eq!((inv.fg, inv.bg), (pal.ansi[4], pal.ansi[1]));
        let hid = r.style(&cell(Color::Indexed(1), Color::Indexed(4), Flags::HIDDEN));
        assert_eq!(hid.fg, pal.ansi[4]);
        let dimmed = r.style(&cell(Color::Indexed(15), Color::Indexed(0), Flags::DIM)).fg;
        assert_eq!(dimmed, lerp_rgb(pal.ansi[0], pal.ansi[15], 0.66));
        assert_eq!(r.color(Color::Named(NamedColor::DimRed)), dim(pal.bg, pal.ansi[1]));
        assert_eq!(r.color(Color::Indexed(196)), TermColor::rgb(255, 0, 0));
        assert_eq!(r.color(Color::Spec(Rgb { r: 1, g: 2, b: 3 })), TermColor::rgb(1, 2, 3));
        let deco =
            r.style(&cell(Color::Indexed(7), Color::Indexed(0), Flags::ITALIC | Flags::UNDERCURL | Flags::STRIKEOUT));
        assert!(deco.italic && deco.underline && deco.strike && !deco.bold);
        // Program overrides (OSC 4 / 11 / 12) win.
        let mut colors = Colors::default();
        colors[1] = Some(Rgb { r: 9, g: 8, b: 7 });
        colors[NamedColor::Background] = Some(Rgb { r: 1, g: 1, b: 1 });
        colors[NamedColor::Cursor] = Some(Rgb { r: 2, g: 2, b: 2 });
        let r = Resolver::new(&pal, &colors, true);
        assert_eq!(r.color(Color::Indexed(1)), TermColor::rgb(9, 8, 7));
        assert_eq!(r.default_bg(), TermColor::rgb(1, 1, 1));
        assert_eq!(r.cursor(), TermColor::rgb(2, 2, 2));
        assert_eq!(lerp_rgb(TermColor::BLACK, TermColor::rgb(255, 255, 255), 0.5), TermColor::rgb(128, 128, 128));
    }

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
