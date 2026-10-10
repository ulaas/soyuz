//! Theme files: a named token palette with a dark and a light variant, an
//! optional accent and a corner-radius scale. Built-ins ship as embedded TOML (the same
//! format users write in `<config dir>/themes/<id>.toml`), so every built-in is also a
//! test of the parser.
//!
//! ```toml
//! name = "Nord"
//! base = "orbit"            # orbit | high-contrast: where missing tokens come from
//! accent = "#88C0D0"        # optional, both variants (hover / subtle derived)
//! [radii]
//! scale = 1.0
//! [dark]
//! "bg.window" = "#242933"
//! [light]
//! "bg.window" = "#E5E9F0"
//! ```

use std::collections::BTreeMap;

use egui::Color32;

use super::tokens::{ORBIT_DARK, ORBIT_HC_DARK, ORBIT_HC_LIGHT, ORBIT_LIGHT, Palette, TOKEN_NAMES, parse_hex};

/// Largest accepted theme file (bytes).
pub const MAX_THEME_BYTES: usize = 64 * 1024;
/// Accepted `[radii] scale` range.
pub const RADIUS_SCALE: std::ops::RangeInclusive<f32> = 0.0..=2.5;

/// Where a theme's missing tokens come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Orbit,
    HighContrast,
}

/// A parsed, validated theme.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeDef {
    /// Lower-case id (`nord`, or the file stem of a user theme).
    pub id: String,
    pub name: String,
    pub author: String,
    pub base: Base,
    /// `accent` for both variants (token tables can still override it per variant).
    pub accent: Option<Color32>,
    pub radius_scale: f32,
    pub dark: BTreeMap<String, String>,
    pub light: BTreeMap<String, String>,
    /// Shipped with the app (cannot be deleted; export makes an editable copy).
    pub builtin: bool,
}

/// Built-in theme files: `(id, toml)`.
pub const BUILTIN_SOURCES: [(&str, &str); 5] = [
    ("orbit", include_str!("../../assets/themes/orbit.toml")),
    ("nord", include_str!("../../assets/themes/nord.toml")),
    ("solarized", include_str!("../../assets/themes/solarized.toml")),
    ("rose-pine", include_str!("../../assets/themes/rose-pine.toml")),
    ("high-contrast", include_str!("../../assets/themes/high-contrast.toml")),
];

/// The built-in themes, parsed once.
pub fn builtins() -> &'static [ThemeDef] {
    static B: std::sync::OnceLock<Vec<ThemeDef>> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        BUILTIN_SOURCES
            .iter()
            .filter_map(|(id, src)| match parse(id, src) {
                Ok((mut t, _)) => {
                    t.builtin = true;
                    Some(t)
                }
                Err(e) => {
                    log::error!("built-in theme {id}: {e}");
                    None
                }
            })
            .collect()
    })
}

/// A theme id from a file stem or name: lower-case, spaces → `-`, only `[a-z0-9-_]`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        let c = match c {
            'é' | 'è' | 'ê' => 'e',
            c => c,
        };
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c.to_ascii_lowercase());
        } else if (c.is_whitespace() || c == '.') && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(48).collect()
}

const TOP_KEYS: [&str; 7] = ["name", "author", "base", "accent", "radii", "dark", "light"];

/// Parse and validate a theme file. Errors make the file unusable; warnings (unknown
/// keys or tokens, low contrast) are reported but the theme still loads.
pub fn parse(id: &str, text: &str) -> Result<(ThemeDef, Vec<String>), String> {
    if text.len() > MAX_THEME_BYTES {
        return Err(format!("theme file larger than {} KiB", MAX_THEME_BYTES / 1024));
    }
    let table: toml::Table = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let mut warnings = Vec::new();
    for k in table.keys() {
        if !TOP_KEYS.contains(&k.as_str()) {
            warnings.push(format!("unknown key \"{k}\""));
        }
    }
    let str_of = |k: &str| -> Result<Option<String>, String> {
        match table.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("\"{k}\" must be a string")),
        }
    };
    let id = slug(id);
    if id.is_empty() {
        return Err("empty theme id".into());
    }
    let name = str_of("name")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());
    if name.chars().count() > 64 {
        return Err("\"name\" is longer than 64 characters".into());
    }
    let author = str_of("author")?.unwrap_or_default();
    let base = match str_of("base")?.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("orbit") => Base::Orbit,
        Some("high-contrast" | "high_contrast" | "hc") => Base::HighContrast,
        Some(other) => return Err(format!("unknown base \"{other}\" (orbit | high-contrast)")),
    };
    let accent = match str_of("accent")? {
        None => None,
        Some(s) => Some(parse_hex(&s).ok_or_else(|| format!("accent: invalid color \"{s}\""))?),
    };
    let radius_scale = match table.get("radii") {
        None => 1.0,
        Some(toml::Value::Table(t)) => match t.get("scale") {
            None => 1.0,
            Some(v) => {
                let f =
                    v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).ok_or("radii.scale must be a number")?
                        as f32;
                if !RADIUS_SCALE.contains(&f) {
                    return Err(format!("radii.scale {f} outside 0.0 – 2.5"));
                }
                f
            }
        },
        Some(_) => return Err("\"radii\" must be a table".into()),
    };
    let mut tokens = |k: &str| -> Result<BTreeMap<String, String>, String> {
        let mut out = BTreeMap::new();
        match table.get(k) {
            None => {}
            Some(toml::Value::Table(t)) => {
                for (name, v) in t {
                    let norm = name.trim().to_ascii_lowercase().replace('_', ".");
                    if !TOKEN_NAMES.contains(&norm.as_str()) {
                        warnings.push(format!("[{k}] unknown token \"{name}\""));
                        continue;
                    }
                    let Some(s) = v.as_str() else {
                        return Err(format!("[{k}] {name}: color must be a string"));
                    };
                    if parse_hex(s).is_none() {
                        return Err(format!("[{k}] {name}: invalid color \"{s}\""));
                    }
                    out.insert(norm, s.to_string());
                }
            }
            Some(_) => return Err(format!("\"{k}\" must be a table")),
        }
        Ok(out)
    };
    let dark = tokens("dark")?;
    let light = tokens("light")?;
    let def = ThemeDef { id, name, author, base, accent, radius_scale, dark, light, builtin: false };
    let (d, l) = def.palettes();
    for (variant, p) in [("dark", &d), ("light", &l)] {
        let r = contrast(p.text_primary, p.bg_window);
        if r < 4.5 {
            warnings.push(format!("{variant}: text.primary on bg.window has contrast {r:.1}:1 (below 4.5:1)"));
        }
    }
    Ok((def, warnings))
}

impl ThemeDef {
    /// The dark and light palettes of this theme.
    pub fn palettes(&self) -> (Palette, Palette) {
        let (mut dark, mut light) = match self.base {
            Base::Orbit => (ORBIT_DARK.clone(), ORBIT_LIGHT.clone()),
            Base::HighContrast => (ORBIT_HC_DARK.clone(), ORBIT_HC_LIGHT.clone()),
        };
        for (p, tokens) in [(&mut dark, &self.dark), (&mut light, &self.light)] {
            if let Some(a) = self.accent {
                p.set_accent(a);
            }
            // A variant's own accent derives its shades unless they are given too.
            if let Some(a) = tokens.get("accent").and_then(|h| parse_hex(h)) {
                p.set_accent(a);
            }
            let _ = p.apply_overrides(tokens);
            p.radius_scale = self.radius_scale;
        }
        (dark, light)
    }

    /// Serialize as a theme file.
    pub fn to_toml(&self) -> String {
        let mut t = toml::Table::new();
        t.insert("name".into(), self.name.clone().into());
        if !self.author.is_empty() {
            t.insert("author".into(), self.author.clone().into());
        }
        t.insert(
            "base".into(),
            match self.base {
                Base::Orbit => "orbit",
                Base::HighContrast => "high-contrast",
            }
            .into(),
        );
        if let Some(a) = self.accent {
            t.insert("accent".into(), hex_string(a).into());
        }
        let mut radii = toml::Table::new();
        radii.insert("scale".into(), toml::Value::Float(f64::from((self.radius_scale * 100.0).round() / 100.0)));
        t.insert("radii".into(), radii.into());
        for (k, m) in [("dark", &self.dark), ("light", &self.light)] {
            let tt: toml::Table = m.iter().map(|(k, v)| (k.clone(), toml::Value::String(v.clone()))).collect();
            t.insert(k.into(), tt.into());
        }
        let body = toml::to_string_pretty(&t).unwrap_or_default();
        format!("# Soyuz theme — edit freely; missing tokens come from `base`.\n{body}")
    }

    /// A self-contained copy of two resolved palettes (every token written out).
    pub fn from_palettes(id: &str, name: &str, dark: &Palette, light: &Palette) -> ThemeDef {
        let all = |p: &Palette| {
            TOKEN_NAMES.iter().filter_map(|t| p.token(t).map(|c| (t.to_string(), hex_string(c)))).collect()
        };
        ThemeDef {
            id: slug(id),
            name: name.to_string(),
            author: String::new(),
            base: Base::Orbit,
            accent: None,
            radius_scale: dark.radius_scale,
            dark: all(dark),
            light: all(light),
            builtin: false,
        }
    }
}

/// `#RRGGBB`, or `#RRGGBBAA` for translucent colors (unmultiplied).
pub fn hex_string(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse_cleanly_and_are_readable() {
        assert_eq!(builtins().len(), BUILTIN_SOURCES.len());
        for (id, src) in BUILTIN_SOURCES {
            let (t, w) = parse(id, src).unwrap();
            assert!(w.is_empty(), "{id}: {w:?}");
            let (d, l) = t.palettes();
            assert!(d.dark && !l.dark);
            for p in [&d, &l] {
                assert!(contrast(p.text_primary, p.bg_window) >= 7.0, "{id}: primary text");
                assert!(contrast(p.text_secondary, p.bg_surface1) >= 4.5, "{id}: secondary text");
                assert!(contrast(p.text_tertiary, p.bg_surface1) >= 2.8, "{id}: tertiary text");
            }
        }
        let orbit = &builtins()[0];
        assert_eq!(orbit.palettes(), (ORBIT_DARK.clone(), ORBIT_LIGHT.clone()));
        let hc = builtins().iter().find(|t| t.id == "high-contrast").unwrap();
        assert_eq!(hc.palettes().0.bg_window, ORBIT_HC_DARK.bg_window);
    }

    #[test]
    fn validation() {
        let ok = "name = \"Mine\"\naccent = \"#FF0000\"\n[radii]\nscale = 2\n[dark]\n\"bg.window\" = \"#101010\"\n";
        let (t, w) = parse("My Theme", ok).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.id, "my-theme");
        assert_eq!(t.radius_scale, 2.0);
        let (d, l) = t.palettes();
        assert_eq!(d.accent, Color32::RED);
        assert_eq!(l.accent, Color32::RED);
        assert_eq!(d.bg_window, Color32::from_gray(0x10));
        assert_eq!(l.bg_window, ORBIT_LIGHT.bg_window, "missing tokens come from the base");

        assert!(parse("x", "accent = \"blue\"").is_err());
        assert!(parse("x", "[radii]\nscale = 9.0").is_err());
        assert!(parse("x", "base = \"neon\"").is_err());
        assert!(parse("x", "[dark]\naccent = \"#12\"").is_err());
        assert!(parse("x", "[dark\n").is_err());
        assert!(parse("", "").is_err());
        let (_, w) = parse("x", "colour = 1\n[dark]\nnope = \"#000000\"").unwrap();
        assert_eq!(w.len(), 2, "{w:?}");
        let (_, w) = parse("x", "[dark]\n\"text.primary\" = \"#101010\"\n\"bg.window\" = \"#111111\"").unwrap();
        assert!(w.iter().any(|w| w.contains("contrast")), "{w:?}");
        assert!(parse("x", &"#".repeat(MAX_THEME_BYTES + 1)).is_err());
    }

    #[test]
    fn export_round_trips() {
        let nord = builtins().iter().find(|t| t.id == "nord").unwrap();
        let (d, l) = nord.palettes();
        let copy = ThemeDef::from_palettes("Nord copy", "Nord copy", &d, &l);
        let (back, w) = parse(&copy.id, &copy.to_toml()).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.id, "nord-copy");
        let (d2, l2) = back.palettes();
        assert_eq!(d2, d);
        assert_eq!(l2, l);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Rosé Pine"), "rose-pine");
        assert_eq!(slug("  My.Theme  v2 "), "my-theme-v2");
        assert_eq!(slug("../evil"), "evil");
    }
}
