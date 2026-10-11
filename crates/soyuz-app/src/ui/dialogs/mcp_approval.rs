//! Modal approval card for a plan an MCP client wants to run (`[ai.mcp] approval_ui =
//! "dialog"`). It renders the assistant's plan card widget (`ui::ai::cards`) for one
//! card of [`McpApprovals`](crate::ai::mcp::McpApprovals): summary, tree / rename diff,
//! Approve / Edit / Reject / "Approve all of this kind for this client". Closing the
//! dialog or Esc rejects the request; a request the client withdrew (cancelled, timed
//! out) closes it by itself. The default mode shows the same cards in the assistant
//! panel instead. The answers are the core's [`McpApprovalModel`]; this file draws it.

use egui::{RichText, ScrollArea, Ui};

use super::DialogCtx;
use crate::models::dialogs::McpApprovalModel;
use crate::theme::icons::ph;
use crate::theme::palette;
use crate::ui::ai::cards::{self, CardAction};

pub(crate) fn icon(m: &McpApprovalModel) -> &'static str {
    if m.destructive { ph::SHIELD_WARNING } else { ph::PLUGS_CONNECTED }
}

/// Draw the approval card. Returns `false` to close.
pub(crate) fn show(ui: &mut Ui, m: &mut McpApprovalModel, ctx: &mut DialogCtx<'_>) -> bool {
    let p = palette(ui.ctx());
    let mut acts = Vec::new();
    let action = {
        let approvals = m.approvals.clone();
        let mut a = approvals.lock();
        let Some(c) = a.card_mut(m.id) else { return false };
        if !c.is_pending() {
            // Answered, withdrawn by the client or timed out.
            return false;
        }
        ScrollArea::vertical()
            .max_height(ui.ctx().content_rect().height() * 0.7)
            .auto_shrink([false, true])
            .show(ui, |ui| cards::show(ui, &p, &mut c.card, true, &mut acts))
            .inner
    };
    ui.add_space(6.0);
    ui.label(RichText::new(tr!("dialogs-mcp-note")).size(11.0).color(p.text_tertiary));
    match action {
        Some(CardAction::Decide(d)) => {
            m.decide(d);
            false
        }
        Some(CardAction::ApproveKind) => {
            m.approve_kind();
            false
        }
        None => {
            for act in acts {
                if let crate::ui::ai::PanelAct::Reveal(path) = act {
                    ctx.cmds.push(crate::cmd::Cmd::RevealPath(path));
                }
            }
            true
        }
    }
}
