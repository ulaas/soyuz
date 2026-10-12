//! UI of scripts and automations: the approval card for a script's planned
//! changes, and the Scripts / Automations settings pages.

pub mod approval;
pub mod rules_page;
pub mod scripts_page;

use egui::{CornerRadius, Frame, Margin, Stroke, Ui};

use crate::theme::palette;

/// A settings card (same look as the custom-command cards).
pub(crate) fn card(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    Frame::new()
        .fill(p.bg_surface2.gamma_multiply(if p.dark { 0.7 } else { 1.0 }))
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        });
}
