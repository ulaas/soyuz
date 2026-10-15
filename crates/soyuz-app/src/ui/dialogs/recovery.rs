//! "Soyuz closed during N operations": the startup recovery dialog for jobs found in a
//! crash journal. Per operation it shows what it was, how far it got and
//! when it started; the user picks Resume (run what is left, skipping what arrived),
//! Roll back (undo the completed part, through the trash) or Discard. Leftover temp files
//! under the touched folders are scanned on a worker thread and can be moved to the trash
//! with the same click. Esc / "Decide later" keeps everything for the next start.

use egui::{Align, CornerRadius, FontId, Layout, Margin, RichText, ScrollArea, Sense, Ui, vec2};
use soyuz_jobs::journal::Recovered;
use soyuz_jobs::{JobKindBrief, JournalJob};

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::models::requests::recovery::{RecoveryModel, job_line};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons::ph, palette};

pub struct RecoveryDialog {
    m: RecoveryModel,
}

impl RecoveryDialog {
    pub fn new(ctx: &egui::Context, recovered: Vec<Recovered>) -> Self {
        let c = ctx.clone();
        RecoveryDialog { m: RecoveryModel::new(recovered, std::sync::Arc::new(move || c.request_repaint())) }
    }
}

fn kind_icon(b: JobKindBrief) -> &'static str {
    match b {
        JobKindBrief::Copy => ph::COPY,
        JobKindBrief::Move => ph::ARROW_SQUARE_RIGHT,
        JobKindBrief::Delete | JobKindBrief::Trash | JobKindBrief::EmptyTrash => ph::TRASH,
        JobKindBrief::Rename => ph::PENCIL_SIMPLE,
        JobKindBrief::Extract | JobKindBrief::Compress => ph::FILE_ZIP,
        JobKindBrief::Sync => ph::ARROWS_CLOCKWISE,
        JobKindBrief::Media => ph::FILM_STRIP,
        _ => ph::GEAR,
    }
}

/// One interrupted operation as a card with a progress bar.
fn job_card(ui: &mut Ui, job: &JournalJob, p: &Palette) {
    egui::Frame::new()
        .fill(p.bg_surface2)
        .stroke(egui::Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(kind_icon(job.kind.brief())).size(18.0).color(p.accent));
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&job.label).color(p.text_primary).font(FontId::proportional(13.0)));
                    let line = job_line(job);
                    ui.label(RichText::new(line).font(FontId::proportional(11.5)).color(p.text_tertiary));
                });
            });
            if let Some(f) = job.fraction() {
                ui.add_space(6.0);
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(2), p.bg_sunken);
                let mut fr = r;
                fr.set_width((r.width() * f).max(2.0));
                ui.painter().rect_filled(fr, CornerRadius::same(2), p.warning);
            }
        });
}

impl Dialog for RecoveryDialog {
    fn title(&self) -> &str {
        &self.m.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::LIFEBUOY)
    }

    fn width(&self) -> f32 {
        540.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(tr!("dialogs-recovery-body")).color(p.text_secondary));
        ui.add_space(10.0);
        ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
            for job in &self.m.jobs {
                job_card(ui, job, &p);
                ui.add_space(6.0);
            }
        });
        ui.add_space(6.0);
        let leftovers = self.m.leftovers();
        match &leftovers {
            None => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0).color(p.text_tertiary));
                    ui.label(
                        RichText::new(tr!("dialogs-recovery-scanning"))
                            .font(FontId::proportional(12.0))
                            .color(p.text_tertiary),
                    );
                });
            }
            Some(v) if v.is_empty() => {
                ui.label(
                    RichText::new(format!("{}  {}", ph::CHECK_CIRCLE, tr!("dialogs-recovery-no-leftovers")))
                        .font(FontId::proportional(12.0))
                        .color(p.success),
                );
            }
            Some(v) => {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.m.clean, tr!("dialogs-recovery-clean", count = v.len()));
                    let t = if self.m.show_leftovers { tr!("dialogs-hide") } else { tr!("dialogs-show") };
                    if ui.link(RichText::new(t).font(FontId::proportional(12.0)).color(p.accent)).clicked() {
                        self.m.show_leftovers = !self.m.show_leftovers;
                    }
                });
                if self.m.show_leftovers {
                    egui::Frame::new()
                        .fill(p.bg_sunken)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ScrollArea::vertical().id_salt("leftovers").max_height(110.0).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                for path in v {
                                    ui.label(
                                        RichText::new(soyuz_fs::strip_verbatim(path).display().to_string())
                                            .font(FontId::monospace(11.0))
                                            .color(p.text_secondary),
                                    );
                                }
                            });
                        });
                }
            }
        }
        ui.add_space(16.0);
        let can_roll_back = self.m.can_roll_back();
        let (mut resume, mut rollback, mut discard, mut later) = (false, false, false, false);
        ui.horizontal(|ui| {
            later = button(ui, tr!("dialogs-recovery-later"), Btn::Ghost)
                .on_hover_text(tr!("dialogs-recovery-later-tip"))
                .clicked();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                resume = button(ui, tr!("dialogs-recovery-resume"), Btn::Primary)
                    .on_hover_text(tr!("dialogs-recovery-resume-tip"))
                    .clicked();
                ui.add_enabled_ui(can_roll_back, |ui| {
                    rollback = button(ui, tr!("dialogs-recovery-rollback"), Btn::Secondary)
                        .on_hover_text(tr!("dialogs-recovery-rollback-tip"))
                        .on_disabled_hover_text(tr!("dialogs-recovery-rollback-none"))
                        .clicked();
                });
                discard = button(ui, tr!("dialogs-recovery-discard"), Btn::Secondary)
                    .on_hover_text(tr!("dialogs-recovery-discard-tip"))
                    .clicked();
            });
        });
        if resume {
            self.m.resume();
            return false;
        }
        if rollback && self.m.roll_back() {
            return false;
        }
        if discard {
            self.m.discard();
            return false;
        }
        !later
    }
}
