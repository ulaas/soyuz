//! Export the design tokens into `egui::Style` / `egui::Visuals`.

use egui::style::{ScrollStyle, Selection, WidgetVisuals, Widgets};
use egui::{
    Color32, Context, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, ThemePreference,
    Visuals, vec2,
};

use soyuz_core::config::Density;

use super::fonts::{CAPTION as CAPTION_FAMILY, DISPLAY as DISPLAY_FAMILY, SEMIBOLD};
use super::tokens::{Palette, motion, radius, type_scale};

/// Named text styles beyond egui's built-ins.
pub const TITLE: &str = "Title";
pub const CAPTION: &str = "Caption";
pub const DISPLAY: &str = "Display";

/// Inputs for [`apply`] (from `[appearance]`).
#[derive(Clone, Debug)]
pub struct ThemeOptions {
    pub preference: ThemePreference,
    pub dark: Palette,
    pub light: Palette,
    /// Base body size (13 pt by default); other sizes scale proportionally.
    pub font_size: f32,
    /// `[fonts.ui] size_offset`: points added to the body / small / button styles.
    pub ui_offset: f32,
    /// `[fonts.ui_semibold] size_offset`: points added to headings / titles.
    pub semibold_offset: f32,
    /// `[fonts.code] size_offset`: points added to the monospace style.
    pub code_offset: f32,
    pub reduced_motion: bool,
    pub ui_scale: f32,
    /// Control sizes (touch mode enlarges every interactive target).
    pub density: Density,
    /// Thicker strokes for widget outlines and focus.
    pub high_contrast: bool,
    /// `[fonts] text_gamma_dark` / `text_gamma_light` (`None`: egui's curve for the theme).
    pub text_gamma: [Option<f32>; 2],
}

/// Install both palettes as egui styles and select the theme preference.
pub fn apply(ctx: &Context, opts: &ThemeOptions) {
    for (theme, p) in [(Theme::Dark, &opts.dark), (Theme::Light, &opts.light)] {
        let mut s = style(p, opts.font_size, opts.reduced_motion);
        offset_text(&mut s, opts.ui_offset, opts.semibold_offset, opts.code_offset);
        tune(&mut s, opts.density, opts.high_contrast);
        // Coverage → opacity: a lower gamma draws grayscale-AA text heavier (egui's own
        // curves: `2c - c²` ≈ 0.5 on dark, linear on light backgrounds).
        if let Some(g) = opts.text_gamma[usize::from(theme == Theme::Light)] {
            s.visuals.text_options.color_transfer_function = egui::epaint::FontColorTransferFunction::Gamma(g);
        }
        ctx.set_style_of(theme, s);
    }
    ctx.set_theme(opts.preference);
    if (opts.ui_scale - 1.0).abs() > f32::EPSILON && opts.ui_scale > 0.25 {
        ctx.set_zoom_factor(opts.ui_scale);
    }
}

/// Add the `[fonts]` role size offsets to the text styles of [`style`].
pub fn offset_text(s: &mut egui::Style, ui: f32, semibold: f32, code: f32) {
    for (ts, f) in s.text_styles.iter_mut() {
        let add = match &f.family {
            FontFamily::Monospace => code,
            FontFamily::Name(n) if n.as_ref() == SEMIBOLD || n.as_ref() == DISPLAY_FAMILY => semibold,
            FontFamily::Name(n) if n.as_ref() == CAPTION_FAMILY => ui,
            _ if matches!(ts, TextStyle::Small | TextStyle::Body | TextStyle::Button)
                || f.family == FontFamily::Proportional =>
            {
                ui
            }
            _ => 0.0,
        };
        f.size = (f.size + add).max(4.0);
    }
}

/// Density / contrast adjustments on top of [`style`].
pub fn tune(s: &mut egui::Style, density: Density, high_contrast: bool) {
    if density == Density::Touch {
        let sp = &mut s.spacing;
        sp.interact_size = vec2(48.0, density.control_height());
        sp.button_padding = vec2(14.0, 9.0);
        sp.item_spacing = vec2(10.0, 8.0);
        sp.icon_width = 22.0;
        sp.icon_width_inner = 14.0;
        sp.scroll.bar_width = 14.0;
        sp.combo_height = 320.0;
    }
    if high_contrast {
        let w = &mut s.visuals.widgets;
        for v in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            v.bg_stroke.width = v.bg_stroke.width.max(1.0) + 1.0;
            v.fg_stroke.width = 1.5;
        }
        w.inactive.bg_stroke.color = w.noninteractive.bg_stroke.color;
        s.visuals.selection.stroke.width = 2.0;
        s.visuals.striped = false;
    }
}

/// Map `[appearance] theme` to a preference (`"system" | "dark" | "light"`; anything
/// else follows the system).
pub fn preference_from_config(theme: &str) -> ThemePreference {
    crate::host::to_egui_theme(crate::host::ThemePref::from_config(theme))
}

/// The complete style for one palette.
pub fn style(p: &Palette, font_size: f32, reduced_motion: bool) -> egui::Style {
    let base = if p.dark { Theme::Dark } else { Theme::Light };
    let mut s = base.default_style();
    let k = if font_size > 4.0 { font_size / type_scale::BODY } else { 1.0 };
    let prop = |size: f32| FontId::new(size * k, FontFamily::Proportional);
    let semi = |size: f32| FontId::new(size * k, FontFamily::Name(SEMIBOLD.into()));
    // Small text and large headings have families of their own so a variable ui font
    // can draw them at another optical size (`fonts::CAPTION` / `fonts::DISPLAY`).
    let caption = |size: f32| FontId::new(size * k, FontFamily::Name(CAPTION_FAMILY.into()));
    let display = |size: f32| FontId::new(size * k, FontFamily::Name(DISPLAY_FAMILY.into()));
    s.text_styles = [
        (TextStyle::Small, caption(type_scale::SMALL)),
        (TextStyle::Body, prop(type_scale::BODY)),
        (TextStyle::Button, prop(type_scale::BUTTON)),
        (TextStyle::Heading, display(type_scale::HEADING)),
        (TextStyle::Monospace, FontId::new(type_scale::MONOSPACE * k, FontFamily::Monospace)),
        (TextStyle::Name(TITLE.into()), semi(type_scale::TITLE)),
        (TextStyle::Name(CAPTION.into()), caption(type_scale::CAPTION)),
        (TextStyle::Name(DISPLAY.into()), display(type_scale::DISPLAY)),
    ]
    .into();

    let sp = &mut s.spacing;
    sp.item_spacing = vec2(8.0, 4.0);
    sp.button_padding = vec2(10.0, 5.0);
    sp.window_margin = Margin::same(16);
    sp.menu_margin = Margin::same(6);
    sp.indent = 16.0;
    sp.interact_size = vec2(40.0, 24.0);
    sp.icon_width = 16.0;
    sp.icon_width_inner = 10.0;
    sp.icon_spacing = 6.0;
    sp.tooltip_width = 360.0;
    sp.menu_spacing = 4.0;
    sp.scroll =
        ScrollStyle { bar_width: 8.0, floating: true, floating_allocated_width: 0.0, ..ScrollStyle::floating() };

    s.animation_time = if reduced_motion { 0.0 } else { motion::HOVER };
    s.visuals = visuals(p);
    s
}

fn widget(bg: Color32, weak_bg: Color32, stroke: Stroke, fg: Color32, r: u8) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: weak_bg,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(r),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    }
}

/// Token → `Visuals` mapping.
pub fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    let hover_fill = if p.dark { p.bg_surface3 } else { p.bg_sunken };
    let row = p.radius(radius::ROW);
    v.widgets = Widgets {
        noninteractive: widget(p.bg_surface1, p.bg_surface1, Stroke::new(1.0, p.border_subtle), p.text_primary, row),
        inactive: widget(p.bg_surface2, p.bg_surface2, Stroke::NONE, p.text_primary, row),
        hovered: widget(hover_fill, hover_fill, Stroke::new(1.0, p.border_strong), p.text_primary, row),
        active: widget(p.accent_subtle, p.accent_subtle, Stroke::new(1.0, p.accent), p.text_primary, row),
        open: widget(p.bg_surface3, p.bg_surface3, Stroke::new(1.0, p.border_strong), p.text_primary, row),
    };
    v.selection = Selection { bg_fill: p.accent_subtle, stroke: Stroke::new(1.0, p.accent) };
    v.weak_text_color = Some(p.text_secondary);
    v.hyperlink_color = p.accent;
    v.faint_bg_color = p.bg_surface1;
    v.extreme_bg_color = p.bg_sunken;
    v.code_bg_color = p.bg_sunken;
    v.text_edit_bg_color = Some(p.bg_surface2);
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_corner_radius = CornerRadius::same(p.radius(radius::MODAL));
    v.menu_corner_radius = CornerRadius::same(p.radius(radius::CARD));
    v.window_fill = p.bg_surface3;
    v.window_stroke = Stroke::new(1.0, p.border_subtle);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
    };
    v.panel_fill = p.bg_surface1;
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.striped = true;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::{ORBIT_DARK, ORBIT_LIGHT};

    #[test]
    fn styles_follow_tokens() {
        let s = style(&ORBIT_DARK, 13.0, false);
        assert_eq!(s.visuals.panel_fill, ORBIT_DARK.bg_surface1);
        assert_eq!(s.visuals.selection.bg_fill, ORBIT_DARK.accent_subtle);
        assert_eq!(s.text_styles[&TextStyle::Body].size, 13.0);
        assert_eq!(s.text_styles[&TextStyle::Name(DISPLAY.into())].size, 22.0);
        assert!(s.visuals.dark_mode);
        let l = style(&ORBIT_LIGHT, 15.0, true);
        assert!(!l.visuals.dark_mode);
        assert_eq!(l.animation_time, 0.0);
        assert!((l.text_styles[&TextStyle::Body].size - 15.0).abs() < 1e-4);
    }

    #[test]
    fn preferences() {
        assert_eq!(preference_from_config("Dark"), ThemePreference::Dark);
        assert_eq!(preference_from_config("light"), ThemePreference::Light);
        assert_eq!(preference_from_config("system"), ThemePreference::System);
        assert_eq!(preference_from_config("my-theme"), ThemePreference::System);
    }
}
