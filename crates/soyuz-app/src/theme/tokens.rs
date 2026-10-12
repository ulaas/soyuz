//! Design tokens: the "Orbit Dark" / "Orbit Light" palettes plus the spacing,
//! radius, size and typography scales. Everything visual in the app reads from here.
//!
//! The colour palettes themselves live toolkit-free in `soyuz_appcore::theme::tokens`
//! ([`Tokens`]); [`Palette`] is their egui form ([`Palette::from_tokens`], bit-exact: see
//! the appcore module docs).

use std::collections::BTreeMap;

use egui::Color32;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub use soyuz_appcore::theme::tokens::{TOKEN_NAMES, Tokens, token_index};

/// One complete color palette. Field names mirror the token names (`bg.window` → `bg_window`).
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg_window: Color32,
    pub bg_surface1: Color32,
    pub bg_surface2: Color32,
    pub bg_surface3: Color32,
    pub bg_sunken: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_subtle: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub info: Color32,
    pub glyph_folder: Color32,
    pub glyph_image: Color32,
    pub glyph_video: Color32,
    pub glyph_audio: Color32,
    pub glyph_archive: Color32,
    pub glyph_code: Color32,
    pub glyph_doc: Color32,
    pub glyph_exe: Color32,
    /// red, orange, yellow, green, cyan, blue, purple, gray.
    pub tags: [Color32; 8],
    /// Popup shadow alpha (0.35 dark / 0.12 light).
    pub shadow_alpha: f32,
    /// Multiplier of the corner radii of widgets, menus and windows (theme files'
    /// `[radii] scale`; 1.0 = the token scale).
    pub radius_scale: f32,
}

/// "Orbit Dark" (default).
pub const ORBIT_DARK: Palette = Palette::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_DARK);
/// "Orbit Light".
pub const ORBIT_LIGHT: Palette = Palette::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_LIGHT);
/// "Orbit High Contrast" (dark): near-black surfaces, white text, bright borders and a
/// strong accent. Meets WCAG AAA for body text; borders are clearly visible.
pub const ORBIT_HC_DARK: Palette = Palette::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_HC_DARK);
/// "Orbit High Contrast" (light): white surfaces, black text, dark borders.
pub const ORBIT_HC_LIGHT: Palette = Palette::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_HC_LIGHT);

/// A token colour as an egui colour (`const` form of [`color32`]).
const fn c32(c: crate::color::Rgba8) -> Color32 {
    Color32::from_rgba_unmultiplied_const(c.r, c.g, c.b, c.a)
}

impl Palette {
    /// The egui palette of toolkit-free tokens. Bit-exact with the palettes the GUI
    /// defined itself before the tokens moved to `soyuz-appcore` (pinned by
    /// `golden_tokens.txt`).
    pub const fn from_tokens(t: &Tokens) -> Palette {
        let g = &t.tags;
        Palette {
            dark: t.dark,
            bg_window: c32(t.bg_window),
            bg_surface1: c32(t.bg_surface1),
            bg_surface2: c32(t.bg_surface2),
            bg_surface3: c32(t.bg_surface3),
            bg_sunken: c32(t.bg_sunken),
            border_subtle: c32(t.border_subtle),
            border_strong: c32(t.border_strong),
            text_primary: c32(t.text_primary),
            text_secondary: c32(t.text_secondary),
            text_tertiary: c32(t.text_tertiary),
            accent: c32(t.accent),
            accent_hover: c32(t.accent_hover),
            accent_subtle: c32(t.accent_subtle),
            success: c32(t.success),
            warning: c32(t.warning),
            danger: c32(t.danger),
            info: c32(t.info),
            glyph_folder: c32(t.glyph_folder),
            glyph_image: c32(t.glyph_image),
            glyph_video: c32(t.glyph_video),
            glyph_audio: c32(t.glyph_audio),
            glyph_archive: c32(t.glyph_archive),
            glyph_code: c32(t.glyph_code),
            glyph_doc: c32(t.glyph_doc),
            glyph_exe: c32(t.glyph_exe),
            tags: [c32(g[0]), c32(g[1]), c32(g[2]), c32(g[3]), c32(g[4]), c32(g[5]), c32(g[6]), c32(g[7])],
            shadow_alpha: t.shadow_alpha,
            radius_scale: t.radius_scale,
        }
    }

    /// Make the window-level fills translucent so a system backdrop (Windows 11 Mica /
    /// Acrylic) shows through. Popups, cards and text keep their opaque colors.
    pub fn translucent(&mut self) {
        let a = if self.dark { 0.62 } else { 0.55 };
        for c in [&mut self.bg_window, &mut self.bg_surface1, &mut self.bg_sunken] {
            *c = c.gamma_multiply(a);
        }
    }

    /// Text / icon color on an accent fill: white, or near-black for light accents.
    pub fn on_accent(&self) -> Color32 {
        let c = self.accent;
        let lum = 0.2126 * f32::from(c.r()) + 0.7152 * f32::from(c.g()) + 0.0722 * f32::from(c.b());
        if lum > 165.0 { hex(0x10131A) } else { Color32::WHITE }
    }

    /// High-contrast palettes (borders at ≥ 3:1 against the window) keep outlines on
    /// controls that are otherwise borderless (toolbar fields, tabs).
    pub fn high_contrast(&self) -> bool {
        contrast(self.border_subtle, self.bg_window) >= 3.0
    }

    /// Fills of the borderless toolbar rail (omnibar, filter, icon buttons), derived
    /// from the palette so every theme gets them. The fields rest on `bg.surface1`.
    pub fn rail(&self, state: Rail) -> Color32 {
        let field = self.bg_surface1;
        match state {
            Rail::Field => field,
            Rail::FieldHover => mix(field, self.text_primary, 0.045),
            Rail::FieldFocus => mix(field, self.accent, if self.dark { 0.13 } else { 0.07 }),
            Rail::ButtonHover => mix(self.bg_window, self.text_primary, if self.dark { 0.07 } else { 0.055 }),
            Rail::ButtonPressed => mix(self.bg_window, self.text_primary, if self.dark { 0.12 } else { 0.10 }),
            Rail::ButtonFocus => mix(self.bg_window, self.accent, if self.dark { 0.28 } else { 0.18 }),
        }
    }

    /// The recessed ("engraved") well of the active tab and its edge shades: `(fill,
    /// inner shadow along the top, highlight along the bottom)`.
    pub fn tab_well(&self) -> (Color32, Color32, Color32) {
        if self.dark {
            (mix(self.bg_window, Color32::BLACK, 0.32), Color32::from_black_alpha(150), Color32::from_white_alpha(14))
        } else {
            (
                mix(self.bg_window, self.text_primary, 0.065),
                Color32::from_black_alpha(34),
                Color32::from_white_alpha(200),
            )
        }
    }

    /// Hover fill of an inactive tab.
    pub fn tab_hover(&self) -> Color32 {
        mix(self.bg_window, self.text_primary, if self.dark { 0.05 } else { 0.04 })
    }

    /// The slanted separators between inactive tabs.
    pub fn tab_separator(&self) -> Color32 {
        if self.high_contrast() { self.border_subtle } else { self.border_strong }
    }

    /// A token by name (`bg.window`, `tag.red`, …), as accepted by [`Self::apply_overrides`].
    pub fn token(&self, token: &str) -> Option<Color32> {
        self.clone().slot_mut(token).map(|c| *c)
    }

    /// Scale a token radius by the palette's [`Self::radius_scale`].
    pub fn radius(&self, r: u8) -> u8 {
        (f32::from(r) * self.radius_scale).round().clamp(0.0, 32.0) as u8
    }

    /// Apply `[theme.colors]` overrides (token name → `#RRGGBB` / `#RRGGBBAA`).
    /// Returns the warnings for unknown tokens / bad colors.
    pub fn apply_overrides(&mut self, overrides: &BTreeMap<String, String>) -> Vec<String> {
        let mut warnings = Vec::new();
        for (name, value) in overrides {
            let Some(color) = parse_hex(value) else {
                warnings.push(format!("theme.colors.{name}: invalid color \"{value}\""));
                continue;
            };
            match self.slot_mut(name) {
                Some(slot) => *slot = color,
                None => warnings.push(format!("theme.colors: unknown token \"{name}\"")),
            }
        }
        warnings
    }

    fn slot_mut(&mut self, token: &str) -> Option<&mut Color32> {
        let i = token_index(token)?;
        let [t0, t1, t2, t3, t4, t5, t6, t7] = &mut self.tags;
        let slots = [
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
        ];
        slots.into_iter().nth(i)
    }
}

/// States of a borderless toolbar-rail control ([`Palette::rail`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rail {
    /// Omnibar / filter field at rest.
    Field,
    FieldHover,
    /// Editing / keyboard focus: a soft accent tint.
    FieldFocus,
    ButtonHover,
    ButtonPressed,
    /// Keyboard focus on an icon button: a stronger accent tint (replaces the ring).
    ButtonFocus,
}

/// `a` blended toward `b` by `t` (0..=1), alpha included (premultiplied space).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    let (a, b) = (a.to_array(), b.to_array());
    // Keep `a`'s alpha so translucent (Mica) fills stay translucent.
    let alpha = a[3];
    let scale = |c: u8| if b[3] == 0 { 0 } else { (u16::from(c) * u16::from(alpha) / u16::from(b[3])) as u8 };
    Color32::from_rgba_premultiplied(l(a[0], scale(b[0])), l(a[1], scale(b[1])), l(a[2], scale(b[2])), alpha)
}

/// WCAG contrast ratio of two opaque colors.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn lum(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Parse `#RGB`, `#RRGGBB` or `#RRGGBBAA` (leading `#` optional; see [`crate::color::parse_hex`]).
pub fn parse_hex(s: &str) -> Option<Color32> {
    crate::color::parse_hex(s).map(color32)
}

/// A core colour as an egui colour (premultiplied).
pub fn color32(c: crate::color::Rgba8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// An egui colour as a core colour (unmultiplied).
pub fn rgba8(c: Color32) -> crate::color::Rgba8 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    crate::color::Rgba8::rgba(r, g, b, a)
}

/// Spacing scale (px).
pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
    pub const XXL: f32 = 24.0;
    pub const XXXL: f32 = 32.0;
}

/// Corner radii.
pub mod radius {
    pub const CONTROL: u8 = 4;
    pub const ROW: u8 = 6;
    pub const CARD: u8 = 8;
    pub const MODAL: u8 = 10;
}

/// Fixed component sizes (points).
pub mod size {
    pub const ROW_COMPACT: f32 = 24.0;
    pub const ROW_COZY: f32 = 28.0;
    pub const ROW_COMFORTABLE: f32 = 32.0;
    pub const SIDEBAR_DEFAULT: f32 = 220.0;
    pub const SIDEBAR_MIN: f32 = 160.0;
    pub const SIDEBAR_MAX: f32 = 420.0;
    pub const TOOLBAR: f32 = 44.0;
    pub const TAB_STRIP: f32 = 36.0;
    pub const STATUS_BAR: f32 = 24.0;
    pub const TERMINAL_DEFAULT: f32 = 260.0;
    pub const ACTIVITY_PANEL: f32 = 360.0;
    pub const ICON: f32 = 16.0;
    pub const ICON_TOOLBAR: f32 = 20.0;
}

/// Typography scale (points) at the default 13 pt body size.
pub mod type_scale {
    pub const SMALL: f32 = 11.0;
    pub const BODY: f32 = 13.0;
    pub const BUTTON: f32 = 13.0;
    pub const HEADING: f32 = 18.0;
    pub const MONOSPACE: f32 = 13.0;
    pub const TITLE: f32 = 15.0;
    pub const CAPTION: f32 = 11.5;
    pub const DISPLAY: f32 = 22.0;
}

/// Animation timings (seconds); all become 0 with `reduced_motion`.
pub mod motion {
    pub const HOVER: f32 = 0.12;
    pub const PANEL: f32 = 0.18;
    pub const MODAL: f32 = 0.16;
    pub const TOAST: f32 = 0.2;
}

/// Alpha multipliers for special entry states.
pub mod alpha {
    pub const CUT: f32 = 0.55;
    pub const HIDDEN: f32 = 0.60;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#5B8CFF"), Some(Color32::from_rgb(0x5B, 0x8C, 0xFF)));
        assert_eq!(parse_hex("fff"), Some(Color32::WHITE));
        assert_eq!(parse_hex("#12345678").map(|c| c.a()), Some(0x78));
        assert_eq!(parse_hex("#12"), None);
        assert_eq!(parse_hex("#GGGGGG"), None);
    }

    #[test]
    fn overrides() {
        let mut p = ORBIT_DARK.clone();
        let mut o = BTreeMap::new();
        o.insert("accent".to_string(), "#FF0000".to_string());
        o.insert("bg.window".to_string(), "#000000".to_string());
        o.insert("nope".to_string(), "#000000".to_string());
        o.insert("danger".to_string(), "red".to_string());
        let w = p.apply_overrides(&o);
        assert_eq!(w.len(), 2);
        assert_eq!(p.accent, Color32::RED);
        assert_eq!(p.bg_window, Color32::BLACK);
    }

    #[test]
    fn high_contrast_text_contrast() {
        // Relative luminance contrast ≥ 7:1 (WCAG AAA) for primary and secondary text.
        fn lum(c: Color32) -> f32 {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        }
        fn ratio(a: Color32, b: Color32) -> f32 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        for p in [&ORBIT_HC_DARK, &ORBIT_HC_LIGHT] {
            assert!(ratio(p.text_primary, p.bg_window) >= 7.0);
            assert!(ratio(p.text_secondary, p.bg_surface1) >= 7.0);
            assert!(ratio(p.text_tertiary, p.bg_surface2) >= 4.5);
            assert!(ratio(p.border_subtle, p.bg_window) >= 3.0, "visible borders");
        }
    }

    #[test]
    fn token_names_resolve_and_accent_shades() {
        for t in TOKEN_NAMES {
            assert!(ORBIT_DARK.token(t).is_some(), "{t}");
        }
        assert_eq!(ORBIT_DARK.token("nope"), None);
        let tinted = |t: &Tokens, c: u32| {
            let mut t = t.clone();
            t.set_accent(crate::color::Rgba8::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8));
            Palette::from_tokens(&t)
        };
        let p = tinted(&soyuz_appcore::theme::tokens::ORBIT_LIGHT, 0xC86432);
        assert!(p.accent_hover.r() < 200, "light themes darken on hover");
        let mut d = tinted(&soyuz_appcore::theme::tokens::ORBIT_DARK, 0xC86432);
        assert!(d.accent_hover.r() > 200, "dark themes lighten on hover");
        assert!(d.accent_subtle.a() > 0 && d.accent_subtle.a() < 60);
        assert_eq!(ORBIT_DARK.on_accent(), Color32::WHITE);
        assert_ne!(
            tinted(&soyuz_appcore::theme::tokens::ORBIT_DARK, 0x88C0D0).on_accent(),
            Color32::WHITE,
            "light accents get dark text"
        );
        d.radius_scale = 1.5;
        assert_eq!(d.radius(radius::ROW), 9);
    }

    #[test]
    fn rail_and_tab_shades() {
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
        let half = mix(Color32::BLACK.gamma_multiply(0.5), Color32::WHITE, 1.0);
        assert_eq!(half.a(), Color32::BLACK.gamma_multiply(0.5).a(), "keeps the base alpha (Mica)");
        assert!(ORBIT_HC_DARK.high_contrast() && ORBIT_HC_LIGHT.high_contrast());
        assert!(!ORBIT_DARK.high_contrast() && !ORBIT_LIGHT.high_contrast());
        for p in [&ORBIT_DARK, &ORBIT_LIGHT] {
            let field = p.rail(Rail::Field);
            for s in [Rail::FieldHover, Rail::FieldFocus] {
                assert_ne!(p.rail(s), field, "{s:?} differs from rest");
            }
            assert_ne!(p.rail(Rail::ButtonFocus), p.rail(Rail::ButtonHover), "focus is not just hover");
            assert_ne!(p.tab_well().0, p.bg_window, "the active tab is recessed");
        }
    }

    #[test]
    fn accent_subtle_is_translucent() {
        assert!(ORBIT_DARK.accent_subtle.a() > 40 && ORBIT_DARK.accent_subtle.a() < 50);
        assert!(ORBIT_LIGHT.accent_subtle.a() < ORBIT_DARK.accent_subtle.a());
    }

    /// Every token of the built-in palettes and bundled themes (and three accents on each
    /// theme) is bit-identical to the premultiplied colours the GUI drew before the tokens
    /// moved to `soyuz-appcore` (table generated from the old GUI code).
    #[test]
    fn palettes_from_tokens_are_bit_exact() {
        use soyuz_appcore::theme::themes::builtins;
        const GOLDEN: &str = include_str!("../../../soyuz-appcore/src/theme/golden_tokens.txt");
        let mut n = 0;
        for line in GOLDEN.lines().filter(|l| !l.trim().is_empty()) {
            let name = line.split_whitespace().next().unwrap();
            let parts: Vec<&str> = name.split(':').collect();
            let pick = |(d, l): (Tokens, Tokens), v: &str| if v == "dark" { d } else { l };
            let p = match parts.as_slice() {
                ["ORBIT_DARK"] => ORBIT_DARK.clone(),
                ["ORBIT_LIGHT"] => ORBIT_LIGHT.clone(),
                ["ORBIT_HC_DARK"] => ORBIT_HC_DARK.clone(),
                ["ORBIT_HC_LIGHT"] => ORBIT_HC_LIGHT.clone(),
                ["theme", id, v] => {
                    Palette::from_tokens(&pick(builtins().iter().find(|t| t.id == *id).unwrap().palettes(), v))
                }
                ["accent", id, hex, v] => {
                    let mut t = pick(builtins().iter().find(|t| t.id == *id).unwrap().palettes(), v);
                    t.set_accent(crate::color::parse_hex(hex).unwrap());
                    Palette::from_tokens(&t)
                }
                _ => panic!("golden line {name}"),
            };
            let mut s = String::new();
            for t in TOKEN_NAMES {
                let [r, g, b, a] = p.token(t).unwrap().to_array();
                s.push_str(&format!("{r:02X}{g:02X}{b:02X}{a:02X}"));
            }
            assert_eq!(format!("{name} {} {} {} {s}", p.dark, p.shadow_alpha, p.radius_scale), line.trim_end());
            n += 1;
        }
        assert_eq!(n, 44);
    }
}
