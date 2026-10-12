//! The view of MCP approval cards (`DialogModel::McpApproval`), after the GUI's plan
//! card (`ui/ai/cards.rs`): the client, its transport and the time left, the plan's
//! summary with the tool and the totals, the warnings (destructive, external client,
//! tainted, the plan's own, blocked), the planned items as a list (what happens to each:
//! moved / renamed / added / deleted / changed, conflicts marked) and the buttons. The
//! client's request and every name are untrusted: `crate::dialog::safe`.

use std::path::Path;
use std::time::Instant;

use soyuz_appcore::ai::state::{CardOrigin, CardState, PlanCard};
use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;
use soyuz_core::{SizeStyle, format_size};

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;
use crate::theme::Role;
use crate::widgets::ListRow;

/// Box width: the plan's paths need room.
pub const WIDTH: u16 = 80;

/// What the dialog shows.
pub fn vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    let DialogModel::McpApproval(a) = m else { return crate::dialog::fallback(m) };
    let mut d = DialogVm {
        hints: tr!("tui-dialogs-hint-approval").to_string(),
        width: WIDTH,
        alert: a.destructive,
        list_state: ui.list.clone(),
        ..DialogVm::from_form(m.title(), &ui.form)
    };
    let approvals = a.approvals.lock();
    if let Some(c) = approvals.card(a.id) {
        d.body = body(&c.card, a.destructive);
        // Rows built here: the plan's item type lives in `soyuz-ai`.
        d.list = c
            .card
            .plan
            .items
            .iter()
            .map(|i| row(&format!("{:?}", i.op), i.src.as_deref(), i.dst.as_deref(), i.conflict.is_some()))
            .collect();
    }
    d.body.push(tr!("dialogs-mcp-note").to_string());
    d
}

/// The card's text: origin, summary, totals, warnings (`!` first).
pub fn body(card: &PlanCard, destructive: bool) -> Vec<String> {
    let mut b = Vec::new();
    if let CardOrigin::Mcp { client, via } = &card.origin {
        let mut line = format!(
            "{}: {} \u{00b7} {}",
            tr!("ai-card-external"),
            safe(client),
            tr!("ai-card-via", transport = safe(via))
        );
        if card.state == CardState::Pending
            && let Some(exp) = card.expires
        {
            let left = exp.saturating_duration_since(Instant::now()).as_secs();
            line.push_str(" \u{00b7} ");
            line.push_str(&tr!("ai-card-left", time = format!("{}:{:02}", left / 60, left % 60)));
        }
        b.push(line);
    }
    b.push(safe(card.plan.summary.trim_start_matches("AI: ")));
    b.push(format!("{} \u{00b7} {}", safe(&card.tool), totals(card)));
    if destructive {
        b.push(format!("! {}", tr!("ai-card-destructive")));
    }
    if card.origin != CardOrigin::Assistant && card.state == CardState::Pending {
        b.push(format!("! {}", tr!("ai-card-external-warning")));
    }
    if card.tainted {
        b.push(format!("! {}", tr!("ai-card-tainted")));
    }
    for w in &card.plan.warnings {
        b.push(format!("! {}", safe(w)));
    }
    if !card.plan.executable && card.state == CardState::Pending {
        b.push(format!("! {}", tr!("ai-card-blocked")));
    }
    b
}

/// "3 items · 30 B · 2 conflicts", like the GUI.
fn totals(card: &PlanCard) -> String {
    let plan = &card.plan;
    let mut parts = vec![tr!("ai-card-items", count = plan.items.len())];
    if plan.total_bytes > 0 {
        parts.push(format_size(plan.total_bytes, SizeStyle::Binary));
    }
    if plan.total_entries > plan.items.len() as u64 {
        parts.push(tr!("ai-card-entries", count = plan.total_entries));
    }
    if plan.conflicts > 0 {
        parts.push(tr!("ai-card-conflicts", count = plan.conflicts));
    }
    parts.join(" \u{00b7} ")
}

fn path(p: &Path) -> String {
    safe(&soyuz_fs::strip_verbatim(p).display().to_string())
}

/// What a plan operation (by its name) does to its item, with the GUI's tree diff
/// colours.
fn change(op: &str) -> (&'static str, Role) {
    match op {
        "Move" => (tr!("ai-diff-moved"), Role::Accent),
        "Rename" => (tr!("ai-diff-renamed"), Role::Info),
        "Copy" | "SyncCopy" | "Create" | "Mkdir" | "Extract" | "Compress" | "Restore" => {
            (tr!("ai-diff-added"), Role::Success)
        }
        "Trash" | "Delete" | "SyncDelete" | "EmptyTrash" => (tr!("ai-diff-deleted"), Role::Danger),
        _ => (tr!("ai-diff-changed"), Role::Warning),
    }
}

/// One planned item (operation `op`): `src → dst` (a rename shows the new name only)
/// with what happens to it on the right; conflicts in the warning colour.
pub fn row(op: &str, src: Option<&Path>, dst: Option<&Path>, conflict: bool) -> ListRow {
    let (label, role) = change(op);
    let text = match (src, dst) {
        (Some(s), Some(d)) if op == "Rename" => {
            let name = d.file_name().map(|n| safe(&n.to_string_lossy())).unwrap_or_default();
            format!("{} \u{2192} {name}", path(s))
        }
        (Some(s), Some(d)) => format!("{} \u{2192} {}", path(s), path(d)),
        (Some(p), None) | (None, Some(p)) => path(p),
        (None, None) => String::new(),
    };
    ListRow {
        text,
        right: if conflict {
            format!("{label} \u{00b7} {}", tr!("tui-dialogs-mcp-conflict"))
        } else {
            label.to_string()
        },
        role: Some(if conflict { Role::Warning } else { role }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::dialogs::approval_mcp;

    #[test]
    fn card_text_items_and_buttons() {
        let (m, _rx) = approval_mcp::tests::model("move", true);
        let ui = approval_mcp::ui(&m);
        let d = vm(&m, &ui);
        let all = d.body.join("\n");
        assert!(all.contains("test-client") && all.contains("Move 3 items") && all.contains("3 items"), "{all}");
        assert!(all.contains("! careful"), "the plan's warning: {all:?}");
        assert!(!all.chars().any(|c| c.is_control() && c != '\n'), "made safe: {all:?}");
        assert!(all.contains(tr!("dialogs-mcp-note")));
        assert_eq!(d.list.len(), 3);
        assert!(d.list[0].text.contains("a0.txt") && d.list[0].text.contains('\u{2192}'), "{:?}", d.list[0]);
        assert_eq!(d.list[0].right, tr!("ai-diff-moved"));
        assert_eq!(d.buttons.len(), 3);
        assert!(!d.alert);
    }

    #[test]
    fn rows_by_operation() {
        let r = row("Rename", Some(Path::new("dir/a.txt")), Some(Path::new("dir/b.txt")), false);
        assert!(r.text.ends_with("\u{2192} b.txt"), "{}", r.text);
        let r = row("Delete", Some(Path::new("x")), None, true);
        assert_eq!(r.role, Some(Role::Warning));
        assert!(r.right.contains(tr!("tui-dialogs-mcp-conflict")));
        assert_eq!(row("Mkdir", None, Some(Path::new("n")), false).role, Some(Role::Success));
    }
}
