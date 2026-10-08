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

/// Install the fonts with `[appearance] font` / `mono_font` in front of the bundled ones
/// (which stay as fallbacks). Returns warnings for fonts that could not be found.
pub fn install(ctx: &Context, font: &str, mono_font: &str) -> Vec<String> {
    let mut fonts = definitions();
    let mut warnings = Vec::new();
    for (setting, name, bundled, family) in [
        ("font", font, "Inter", FontFamily::Proportional),
        ("mono_font", mono_font, "JetBrains Mono", FontFamily::Monospace),
    ] {
        if is_bundled(name, bundled) {
            continue;
        }
        match find_font(name) {
            Some(bytes) => {
                let key = format!("user-{setting}");
                fonts.font_data.insert(key.clone(), FontData::from_owned(bytes).into());
                fonts.families.entry(family).or_default().insert(0, key);
            }
            None => warnings.push(format!(
                "[appearance] {setting} = \"{name}\": font not found (use a font file name or path); using {bundled}"
            )),
        }
    }
    ctx.set_fonts(fonts);
    warnings
}

/// Normalized font name for matching: lowercase alphanumerics only.
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn is_bundled(name: &str, bundled: &str) -> bool {
    let n = norm(name);
    n.is_empty() || n == norm(bundled)
}

/// Whether a font file name (stem) matches the configured family `want` (normalized):
/// `Cascadia Mono` ↔ `CascadiaMono.ttf`, `Fira Code` ↔ `FiraCode-Regular.otf`.
fn stem_matches(stem: &str, want: &str) -> bool {
    let s = norm(stem);
    s == want || s == format!("{want}regular")
}

/// A font by path, or by family name searched in the system / user font folders.
fn find_font(name: &str) -> Option<Vec<u8>> {
    let path = std::path::Path::new(name.trim());
    if path.is_absolute() {
        return std::fs::read(path).ok();
    }
    let want = norm(name);
    let mut stack: Vec<(std::path::PathBuf, u8)> = font_dirs().into_iter().map(|d| (d, 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 4 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if matches!(ext.as_str(), "ttf" | "otf")
                && p.file_stem().is_some_and(|s| stem_matches(&s.to_string_lossy(), &want))
            {
                return std::fs::read(&p).ok();
            }
        }
    }
    None
}

fn font_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    let home = soyuz_fs::home();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").map(std::path::PathBuf::from);
        v.push(windir.unwrap_or_else(|| "C:\\Windows".into()).join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            v.push(std::path::PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        v.extend(["/System/Library/Fonts", "/Library/Fonts"].map(std::path::PathBuf::from));
        v.push(home.join("Library/Fonts"));
    } else {
        v.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(std::path::PathBuf::from));
        v.push(home.join(".local/share/fonts"));
        v.push(home.join(".fonts"));
    }
    v
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

    #[test]
    fn font_name_matching() {
        assert!(is_bundled("JetBrains Mono", "JetBrains Mono"));
        assert!(is_bundled("  ", "Inter"));
        assert!(!is_bundled("Fira Code", "JetBrains Mono"));
        let want = norm("Cascadia Mono");
        assert!(stem_matches("CascadiaMono", &want));
        assert!(stem_matches("cascadia-mono-regular", &want));
        assert!(!stem_matches("CascadiaMono-Bold", &want));
        assert!(find_font("No Such Font Family 1234").is_none());
    }
}
