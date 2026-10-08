//! `[appearance] accent`: the theme's own accent, the OS accent color or a fixed color.

use egui::Color32;

use super::tokens::parse_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccentChoice {
    /// The color theme's accent.
    Theme,
    /// The OS accent color (Windows registry), followed live.
    System,
    Custom(Color32),
}

impl AccentChoice {
    /// Parse the config value. Unknown values fall back to [`AccentChoice::Theme`] with a
    /// warning.
    pub fn parse(s: &str) -> (AccentChoice, Option<String>) {
        let t = s.trim();
        match t.to_ascii_lowercase().as_str() {
            "" | "theme" | "default" => (AccentChoice::Theme, None),
            "system" | "os" => (AccentChoice::System, None),
            _ => match parse_hex(t) {
                Some(c) if t.starts_with('#') => (AccentChoice::Custom(c.to_opaque()), None),
                _ => (AccentChoice::Theme, Some(format!("appearance.accent: \"{s}\" is not theme | system | #RRGGBB"))),
            },
        }
    }

    /// The config value for this choice.
    pub fn to_config(self) -> String {
        match self {
            AccentChoice::Theme => "theme".into(),
            AccentChoice::System => "system".into(),
            AccentChoice::Custom(c) => super::themes::hex_string(c.to_opaque()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(AccentChoice::parse("theme").0, AccentChoice::Theme);
        assert_eq!(AccentChoice::parse("").0, AccentChoice::Theme);
        assert_eq!(AccentChoice::parse(" System ").0, AccentChoice::System);
        assert_eq!(AccentChoice::parse("#FF0000").0, AccentChoice::Custom(Color32::RED));
        assert_eq!(AccentChoice::parse("#f00").0, AccentChoice::Custom(Color32::RED));
        let (c, w) = AccentChoice::parse("red");
        assert_eq!(c, AccentChoice::Theme);
        assert!(w.is_some());
        assert!(AccentChoice::parse("abc").1.is_some(), "hex needs the leading #");
        assert_eq!(AccentChoice::Custom(Color32::from_rgb(1, 2, 3)).to_config(), "#010203");
        for s in ["theme", "system", "#5B8CFF"] {
            assert_eq!(AccentChoice::parse(s).0.to_config(), s);
        }
    }
}
