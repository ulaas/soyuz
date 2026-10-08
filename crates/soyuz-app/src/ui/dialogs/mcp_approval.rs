//! Modal approval card for a plan an MCP client wants to run (`[ai.mcp] approval_ui =
//! "dialog"`). It renders the assistant's plan card widget (`ui::ai::cards`) for one
//! card of [`McpApprovals`]: summary, tree / rename diff, Approve / Edit / Reject /
//! "Approve all of this kind for this client". Closing the dialog or Esc rejects the
//! request; a request the client withdrew (cancelled, timed out) closes it by itself.
//! The default mode shows the same cards in the assistant panel instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{RichText, ScrollArea, Ui};
use parking_lot::Mutex;
use soyuz_ai::tools::Risk;

use super::{Dialog, DialogCtx};
use crate::ai::agent::Decision;
use crate::ai::mcp::McpApprovals;
use crate::theme::icons::ph;
use crate::theme::palette;
use crate::ui::ai::cards::{self, CardAction};

pub struct McpApprovalDialog {
    id: u64,
    approvals: Arc<Mutex<McpApprovals>>,
    /// Shared with the bridge; cleared when the dialog goes away.
    open: Arc<AtomicBool>,
    title: String,
    destructive: bool,
}

impl McpApprovalDialog {
    pub fn new(id: u64, approvals: Arc<Mutex<McpApprovals>>, open: Arc<AtomicBool>) -> Self {
        let (title, destructive) = match approvals.lock().card(id) {
            Some(c) if c.card.plan.risk == Risk::Destructive => {
                (format!("{} wants to delete or overwrite files", c.client()), true)
            }
            Some(c) => (format!("{} wants to change files", c.client()), false),
            None => ("An AI client wants to change files".to_string(), false),
        };
        McpApprovalDialog { id, approvals, open, title, destructive }
    }
}

impl Drop for McpApprovalDialog {
    fn drop(&mut self) {
        // Closing without an answer rejects the request (a no-op once answered).
        self.approvals.lock().decide(self.id, Decision::Reject("the user dismissed the approval card".into()));
        self.open.store(false, Ordering::SeqCst);
    }
}

impl Dialog for McpApprovalDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(if self.destructive { ph::SHIELD_WARNING } else { ph::PLUGS_CONNECTED })
    }

    fn width(&self) -> f32 {
        600.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let approvals = self.approvals.clone();
        let mut a = approvals.lock();
        let Some(c) = a.card_mut(self.id) else { return false };
        if !c.is_pending() {
            // Answered, withdrawn by the client or timed out.
            return false;
        }
        let mut acts = Vec::new();
        let action = ScrollArea::vertical()
            .max_height(ui.ctx().content_rect().height() * 0.7)
            .auto_shrink([false, true])
            .show(ui, |ui| cards::show(ui, &p, &mut c.card, true, &mut acts))
            .inner;
        ui.add_space(6.0);
        ui.label(
            RichText::new("Approved plans run as normal background jobs you can pause, cancel and undo.")
                .size(11.0)
                .color(p.text_tertiary),
        );
        match action {
            Some(CardAction::Decide(d)) => {
                a.decide(self.id, d);
                false
            }
            Some(CardAction::ApproveKind) => {
                a.approve_kind(self.id);
                false
            }
            None => {
                drop(a);
                for act in acts {
                    if let crate::ui::ai::PanelAct::Reveal(path) = act {
                        ctx.cmds.push(crate::cmd::Cmd::RevealPath(path));
                    }
                }
                true
            }
        }
    }
}
