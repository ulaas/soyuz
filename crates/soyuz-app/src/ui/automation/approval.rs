//! Approval card for the changes a script action planned (`[scripting] approve_writes`):
//! the action, every planned job, and Run / Cancel. Run submits them as normal jobs
//! (activity panel, conflicts, undo); Cancel, Esc or clicking outside drops them.

use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke, Ui};
use soyuz_jobs::JobKindBrief;

use crate::models::dialogs::ScriptApprovalModel;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dialogs::DialogCtx;
use crate::ui::widgets::pill_button;

const SHOWN: usize = 14;

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

pub(crate) fn icon(m: &ScriptApprovalModel) -> &'static str {
    if m.destructive { ph::SHIELD_WARNING } else { ph::CODE }
}

/// Draw the script approval card. Returns `false` to close.
pub(crate) fn show(ui: &mut Ui, m: &mut ScriptApprovalModel, ctx: &mut DialogCtx<'_>) -> bool {
    let p = palette(ui.ctx());
    ui.label(RichText::new(tr!("automation-approval-intro")).size(12.5).color(p.text_secondary));
    ui.add_space(10.0);
    Frame::new()
        .fill(p.bg_surface2)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                for j in m.jobs.iter().take(SHOWN) {
                    ui.horizontal(|ui| {
                        let b = j.kind.brief();
                        let danger = matches!(b, JobKindBrief::Trash | JobKindBrief::Delete);
                        ui.label(icons::icon(glyph(b), 14.0).color(if danger { p.danger } else { p.accent }));
                        // Names come from disk: plain text, never interpreted.
                        ui.add(egui::Label::new(RichText::new(&j.summary).size(12.5).color(p.text_primary)).truncate());
                    });
                }
                if m.jobs.len() > SHOWN {
                    ui.label(
                        RichText::new(tr!("automation-and-more", count = m.jobs.len() - SHOWN))
                            .size(11.5)
                            .color(p.text_tertiary),
                    );
                }
            });
        });
    ui.add_space(8.0);
    ui.label(RichText::new(tr!("automation-approval-note")).size(11.5).color(p.text_tertiary));
    ui.add_space(14.0);
    let mut keep = true;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if pill_button(ui, Some(ph::PLAY), tr!("automation-approval-run"), true).clicked() {
            if let Some(cmd) = m.run() {
                ctx.cmds.push(cmd);
            }
            keep = false;
        }
        ui.add_space(6.0);
        if pill_button(ui, None, tr!("common-cancel"), false).clicked() {
            m.cancel();
            keep = false;
        }
    });
    keep
}
