//! The Soyuz design system: tokens, egui style export, fonts and icons.

pub mod accent;
pub mod apply;
pub mod font_fallback;
pub mod fonts;
pub mod icons;
pub mod library;
pub mod sysfonts;
pub mod themes;
pub mod tokens;

use egui::{Context, Theme};
use soyuz_core::config::{Appearance, FontRoleKind, Fonts};

pub use apply::{ThemeOptions, preference_from_config};
pub use tokens::{ORBIT_DARK, ORBIT_HC_DARK, ORBIT_HC_LIGHT, ORBIT_LIGHT, Palette};

/// The active palette for the context's current theme (set by [`install`]).
pub fn palette(ctx: &Context) -> Palette {
    ctx.data(|d| d.get_temp::<Palettes>(palettes_id())).map_or_else(
        || if ctx.theme() == Theme::Dark { ORBIT_DARK.clone() } else { ORBIT_LIGHT.clone() },
        |p| if ctx.theme() == Theme::Dark { p.dark } else { p.light },
    )
}

#[derive(Clone)]
struct Palettes {
    dark: Palette,
    light: Palette,
}

/// Fonts + styles for both themes from `[appearance]`, `[fonts]` and `[theme.colors]`.
/// Returns warnings for invalid color overrides and missing fonts.
pub fn install(
    ctx: &Context,
    appearance: &Appearance,
    font_roles: &Fonts,
    overrides: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    font_fallback::configure(ctx, appearance.fallback_fonts, font_roles);
    library::start(ctx);
    let font_warnings = fonts::install(ctx, font_roles);
    let (mut dark, mut light, mut warnings) = resolve(appearance);
    warnings.extend(dark.apply_overrides(overrides));
    warnings.extend(font_warnings);
    // The same overrides apply to both palettes; report problems only once.
    let _ = light.apply_overrides(overrides);
    if crate::ui::native_chrome::backdrop_active() {
        // A Windows 11 system backdrop shows through the window-level fills.
        dark.translucent();
        light.translucent();
    }
    let opts = ThemeOptions {
        preference: preference_from_config(&appearance.theme),
        dark: dark.clone(),
        light: light.clone(),
        font_size: appearance.font_size,
        ui_offset: font_roles.role(FontRoleKind::Ui).offset(),
        semibold_offset: font_roles.role(FontRoleKind::UiSemibold).offset(),
        code_offset: font_roles.role(FontRoleKind::Code).offset(),
        reduced_motion: appearance.reduced_motion,
        ui_scale: appearance.ui_scale,
        density: appearance.density,
        high_contrast: appearance.high_contrast,
    };
    set_reduced_motion(ctx, appearance.reduced_motion);
    apply::apply(ctx, &opts);
    ctx.data_mut(|d| d.insert_temp(palettes_id(), Palettes { dark, light }));
    warnings.dedup();
    warnings
}

/// The dark and light palettes for `[appearance]` (color theme, high contrast, accent),
/// before `[theme.colors]` overrides. Returns warnings for unknown themes / accents.
pub fn resolve(appearance: &Appearance) -> (Palette, Palette, Vec<String>) {
    let mut warnings = Vec::new();
    let (mut dark, mut light) = if appearance.high_contrast {
        (ORBIT_HC_DARK.clone(), ORBIT_HC_LIGHT.clone())
    } else {
        library::ensure_loaded(&appearance.color_theme);
        match library::find(&appearance.color_theme) {
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
    let (choice, w) = accent::AccentChoice::parse(&appearance.accent);
    warnings.extend(w);
    let color = match choice {
        accent::AccentChoice::Theme => None,
        accent::AccentChoice::System => library::os_accent(),
        accent::AccentChoice::Custom(c) => Some(c),
    };
    if let Some(c) = color {
        dark.set_accent(c);
        light.set_accent(c);
    }
    (dark, light, warnings)
}

fn palettes_id() -> egui::Id {
    egui::Id::new("soyuz.theme.palettes")
}

fn motion_id() -> egui::Id {
    egui::Id::new("soyuz.theme.reduced_motion")
}

/// Remember `[appearance] reduced_motion` for [`anim`].
pub fn set_reduced_motion(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(motion_id(), on));
}

/// Whether animations are disabled (`[appearance] reduced_motion`).
pub fn reduced_motion(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(motion_id())).unwrap_or(false)
}

/// An animation duration that respects reduced motion (0 = jump to the end state).
pub fn anim(ctx: &Context, secs: f32) -> f32 {
    if reduced_motion(ctx) { 0.0 } else { secs }
}
