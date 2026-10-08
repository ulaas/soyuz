//! Approval card for a plan an MCP client wants to run (roadmap A3): who asks, which
//! tool, the plan summary, totals, conflicts and the first items. Approve runs the plan
//! as a normal (undoable) job; Reject, Esc or closing the dialog refuses it. The body
//! is deliberately simple; the assistant's richer plan cards (A6) can replace it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke, Ui};
use soyuz_ai::tools::{PlanConflict, PlanItem, PlanOp, Risk};
use soyuz_core::{SizeStyle, format_size};
use soyuz_mcp::ApprovalDecision;

use super::{Dialog, DialogCtx};
use crate::services::mcp_bridge::PendingMcpApproval;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::pill_button;

/// Items listed in the card.
const SHOWN_ITEMS: usize = 12;

pub struct McpApprovalDialog {
    p: PendingMcpApproval,
    /// Shared with the bridge; cleared when the dialog goes away.
    open: Arc<AtomicBool>,
    decided: bool,
    title: String,
}

impl McpApprovalDialog {
    pub fn new(p: PendingMcpApproval, open: Arc<AtomicBool>) -> Self {
        let title = if p.plan.risk == Risk::Destructive {
            "An AI client wants to delete or overwrite files".to_string()
        } else {
            "An AI client wants to change files".to_string()
        };
        McpApprovalDialog { p, open, decided: false, title }
    }

    fn decide(&mut self, d: ApprovalDecision) {
        self.decided = true;
        self.p.decide(d);
    }
}

impl Drop for McpApprovalDialog {
    fn drop(&mut self) {
        if !self.decided {
            self.p.decide(ApprovalDecision::Rejected("the user dismissed the approval card".into()));
        }
        self.open.store(false, Ordering::SeqCst);
    }
}

fn op_label(op: PlanOp) -> &'static str {
    match op {
        PlanOp::Copy | PlanOp::SyncCopy => "copy",
        PlanOp::Move => "move",
        PlanOp::Rename => "rename",
        PlanOp::Create => "new file",
        PlanOp::Mkdir => "new folder",
        PlanOp::SetMeta => "attributes",
        PlanOp::Extract => "extract",
        PlanOp::Compress => "compress",
        PlanOp::Restore => "restore",
        PlanOp::Trash => "trash",
        PlanOp::Delete | PlanOp::SyncDelete => "delete",
        PlanOp::EmptyTrash => "empty trash",
        PlanOp::Tag => "tag",
        PlanOp::Undo => "undo",
        PlanOp::RunCommand => "run",
    }
}

fn short(p: &std::path::Path) -> String {
    let s = soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned();
    if s.chars().count() > 70 {
        let tail: String = s.chars().rev().take(66).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{tail}")
    } else {
        s
    }
}

fn item_row(ui: &mut Ui, it: &PlanItem) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        let danger = matches!(it.op, PlanOp::Delete | PlanOp::SyncDelete | PlanOp::EmptyTrash);
        let tag = RichText::new(op_label(it.op)).size(11.0).color(if danger { p.danger } else { p.accent });
        ui.add_sized([70.0, 16.0], egui::Label::new(tag));
        let mut text = it.src.as_deref().map(short).unwrap_or_default();
        if let Some(d) = &it.dst {
            if text.is_empty() {
                text = short(d);
            } else {
                text = format!("{text}  →  {}", short(d));
            }
        }
        // File names are untrusted data: plain monospace text, never interpreted.
        ui.add(egui::Label::new(RichText::new(text).monospace().size(11.5).color(p.text_secondary)).truncate());
        if let Some(c) = &it.conflict {
            let why = match c {
                PlanConflict::Exists { resolution, .. } => format!("exists → {resolution}"),
                PlanConflict::Missing => "missing".into(),
                PlanConflict::IntoSelf => "into itself".into(),
                PlanConflict::SameItem => "same item".into(),
                PlanConflict::DuplicateTarget => "duplicate name".into(),
                PlanConflict::InvalidName { reason } => reason.clone(),
            };
            ui.label(RichText::new(why).size(11.0).color(p.warning));
        }
    });
}

impl Dialog for McpApprovalDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(if self.p.plan.risk == Risk::Destructive { ph::SHIELD_WARNING } else { ph::ROBOT })
    }

    fn width(&self) -> f32 {
        560.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        if self.p.ctl.is_cancelled() {
            // The client gave up (cancelled or timed out).
            self.decided = true;
            return false;
        }
        let plan = &self.p.plan;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&self.p.client).strong().color(p.text_primary));
            ui.label(
                RichText::new(format!("(MCP client, via {}) asks to run", self.p.transport.label()))
                    .color(p.text_secondary),
            );
            ui.label(RichText::new(&self.p.tool).monospace().color(p.accent));
        });
        ui.add_space(10.0);
        Frame::new()
            .fill(p.bg_surface2)
            .stroke(Stroke::new(1.0, p.border_subtle))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(icons::icon(ph::LIST_CHECKS, 15.0).color(p.accent));
                    ui.add(egui::Label::new(RichText::new(&plan.summary).size(13.5).color(p.text_primary)).wrap());
                });
                ui.add_space(4.0);
                let mut facts = vec![format!("{} item(s)", plan.items.len())];
                if plan.total_bytes > 0 {
                    facts.push(format_size(plan.total_bytes, SizeStyle::Binary));
                }
                if plan.conflicts > 0 {
                    facts.push(format!("{} conflict(s)", plan.conflicts));
                }
                facts.push(if plan.risk == Risk::Destructive { "destructive".into() } else { "undoable job".into() });
                ui.label(RichText::new(facts.join(" · ")).size(11.5).color(p.text_tertiary));
                ui.add_space(6.0);
                ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                    for it in plan.items.iter().take(SHOWN_ITEMS) {
                        item_row(ui, it);
                    }
                    if plan.items.len() > SHOWN_ITEMS {
                        ui.label(
                            RichText::new(format!("…and {} more", plan.items.len() - SHOWN_ITEMS))
                                .size(11.5)
                                .color(p.text_tertiary),
                        );
                    }
                });
                for w in &plan.warnings {
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::WARNING, 12.0).color(p.warning));
                        ui.label(RichText::new(w).size(11.5).color(p.warning));
                    });
                }
            });
        ui.add_space(8.0);
        let note = if plan.risk == Risk::Destructive {
            "Check the items carefully. Trashed items can be restored; permanent deletes cannot be undone."
        } else {
            "It runs as a background job you can pause, cancel and undo."
        };
        ui.label(RichText::new(note).size(11.5).color(if plan.risk == Risk::Destructive {
            p.danger
        } else {
            p.text_tertiary
        }));
        ui.label(
            RichText::new("File names come from your disk and the request from the AI client; neither can approve anything by itself.")
                .size(11.0)
                .color(p.text_tertiary),
        );
        ui.add_space(14.0);
        let (mut approve, mut reject) = (false, false);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            approve = pill_button(ui, Some(ph::CHECK), "Approve", true).clicked();
            ui.add_space(6.0);
            reject = pill_button(ui, Some(ph::X), "Reject", false).clicked();
        });
        if approve {
            self.decide(ApprovalDecision::Approved);
            return false;
        }
        if reject {
            self.decide(ApprovalDecision::Rejected("the user rejected the plan".into()));
            return false;
        }
        true
    }
}
