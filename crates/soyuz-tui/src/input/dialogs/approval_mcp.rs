//! MCP approval cards (`DialogModel::McpApproval`, `[ai.mcp] approval_ui = "dialog"`),
//! like the GUI's `ui/dialogs/mcp_approval.rs` with the plan card of `ui/ai/cards.rs`:
//! Reject, "Approve all “tool” from client" (remember: later requests of the same kind
//! from this client run without asking until Soyuz restarts; not offered for
//! irreversible tools) and Approve (the default; disabled while the plan has blocking
//! conflicts). Up / Down / PageUp / PageDown / Home / End and the wheel scroll the plan's
//! items. Esc rejects (`DialogModel::dismiss`); a request the client withdrew or that
//! timed out closes by itself ([`stale`]).
//!
//! Editing a plan (the GUI's Edit: new names / JSON arguments) is not offered here.

use soyuz_appcore::ai::agent::Decision;
use soyuz_appcore::ai::mcp::irreversible;
use soyuz_appcore::models::dialogs::{DialogModel, McpApprovalModel};
use soyuz_appcore::tr;

use super::prompt::{buttons, click, left_click, list_key, wheel};
use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::layout::HitTarget;
use crate::widgets::{Form, FormOutcome};

/// What a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Reject,
    /// "Approve all of this kind for this client".
    ApproveKind,
    Approve,
}

/// A button: label, action, enabled.
type CardButton = (String, Act, bool);

/// The buttons (left to right) from the card (`None` once it is gone).
fn card_buttons(m: &McpApprovalModel) -> Option<Vec<CardButton>> {
    let a = m.approvals.lock();
    let c = a.card(m.id)?;
    let can = c.card.plan.executable;
    let mut b = vec![(tr!("ai-card-reject").to_string(), Act::Reject, true)];
    if !irreversible(&c.card.tool) {
        let label = tr!("ai-card-approve-kind-client", tool = &c.card.tool, client = c.client());
        b.push((label, Act::ApproveKind, can));
    }
    b.push((tr!("ai-card-approve").to_string(), Act::Approve, can));
    Some(b)
}

/// The plan's item count (0 once the card is gone).
pub fn items(m: &McpApprovalModel) -> usize {
    m.approvals.lock().card(m.id).map_or(0, |c| c.card.plan.items.len())
}

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(m: &DialogModel) -> ModelUi {
    let DialogModel::McpApproval(a) = m else { return ModelUi::default() };
    let Some(b) = card_buttons(a) else { return ModelUi::default() };
    let labels: Vec<&str> = b.iter().map(|(l, _, _)| l.as_str()).collect();
    let mut bs = buttons(&labels);
    for (btn, (_, _, on)) in bs.iter_mut().zip(&b) {
        btn.enabled = *on;
    }
    let approve = b.len() - 1;
    let mut form = Form::new(Vec::new(), bs).default_button(approve);
    // Approve has the focus, unless the plan is blocked: then Reject.
    form.focus_button(approve);
    ModelUi { form, ..ModelUi::default() }
}

fn on_form(a: &McpApprovalModel, out: FormOutcome) -> Option<Outcome> {
    match out {
        FormOutcome::Pressed(i) => {
            let b = card_buttons(a)?;
            match b.get(i)?.1 {
                Act::Reject => a.decide(Decision::Reject(String::new())),
                Act::ApproveKind => a.approve_kind(),
                Act::Approve => a.decide(Decision::Approve),
            }
            Some(Outcome::Close(None))
        }
        // Rejects (`DialogModel::dismiss`).
        FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    let DialogModel::McpApproval(a) = m else { return Outcome::Keep };
    let n = items(a);
    for ev in &i.events {
        if list_key(&mut ui.list, ev, n) {
            continue;
        }
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(a, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event: buttons; a click on an item selects it, the wheel scrolls.
pub fn on_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let DialogModel::McpApproval(a) = m else { return Outcome::Keep };
    let n = items(a);
    if let (true, Some(HitTarget::DialogRow(r))) = (left_click(e), e.target) {
        ui.list.select(r, n);
        return Outcome::Keep;
    }
    if wheel(&mut ui.list, e, n) {
        return Outcome::Keep;
    }
    let out = click(&mut ui.form, e);
    on_form(a, out).unwrap_or(Outcome::Keep)
}

/// The request went away by itself (answered, withdrawn, timed out).
pub fn stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use crossbeam_channel::Receiver;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use parking_lot::Mutex;
    use soyuz_appcore::ai::mcp::{McpApprovals, PendingMcpApproval};
    use soyuz_appcore::keys::input::InputEvent;

    use super::*;

    fn chord(s: &str) -> DialogInput {
        DialogInput {
            key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
            events: vec![InputEvent::Chord(s.parse().unwrap())],
        }
    }

    /// A pending request of `tool` with three items, as a dialog model.
    pub(crate) fn model(tool: &str, executable: bool) -> (DialogModel, Receiver<soyuz_mcp::ApprovalDecision>) {
        let approvals = Arc::new(Mutex::new(McpApprovals::default()));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let items = (0..3)
            .map(|i| soyuz_ai::tools::PlanItem {
                op: soyuz_ai::tools::PlanOp::Move,
                src: Some(std::path::PathBuf::from(format!("a{i}.txt"))),
                dst: Some(std::path::PathBuf::from("dest").join(format!("a{i}.txt"))),
                kind: "file".into(),
                bytes: 10,
                entries: 1,
                conflict: None,
            })
            .collect();
        let plan = soyuz_ai::tools::Plan {
            tool: tool.into(),
            summary: "Move 3 items".into(),
            risk: soyuz_ai::tools::Risk::Write,
            items,
            total_bytes: 30,
            total_entries: 3,
            conflicts: 0,
            executable,
            warnings: vec!["careful\x1b[2J".into()],
            action: soyuz_ai::tools::PlanAction::None,
        };
        let req = soyuz_mcp::ApprovalRequest {
            client: "test-client".into(),
            transport: soyuz_mcp::Transport::Memory,
            tool: tool.into(),
            args: serde_json::Value::Null,
            plan,
        };
        let p = PendingMcpApproval::new(req, soyuz_core::OpControl::new(), Duration::from_secs(60), 1, tx);
        let id = approvals.lock().push(p);
        (McpApprovalModel::new(id, approvals, Arc::new(AtomicBool::new(true))).into(), rx)
    }

    #[test]
    fn approve_reject_and_remember() {
        let (mut m, rx) = model("move", true);
        let mut u = ui(&m);
        assert_eq!(u.form.buttons.len(), 3);
        assert_eq!(u.form.focused_button(), Some(2));
        // The list scrolls, Enter approves.
        assert!(matches!(on_input(&mut m, &mut u, &chord("Down")), Outcome::Keep));
        assert_eq!(u.list.cursor, 1);
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Close(None)));
        assert_eq!(rx.try_recv().ok(), Some(soyuz_mcp::ApprovalDecision::Approved));
        assert!(stale(&mut m), "answered: the dialog closes");

        let (mut m, rx) = model("move", true);
        let mut u = ui(&m);
        assert!(matches!(on_input(&mut m, &mut u, &chord("R")), Outcome::Close(None)));
        assert!(matches!(rx.try_recv().ok(), Some(soyuz_mcp::ApprovalDecision::Rejected(_))));

        let (mut m, rx) = model("move", true);
        let mut u = ui(&m);
        on_input(&mut m, &mut u, &chord("Left"));
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Close(None)));
        assert_eq!(rx.try_recv().ok(), Some(soyuz_mcp::ApprovalDecision::Approved));
        let DialogModel::McpApproval(a) = &m else { panic!() };
        assert!(a.approvals.lock().approve_kinds.contains(&("test-client".into(), "move".into())));
    }

    #[test]
    fn blocked_and_irreversible_plans() {
        let (m, _rx) = model("delete_permanent", false);
        let u = ui(&m);
        assert_eq!(u.form.buttons.len(), 2, "no remembering for irreversible tools");
        assert!(!u.form.buttons[1].enabled, "a blocked plan cannot be approved");
        assert_eq!(u.form.focused_button(), Some(0), "Reject has the focus");
        // Blocked: Approve is skipped, Enter presses the focused Reject.
        let (mut m, rx) = model("move", false);
        let mut u = ui(&m);
        assert_eq!(u.form.focused_button(), Some(0));
        assert!(matches!(on_input(&mut m, &mut u, &chord("A")), Outcome::Keep), "Approve is disabled");
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Close(None)));
        assert!(matches!(rx.try_recv().ok(), Some(soyuz_mcp::ApprovalDecision::Rejected(_))));
    }
}
