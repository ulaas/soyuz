//! Crash reports offered on the next start (ROADMAP R3): view the report, open the folder,
//! delete the reports or restart in safe mode. Reports never leave the computer.

use egui::{Align, CornerRadius, FontId, Layout, Margin, RichText, ScrollArea, Ui};

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::services::crash::{CrashReport, RecoveryAction, request, safe_mode};
use crate::theme::{icons::ph, palette};

/// Shown text is capped (reports with huge log lines stay readable).
const VIEW_CAP: usize = 64 * 1024;

pub struct CrashDialog {
    title: String,
    reports: Vec<CrashReport>,
    /// Index of the report being viewed and its text.
    viewing: Option<(usize, String)>,
}

impl CrashDialog {
    pub fn new(reports: Vec<CrashReport>) -> Self {
        let title = if reports.len() == 1 {
            "Soyuz closed unexpectedly".to_string()
        } else {
            format!("Soyuz recorded {} crash reports", reports.len())
        };
        CrashDialog { title, reports, viewing: None }
    }

    fn view(&mut self, i: usize) {
        if self.viewing.as_ref().is_some_and(|(v, _)| *v == i) {
            self.viewing = None;
            return;
        }
        let mut text = std::fs::read_to_string(&self.reports[i].path).unwrap_or_else(|e| format!("({e})"));
        if text.len() > VIEW_CAP {
            let mut cut = VIEW_CAP;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str("\n… (open the file to see the rest)");
        }
        self.viewing = Some((i, text));
    }
}

impl Dialog for CrashDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::BUG)
    }

    fn width(&self) -> f32 {
        560.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(
            RichText::new(
                "A crash report was saved on this computer. It contains the version, the operating system, \
                 where the problem happened and the last log lines. It is never sent anywhere.",
            )
            .color(p.text_secondary),
        );
        ui.add_space(10.0);
        let mut view = None;
        for (i, r) in self.reports.iter().enumerate().take(8) {
            ui.horizontal(|ui| {
                let when = r
                    .when
                    .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%b %-d, %H:%M").to_string())
                    .unwrap_or_default();
                ui.label(RichText::new(when).font(FontId::proportional(12.0)).color(p.text_tertiary));
                let summary = if r.summary.is_empty() { r.name.clone() } else { r.summary.clone() };
                ui.label(RichText::new(summary).font(FontId::proportional(12.5)).color(p.text_primary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = self.viewing.as_ref().is_some_and(|(v, _)| *v == i);
                    if ui.link(RichText::new(if open { "Hide" } else { "View" }).color(p.accent)).clicked() {
                        view = Some(i);
                    }
                });
            });
        }
        if let Some(i) = view {
            self.view(i);
        }
        if let Some((_, text)) = &self.viewing {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(p.bg_sunken)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ScrollArea::both().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.label(RichText::new(text).font(FontId::monospace(11.0)).color(p.text_secondary));
                    });
                });
        }
        ui.add_space(16.0);
        let (mut close, mut delete, mut folder, mut safe) = (false, false, false, false);
        ui.horizontal(|ui| {
            delete = button(ui, "Delete reports", Btn::Ghost).clicked();
            folder =
                button(ui, "Open folder", Btn::Ghost).on_hover_text("Show the crashes folder in a new tab").clicked();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                close = button(ui, "Close", Btn::Primary).clicked();
                if !safe_mode() {
                    safe = button(ui, "Restart in safe mode", Btn::Secondary)
                        .on_hover_text("Default settings; no session restore, watchers, thumbnails or git")
                        .clicked();
                }
            });
        });
        if delete {
            request(RecoveryAction::DeleteReports(self.reports.clone()));
            return false;
        }
        if folder {
            request(RecoveryAction::KeepReports(self.reports.clone()));
            request(RecoveryAction::OpenCrashFolder);
            return false;
        }
        if safe {
            request(RecoveryAction::KeepReports(self.reports.clone()));
            request(RecoveryAction::RestartSafeMode);
            return false;
        }
        if close {
            request(RecoveryAction::KeepReports(self.reports.clone()));
            return false;
        }
        true
    }
}
