//! The design system's colours, toolkit-free: tokens and palettes ([`tokens`]), theme
//! files ([`themes`]) and [`resolve`], which turns `[appearance]` + `[theme.colors]` into
//! the dark and light palettes. The egui GUI converts them to its `Palette`
//! (`soyuz-app/src/theme`); the terminal UI maps them to terminal colours
//! (`soyuz-tui/src/theme.rs`).

pub mod themes;
pub mod tokens;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use soyuz_core::config::Appearance;

use crate::color::{Rgba8, parse_hex};
pub use themes::ThemeDef;
pub use tokens::Tokens;
use tokens::{ORBIT_DARK, ORBIT_HC_DARK, ORBIT_HC_LIGHT, ORBIT_LIGHT};

/// `[appearance] accent`: the theme's own accent, the OS accent colour or a fixed colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccentChoice {
    /// The colour theme's accent.
    Theme,
    /// The OS accent colour.
    System,
    Custom(Rgba8),
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
                Some(c) if t.starts_with('#') => (AccentChoice::Custom(c.opaque()), None),
                _ => (AccentChoice::Theme, Some(format!("appearance.accent: \"{s}\" is not theme | system | #RRGGBB"))),
            },
        }
    }
}

/// `<config dir>/themes`: user theme files.
pub fn themes_dir() -> PathBuf {
    soyuz_core::config::config_dir().join("themes")
}

/// The OS accent colour, when the platform reports one (`soyuz_fs::os_theme`).
pub fn os_accent() -> Option<Rgba8> {
    soyuz_fs::os_theme::accent_color().map(|[r, g, b]| Rgba8::rgb(r, g, b))
}

/// The dark and light palettes for `[appearance]` (colour theme, high contrast, accent),
/// before `[theme.colors]` overrides. `find` looks a theme up by id (user themes first);
/// `os_accent` is used for `accent = "system"`. Returns warnings for unknown themes /
/// accents.
pub fn resolve_base(
    appearance: &Appearance,
    find: &dyn Fn(&str) -> Option<ThemeDef>,
    os_accent: Option<Rgba8>,
) -> (Tokens, Tokens, Vec<String>) {
    let mut warnings = Vec::new();
    let (mut dark, mut light) = if appearance.high_contrast {
        (ORBIT_HC_DARK.clone(), ORBIT_HC_LIGHT.clone())
    } else {
        match find(&appearance.color_theme) {
            Some(t) => t.palettes(),
            None => {
                warnings.push(format!(
                    "appearance.color_theme: unknown theme \"{}\" (built-ins: {})",
                    appearance.color_theme,
                    themes::builtins().iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")
                ));
                (ORBIT_DARK.clone(), ORBIT_LIGHT.clone())
            }
        }
    };
    let (choice, w) = AccentChoice::parse(&appearance.accent);
    warnings.extend(w);
    let color = match choice {
        AccentChoice::Theme => None,
        AccentChoice::System => os_accent,
        AccentChoice::Custom(c) => Some(c),
    };
    if let Some(c) = color {
        dark.set_accent(c);
        light.set_accent(c);
    }
    (dark, light, warnings)
}

/// The final dark and light palettes, for frontends without a theme library: user themes
/// are read from `themes_dir` ([`themes::load`]), `accent = "system"` uses `os_accent`,
/// and the `[theme.colors]` overrides apply to both (warnings reported once).
pub fn resolve(
    appearance: &Appearance,
    overrides: &BTreeMap<String, String>,
    themes_dir: &Path,
    os_accent: Option<Rgba8>,
) -> (Tokens, Tokens, Vec<String>) {
    let (mut dark, mut light, mut warnings) = resolve_base(appearance, &|id| themes::load(themes_dir, id), os_accent);
    warnings.extend(dark.apply_overrides(overrides));
    let _ = light.apply_overrides(overrides);
    (dark, light, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every token of every built-in palette and bundled theme (and three accents on
    /// each theme), as the premultiplied bytes the GUI drew before the tokens moved here.
    pub const GOLDEN: &str = include_str!("golden_tokens.txt");

    /// The palettes a golden line names.
    fn palette_of(name: &str) -> Tokens {
        let parts: Vec<&str> = name.split(':').collect();
        let variant = |(d, l): (Tokens, Tokens), v: &str| if v == "dark" { d } else { l };
        match parts.as_slice() {
            ["ORBIT_DARK"] => ORBIT_DARK.clone(),
            ["ORBIT_LIGHT"] => ORBIT_LIGHT.clone(),
            ["ORBIT_HC_DARK"] => ORBIT_HC_DARK.clone(),
            ["ORBIT_HC_LIGHT"] => ORBIT_HC_LIGHT.clone(),
            ["theme", id, v] => variant(themes::builtins().iter().find(|t| t.id == *id).unwrap().palettes(), v),
            ["accent", id, hex, v] => {
                let mut p = variant(themes::builtins().iter().find(|t| t.id == *id).unwrap().palettes(), v);
                p.set_accent(parse_hex(hex).unwrap());
                p
            }
            _ => panic!("golden line {name}"),
        }
    }

    #[test]
    fn golden_tokens() {
        let mut n = 0;
        for line in GOLDEN.lines().filter(|l| !l.trim().is_empty()) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let p = palette_of(f[0]);
            let mut s = String::new();
            for t in tokens::TOKEN_NAMES {
                let [r, g, b, a] = tokens::premultiplied(p.token(t).unwrap());
                s.push_str(&format!("{r:02X}{g:02X}{b:02X}{a:02X}"));
            }
            let got = format!("{} {} {} {} {s}", f[0], p.dark, p.shadow_alpha, p.radius_scale);
            assert_eq!(got, line.trim_end(), "{}", f[0]);
            n += 1;
        }
        assert_eq!(n, 4 + 5 * 2 * 4);
    }

    #[test]
    fn accent_parsing() {
        assert_eq!(AccentChoice::parse("theme").0, AccentChoice::Theme);
        assert_eq!(AccentChoice::parse(" System ").0, AccentChoice::System);
        assert_eq!(AccentChoice::parse("#f00").0, AccentChoice::Custom(Rgba8::rgb(255, 0, 0)));
        assert!(AccentChoice::parse("abc").1.is_some(), "hex needs the leading #");
    }

    #[test]
    fn resolve_reads_user_themes_accent_and_overrides() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mine.toml"), "[dark]\n\"bg.window\" = \"#101010\"\n").unwrap();
        let mut a = Appearance { color_theme: "mine".into(), accent: "system".into(), ..Appearance::default() };
        let mut o = BTreeMap::new();
        o.insert("danger".to_string(), "#010203".to_string());
        o.insert("nope".to_string(), "#010203".to_string());
        let blue = Rgba8::rgb(0, 0x78, 0xD4);
        let os = Some(blue);
        let (d, l, w) = resolve(&a, &o, dir.path(), os);
        assert_eq!(d.bg_window, Rgba8::rgb(0x10, 0x10, 0x10));
        assert_eq!((d.accent, l.accent), (blue, blue));
        assert_eq!((d.danger, l.danger), (Rgba8::rgb(1, 2, 3), Rgba8::rgb(1, 2, 3)));
        assert_eq!(w.len(), 1, "{w:?}");
        a.color_theme = "missing".into();
        a.accent = "theme".into();
        let (d, _, w) = resolve(&a, &BTreeMap::new(), dir.path(), os);
        assert_eq!(d, ORBIT_DARK);
        assert!(w[0].contains("unknown theme"), "{w:?}");
        a.high_contrast = true;
        assert_eq!(resolve(&a, &BTreeMap::new(), dir.path(), None).1, ORBIT_HC_LIGHT);
    }
}
