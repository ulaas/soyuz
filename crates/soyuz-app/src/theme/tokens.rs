//! Design tokens: the "Orbit Dark" / "Orbit Light" palettes plus the spacing,
//! radius, size and typography scales. Everything visual in the app reads from here.

use std::collections::BTreeMap;

use egui::Color32;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `rgba(r, g, b, a)` with `a` in 0..=1, premultiplied for egui.
const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    let a8 = (a * 255.0 + 0.5) as u8;
    Color32::from_rgba_premultiplied(
        ((r as u16 * a8 as u16) / 255) as u8,
        ((g as u16 * a8 as u16) / 255) as u8,
        ((b as u16 * a8 as u16) / 255) as u8,
        a8,
    )
}

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

const TAGS: [Color32; 8] = [
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
pub const ORBIT_DARK: Palette = Palette {
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
pub const ORBIT_LIGHT: Palette = Palette {
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
pub const ORBIT_HC_DARK: Palette = Palette {
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
pub const ORBIT_HC_LIGHT: Palette = Palette {
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

impl Palette {
    /// Make the window-level fills translucent so a system backdrop (Windows 11 Mica /
    /// Acrylic) shows through. Popups, cards and text keep their opaque colors.
    pub fn translucent(&mut self) {
        let a = if self.dark { 0.62 } else { 0.55 };
        for c in [&mut self.bg_window, &mut self.bg_surface1, &mut self.bg_sunken] {
            *c = c.gamma_multiply(a);
        }
    }

    /// Replace the accent and derive `accent.hover` / `accent.subtle` from it (lighter
    /// on dark palettes, darker on light ones).
    pub fn set_accent(&mut self, accent: Color32) {
        let (hover, subtle) = accent_shades(accent, self.dark);
        self.accent = accent;
        self.accent_hover = hover;
        self.accent_subtle = subtle;
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
        let t = token.trim().to_ascii_lowercase().replace('_', ".");
        Some(match t.as_str() {
            "bg.window" => &mut self.bg_window,
            "bg.surface1" => &mut self.bg_surface1,
            "bg.surface2" => &mut self.bg_surface2,
            "bg.surface3" => &mut self.bg_surface3,
            "bg.sunken" => &mut self.bg_sunken,
            "border.subtle" => &mut self.border_subtle,
            "border.strong" => &mut self.border_strong,
            "text.primary" => &mut self.text_primary,
            "text.secondary" => &mut self.text_secondary,
            "text.tertiary" => &mut self.text_tertiary,
            "accent" => &mut self.accent,
            "accent.hover" => &mut self.accent_hover,
            "accent.subtle" => &mut self.accent_subtle,
            "success" => &mut self.success,
            "warning" => &mut self.warning,
            "danger" => &mut self.danger,
            "info" => &mut self.info,
            "glyph.folder" => &mut self.glyph_folder,
            "glyph.image" => &mut self.glyph_image,
            "glyph.video" => &mut self.glyph_video,
            "glyph.audio" => &mut self.glyph_audio,
            "glyph.archive" => &mut self.glyph_archive,
            "glyph.code" => &mut self.glyph_code,
            "glyph.doc" => &mut self.glyph_doc,
            "glyph.exe" => &mut self.glyph_exe,
            "tag.red" => &mut self.tags[0],
            "tag.orange" => &mut self.tags[1],
            "tag.yellow" => &mut self.tags[2],
            "tag.green" => &mut self.tags[3],
            "tag.cyan" => &mut self.tags[4],
            "tag.blue" => &mut self.tags[5],
            "tag.purple" => &mut self.tags[6],
            "tag.gray" => &mut self.tags[7],
            _ => return None,
        })
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

/// Hover and subtle (selection fill) shades of an accent color.
pub fn accent_shades(accent: Color32, dark: bool) -> (Color32, Color32) {
    let [r, g, b, _] = accent.to_array();
    let hover = if dark {
        let lift = |c: u8| (f32::from(c) + (255.0 - f32::from(c)) * 0.2).round() as u8;
        Color32::from_rgb(lift(r), lift(g), lift(b))
    } else {
        let sink = |c: u8| (f32::from(c) * 0.86).round() as u8;
        Color32::from_rgb(sink(r), sink(g), sink(b))
    };
    let subtle = Color32::from_rgba_unmultiplied(r, g, b, if dark { 46 } else { 36 });
    (hover, subtle)
}

/// Every token name accepted by [`Palette::apply_overrides`] / theme files.
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

/// Parse `#RGB`, `#RRGGBB` or `#RRGGBBAA` (leading `#` optional).
pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        3 => {
            let n = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
            Some(Color32::from_rgb(n(0)?, n(1)?, n(2)?))
        }
        6 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
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
        let mut p = ORBIT_LIGHT.clone();
        p.set_accent(Color32::from_rgb(200, 100, 50));
        assert!(p.accent_hover.r() < 200, "light themes darken on hover");
        let mut d = ORBIT_DARK.clone();
        d.set_accent(Color32::from_rgb(200, 100, 50));
        assert!(d.accent_hover.r() > 200, "dark themes lighten on hover");
        assert!(d.accent_subtle.a() > 0 && d.accent_subtle.a() < 60);
        assert_eq!(ORBIT_DARK.on_accent(), Color32::WHITE);
        d.set_accent(Color32::from_rgb(0x88, 0xC0, 0xD0));
        assert_ne!(d.on_accent(), Color32::WHITE, "light accents get dark text");
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
}
