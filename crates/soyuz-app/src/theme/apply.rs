//! Export the design tokens into `egui::Style` / `egui::Visuals`.

use egui::style::{ScrollStyle, Selection, WidgetVisuals, Widgets};
use egui::{
    Color32, Context, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, ThemePreference,
    Visuals, vec2,
};

use super::fonts::SEMIBOLD;
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
    pub reduced_motion: bool,
    pub ui_scale: f32,
}

/// Install both palettes as egui styles and select the theme preference.
pub fn apply(ctx: &Context, opts: &ThemeOptions) {
    ctx.set_style_of(Theme::Dark, style(&opts.dark, opts.font_size, opts.reduced_motion));
    ctx.set_style_of(Theme::Light, style(&opts.light, opts.font_size, opts.reduced_motion));
    ctx.set_theme(opts.preference);
    if (opts.ui_scale - 1.0).abs() > f32::EPSILON && opts.ui_scale > 0.25 {
        ctx.set_zoom_factor(opts.ui_scale);
    }
}

/// Map `[appearance] theme` to a preference (`"system" | "dark" | "light"`; user theme
/// names resolve to System until user themes land).
pub fn preference_from_config(theme: &str) -> ThemePreference {
    match theme.trim().to_ascii_lowercase().as_str() {
        "dark" => ThemePreference::Dark,
        "light" => ThemePreference::Light,
        _ => ThemePreference::System,
    }
}

/// The complete style for one palette.
pub fn style(p: &Palette, font_size: f32, reduced_motion: bool) -> egui::Style {
    let base = if p.dark { Theme::Dark } else { Theme::Light };
    let mut s = base.default_style();
    let k = if font_size > 4.0 { font_size / type_scale::BODY } else { 1.0 };
    let prop = |size: f32| FontId::new(size * k, FontFamily::Proportional);
    let semi = |size: f32| FontId::new(size * k, FontFamily::Name(SEMIBOLD.into()));
    s.text_styles = [
        (TextStyle::Small, prop(type_scale::SMALL)),
        (TextStyle::Body, prop(type_scale::BODY)),
        (TextStyle::Button, prop(type_scale::BUTTON)),
        (TextStyle::Heading, semi(type_scale::HEADING)),
        (TextStyle::Monospace, FontId::new(type_scale::MONOSPACE * k, FontFamily::Monospace)),
        (TextStyle::Name(TITLE.into()), semi(type_scale::TITLE)),
        (TextStyle::Name(CAPTION.into()), prop(type_scale::CAPTION)),
        (TextStyle::Name(DISPLAY.into()), semi(type_scale::DISPLAY)),
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

/// Token → `Visuals` mapping (PLAN §1.2 table).
pub fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    let hover_fill = if p.dark { p.bg_surface3 } else { p.bg_sunken };
    v.widgets = Widgets {
        noninteractive: widget(
            p.bg_surface1,
            p.bg_surface1,
            Stroke::new(1.0, p.border_subtle),
            p.text_primary,
            radius::ROW,
        ),
        inactive: widget(p.bg_surface2, p.bg_surface2, Stroke::NONE, p.text_primary, radius::ROW),
        hovered: widget(hover_fill, hover_fill, Stroke::new(1.0, p.border_strong), p.text_primary, radius::ROW),
        active: widget(p.accent_subtle, p.accent_subtle, Stroke::new(1.0, p.accent), p.text_primary, radius::ROW),
        open: widget(p.bg_surface3, p.bg_surface3, Stroke::new(1.0, p.border_strong), p.text_primary, radius::ROW),
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
    v.window_corner_radius = CornerRadius::same(radius::MODAL);
    v.menu_corner_radius = CornerRadius::same(radius::CARD);
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
