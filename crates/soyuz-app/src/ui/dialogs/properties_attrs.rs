//! Attribute and permission editing shared by the Properties dialog and the bulk
//! "Attributes & timestamps" dialog: the egui side of
//! `crate::models::attrs_edit` (tri-state attribute flags, the unix mode grid with a
//! two-way octal field). The state, the octal / symbolic forms and the conversion of the
//! edits into `SetMeta` jobs live in the core.

use egui::{Align2, FontId, Margin, RichText, Sense, TextEdit, Ui, pos2, vec2};

use super::properties_widgets::{caption, tri_check};
use crate::theme::palette;

pub use crate::models::attrs_edit::{
    AttrEdit, MODE_BITS, MetaJob, ModeEdit, Tri, editable_attrs, info_attrs, mode_string, mode_to_octal, parse_octal,
    plan_jobs,
};

/// Draws an [`AttrEdit`].
pub trait AttrEditUi {
    fn show(&mut self, ui: &mut Ui);
}

impl AttrEditUi for AttrEdit {
    fn show(&mut self, ui: &mut Ui) {
        let labels = editable_attrs();
        let p = palette(ui.ctx());
        for (row, (_, label, hint)) in self.rows.iter_mut().zip(labels.iter()) {
            ui.horizontal(|ui| {
                let mut t = row.1;
                if tri_check(ui, &mut t, label, true).on_hover_text(*hint).clicked() {
                    row.1 = t;
                }
                if row.1 != row.2 {
                    ui.label(
                        RichText::new(tr!("dialogs-props-changed")).font(FontId::proportional(10.5)).color(p.accent),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(*hint).font(FontId::proportional(11.0)).color(p.text_tertiary));
                });
            });
        }
    }
}

/// Draws a [`ModeEdit`].
pub trait ModeEditUi {
    fn show(&mut self, ui: &mut Ui);
}

impl ModeEditUi for ModeEdit {
    fn show(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        caption(ui, tr!("dialogs-mode-permissions"));
        let col_w = 64.0;
        let label_w = 70.0;
        let (head, _) = ui.allocate_exact_size(vec2(label_w + col_w * 3.0, 18.0), Sense::hover());
        let heads = [tr!("dialogs-mode-read"), tr!("dialogs-mode-write"), tr!("dialogs-mode-execute")];
        for (i, h) in heads.iter().enumerate() {
            ui.painter().text(
                pos2(head.left() + label_w + col_w * i as f32 + 8.0, head.center().y),
                Align2::LEFT_CENTER,
                *h,
                FontId::proportional(11.0),
                p.text_tertiary,
            );
        }
        let mut changed = false;
        let whos = [tr!("dialogs-mode-owner"), tr!("dialogs-mode-group"), tr!("dialogs-mode-others")];
        for (row, who) in whos.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (r, _) = ui.allocate_exact_size(vec2(label_w, 22.0), Sense::hover());
                ui.painter().text(
                    pos2(r.left(), r.center().y),
                    Align2::LEFT_CENTER,
                    *who,
                    FontId::proportional(12.5),
                    p.text_secondary,
                );
                for col in 0..3 {
                    let idx = 3 + row * 3 + col;
                    ui.allocate_ui(vec2(col_w, 22.0), |ui| {
                        let mut t = self.bits[idx];
                        if tri_check(ui, &mut t, "", true).clicked() {
                            self.bits[idx] = t;
                            changed = true;
                        }
                    });
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let specials =
                [(0, tr!("dialogs-mode-setuid")), (1, tr!("dialogs-mode-setgid")), (2, tr!("dialogs-mode-sticky"))];
            for (i, label) in specials {
                let mut t = self.bits[i];
                if tri_check(ui, &mut t, label, true).clicked() {
                    self.bits[i] = t;
                    changed = true;
                }
                ui.add_space(10.0);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("dialogs-mode-octal")).color(p.text_tertiary));
            let r = ui.add(
                TextEdit::singleline(&mut self.octal)
                    .desired_width(64.0)
                    .font(FontId::monospace(12.5))
                    .hint_text(tr!("dialogs-mode-mixed"))
                    .margin(Margin::symmetric(8, 4)),
            );
            self.octal_focus = r.has_focus();
            if r.changed()
                && let Some(m) = parse_octal(&self.octal)
            {
                for (i, b) in MODE_BITS.iter().enumerate() {
                    self.bits[i] = if m & b != 0 { Tri::On } else { Tri::Off };
                }
            }
            if let Some(m) = self.uniform() {
                ui.label(RichText::new(mode_string(m)).font(FontId::monospace(12.5)).color(p.text_secondary));
            }
            if self.changed() {
                ui.label(RichText::new(tr!("dialogs-props-changed")).font(FontId::proportional(10.5)).color(p.accent));
            }
        });
        if changed && !self.octal_focus {
            self.sync_octal();
        }
    }
}
