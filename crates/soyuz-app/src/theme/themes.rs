//! Theme files: a named token palette with a dark and a light variant, an optional
//! accent and a corner-radius scale. The format, the parser and the built-ins live
//! toolkit-free in `soyuz_appcore::theme::themes`; this module adds the egui palettes.

use egui::Color32;

pub use soyuz_appcore::theme::themes::{
    BUILTIN_SOURCES, Base, MAX_THEME_BYTES, RADIUS_SCALE, ThemeDef, builtins, parse, slug,
};

use super::tokens::Palette;

/// The dark and light egui palettes of a theme.
pub fn palettes(t: &ThemeDef) -> (Palette, Palette) {
    let (d, l) = t.palettes();
    (Palette::from_tokens(&d), Palette::from_tokens(&l))
}

/// `#RRGGBB`, or `#RRGGBBAA` for translucent colors (unmultiplied).
pub fn hex_string(c: Color32) -> String {
    crate::color::hex_string(super::tokens::rgba8(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::{ORBIT_DARK, ORBIT_HC_DARK, ORBIT_LIGHT};

    #[test]
    fn builtin_palettes() {
        let orbit = &builtins()[0];
        assert_eq!(palettes(orbit), (ORBIT_DARK.clone(), ORBIT_LIGHT.clone()));
        let hc = builtins().iter().find(|t| t.id == "high-contrast").unwrap();
        assert_eq!(palettes(hc).0.bg_window, ORBIT_HC_DARK.bg_window);
        assert_eq!(hex_string(Color32::from_rgb(1, 2, 255)), "#0102FF");
    }
}
