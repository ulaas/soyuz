//! Bundled fonts (Inter, JetBrains Mono — both OFL, see `assets/fonts/*LICENSE*`) and the
//! Phosphor icon fonts.

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// Font family used for headings / titles (Inter SemiBold).
pub const SEMIBOLD: &str = "semibold";
/// Phosphor Regular as a dedicated family (icon font first). Always draw icons with
/// this family (or [`PHOSPHOR_FILL`]): Inter defines glyphs in the Private Use Area that
/// would otherwise shadow Phosphor's codepoints in plain proportional text.
pub const ICONS: &str = "icons";
/// Phosphor Fill variant, used for the active sidebar item.
pub const PHOSPHOR_FILL: &str = "phosphor-fill";

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

/// Build the font definitions: Inter / JetBrains Mono first, egui defaults kept as
/// fallback (emoji), Phosphor Regular mixed into proportional text, plus the
/// `semibold` and `phosphor-fill` families.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("Inter".into(), FontData::from_static(INTER_REGULAR).into());
    fonts.font_data.insert("Inter-SemiBold".into(), FontData::from_static(INTER_SEMIBOLD).into());
    fonts.font_data.insert("JetBrainsMono".into(), FontData::from_static(JETBRAINS_MONO).into());
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "Inter".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "JetBrainsMono".into());

    // Phosphor Regular goes in at index 1 of Proportional (after Inter).
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    egui_phosphor::add_font_bytes_as_family(&mut fonts, PHOSPHOR_FILL, egui_phosphor::Variant::Fill.font_bytes());

    // SemiBold family: same fallbacks as Proportional with Inter SemiBold first.
    let mut semibold: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f != "Inter")
        .collect();
    semibold.insert(0, "Inter-SemiBold".into());
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);

    // Icon families: the icon font *first*, with the regular fallbacks after it.
    let fallbacks: Vec<String> =
        fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default().into_iter().skip(2).collect();
    let with_fallbacks = |first: &str| std::iter::once(first.to_string()).chain(fallbacks.iter().cloned()).collect();
    fonts.families.insert(FontFamily::Name(ICONS.into()), with_fallbacks("phosphor"));
    fonts.families.insert(FontFamily::Name(PHOSPHOR_FILL.into()), with_fallbacks(PHOSPHOR_FILL));
    fonts
}

pub fn install(ctx: &Context) {
    ctx.set_fonts(definitions());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_ordered() {
        let f = definitions();
        let prop = &f.families[&FontFamily::Proportional];
        assert_eq!(prop[0], "Inter");
        assert_eq!(prop[1], "phosphor");
        assert_eq!(f.families[&FontFamily::Monospace][0], "JetBrainsMono");
        assert_eq!(f.families[&FontFamily::Name(SEMIBOLD.into())][0], "Inter-SemiBold");
        assert_eq!(f.families[&FontFamily::Name(ICONS.into())][0], "phosphor");
        assert_eq!(f.families[&FontFamily::Name(PHOSPHOR_FILL.into())][0], PHOSPHOR_FILL);
    }
}
