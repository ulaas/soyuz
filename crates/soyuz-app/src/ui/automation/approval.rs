//! Approval card for the changes a script action planned (`[scripting] approve_writes`):
//! the action, every planned job, and Run / Cancel. Run submits them as normal jobs
//! (activity panel, conflicts, undo); Cancel, Esc or clicking outside drops them.

use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke, Ui};
use soyuz_jobs::JobKindBrief;
use soyuz_script::PlannedJob;

use crate::automation::AutoCmd;
use crate::cmd::Cmd;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dialogs::{Dialog, DialogCtx};
use crate::ui::widgets::pill_button;

const SHOWN: usize = 14;

pub struct ScriptApprovalDialog {
    action: String,
    jobs: Vec<PlannedJob>,
    title: String,
    destructive: bool,
}

impl ScriptApprovalDialog {
    pub fn new(action: String, jobs: Vec<PlannedJob>) -> Self {
        let destructive = jobs.iter().any(|j| matches!(j.kind.brief(), JobKindBrief::Trash | JobKindBrief::Delete));
        let title = format!("“{action}” wants to make {} change{}", jobs.len(), if jobs.len() == 1 { "" } else { "s" });
        ScriptApprovalDialog { action, jobs, title, destructive }
    }
}

fn glyph(b: JobKindBrief) -> &'static str {
    match b {
        JobKindBrief::Copy => ph::COPY,
        JobKindBrief::Move => ph::ARROW_SQUARE_RIGHT,
        JobKindBrief::Trash | JobKindBrief::Delete => ph::TRASH,
        JobKindBrief::Rename => ph::TEXT_T,
        JobKindBrief::Create => ph::FOLDER_PLUS,
        _ => ph::GEAR_SIX,
    }
}

impl Dialog for ScriptApprovalDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(if self.destructive { ph::SHIELD_WARNING } else { ph::CODE })
    }

    fn width(&self) -> f32 {
        560.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(
            RichText::new("A script asked for these changes. Nothing has happened yet.")
                .size(12.5)
                .color(p.text_secondary),
        );
        ui.add_space(10.0);
        Frame::new()
            .fill(p.bg_surface2)
            .stroke(Stroke::new(1.0, p.border_subtle))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                    for j in self.jobs.iter().take(SHOWN) {
                        ui.horizontal(|ui| {
                            let b = j.kind.brief();
                            let danger = matches!(b, JobKindBrief::Trash | JobKindBrief::Delete);
                            ui.label(icons::icon(glyph(b), 14.0).color(if danger { p.danger } else { p.accent }));
                            // Names come from disk: plain text, never interpreted.
                            ui.add(
                                egui::Label::new(RichText::new(&j.summary).size(12.5).color(p.text_primary)).truncate(),
                            );
                        });
                    }
                    if self.jobs.len() > SHOWN {
                        ui.label(
                            RichText::new(format!("…and {} more", self.jobs.len() - SHOWN))
                                .size(11.5)
                                .color(p.text_tertiary),
                        );
                    }
                });
            });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Each change runs as a background job you can pause, cancel and undo. Turn this card off in Settings → Scripts.",
            )
            .size(11.5)
            .color(p.text_tertiary),
        );
        ui.add_space(14.0);
        let mut keep = true;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if pill_button(ui, Some(ph::PLAY), "Run", true).clicked() {
                ctx.cmds.push(Cmd::Automation(AutoCmd::SubmitPlanned {
                    title: self.action.clone(),
                    jobs: std::mem::take(&mut self.jobs),
                }));
                keep = false;
            }
            ui.add_space(6.0);
            if pill_button(ui, None, "Cancel", false).clicked() {
                keep = false;
            }
        });
        keep
    }
}
