//! Centered empty-state illustration used by the content area.

use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, TextStyle, Ui, vec2};

use crate::theme::apply::{CAPTION, DISPLAY};
use crate::theme::palette;
use crate::ui::widgets::kbd;

pub struct EmptyState<'a> {
    pub glyph: &'a str,
    pub glyph_color: Option<Color32>,
    pub title: &'a str,
    pub subtitle: &'a str,
    /// `(shortcut, description)` hints shown under the subtitle.
    pub hints: &'a [(&'a str, &'a str)],
}

impl EmptyState<'_> {
    pub fn show(self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let block_height = 96.0 + 16.0 + 30.0 + 22.0 + 20.0 + self.hints.len() as f32 * 28.0;
        let top = ((ui.available_height() - block_height) / 2.0 - 24.0).max(16.0);
        ui.add_space(top);
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            // Glyph on a soft disc.
            let (rect, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
            let tint = self.glyph_color.unwrap_or(p.accent);
            ui.painter().rect_filled(rect, CornerRadius::same(28), tint.gamma_multiply(0.12));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                self.glyph,
                crate::theme::icons::font_fill(48.0),
                tint,
            );
            ui.add_space(16.0);
            ui.label(RichText::new(self.title).text_style(TextStyle::Name(DISPLAY.into())).color(p.text_primary));
            ui.add_space(4.0);
            ui.label(RichText::new(self.subtitle).text_style(TextStyle::Name(CAPTION.into())).color(p.text_secondary));
            ui.add_space(20.0);
            for (keys, what) in self.hints {
                ui.horizontal(|ui| {
                    // Center the row manually: measure roughly and pad.
                    let approx = keys.len() as f32 * 6.5 + what.len() as f32 * 6.5 + 40.0;
                    ui.add_space(((ui.available_width() - approx) / 2.0).max(0.0));
                    kbd(ui, keys);
                    ui.add_space(4.0);
                    ui.label(RichText::new(*what).color(p.text_tertiary));
                });
                ui.add_space(4.0);
            }
        });
    }
}
