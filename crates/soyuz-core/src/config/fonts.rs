//! `[fonts]`: one font per role (interface, headings, terminal, code editor, hex view,
//! numbers), each with a family, an "embedded font only" switch and a size offset.
//!
//! A family is one of the fonts embedded in the app (Inter, IBM Plex Sans, Noto Sans,
//! Cascadia Mono, JetBrains Mono, …; the defaults), an installed font's name as Windows
//! lists it (`Segoe UI`, `Consolas`, `Segoe UI Semibold`), or a `.ttf` / `.otf` / `.ttc`
//! path. The app always appends the role's embedded default (Inter / Inter SemiBold /
//! Cascadia Mono) after it, so a missing font falls back gracefully.
//!
//! Compatibility: the older `[appearance] font` / `mono_font` keys are still read. A
//! non-default value moves into `[fonts] ui` (resp. `terminal`, `code` and `hex`) when
//! those roles are still at their defaults ([`Fonts::adopt_legacy`]); the old keys are
//! dropped from the file on the next save.

use serde::{Deserialize, Serialize};

/// The font roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontRoleKind {
    /// Body text, labels, buttons, menus, file names.
    Ui,
    /// Headings, dialog titles, tab labels.
    UiSemibold,
    /// The integrated terminal.
    Terminal,
    /// The text editor, code previews and other monospace text.
    Code,
    /// The hex editor / hex preview.
    Hex,
    /// Sizes, dates, counters and progress figures.
    Numbers,
}

impl FontRoleKind {
    pub const ALL: [FontRoleKind; 6] = [
        FontRoleKind::Ui,
        FontRoleKind::UiSemibold,
        FontRoleKind::Terminal,
        FontRoleKind::Code,
        FontRoleKind::Hex,
        FontRoleKind::Numbers,
    ];

    /// The config key (`[fonts.<key>]`).
    pub fn key(self) -> &'static str {
        match self {
            FontRoleKind::Ui => "ui",
            FontRoleKind::UiSemibold => "ui_semibold",
            FontRoleKind::Terminal => "terminal",
            FontRoleKind::Code => "code",
            FontRoleKind::Hex => "hex",
            FontRoleKind::Numbers => "numbers",
        }
    }

    /// Whether the role is drawn with a monospace font.
    pub fn is_mono(self) -> bool {
        matches!(self, FontRoleKind::Terminal | FontRoleKind::Code | FontRoleKind::Hex)
    }
}

/// One role's font.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontRole {
    /// Embedded or installed family name, or a font file path. Empty: `ui_semibold` uses
    /// the semibold face of the `ui` family, `numbers` uses the `ui` font, other roles
    /// their default.
    pub family: String,
    /// Only embedded fonts: the one `family` names when it is embedded, else the role's
    /// embedded default (Inter / Inter SemiBold / Cascadia Mono). Never reads system fonts.
    pub embedded: bool,
    /// Points added to the role's text size (−4 … +8).
    pub size_offset: f32,
}

impl FontRole {
    fn named(family: &str) -> Self {
        FontRole { family: family.into(), embedded: false, size_offset: 0.0 }
    }

    /// `size_offset`, clamped to the supported range.
    pub fn offset(&self) -> f32 {
        if self.size_offset.is_finite() { self.size_offset.clamp(-4.0, 8.0) } else { 0.0 }
    }
}

/// `[fonts]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fonts {
    pub ui: FontRole,
    pub ui_semibold: FontRole,
    pub terminal: FontRole,
    pub code: FontRole,
    pub hex: FontRole,
    pub numbers: FontRole,
    /// TrueType hinting: snap stems to the pixel grid (crisper text at small sizes).
    pub hinting: bool,
    /// Sub-pixel glyph positioning: more even letter spacing, slightly softer stems.
    pub subpixel_positioning: bool,
    /// How hard hinting snaps the outlines to the pixel grid (with `hinting = true`).
    pub hinting_strength: HintingStrength,
    /// Text weight on dark themes: the gamma that maps glyph coverage to opacity (lower =
    /// heavier, 1.0 = linear; 0 = egui's built-in `2c - c²` curve, close to 0.5).
    pub text_gamma_dark: f32,
    /// Text weight on light themes (same scale; 1.0 = linear coverage, egui's default).
    pub text_gamma_light: f32,
}

/// `[fonts] hinting_strength`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HintingStrength {
    /// Snap stems horizontally and vertically: the sharpest text at 100 % scaling.
    #[default]
    Crisp,
    /// egui's default: snap vertically, keep the unhinted letter widths.
    Normal,
    /// Only snap heights (FreeType's "light" mode): truest shapes, softer stems.
    Light,
}

impl Fonts {
    /// `text_gamma_dark` / `text_gamma_light` for a theme, clamped (`None`: egui's curve).
    pub fn text_gamma(&self, dark: bool) -> Option<f32> {
        let g = if dark { self.text_gamma_dark } else { self.text_gamma_light };
        (g.is_finite() && g > 0.0).then(|| g.clamp(0.25, 2.0))
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts {
            ui: FontRole::named(Fonts::DEFAULT_UI),
            ui_semibold: FontRole::named(""),
            terminal: FontRole::named(Fonts::DEFAULT_MONO),
            code: FontRole::named(Fonts::DEFAULT_MONO),
            hex: FontRole::named(Fonts::DEFAULT_MONO),
            numbers: FontRole::named(""),
            hinting: true,
            subpixel_positioning: false,
            hinting_strength: HintingStrength::default(),
            text_gamma_dark: 0.45,
            text_gamma_light: 0.8,
        }
    }
}

impl Fonts {
    /// Default interface family (embedded: drawn well by egui's grayscale renderer).
    pub const DEFAULT_UI: &'static str = "Inter";
    /// Default monospace family for the terminal, editor and hex view (embedded).
    pub const DEFAULT_MONO: &'static str = "Cascadia Mono";
    /// The `[appearance] font` / `mono_font` defaults of schema 1 (they meant "bundled").
    const LEGACY_UI: &'static str = "Inter";
    const LEGACY_MONO: &'static str = "JetBrains Mono";

    pub fn role(&self, kind: FontRoleKind) -> &FontRole {
        match kind {
            FontRoleKind::Ui => &self.ui,
            FontRoleKind::UiSemibold => &self.ui_semibold,
            FontRoleKind::Terminal => &self.terminal,
            FontRoleKind::Code => &self.code,
            FontRoleKind::Hex => &self.hex,
            FontRoleKind::Numbers => &self.numbers,
        }
    }

    pub fn role_mut(&mut self, kind: FontRoleKind) -> &mut FontRole {
        match kind {
            FontRoleKind::Ui => &mut self.ui,
            FontRoleKind::UiSemibold => &mut self.ui_semibold,
            FontRoleKind::Terminal => &mut self.terminal,
            FontRoleKind::Code => &mut self.code,
            FontRoleKind::Hex => &mut self.hex,
            FontRoleKind::Numbers => &mut self.numbers,
        }
    }

    /// Move `[appearance] font` / `mono_font` (legacy keys) into the roles and clear
    /// them. A legacy value only wins over a role that is still at its default; the old
    /// defaults (`Inter`, `JetBrains Mono`) mean "the default" and are dropped. Returns
    /// a note per adopted value.
    pub fn adopt_legacy(&mut self, font: &mut String, mono_font: &mut String) -> Vec<String> {
        let mut notes = Vec::new();
        let d = Fonts::default();
        let ui = std::mem::take(font);
        if !is_legacy_default(&ui, Fonts::LEGACY_UI) && self.ui == d.ui {
            self.ui.family = ui.trim().to_string();
            notes.push(format!("[appearance] font = \"{}\" now lives in [fonts.ui]", ui.trim()));
        }
        let mono = std::mem::take(mono_font);
        if !is_legacy_default(&mono, Fonts::LEGACY_MONO) {
            let mut moved = false;
            for kind in [FontRoleKind::Terminal, FontRoleKind::Code, FontRoleKind::Hex] {
                if self.role(kind) == d.role(kind) {
                    self.role_mut(kind).family = mono.trim().to_string();
                    moved = true;
                }
            }
            if moved {
                notes.push(format!(
                    "[appearance] mono_font = \"{}\" now lives in [fonts.terminal] / [fonts.code] / [fonts.hex]",
                    mono.trim()
                ));
            }
        }
        notes
    }
}

fn is_legacy_default(value: &str, default: &str) -> bool {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let v = norm(value);
    v.is_empty() || v == norm(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_defaults_are_dropped() {
        let mut f = Fonts::default();
        let (mut a, mut b) = ("Inter".to_string(), "JetBrains Mono".to_string());
        assert!(f.adopt_legacy(&mut a, &mut b).is_empty());
        assert_eq!(f, Fonts::default());
        assert!(a.is_empty() && b.is_empty());
    }

    #[test]
    fn legacy_custom_fonts_move_into_roles() {
        let mut f = Fonts::default();
        let (mut a, mut b) = ("Fira Sans".to_string(), " Iosevka ".to_string());
        let notes = f.adopt_legacy(&mut a, &mut b);
        assert_eq!(notes.len(), 2);
        assert_eq!(f.ui.family, "Fira Sans");
        assert_eq!(f.terminal.family, "Iosevka");
        assert_eq!(f.code.family, "Iosevka");
        assert_eq!(f.hex.family, "Iosevka");
        assert_eq!(f.ui_semibold, Fonts::default().ui_semibold);
        assert!(a.is_empty() && b.is_empty());
    }

    #[test]
    fn explicit_roles_win_over_legacy_keys() {
        let mut f = Fonts::default();
        f.terminal.family = "Consolas".into();
        f.ui.size_offset = 1.0;
        let (mut a, mut b) = ("Fira Sans".to_string(), "Iosevka".to_string());
        f.adopt_legacy(&mut a, &mut b);
        assert_eq!(f.ui.family, Fonts::DEFAULT_UI, "ui was customized: legacy font ignored");
        assert_eq!(f.terminal.family, "Consolas");
        assert_eq!(f.code.family, "Iosevka");
    }

    #[test]
    fn offsets_are_clamped() {
        let mut r = FontRole::named("x");
        r.size_offset = 40.0;
        assert_eq!(r.offset(), 8.0);
        r.size_offset = f32::NAN;
        assert_eq!(r.offset(), 0.0);
    }
}
