//! Design tokens, toolkit-free: the colour palettes ("Orbit Dark" / "Orbit Light" and the
//! high-contrast pair), token names, `[theme.colors]` overrides and accent shades.
//!
//! Colours are [`Rgba8`] with straight (unmultiplied) alpha. A frontend converts each
//! token with its own "from unmultiplied" constructor (the egui GUI:
//! `Color32::from_rgba_unmultiplied`, see `soyuz-app/src/theme/tokens.rs`).
//!
//! **Bit-exactness.** The GUI palettes used to be written premultiplied with a truncating
//! premultiply (`c * a / 255`), while egui's `from_rgba_unmultiplied` rounds. The
//! translucent built-in constants ([`rgba`]) therefore store the unmultiplied channel
//! whose *rounded* premultiply equals the old *truncated* one, so the GUI draws exactly
//! what it drew before; [`premultiplied`] mirrors egui's rounding for the few places that
//! work in premultiplied space (accent shades, contrast). `golden_tokens.txt` pins every
//! token of every built-in palette and bundled theme.

use std::collections::BTreeMap;

use crate::color::Rgba8;

const fn hex(rgb: u32) -> Rgba8 {
    Rgba8::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `round(v * a / 255)`, exactly as egui's premultiply (`ecolor::mul_frac_round`).
const fn mul_frac_round(v: u8, a: u8) -> u8 {
    let p = (v as u16) * (a as u16) + 128;
    ((p + (p >> 8)) >> 8) as u8
}

/// The premultiplied channels of `c` (what egui's `Color32::from_rgba_unmultiplied`
/// stores).
pub const fn premultiplied(c: Rgba8) -> [u8; 4] {
    match c.a {
        0 => [0, 0, 0, 0],
        255 => [c.r, c.g, c.b, 255],
        a => [mul_frac_round(c.r, a), mul_frac_round(c.g, a), mul_frac_round(c.b, a), a],
    }
}

/// The unmultiplied channel nearest to `want` whose rounded premultiply by `a` is `pm`.
const fn unmultiply_exact(pm: u8, a: u8, want: u8) -> u8 {
    let mut best = want;
    let mut best_d = 256u16;
    let mut u = 0u16;
    while u <= 255 {
        if mul_frac_round(u as u8, a) == pm {
            let d = u.abs_diff(want as u16);
            if d < best_d {
                best_d = d;
                best = u as u8;
            }
        }
        u += 1;
    }
    best
}

/// `rgba(r, g, b, a)` with `a` in 0..=1 — the historic GUI constant: alpha rounded to a
/// byte, channels premultiplied by truncation. Stored unmultiplied so that a rounding
/// premultiply (egui's) gives the same bytes (see the module docs).
pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba8 {
    let a8 = (a * 255.0 + 0.5) as u8;
    let pr = ((r as u16 * a8 as u16) / 255) as u8;
    let pg = ((g as u16 * a8 as u16) / 255) as u8;
    let pb = ((b as u16 * a8 as u16) / 255) as u8;
    Rgba8::rgba(unmultiply_exact(pr, a8, r), unmultiply_exact(pg, a8, g), unmultiply_exact(pb, a8, b), a8)
}

/// One complete colour palette. Field names mirror the token names (`bg.window` →
/// `bg_window`).
#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub bg_window: Rgba8,
    pub bg_surface1: Rgba8,
    pub bg_surface2: Rgba8,
    pub bg_surface3: Rgba8,
    pub bg_sunken: Rgba8,
    pub border_subtle: Rgba8,
    pub border_strong: Rgba8,
    pub text_primary: Rgba8,
    pub text_secondary: Rgba8,
    pub text_tertiary: Rgba8,
    pub accent: Rgba8,
    pub accent_hover: Rgba8,
    pub accent_subtle: Rgba8,
    pub success: Rgba8,
    pub warning: Rgba8,
    pub danger: Rgba8,
    pub info: Rgba8,
    pub glyph_folder: Rgba8,
    pub glyph_image: Rgba8,
    pub glyph_video: Rgba8,
    pub glyph_audio: Rgba8,
    pub glyph_archive: Rgba8,
    pub glyph_code: Rgba8,
    pub glyph_doc: Rgba8,
    pub glyph_exe: Rgba8,
    /// red, orange, yellow, green, cyan, blue, purple, gray.
    pub tags: [Rgba8; 8],
    /// Popup shadow alpha (0.35 dark / 0.12 light).
    pub shadow_alpha: f32,
    /// Multiplier of the corner radii of widgets, menus and windows (theme files'
    /// `[radii] scale`; 1.0 = the token scale).
    pub radius_scale: f32,
}

const TAGS: [Rgba8; 8] = [
    hex(0xF2555A),
    hex(0xF5B544),
    hex(0xF2D14B),
    hex(0x3DD68C),
    hex(0x4CC2FF),
    hex(0x5B8CFF),
    hex(0xBB86FC),
    hex(0x8A91A0),
];

/// "Orbit Dark" (default).
pub const ORBIT_DARK: Tokens = Tokens {
    dark: true,
    bg_window: hex(0x0F1115),
    bg_surface1: hex(0x161920),
    bg_surface2: hex(0x1C2027),
    bg_surface3: hex(0x232834),
    bg_sunken: hex(0x0B0D11),
    border_subtle: hex(0x262B35),
    border_strong: hex(0x343A47),
    text_primary: hex(0xE6E8EC),
    text_secondary: hex(0xA3A9B7),
    text_tertiary: hex(0x6B7280),
    accent: hex(0x5B8CFF),
    accent_hover: hex(0x7BA3FF),
    accent_subtle: rgba(91, 140, 255, 0.18),
    success: hex(0x3DD68C),
    warning: hex(0xF5B544),
    danger: hex(0xF2555A),
    info: hex(0x4CC2FF),
    glyph_folder: hex(0xF2B84B),
    glyph_image: hex(0x6FCF97),
    glyph_video: hex(0xBB86FC),
    glyph_audio: hex(0xFF8FAB),
    glyph_archive: hex(0xF5B544),
    glyph_code: hex(0x5B8CFF),
    glyph_doc: hex(0xA3A9B7),
    glyph_exe: hex(0x4CC2FF),
    tags: TAGS,
    shadow_alpha: 0.35,
    radius_scale: 1.0,
};

/// "Orbit Light".
pub const ORBIT_LIGHT: Tokens = Tokens {
    dark: false,
    bg_window: hex(0xF6F7F9),
    bg_surface1: hex(0xFFFFFF),
    bg_surface2: hex(0xF0F2F5),
    bg_surface3: hex(0xFFFFFF),
    bg_sunken: hex(0xE9EBEF),
    border_subtle: hex(0xE3E6EB),
    border_strong: hex(0xCBD0D8),
    text_primary: hex(0x171A20),
    text_secondary: hex(0x4B5260),
    text_tertiary: hex(0x8A91A0),
    accent: hex(0x2F6BFF),
    accent_hover: hex(0x1F5BEF),
    accent_subtle: rgba(47, 107, 255, 0.14),
    success: hex(0x1E9E5F),
    warning: hex(0xD08A14),
    danger: hex(0xD93A40),
    info: hex(0x1C8ED6),
    glyph_folder: hex(0xE8A33D),
    glyph_image: hex(0x3FA86E),
    glyph_video: hex(0x8A4FD8),
    glyph_audio: hex(0xE0567A),
    glyph_archive: hex(0xD08A14),
    glyph_code: hex(0x2F6BFF),
    glyph_doc: hex(0x4B5260),
    glyph_exe: hex(0x1C8ED6),
    tags: TAGS,
    shadow_alpha: 0.12,
    radius_scale: 1.0,
};

/// "Orbit High Contrast" (dark): near-black surfaces, white text, bright borders and a
/// strong accent. Meets WCAG AAA for body text; borders are clearly visible.
pub const ORBIT_HC_DARK: Tokens = Tokens {
    dark: true,
    bg_window: hex(0x000000),
    bg_surface1: hex(0x0A0A0A),
    bg_surface2: hex(0x161616),
    bg_surface3: hex(0x1E1E1E),
    bg_sunken: hex(0x000000),
    border_subtle: hex(0x8A8A8A),
    border_strong: hex(0xFFFFFF),
    text_primary: hex(0xFFFFFF),
    text_secondary: hex(0xE0E0E0),
    text_tertiary: hex(0xBDBDBD),
    accent: hex(0x4DA3FF),
    accent_hover: hex(0x80BDFF),
    accent_subtle: rgba(77, 163, 255, 0.34),
    success: hex(0x5CFF9D),
    warning: hex(0xFFD24D),
    danger: hex(0xFF6B6B),
    info: hex(0x6BD8FF),
    glyph_folder: hex(0xFFD24D),
    glyph_image: hex(0x7DFFAF),
    glyph_video: hex(0xD7AEFF),
    glyph_audio: hex(0xFFA8C0),
    glyph_archive: hex(0xFFD24D),
    glyph_code: hex(0x80BDFF),
    glyph_doc: hex(0xE0E0E0),
    glyph_exe: hex(0x6BD8FF),
    tags: TAGS,
    shadow_alpha: 0.6,
    radius_scale: 1.0,
};

/// "Orbit High Contrast" (light): white surfaces, black text, dark borders.
pub const ORBIT_HC_LIGHT: Tokens = Tokens {
    dark: false,
    bg_window: hex(0xFFFFFF),
    bg_surface1: hex(0xFFFFFF),
    bg_surface2: hex(0xEDEDED),
    bg_surface3: hex(0xFFFFFF),
    bg_sunken: hex(0xE0E0E0),
    border_subtle: hex(0x6B6B6B),
    border_strong: hex(0x000000),
    text_primary: hex(0x000000),
    text_secondary: hex(0x1F1F1F),
    text_tertiary: hex(0x3D3D3D),
    accent: hex(0x0040C0),
    accent_hover: hex(0x00309A),
    accent_subtle: rgba(0, 64, 192, 0.20),
    success: hex(0x006B32),
    warning: hex(0x8A5A00),
    danger: hex(0xB00020),
    info: hex(0x005A8C),
    glyph_folder: hex(0x9A6200),
    glyph_image: hex(0x006B32),
    glyph_video: hex(0x5B21B6),
    glyph_audio: hex(0xA3164A),
    glyph_archive: hex(0x8A5A00),
    glyph_code: hex(0x0040C0),
    glyph_doc: hex(0x1F1F1F),
    glyph_exe: hex(0x005A8C),
    tags: TAGS,
    shadow_alpha: 0.25,
    radius_scale: 1.0,
};

/// Every token name accepted by [`Tokens::apply_overrides`] / theme files, in the order
/// of [`Tokens::slots_mut`].
pub const TOKEN_NAMES: [&str; 33] = [
    "bg.window",
    "bg.surface1",
    "bg.surface2",
    "bg.surface3",
    "bg.sunken",
    "border.subtle",
    "border.strong",
    "text.primary",
    "text.secondary",
    "text.tertiary",
    "accent",
    "accent.hover",
    "accent.subtle",
    "success",
    "warning",
    "danger",
    "info",
    "glyph.folder",
    "glyph.image",
    "glyph.video",
    "glyph.audio",
    "glyph.archive",
    "glyph.code",
    "glyph.doc",
    "glyph.exe",
    "tag.red",
    "tag.orange",
    "tag.yellow",
    "tag.green",
    "tag.cyan",
    "tag.blue",
    "tag.purple",
    "tag.gray",
];

/// The index of a token name in [`TOKEN_NAMES`]. Case-insensitive; `_` may stand for `.`
/// (`bg_window`).
pub fn token_index(token: &str) -> Option<usize> {
    let t = token.trim().to_ascii_lowercase().replace('_', ".");
    TOKEN_NAMES.iter().position(|n| *n == t)
}

impl Tokens {
    /// Every colour slot, in [`TOKEN_NAMES`] order.
    pub fn slots_mut(&mut self) -> [&mut Rgba8; 33] {
        let [t0, t1, t2, t3, t4, t5, t6, t7] = &mut self.tags;
        [
            &mut self.bg_window,
            &mut self.bg_surface1,
            &mut self.bg_surface2,
            &mut self.bg_surface3,
            &mut self.bg_sunken,
            &mut self.border_subtle,
            &mut self.border_strong,
            &mut self.text_primary,
            &mut self.text_secondary,
            &mut self.text_tertiary,
            &mut self.accent,
            &mut self.accent_hover,
            &mut self.accent_subtle,
            &mut self.success,
            &mut self.warning,
            &mut self.danger,
            &mut self.info,
            &mut self.glyph_folder,
            &mut self.glyph_image,
            &mut self.glyph_video,
            &mut self.glyph_audio,
            &mut self.glyph_archive,
            &mut self.glyph_code,
            &mut self.glyph_doc,
            &mut self.glyph_exe,
            t0,
            t1,
            t2,
            t3,
            t4,
            t5,
            t6,
            t7,
        ]
    }

    /// A token by name (`bg.window`, `tag.red`, …), as accepted by
    /// [`Self::apply_overrides`].
    pub fn token(&self, token: &str) -> Option<Rgba8> {
        let i = token_index(token)?;
        Some(*self.clone().slots_mut()[i])
    }

    /// Replace the accent and derive `accent.hover` / `accent.subtle` from it (lighter on
    /// dark palettes, darker on light ones).
    pub fn set_accent(&mut self, accent: Rgba8) {
        let (hover, subtle) = accent_shades(accent, self.dark);
        self.accent = accent;
        self.accent_hover = hover;
        self.accent_subtle = subtle;
    }

    /// Apply `[theme.colors]` overrides (token name → `#RRGGBB` / `#RRGGBBAA`). Returns
    /// the warnings for unknown tokens / bad colours.
    pub fn apply_overrides(&mut self, overrides: &BTreeMap<String, String>) -> Vec<String> {
        let mut warnings = Vec::new();
        for (name, value) in overrides {
            let Some(color) = crate::color::parse_hex(value) else {
                warnings.push(format!("theme.colors.{name}: invalid color \"{value}\""));
                continue;
            };
            match token_index(name) {
                Some(i) => *self.slots_mut()[i] = color,
                None => warnings.push(format!("theme.colors: unknown token \"{name}\"")),
            }
        }
        warnings
    }
}

/// Hover and subtle (selection fill) shades of an accent colour. Works on the
/// premultiplied channels, like the GUI always did.
pub fn accent_shades(accent: Rgba8, dark: bool) -> (Rgba8, Rgba8) {
    let [r, g, b, _] = premultiplied(accent);
    let hover = if dark {
        let lift = |c: u8| (f32::from(c) + (255.0 - f32::from(c)) * 0.2).round() as u8;
        Rgba8::rgb(lift(r), lift(g), lift(b))
    } else {
        let sink = |c: u8| (f32::from(c) * 0.86).round() as u8;
        Rgba8::rgb(sink(r), sink(g), sink(b))
    };
    let subtle = Rgba8::rgba(r, g, b, if dark { 46 } else { 36 });
    (hover, subtle)
}

/// WCAG contrast ratio of two opaque colours (premultiplied channels, as the GUI).
pub fn contrast(a: Rgba8, b: Rgba8) -> f32 {
    fn lum(c: Rgba8) -> f32 {
        let [r, g, b, _] = premultiplied(c);
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    }
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `c` composited over the opaque `bg` (straight alpha, rounded): the colour a
/// translucent token shows on a surface. Opaque colours are returned as they are.
pub fn blend_over(c: Rgba8, bg: Rgba8) -> Rgba8 {
    if c.a == 255 {
        return c;
    }
    let a = f32::from(c.a) / 255.0;
    let m = |x: u8, y: u8| (f32::from(x) * a + f32::from(y) * (1.0 - a)).round() as u8;
    Rgba8::rgb(m(c.r, bg.r), m(c.g, bg.g), m(c.b, bg.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiply_matches_egui_rounding() {
        // A few hand-checked values of round(v * a / 255).
        assert_eq!(premultiplied(Rgba8::rgba(255, 128, 0, 128)), [128, 64, 0, 128]);
        assert_eq!(premultiplied(Rgba8::rgba(47, 107, 255, 36)), [7, 15, 36, 36]);
        assert_eq!(premultiplied(Rgba8::rgba(9, 9, 9, 0)), [0, 0, 0, 0]);
    }

    #[test]
    fn rgba_constants_keep_the_truncated_premultiply() {
        // The light accent.subtle: trunc(47 * 36 / 255) = 6, round would give 7.
        let c = rgba(47, 107, 255, 0.14);
        assert_eq!(premultiplied(c), [6, 15, 36, 36]);
        for (r, g, b, a) in [(91, 140, 255, 0.18), (77, 163, 255, 0.34), (0, 64, 192, 0.20)] {
            let a8 = (a * 255.0f32 + 0.5) as u8;
            let t = |c: u8| ((u16::from(c) * u16::from(a8)) / 255) as u8;
            assert_eq!(premultiplied(rgba(r, g, b, a)), [t(r), t(g), t(b), a8]);
        }
    }

    #[test]
    fn overrides_and_tokens() {
        let mut p = ORBIT_DARK.clone();
        let mut o = BTreeMap::new();
        o.insert("accent".to_string(), "#FF0000".to_string());
        o.insert("bg_window".to_string(), "#000000".to_string());
        o.insert("nope".to_string(), "#000000".to_string());
        o.insert("danger".to_string(), "red".to_string());
        assert_eq!(p.apply_overrides(&o).len(), 2);
        assert_eq!(p.accent, Rgba8::rgb(255, 0, 0));
        assert_eq!(p.bg_window, Rgba8::rgb(0, 0, 0));
        for t in TOKEN_NAMES {
            assert!(ORBIT_DARK.token(t).is_some(), "{t}");
        }
        assert_eq!(ORBIT_DARK.token("tag.gray"), Some(hex(0x8A91A0)));
        assert_eq!(ORBIT_DARK.token("nope"), None);
    }

    #[test]
    fn accent_shades_follow_the_variant() {
        let mut l = ORBIT_LIGHT.clone();
        l.set_accent(Rgba8::rgb(200, 100, 50));
        assert!(l.accent_hover.r < 200, "light themes darken on hover");
        let mut d = ORBIT_DARK.clone();
        d.set_accent(Rgba8::rgb(200, 100, 50));
        assert!(d.accent_hover.r > 200, "dark themes lighten on hover");
        assert_eq!(d.accent_subtle, Rgba8::rgba(200, 100, 50, 46));
    }

    #[test]
    fn blending() {
        let bg = Rgba8::rgb(0, 0, 0);
        assert_eq!(blend_over(Rgba8::rgba(255, 255, 255, 128), bg), Rgba8::rgb(128, 128, 128));
        assert_eq!(blend_over(Rgba8::rgb(1, 2, 3), bg), Rgba8::rgb(1, 2, 3));
        assert_eq!(blend_over(Rgba8::rgba(255, 0, 0, 0), Rgba8::rgb(9, 9, 9)), Rgba8::rgb(9, 9, 9));
    }

    #[test]
    fn high_contrast_text_contrast() {
        for p in [&ORBIT_HC_DARK, &ORBIT_HC_LIGHT] {
            assert!(contrast(p.text_primary, p.bg_window) >= 7.0);
            assert!(contrast(p.text_secondary, p.bg_surface1) >= 7.0);
            assert!(contrast(p.text_tertiary, p.bg_surface2) >= 4.5);
            assert!(contrast(p.border_subtle, p.bg_window) >= 3.0, "visible borders");
        }
    }
}
