//! Snapshot of the GUI's cell colour resolution (`Palette::{color, style}` in
//! `render.rs`): every indexed / named / spec colour, with bold (bright), dim, inverse,
//! hidden and the decoration flags, over the built-in themes (incl. a translucent
//! background and `bright_bold` off) and with OSC 4 / 10 / 11 overrides. The digest was
//! recorded from the resolver before it moved to `color::resolve`, so the egui widget
//! keeps drawing bit-identical colours.

use super::*;

pub(super) fn named_all() -> Vec<NamedColor> {
    use NamedColor::*;
    vec![
        Black,
        Red,
        Green,
        Yellow,
        Blue,
        Magenta,
        Cyan,
        White,
        BrightBlack,
        BrightRed,
        BrightGreen,
        BrightYellow,
        BrightBlue,
        BrightMagenta,
        BrightCyan,
        BrightWhite,
        Foreground,
        Background,
        Cursor,
        DimBlack,
        DimRed,
        DimGreen,
        DimYellow,
        DimBlue,
        DimMagenta,
        DimCyan,
        DimWhite,
        BrightForeground,
        DimForeground,
    ]
}

pub(super) fn fg_specs() -> Vec<Color> {
    let mut v: Vec<Color> = (0..=255u8).map(Color::Indexed).collect();
    v.extend(named_all().into_iter().map(Color::Named));
    v.push(Color::Spec(Rgb { r: 1, g: 2, b: 3 }));
    v.push(Color::Spec(Rgb { r: 250, g: 128, b: 7 }));
    v
}

pub(super) fn bg_specs() -> Vec<Color> {
    vec![
        Color::Named(NamedColor::Background),
        Color::Named(NamedColor::Foreground),
        Color::Named(NamedColor::DimRed),
        Color::Named(NamedColor::Blue),
        Color::Indexed(0),
        Color::Indexed(9),
        Color::Indexed(200),
        Color::Spec(Rgb { r: 30, g: 60, b: 90 }),
    ]
}

pub(super) fn flag_sets() -> Vec<Flags> {
    let base = [Flags::BOLD, Flags::DIM, Flags::INVERSE, Flags::HIDDEN];
    let extra = [Flags::empty(), Flags::ITALIC | Flags::UNDERLINE | Flags::STRIKEOUT, Flags::UNDERCURL];
    let mut out = Vec::new();
    for bits in 0..16u32 {
        let mut f = Flags::empty();
        for (i, b) in base.iter().enumerate() {
            if bits & (1 << i) != 0 {
                f |= *b;
            }
        }
        for e in extra {
            out.push(f | e);
        }
    }
    out
}

pub(super) fn themes() -> Vec<TermTheme> {
    let mut out = Vec::new();
    for mut t in [TermTheme::dark(), TermTheme::light()] {
        out.push(t.clone());
        t.bright_bold = false;
        out.push(t);
    }
    // The GUI's translucent backdrop (`Palette::translucent`): a premultiplied background.
    let mut t = TermTheme::dark();
    t.bg = t.bg.gamma_multiply(0.62);
    t.ansi[3] = t.ansi[3].gamma_multiply(0.5);
    out.push(t);
    out
}

pub(super) fn color_sets() -> Vec<Colors> {
    let mut c = Colors::default();
    c[NamedColor::Foreground] = Some(Rgb { r: 10, g: 20, b: 30 });
    c[NamedColor::Background] = Some(Rgb { r: 40, g: 50, b: 60 });
    c[NamedColor::Cursor] = Some(Rgb { r: 70, g: 80, b: 90 });
    c[1] = Some(Rgb { r: 200, g: 1, b: 2 });
    c[9] = Some(Rgb { r: 201, g: 3, b: 4 });
    c[100] = Some(Rgb { r: 5, g: 6, b: 7 });
    c[NamedColor::DimRed] = Some(Rgb { r: 8, g: 9, b: 10 });
    vec![Colors::default(), c]
}

/// FNV digest of `color` and `style` over every combination.
pub(super) fn digest(
    color: &dyn Fn(&TermTheme, &Colors, Color) -> Color32,
    style: &dyn Fn(&TermTheme, &Colors, &Cell) -> Style,
) -> u64 {
    let mut h = Fnv::new();
    let c64 = |c: Color32| u64::from(u32::from_le_bytes(c.to_array()));
    let mut styles = Vec::new();
    for theme in themes() {
        for colors in color_sets() {
            for spec in fg_specs() {
                h.u64(c64(color(&theme, &colors, spec)));
            }
            // Indexed 16-255 only differ in the table lookup (covered above).
            let style_fgs = fg_specs()
                .into_iter()
                .filter(|c| !matches!(c, Color::Indexed(i) if (21..250).contains(i) && *i != 100));
            for fg in style_fgs {
                for bg in bg_specs() {
                    for flags in flag_sets() {
                        let cell = Cell { fg, bg, flags, ..Cell::default() };
                        styles.push(style(&theme, &colors, &cell));
                    }
                }
            }
        }
    }
    for s in styles {
        h.u64(c64(s.fg));
        h.u64(c64(s.bg));
        h.u64(u64::from(s.bold) | u64::from(s.italic) << 1 | u64::from(s.underline) << 2 | u64::from(s.strike) << 3);
    }
    h.0
}

/// Recorded from `Palette::{color, style}` before the resolver moved to `color.rs`.
const DIGEST: u64 = 0x4d84_4f8c_f7a4_df85;

fn current_color(theme: &TermTheme, colors: &Colors, c: Color) -> Color32 {
    let raw = raw_palette(theme);
    Palette::new(&raw, colors, theme.bright_bold).color(c)
}

fn current_style(theme: &TermTheme, colors: &Colors, cell: &Cell) -> Style {
    let raw = raw_palette(theme);
    Palette::new(&raw, colors, theme.bright_bold).style(cell)
}

#[test]
fn cell_colors_match_the_recorded_snapshot() {
    let d = digest(&current_color, &current_style);
    assert_eq!(d, DIGEST, "colour resolution changed: digest {d:#x}");
}
