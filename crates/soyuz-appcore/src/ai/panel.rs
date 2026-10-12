//! The assistant panel's logic without a toolkit: what the user did ([`PanelAct`],
//! applied after the panel is drawn by [`apply`]), card answers ([`CardAction`]), plan
//! card helpers ([`plan_totals`], [`is_rename`], the [`edit`] module for name tables and
//! JSON arguments), the token meter ([`tokens`]), `@` mention candidates
//! ([`mention_candidates`]) and the "Esc stops the running task" rule
//! ([`escape_stops_task`]). The GUI (`ui::ai`) and the terminal UI draw the panel; both
//! push the same acts.
//!
//! Safety: nothing here touches the disk. Plan answers go through [`AiState::decide`]
//! (the agent rebuilds and runs approved plans as jobs) or `McpApprovals::decide`.

use std::path::PathBuf;

use soyuz_ai::tools::Plan;
use soyuz_core::Location;
use soyuz_core::config::AiAccess;

use super::TaskId;
use super::agent::Decision;
use super::model::Route;
use super::recipe::Recipe;
use super::state::{self, AiState, CardState, ChatItem, PanelTab};
use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::host::Host;
use crate::icon::IconId;
use crate::models::frontend::FrontendCmd;
use crate::models::settings::Section;

/// Something the user did in the panel (applied after drawing).
#[derive(Clone, Debug)]
pub enum PanelAct {
    /// Send the composer text (slash commands expand).
    Send,
    Stop,
    NewChat,
    Close,
    Decide(u64, TaskId, Decision),
    /// "Approve all of this kind": approves the card and the other pending, untainted
    /// cards of the same tool in the task.
    ApproveKind(u64, TaskId, String),
    ApproveAllPending(TaskId),
    Undo(TaskId),
    SaveRecipe(TaskId, String),
    RunRecipe(Box<Recipe>),
    DeleteRecipe(PathBuf),
    /// Answer the cost confirmation of a big task.
    Confirm(bool),
    SetRoute(Route),
    SetProfile(Option<String>),
    FolderAccess(PathBuf, Option<AiAccess>),
    Reveal(PathBuf),
    Tab(PanelTab),
    SmartParse,
    SmartRun,
    SmartAskModel,
    ReloadAudit,
    ReloadRecipes,
    /// Send this text as if typed (idea chips, follow-ups).
    Prompt(String),
    /// The AI section of the settings (`FrontendCmd::OpenSettings`).
    OpenSettings,
    /// A card of an external MCP client.
    Mcp(u64, CardAction),
    /// "Undo everything this client did in this session".
    McpUndoClient(String),
    /// Hide finished MCP requests.
    McpClear,
}

/// What the user did on a plan card.
#[derive(Clone, Debug, PartialEq)]
pub enum CardAction {
    Decide(Decision),
    /// "Approve all of this kind" (this chat / this MCP client).
    ApproveKind,
}

/// The model prompt behind "Ask the model" on the smart search card.
pub fn smart_ask_prompt(input: &str) -> String {
    format!(
        "Turn this into one `search` tool call for the current folder and run it, then select the best matches: {input}"
    )
}

/// Apply panel actions (after drawing).
pub fn apply(app: &mut AppCore, host: &dyn Host, acts: Vec<PanelAct>) {
    for a in acts {
        match a {
            PanelAct::Send => {
                let text = std::mem::take(&mut app.ai.panel.composer);
                state::submit_composer_text(app, host, text);
            }
            PanelAct::Prompt(text) => state::submit_composer_text(app, host, text),
            PanelAct::Stop => app.ai.stop(),
            PanelAct::NewChat => app.ai.new_chat(),
            PanelAct::Close => app.ai.panel.open = false,
            PanelAct::Decide(id, task, d) => app.ai.decide(id, task, d),
            PanelAct::ApproveKind(id, task, tool) => approve_kind(&mut app.ai, id, task, &tool),
            PanelAct::ApproveAllPending(task) => approve_all_pending(&mut app.ai, task),
            PanelAct::Undo(task) => state::undo_task(app, task),
            PanelAct::SaveRecipe(task, name) => state::save_recipe(app, task, name),
            PanelAct::RunRecipe(r) => state::run_recipe(app, &r),
            PanelAct::DeleteRecipe(p) => state::delete_recipe(app, p),
            PanelAct::Confirm(go) => state::confirm_pending(app, host, go),
            PanelAct::SetRoute(r) => app.ai.panel.route = r,
            PanelAct::SetProfile(p) => app.ai.panel.profile_override = p,
            PanelAct::FolderAccess(path, access) => state::set_folder_access(app, host, path, access),
            PanelAct::Reveal(path) => app.cmds.push(Cmd::RevealPath(path)),
            PanelAct::Tab(t) => {
                app.ai.panel.tab = t;
                match t {
                    PanelTab::Recipes => app.ai.recipes.loaded = false,
                    PanelTab::Audit => app.ai.audit.loaded = false,
                    PanelTab::Chat => {}
                }
            }
            PanelAct::SmartParse => {
                let today = chrono::Local::now().date_naive();
                let pills = super::smart_search::parse_nl(&app.ai.smart.input, today);
                app.ai.smart.query_text = super::smart_search::to_text(&pills);
                app.ai.smart.pills = pills;
                app.ai.smart.error = None;
            }
            PanelAct::SmartAskModel => {
                let text = smart_ask_prompt(&app.ai.smart.input);
                let shown = tr!("ai-smart-ask-shown", text = app.ai.smart.input.trim());
                state::start_chat_shown(app, host, shown, text, Route::Fast, Vec::new());
            }
            PanelAct::SmartRun => state::run_smart_search(app),
            PanelAct::ReloadAudit => app.ai.audit.loaded = false,
            PanelAct::ReloadRecipes => app.ai.recipes.loaded = false,
            PanelAct::OpenSettings => app.frontend.push(FrontendCmd::OpenSettings(Some(Section::Ai))),
            PanelAct::Mcp(id, act) => apply_mcp(app, id, act),
            PanelAct::McpUndoClient(client) => state::undo_mcp_client(app, client),
            PanelAct::McpClear => app.services.mcp.approvals.lock().clear_resolved(),
        }
    }
}

/// Apply a card action to the MCP approvals (external clients' cards).
pub fn apply_mcp(app: &mut AppCore, id: u64, act: CardAction) {
    let mut a = app.services.mcp.approvals.lock();
    match act {
        CardAction::Decide(d) => a.decide(id, d),
        CardAction::ApproveKind => a.approve_kind(id),
    }
}

/// "Approve all of this kind" on card `id`: remembers the tool for this chat, approves
/// the card, then every other pending card of the same tool in the task that is not
/// tainted by untrusted content.
pub fn approve_kind(ai: &mut AiState, id: u64, task: TaskId, tool: &str) {
    ai.approve_kinds.insert(tool.to_string());
    ai.decide(id, task, Decision::Approve);
    for pid in approve_kind_targets(&ai.chat, id, task, tool) {
        ai.decide(pid, task, Decision::ApproveKind);
    }
}

/// The other cards "Approve all of this kind" on `id` approves: pending, same task and
/// tool, not tainted.
pub fn approve_kind_targets(chat: &[ChatItem], id: u64, task: TaskId, tool: &str) -> Vec<u64> {
    chat.iter()
        .filter_map(|c| match c {
            ChatItem::Plan(p)
                if p.task == task && p.tool == tool && p.id != id && p.state == CardState::Pending && !p.tainted =>
            {
                Some(p.id)
            }
            _ => None,
        })
        .collect()
}

/// "Approve all": every pending, executable card of the task.
pub fn approve_all_pending(ai: &mut AiState, task: TaskId) {
    for id in approve_all_targets(&ai.chat, task) {
        ai.decide(id, task, Decision::Approve);
    }
}

/// The cards "Approve all" approves.
pub fn approve_all_targets(chat: &[ChatItem], task: TaskId) -> Vec<u64> {
    chat.iter()
        .filter_map(|c| match c {
            ChatItem::Plan(p) if p.task == task && p.state == CardState::Pending && p.plan.executable => Some(p.id),
            _ => None,
        })
        .collect()
}

/// Rename tools get a name table instead of a JSON editor.
pub fn is_rename(tool: &str) -> bool {
    matches!(tool, "rename" | "batch_rename")
}

/// "3 items · 1.2 MiB · 40 entries · 2 conflicts".
pub fn plan_totals(plan: &Plan) -> String {
    let mut parts = vec![tr!("ai-card-items", count = plan.items.len())];
    if plan.total_bytes > 0 {
        parts.push(soyuz_core::format_size(plan.total_bytes, soyuz_core::SizeStyle::Binary));
    }
    if plan.total_entries > plan.items.len() as u64 {
        parts.push(tr!("ai-card-entries", count = plan.total_entries));
    }
    if plan.conflicts > 0 {
        parts.push(tr!("ai-card-conflicts", count = plan.conflicts));
    }
    parts.join(" \u{00b7} ")
}

/// Editing a plan card's arguments before approving it: a name table for renames, the
/// full arguments as JSON otherwise. The result goes back as `Decision::Edit(args)`; the
/// agent dry-runs the edited call and proposes a new card.
pub mod edit {
    use serde_json::{Value, json};

    use super::is_rename;
    use crate::ai::state::{PlanCard, PlanEdit};

    /// Start editing `card` (fills [`PlanCard::edit`] and expands the card).
    pub fn begin(card: &mut PlanCard) {
        card.edit = Some(PlanEdit { json: card_args_json(card), names: names(card), error: None });
        card.expanded = true;
    }

    /// The current new names of a rename plan (one per item); empty for other tools.
    pub fn names(card: &PlanCard) -> Vec<String> {
        if !is_rename(&card.tool) {
            return Vec::new();
        }
        card.plan.items.iter().map(|i| crate::ai::diff::name_of(&i.dst)).collect()
    }

    /// The card's arguments as pretty JSON (the JSON editor's starting text).
    pub fn card_args_json(card: &PlanCard) -> String {
        serde_json::to_string_pretty(&card.args).unwrap_or_default()
    }

    /// The edited arguments, or an error to show. Rename plans with names use the name
    /// table; everything else parses [`PlanEdit::json`] (the GUI's rule).
    pub fn args(card: &PlanCard, e: &PlanEdit) -> Result<Value, String> {
        if is_rename(&card.tool) && !e.names.is_empty() {
            return rename_args(card, &e.names);
        }
        parse(&e.json)
    }

    /// Arguments of a rename plan from one new name per item.
    pub fn rename_args(card: &PlanCard, names: &[String]) -> Result<Value, String> {
        let pairs: Vec<Value> = card
            .plan
            .items
            .iter()
            .zip(names)
            .filter_map(|(i, n)| i.src.as_ref().map(|s| json!({"path": s, "new_name": n.trim()})))
            .collect();
        if pairs.iter().any(|p| p["new_name"].as_str().is_some_and(str::is_empty)) {
            return Err(tr!("ai-card-names-empty").into());
        }
        Ok(if card.tool == "rename" && pairs.len() == 1 { pairs[0].clone() } else { json!({"pairs": pairs}) })
    }

    fn parse(text: &str) -> Result<Value, String> {
        serde_json::from_str::<Value>(text).map_err(|err| tr!("ai-card-invalid-json", error = err.to_string()))
    }

    /// Full JSON argument editing (terminal UI JSON dialog, "Edit in $EDITOR"): parses
    /// `text` and checks it against the tool's argument schema (the same validator the
    /// agent runs before it re-plans), so a bad edit is caught before it is sent.
    /// Tools outside the catalog (none today) only need valid JSON.
    pub fn args_from_json(card: &PlanCard, text: &str) -> Result<Value, String> {
        let args = parse(text)?;
        validate(&card.tool, &args)?;
        Ok(args)
    }

    /// Check `args` against the schema of `tool`.
    pub fn validate(tool: &str, args: &Value) -> Result<(), String> {
        let Some(def) = soyuz_ai::tools::def(tool) else { return Ok(()) };
        let checked = if args.is_null() { json!({}) } else { args.clone() };
        let errs = soyuz_ai::schema::validate(&def.schema, &checked);
        if errs.is_empty() { Ok(()) } else { Err(tr!("ai-notice-edit-invalid", error = errs.join("; "))) }
    }
}

/// `12.4k` style token counts (locale decimal separator).
pub fn tokens(n: u64) -> String {
    use soyuz_core::locale::format_decimal;
    match n {
        0..=999 => n.to_string(),
        1000..=999_999 => format!("{}k", format_decimal(n as f64 / 1000.0, 1)),
        _ => format!("{}M", format_decimal(n as f64 / 1_000_000.0, 2)),
    }
}

/// One `@` mention candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    pub icon: IconId,
    pub label: String,
    pub paths: Vec<PathBuf>,
}

/// Candidates for an `@query` mention: the selection, this folder, open tabs and
/// matching entries of the active pane (at most 10).
pub fn mention_candidates(app: &AppCore, q: &str) -> Vec<Mention> {
    let q = q.to_lowercase();
    let pane = app.ws.pane();
    let mut out: Vec<Mention> = Vec::new();
    let sel: Vec<PathBuf> = pane.selected_entries().iter().map(|e| e.path.clone()).collect();
    if !sel.is_empty() && "selection".contains(&q) {
        out.push(Mention {
            icon: IconId::SelectionAll,
            label: tr!("ai-mention-selection", count = sel.len()),
            paths: sel,
        });
    }
    if let Location::Dir(d) = &pane.location
        && ("folder".contains(&q) || "here".contains(&q))
    {
        out.push(Mention {
            icon: IconId::FolderOpen,
            label: tr!("ai-mention-folder", name = soyuz_core::entry::display_name(d)),
            paths: vec![d.clone()],
        });
    }
    for t in &app.ws.tabs {
        if let Location::Dir(d) = &t.active().location {
            let name = soyuz_core::entry::display_name(d);
            if (q.is_empty() || name.to_lowercase().contains(&q) || "tab".contains(&q))
                && !out.iter().any(|m| m.paths.len() == 1 && &m.paths[0] == d)
            {
                out.push(Mention {
                    icon: IconId::Browsers,
                    label: tr!("ai-mention-tab", name = &name),
                    paths: vec![d.clone()],
                });
            }
        }
    }
    if !q.is_empty() {
        for e in pane.entries.iter().filter(|e| e.name.to_lowercase().contains(&q)).take(8) {
            let icon = if e.is_dir_like() { IconId::FolderSimple } else { IconId::File };
            out.push(Mention { icon, label: e.name.clone(), paths: vec![e.path.clone()] });
        }
    }
    out.truncate(10);
    out
}

/// Whether Esc should stop the running task: a task runs, the panel is open and no core
/// dialog is up. Frontends add their own conditions (their dialogs, whether Esc was
/// pressed and not consumed) and then call [`AiState::stop`].
pub fn escape_stops_task(app: &AppCore) -> bool {
    escape_rule(app.ai.running.is_some(), app.ai.panel.open, app.dialogs.is_open())
}

fn escape_rule(running: bool, panel_open: bool, dialog_open: bool) -> bool {
    running && panel_open && !dialog_open
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use serde_json::{Value, json};
    use soyuz_ai::tools::{PlanAction, PlanItem, PlanOp, Risk};

    use super::*;
    use crate::ai::state::{CardOrigin, PlanCard, PlanEdit};

    fn root() -> PathBuf {
        if cfg!(windows) { PathBuf::from("C:\\w") } else { PathBuf::from("/w") }
    }

    fn plan(tool: &str, items: Vec<PlanItem>) -> Plan {
        Plan {
            tool: tool.into(),
            summary: String::new(),
            risk: Risk::Write,
            items,
            total_bytes: 0,
            total_entries: 0,
            conflicts: 0,
            executable: true,
            warnings: Vec::new(),
            action: PlanAction::None,
        }
    }

    fn rename_item(old: &str, new: &str) -> PlanItem {
        PlanItem {
            op: PlanOp::Rename,
            src: Some(root().join(old)),
            dst: Some(root().join(new)),
            kind: "file".into(),
            bytes: 0,
            entries: 1,
            conflict: None,
        }
    }

    fn card(id: u64, task: u64, tool: &str, args: Value, items: Vec<PlanItem>) -> PlanCard {
        PlanCard {
            id,
            task: TaskId(task),
            tool: tool.into(),
            args,
            plan: plan(tool, items),
            tainted: false,
            state: CardState::Pending,
            expanded: false,
            edit: None,
            tree: true,
            origin: CardOrigin::Assistant,
            expires: None::<Instant>,
        }
    }

    fn chat(cards: Vec<PlanCard>) -> Vec<ChatItem> {
        cards.into_iter().map(|c| ChatItem::Plan(Box::new(c))).collect()
    }

    #[test]
    fn approve_kind_cascade_picks_pending_untainted_same_tool_cards_of_the_task() {
        let mut tainted = card(4, 1, "move", json!({}), vec![]);
        tainted.tainted = true;
        let mut done = card(5, 1, "move", json!({}), vec![]);
        done.state = CardState::Rejected;
        let chat = chat(vec![
            card(1, 1, "move", json!({}), vec![]),
            card(2, 1, "move", json!({}), vec![]),
            card(3, 1, "copy", json!({}), vec![]),
            tainted,
            done,
            card(6, 2, "move", json!({}), vec![]),
            card(7, 1, "move", json!({}), vec![]),
        ]);
        assert_eq!(approve_kind_targets(&chat, 1, TaskId(1), "move"), vec![2, 7]);
        assert_eq!(approve_kind_targets(&chat, 3, TaskId(1), "copy"), Vec::<u64>::new());
        assert_eq!(approve_kind_targets(&chat, 6, TaskId(2), "move"), Vec::<u64>::new());
    }

    #[test]
    fn approve_all_takes_every_pending_executable_card_of_the_task() {
        let mut blocked = card(3, 1, "trash", json!({}), vec![]);
        blocked.plan.executable = false;
        let mut tainted = card(4, 1, "copy", json!({}), vec![]);
        tainted.tainted = true;
        let chat =
            chat(vec![card(1, 1, "move", json!({}), vec![]), card(2, 2, "move", json!({}), vec![]), blocked, tainted]);
        assert_eq!(approve_all_targets(&chat, TaskId(1)), vec![1, 4]);
    }

    #[test]
    fn approve_kind_remembers_the_tool_without_a_running_task() {
        let mut ai = AiState::new(Arc::new(|| {}));
        ai.chat = chat(vec![card(1, 1, "move", json!({}), vec![]), card(2, 1, "move", json!({}), vec![])]);
        approve_kind(&mut ai, 1, TaskId(1), "move");
        assert!(ai.approve_kinds.contains("move"));
        // No task runs: decide is a no-op, so nothing changes state.
        assert!(ai.chat.iter().all(|c| matches!(c, ChatItem::Plan(p) if p.state == CardState::Pending)));
    }

    #[test]
    fn rename_edits_use_the_name_table() {
        let mut c = card(
            1,
            1,
            "batch_rename",
            json!({"pairs": []}),
            vec![rename_item("a.txt", "A.txt"), rename_item("b.txt", "B.txt")],
        );
        edit::begin(&mut c);
        assert!(c.expanded);
        let e = c.edit.clone().expect("editing");
        assert_eq!(e.names, vec!["A.txt", "B.txt"]);
        let mut e2 = e.clone();
        e2.names[1] = "  renamed.txt ".into();
        let args = edit::args(&c, &e2).unwrap();
        assert_eq!(args["pairs"][1]["new_name"], "renamed.txt");
        assert_eq!(args["pairs"][0]["path"], json!(root().join("a.txt")));
        e2.names[0] = "   ".into();
        assert!(edit::args(&c, &e2).is_err(), "empty names are refused");

        // A single `rename` sends one pair, not a list.
        let one = card(2, 1, "rename", json!({}), vec![rename_item("x", "y")]);
        let args = edit::rename_args(&one, &["z".into()]).unwrap();
        assert_eq!(args, json!({"path": root().join("x"), "new_name": "z"}));
    }

    #[test]
    fn json_edits_parse_and_report_invalid_json() {
        let args = json!({"sources": [root().join("a.jpg")], "dest_dir": root().join("Images")});
        let mut c = card(1, 1, "move", args.clone(), vec![]);
        edit::begin(&mut c);
        let e = c.edit.clone().unwrap();
        assert!(e.names.is_empty(), "no name table for move");
        assert_eq!(edit::args(&c, &e).unwrap(), args);
        let bad = PlanEdit { json: "{ not json".into(), names: Vec::new(), error: None };
        let err = edit::args(&c, &bad).unwrap_err();
        assert!(!err.is_empty());
        // A rename card whose name table was cleared falls back to the JSON text.
        let mut r = card(2, 1, "rename", json!({"path": "x", "new_name": "y"}), vec![]);
        edit::begin(&mut r);
        let e = r.edit.clone().unwrap();
        assert!(e.names.is_empty());
        assert_eq!(edit::args(&r, &e).unwrap(), json!({"path": "x", "new_name": "y"}));
    }

    #[test]
    fn args_from_json_round_trips_and_checks_the_schema() {
        let args = json!({"sources": [root().join("a.jpg")], "dest_dir": root().join("Images")});
        let c = card(1, 1, "move", args.clone(), vec![]);
        let text = edit::card_args_json(&c);
        assert!(text.contains('\n'), "pretty JSON");
        assert_eq!(edit::args_from_json(&c, &text).unwrap(), args);
        assert!(edit::args_from_json(&c, "[1,").is_err(), "invalid JSON");
        // Valid JSON, wrong shape for the tool.
        let err = edit::args_from_json(&c, r#"{"sources": "not a list"}"#).unwrap_err();
        assert!(!err.is_empty());
        assert!(edit::validate("move", &args).is_ok());
        assert!(edit::validate("no_such_tool", &json!({"x": 1})).is_ok());
    }

    #[test]
    fn helpers() {
        assert!(is_rename("rename") && is_rename("batch_rename") && !is_rename("move"));
        let mut p = plan("move", vec![rename_item("a", "b"), rename_item("c", "d")]);
        p.total_entries = 5;
        p.conflicts = 1;
        let t = plan_totals(&p);
        assert_eq!(t.matches(" \u{00b7} ").count(), 2, "{t}");
        assert_eq!(tokens(999), "999");
        assert!(tokens(12_400).ends_with('k'));
        assert!(tokens(3_000_000).ends_with('M'));
        assert!(smart_ask_prompt("big pdfs").ends_with(": big pdfs"));
    }

    #[test]
    fn escape_rule() {
        assert!(super::escape_rule(true, true, false));
        assert!(!super::escape_rule(false, true, false), "nothing runs");
        assert!(!super::escape_rule(true, false, false), "panel closed");
        assert!(!super::escape_rule(true, true, true), "a dialog takes Esc");
    }
}
