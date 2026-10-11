//! Delete confirmation: move to trash (when `general.confirm_delete`), permanent delete
//! (Shift+Delete), secure wipe, permanent delete of trashed items and "Empty trash".
//! The state and the answer are the core's [`DeleteConfirmModel`]; this file draws it.

use egui::{Align, CornerRadius, FontId, Key, Layout, Margin, RichText, Ui};

use super::DialogCtx;
use super::conflict::{Btn, button};
pub use crate::models::dialogs::{ConfirmKind, DeleteConfirmModel};
use crate::theme::{icons::ph, palette};

pub(crate) fn icon(m: &DeleteConfirmModel) -> &'static str {
    if m.to_trash() { ph::TRASH } else { ph::WARNING }
}

/// Draw a delete confirmation. Returns `false` to close.
pub(crate) fn show(ui: &mut Ui, m: &mut DeleteConfirmModel, ctx: &mut DialogCtx<'_>) -> bool {
    let p = palette(ui.ctx());
    let (msg, confirm, danger) = m.texts();
    ui.label(RichText::new(msg).color(p.text_secondary));
    let names = m.names();
    if !names.is_empty() {
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(p.bg_sunken)
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for n in names.iter().take(5) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(
                            RichText::new(crate::ui::widgets::shown_name(n))
                                .font(FontId::proportional(12.5))
                                .color(p.text_primary),
                        );
                        crate::ui::widgets::hidden_badge(ui, n);
                    });
                }
                if names.len() > 5 {
                    ui.label(
                        RichText::new(tr!("dialogs-delete-and-more", count = names.len() - 5))
                            .font(FontId::proportional(12.0))
                            .color(p.text_tertiary),
                    );
                }
            });
    }
    ui.add_space(18.0);
    let mut ok = false;
    let mut cancel = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ok = button(ui, confirm, if danger { Btn::Danger } else { Btn::Primary }).clicked();
        cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
    });
    if ui.input(|i| i.key_pressed(Key::Enter)) {
        ok = true;
    }
    if ok {
        if let Some(cmd) = m.confirm() {
            ctx.cmds.push(cmd);
        }
        return false;
    }
    if cancel {
        m.cancel();
    }
    !cancel
}
