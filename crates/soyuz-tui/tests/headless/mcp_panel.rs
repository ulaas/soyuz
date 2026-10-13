//! Headless tests of T5: MCP approval cards in the AI panel (W2-ai-panel). Each test runs in its
//! own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! A request of an external MCP client is pushed into the core's approvals the way the
//! MCP bridge does (`McpApprovals::push`); with `[ai.mcp] approval_ui = "panel"` the
//! panel opens with the keyboard on the waiting card, and every answer reaches the
//! request's reply channel (`McpApprovals::decide`), which the server thread would read.

use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::Receiver;
use crossterm::event::KeyCode;
use soyuz_appcore::ai::mcp::PendingMcpApproval;
use soyuz_core::config::AiMcpApprovalUi;
use soyuz_mcp::ApprovalDecision;
use soyuz_tui::app::App;
use soyuz_tui::hits::CardButton;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;

use super::ai::{DynStep, ai_config, click_on, panel_text, run_dyn};
use crate::common::{self, Run, Step, ctrl, key};

/// A waiting request of `client` to move three files, as the bridge would push it.
fn request(app: &mut App, client: &str, tool: &str) -> (u64, Receiver<ApprovalDecision>) {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let items = (0..3)
        .map(|i| soyuz_ai::tools::PlanItem {
            op: soyuz_ai::tools::PlanOp::Move,
            src: Some(PathBuf::from(format!("a{i}.txt"))),
            dst: Some(PathBuf::from("dest").join(format!("a{i}.txt"))),
            kind: "file".into(),
            bytes: 10,
            entries: 1,
            conflict: None,
        })
        .collect();
    let plan = soyuz_ai::tools::Plan {
        tool: tool.into(),
        summary: format!("Move 3 items for {client}"),
        risk: soyuz_ai::tools::Risk::Write,
        items,
        total_bytes: 30,
        total_entries: 3,
        conflicts: 0,
        executable: true,
        warnings: Vec::new(),
        action: soyuz_ai::tools::PlanAction::None,
    };
    let req = soyuz_mcp::ApprovalRequest {
        client: client.into(),
        transport: soyuz_mcp::Transport::Memory,
        tool: tool.into(),
        args: serde_json::Value::Null,
        plan,
    };
    let p = PendingMcpApproval::new(req, soyuz_core::OpControl::new(), Duration::from_secs(120), 1, tx);
    let id = app.core.services.mcp.approvals.lock().push(p);
    (id, rx)
}

#[test]
fn panel_cards_answer_the_client() {
    match common::relaunch(module_path!(), "panel_cards_answer_the_client", |_| {}) {
        Run::Child(data) => panel_cards(data),
        Run::Parent(_) => {}
    }
}

fn panel_cards(data: PathBuf) {
    let mut c = ai_config(&data);
    c.ai.mcp.approval_ui = AiMcpApprovalUi::Panel;
    let mut app = common::new_app(c, &data);
    let (first, rx1) = request(&mut app, "client-one", "move");
    let (second, rx2) = request(&mut app, "client-two", "move");
    let (r1, r2) = (rx1.clone(), rx2.clone());
    let steps = vec![
        // The request opened the panel with the keyboard on the first card.
        DynStep::when(
            "the panel with the external cards",
            |a| a.focus() == TuiFocus::Ai && panel_text(a).contains("Move 3 items for client-one"),
            key(KeyCode::Char('a')),
        ),
        // The second card gets the selection; its Reject is clicked.
        DynStep::new("Reject on the second card", move |a| {
            if r1.is_empty() {
                return None;
            }
            click_on(a, |t| t == HitTarget::AiCardButton { id: second, task: 0, button: CardButton::Reject })
        }),
        DynStep::when("answered", move |_| !r2.is_empty(), ctrl('q')),
    ];
    run_dyn(&mut app, steps, 120, 30);
    assert_eq!(rx1.try_recv().ok(), Some(ApprovalDecision::Approved));
    assert!(matches!(rx2.try_recv().ok(), Some(ApprovalDecision::Rejected(_))));
    let a = app.core.services.mcp.approvals.lock();
    assert!(!a.card(first).is_some_and(|c| c.is_pending()) && a.pending() == 0);
    drop(a);
    app.finish();
    common::ok();
}

#[test]
fn approve_all_of_a_kind_for_a_client() {
    match common::relaunch(module_path!(), "approve_all_of_a_kind_for_a_client", |_| {}) {
        Run::Child(data) => approve_kind(data),
        Run::Parent(_) => {}
    }
}

fn approve_kind(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let (_, rx1) = request(&mut app, "client", "move");
    let (_, rx2) = request(&mut app, "client", "move");
    let (r1, r2) = (rx1.clone(), rx2.clone());
    let steps = vec![
        Step::new(
            "the cards",
            |a| a.focus() == TuiFocus::Ai && panel_text(a).contains("[Approve all \u{201c}move\u{201d} from client]"),
            key(KeyCode::Char('k')),
        ),
        Step::new("both answered", move |_| !r1.is_empty() && !r2.is_empty(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 120, 30);
    assert_eq!(rx1.try_recv().ok(), Some(ApprovalDecision::Approved));
    assert_eq!(rx2.try_recv().ok(), Some(ApprovalDecision::Approved));
    assert!(app.core.services.mcp.approvals.lock().approve_kinds.contains(&("client".into(), "move".into())));
    app.finish();
    common::ok();
}

#[test]
fn dialog_mode_keeps_the_modal_card() {
    match common::relaunch(module_path!(), "dialog_mode_keeps_the_modal_card", |_| {}) {
        Run::Child(data) => dialog_mode(data),
        Run::Parent(_) => {}
    }
}

fn dialog_mode(data: PathBuf) {
    let mut c = ai_config(&data);
    c.ai.mcp.approval_ui = AiMcpApprovalUi::Dialog;
    let mut app = common::new_app(c, &data);
    let (_, rx) = request(&mut app, "client", "move");
    let r = rx.clone();
    let steps = vec![
        Step::new("the modal card", |a| a.focus() == TuiFocus::Dialog && !a.core.ai.panel.open, key(KeyCode::Esc)),
        Step::new("rejected", move |_| !r.is_empty(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 120, 30);
    assert!(matches!(rx.try_recv().ok(), Some(ApprovalDecision::Rejected(_))));
    assert!(!app.core.ai.panel.open, "the panel stays closed in dialog mode");
    app.finish();
    common::ok();
}
