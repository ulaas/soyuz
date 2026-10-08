//! "Soyuz closed during N operations": the startup recovery dialog for jobs found in a
//! crash journal (ROADMAP R3). Per operation it shows what it was, how far it got and
//! when it started; the user picks Resume (run what is left, skipping what arrived),
//! Roll back (undo the completed part, through the trash) or Discard. Leftover temp files
//! under the touched folders are scanned on a worker thread and can be moved to the trash
//! with the same click. Esc / "Decide later" keeps everything for the next start.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align, CornerRadius, FontId, Layout, Margin, RichText, ScrollArea, Sense, Ui, vec2};
use parking_lot::Mutex;
use soyuz_core::OpControl;
use soyuz_jobs::journal::{self, Recovered};
use soyuz_jobs::{JobKindBrief, JournalJob};

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::services::crash::{RecoveryAction, request};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons::ph, palette};

/// Visit at most this many entries while looking for leftovers.
const SCAN_LIMIT: usize = 200_000;

pub struct RecoveryDialog {
    title: String,
    recovered: Vec<Recovered>,
    jobs: Vec<JournalJob>,
    /// `None` while the scan runs.
    leftovers: Arc<Mutex<Option<Vec<PathBuf>>>>,
    scan_ctl: Arc<OpControl>,
    clean: bool,
    show_leftovers: bool,
}

impl RecoveryDialog {
    pub fn new(ctx: &egui::Context, recovered: Vec<Recovered>) -> Self {
        let jobs: Vec<JournalJob> = recovered.iter().flat_map(|r| r.jobs.iter().cloned()).collect();
        let n = jobs.len();
        let title = tr!("dialogs-recovery-title", count = n);
        let leftovers = Arc::new(Mutex::new(None));
        let scan_ctl = OpControl::new();
        let (out, ctl, scan_jobs, c) = (leftovers.clone(), scan_ctl.clone(), jobs.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("soyuz-recovery-scan".into()).spawn(move || {
            let found = journal::scan_leftovers(&scan_jobs, &ctl, SCAN_LIMIT);
            *out.lock() = Some(found);
            c.request_repaint();
        });
        if spawned.is_err() {
            *leftovers.lock() = Some(Vec::new());
        }
        RecoveryDialog { title, recovered, jobs, leftovers, scan_ctl, clean: true, show_leftovers: false }
    }

    fn cleanup(&self) -> Vec<PathBuf> {
        if !self.clean {
            return Vec::new();
        }
        self.leftovers.lock().clone().unwrap_or_default()
    }
}

impl Drop for RecoveryDialog {
    fn drop(&mut self) {
        self.scan_ctl.cancel();
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
        _ => ph::GEAR,
    }
}

fn started(job: &JournalJob) -> String {
    soyuz_core::format_time(job.started, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Short))
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
                    let done = match job.fraction() {
                        Some(f) => tr!("dialogs-recovery-done", percent = format!("{:.0}", f * 100.0)),
                        None => tr!("dialogs-recovery-progress-unknown").to_string(),
                    };
                    let line = if job.can_roll_back() {
                        tr!(
                            "dialogs-recovery-job-line-undo",
                            started = started(job),
                            done = done,
                            count = job.records.len()
                        )
                    } else {
                        tr!("dialogs-recovery-job-line", started = started(job), done = done)
                    };
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
        &self.title
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
            for job in &self.jobs {
                job_card(ui, job, &p);
                ui.add_space(6.0);
            }
        });
        ui.add_space(6.0);
        let leftovers = self.leftovers.lock().clone();
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
                    ui.checkbox(&mut self.clean, tr!("dialogs-recovery-clean", count = v.len()));
                    let t = if self.show_leftovers { tr!("dialogs-hide") } else { tr!("dialogs-show") };
                    if ui.link(RichText::new(t).font(FontId::proportional(12.0)).color(p.accent)).clicked() {
                        self.show_leftovers = !self.show_leftovers;
                    }
                });
                if self.show_leftovers {
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
        let can_roll_back = self.jobs.iter().any(JournalJob::can_roll_back);
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
        let dismiss = || self.recovered.clone();
        if resume {
            request(RecoveryAction::Resume { jobs: self.jobs.clone(), cleanup: self.cleanup(), dismiss: dismiss() });
            return false;
        }
        if rollback && can_roll_back {
            request(RecoveryAction::RollBack { jobs: self.jobs.clone(), cleanup: self.cleanup(), dismiss: dismiss() });
            return false;
        }
        if discard {
            request(RecoveryAction::Discard { cleanup: self.cleanup(), dismiss: dismiss() });
            return false;
        }
        !later
    }
}
