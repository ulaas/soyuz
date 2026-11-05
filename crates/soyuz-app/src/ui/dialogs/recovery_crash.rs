//! Crash reports offered on the next start: view the report, open the folder,
//! delete the reports or restart in safe mode. Reports never leave the computer.

use egui::{Align, Layout, Margin, RichText, ScrollArea, Ui};

use super::{Dialog, DialogCtx};
use crate::models::requests::crash_reports::{self as model, CrashReportsModel};
use crate::services::crash::{CrashReport, safe_mode};
use crate::theme::{icons::ph, palette};
use crate::ui::widgets::{Btn, pill_button};

pub struct CrashDialog {
    m: CrashReportsModel,
}

impl CrashDialog {
    pub fn new(reports: Vec<CrashReport>) -> Self {
        CrashDialog { m: CrashReportsModel::new(reports) }
    }
}

impl Dialog for CrashDialog {
    fn title(&self) -> &str {
        &self.m.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::BUG)
    }

    fn width(&self) -> f32 {
        560.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(tr!("dialogs-crash-body")).color(p.text_secondary));
        ui.add_space(10.0);
        let mut view = None;
        for (i, r) in self.m.listed().iter().enumerate() {
            ui.horizontal(|ui| {
                let when = model::when(r);
                ui.label(RichText::new(when).font(crate::theme::font::ui(12.0)).color(p.text_tertiary));
                let summary = model::summary(r);
                ui.label(RichText::new(summary).font(crate::theme::font::ui(12.5)).color(p.text_primary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = self.m.is_viewing(i);
                    if ui
                        .link(
                            RichText::new(if open { tr!("dialogs-hide") } else { tr!("dialogs-crash-view") })
                                .color(p.accent),
                        )
                        .clicked()
                    {
                        view = Some(i);
                    }
                });
            });
        }
        if let Some(i) = view {
            self.m.toggle_view(i);
        }
        if let Some((_, text)) = &self.m.viewing {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(p.bg_sunken)
                .corner_radius(crate::theme::tokens::radius::same(6))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ScrollArea::both().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.label(RichText::new(text).font(crate::theme::font::mono(11.0)).color(p.text_secondary));
                    });
                });
        }
        ui.add_space(16.0);
        let (mut close, mut delete, mut folder, mut safe) = (false, false, false, false);
        ui.horizontal(|ui| {
            delete = pill_button(ui, None, tr!("dialogs-crash-delete"), Btn::Ghost).clicked();
            folder = pill_button(ui, None, tr!("dialogs-crash-open-folder"), Btn::Ghost)
                .on_hover_text(tr!("dialogs-crash-open-folder-tip"))
                .clicked();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                close = pill_button(ui, None, tr!("common-close"), Btn::Primary).clicked();
                if !safe_mode() {
                    safe = pill_button(ui, None, tr!("dialogs-crash-safe-mode"), Btn::Secondary)
                        .on_hover_text(tr!("dialogs-crash-safe-mode-tip"))
                        .clicked();
                }
            });
        });
        if delete {
            self.m.delete();
            return false;
        }
        if folder {
            self.m.open_folder();
            return false;
        }
        if safe {
            self.m.restart_safe_mode();
            return false;
        }
        if close {
            self.m.keep();
            return false;
        }
        true
    }
}
