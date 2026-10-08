//! Bottom status bar: item counts, theme switcher, renderer and version.

use egui::{Align, Frame, Layout, Margin, Panel, RichText, TextStyle, ThemePreference, Ui};

use crate::theme::tokens::size;
use crate::theme::{icons, icons::ph, palette};

pub fn show(root: &mut Ui, left_text: &str, renderer: &str) {
    let p = palette(root.ctx());
    Panel::bottom("soyuz.statusbar")
        .exact_size(size::STATUS_BAR)
        .frame(Frame::new().fill(p.bg_surface1).inner_margin(Margin::symmetric(12, 0)))
        .show(root, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let small = |s: &str| RichText::new(s).text_style(TextStyle::Small).color(p.text_secondary);
                ui.label(small(left_text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(small(&format!("Soyuz {}", env!("CARGO_PKG_VERSION"))));
                    ui.label(RichText::new("\u{00B7}").color(p.text_tertiary));
                    ui.label(small(renderer));
                    ui.label(RichText::new("\u{00B7}").color(p.text_tertiary));
                    theme_switch(ui);
                });
            });
        });
}

/// Three tiny toggles: system / light / dark.
fn theme_switch(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let current = ui.ctx().options(|o| o.theme_preference);
    ui.spacing_mut().item_spacing.x = 0.0;
    for (pref, glyph, tip) in [
        (ThemePreference::Dark, ph::MOON, "Dark theme"),
        (ThemePreference::Light, ph::SUN, "Light theme"),
        (ThemePreference::System, ph::CIRCLE_HALF, "Follow system theme"),
    ] {
        let active = current == pref;
        let color = if active { p.accent } else { p.text_tertiary };
        let b = egui::Button::new(icons::icon(glyph, 12.0).color(color))
            .frame_when_inactive(false)
            .min_size(egui::vec2(20.0, 18.0));
        if ui.add(b).on_hover_text(tip).clicked() {
            ui.ctx().set_theme(pref);
        }
    }
}
