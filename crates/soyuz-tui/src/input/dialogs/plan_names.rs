//! The dialog for editing the new names of an AI plan card (`TuiDialog::PlanNames`): the
//! GUI's name table (`ui/ai/cards.rs::editor`) for `rename` / `batch_rename` plans of the
//! assistant or of an external MCP client. The list shows every item (`old → new`), the
//! field edits the new name of the row under the cursor; Up / Down / PageUp / PageDown /
//! Home / End (and clicks, the wheel) move between rows. "Use these changes" sends the
//! names as `Decision::Edit` (`soyuz_appcore::ai::panel::edit::rename_args`): the agent
//! or the MCP server plans the edited call again and proposes a new card. Empty names
//! are refused. Esc / Cancel keep the card as it is. The dialog closes by itself when the
//! card stops waiting (answered, withdrawn, the task ended).

use soyuz_appcore::ai::TaskId;
use soyuz_appcore::ai::agent::Decision;
use soyuz_appcore::ai::diff::name_of;
use soyuz_appcore::ai::panel::edit;
use soyuz_appcore::ai::state::{CardState, PlanCard};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::prompt::{LIST_PAGE, left_click, wheel};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::input::ai::Sel;
use crate::layout::HitTarget;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// The card (`Sel::Plan` or `Sel::Mcp`; `None` only in a default state).
    pub card: Option<Sel>,
    /// The current names of the items.
    pub olds: Vec<String>,
    /// The new names (one per item).
    pub names: Vec<String>,
    pub list: SelectList,
    /// Field: the new name of the cursor row; buttons: use, cancel.
    pub form: Form,
}

fn form(name: &str) -> Form {
    Form::new(
        vec![Field::text(tr!("ai-diff-new-name"), name)],
        vec![Button::new(tr!("ai-card-use-edit")), Button::new(tr!("common-cancel"))],
    )
    .default_button(0)
}

/// Open the dialog for `card` (`sel`: where it lives).
pub fn open(app: &mut App, sel: Sel, card: &PlanCard) {
    app.push_dialog(TuiDialog::PlanNames(state(sel, card)));
}

/// The dialog's state for `card`.
pub fn state(sel: Sel, card: &PlanCard) -> State {
    let names = edit::names(card);
    let olds = card.plan.items.iter().map(|i| name_of(&i.src)).collect();
    State {
        card: Some(sel),
        olds,
        form: form(names.first().map_or("", String::as_str)),
        names,
        list: SelectList::new(),
    }
}

impl State {
    /// The field's text into the cursor row.
    fn store(&mut self) {
        let v = self.form.fields.first().and_then(|f| f.value()).unwrap_or_default().to_string();
        if let Some(n) = self.names.get_mut(self.list.cursor) {
            *n = v;
        }
    }

    /// The cursor row's name into the field (keeps the focus).
    fn load(&mut self) {
        let name = self.names.get(self.list.cursor).cloned().unwrap_or_default();
        if let Some(f) = self.form.fields.first_mut() {
            *f = Field::text(tr!("ai-diff-new-name"), name);
        }
    }

    fn select(&mut self, i: usize) {
        self.store();
        self.list.select(i, self.names.len());
        self.load();
    }
}

/// The card as it is now (`None` once it is gone).
fn card(core: &AppCore, sel: Sel) -> Option<PlanCard> {
    match sel {
        Sel::Plan { id, task } => core.ai.chat.iter().find_map(|c| match c {
            soyuz_appcore::ai::state::ChatItem::Plan(p) if p.id == id && p.task.0 == task => Some(p.as_ref().clone()),
            _ => None,
        }),
        Sel::Mcp(id) => core.services.mcp.approvals.lock().card(id).map(|c| c.card.clone()),
        _ => None,
    }
}

/// Send the names (`Decision::Edit`), or show why not.
fn submit(core: &mut AppCore, s: &mut State) -> Outcome {
    s.store();
    let Some(sel) = s.card else { return Outcome::Close(None) };
    let Some(c) = card(core, sel) else { return Outcome::Close(None) };
    match edit::rename_args(&c, &s.names) {
        Ok(args) => {
            match sel {
                Sel::Plan { id, task } => core.ai.decide(id, TaskId(task), Decision::Edit(args)),
                Sel::Mcp(id) => core.services.mcp.approvals.lock().decide(id, Decision::Edit(args)),
                _ => {}
            }
            Outcome::Close(None)
        }
        Err(e) => {
            s.form.error = Some(e);
            Outcome::Keep
        }
    }
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => Some(submit(core, s)),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            s.form.error = None;
            s.store();
            None
        }
        _ => None,
    }
}

/// Up / Down / PageUp / PageDown / Home / End move between the rows.
fn nav(s: &mut State, ev: &InputEvent) -> bool {
    let InputEvent::Chord(c) = ev else { return false };
    if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT) {
        return false;
    }
    let keys = [KeyName::ArrowUp, KeyName::ArrowDown, KeyName::PageUp, KeyName::PageDown];
    if !keys.contains(&c.key) || s.names.is_empty() {
        return false;
    }
    s.store();
    s.list.apply(ev, s.names.len(), LIST_PAGE);
    s.load();
    true
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        if nav(s, ev) {
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = outcome(core, s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event: a row selects it, the wheel scrolls, the form's fields and buttons.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    if let (true, Some(HitTarget::DialogRow(r))) = (left_click(m), m.target) {
        s.select(r);
        return Outcome::Keep;
    }
    let n = s.names.len();
    let before = s.list.cursor;
    if wheel(&mut s.list, m, n) {
        if s.list.cursor != before {
            let after = s.list.cursor;
            s.list.cursor = before;
            s.select(after);
        }
        return Outcome::Keep;
    }
    match super::confirm::click(&mut s.form, m) {
        Some(o) => outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself: the card no longer waits.
pub fn stale(core: &AppCore, s: &State) -> bool {
    let Some(sel) = s.card else { return true };
    match sel {
        Sel::Plan { task, .. } => {
            let live = core.ai.running.as_ref().is_some_and(|r| r.task.0 == task);
            !live || card(core, sel).is_none_or(|c| c.state != CardState::Pending)
        }
        Sel::Mcp(id) => !core.services.mcp.approvals.lock().card(id).is_some_and(|c| c.is_pending()),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use soyuz_ai::tools::{Plan, PlanAction, PlanItem, PlanOp, Risk};
    use soyuz_appcore::ai::state::CardOrigin;

    use super::*;

    fn card() -> PlanCard {
        let item = |a: &str, b: &str| PlanItem {
            op: PlanOp::Rename,
            src: Some(PathBuf::from("d").join(a)),
            dst: Some(PathBuf::from("d").join(b)),
            kind: "file".into(),
            bytes: 0,
            entries: 1,
            conflict: None,
        };
        PlanCard {
            id: 1,
            task: TaskId(5),
            tool: "batch_rename".into(),
            args: json!({}),
            plan: Plan {
                tool: "batch_rename".into(),
                summary: "Rename 2".into(),
                risk: Risk::Write,
                items: vec![item("a.txt", "A.txt"), item("b.txt", "B.txt")],
                total_bytes: 0,
                total_entries: 2,
                conflicts: 0,
                executable: true,
                warnings: Vec::new(),
                action: PlanAction::None,
            },
            tainted: false,
            state: CardState::Pending,
            expanded: false,
            edit: None,
            tree: true,
            origin: CardOrigin::Assistant,
            expires: None,
        }
    }

    #[test]
    fn rows_and_the_field_follow_each_other() {
        let mut s = state(Sel::Plan { id: 1, task: 5 }, &card());
        assert_eq!(s.olds, ["a.txt", "b.txt"]);
        assert_eq!(s.names, ["A.txt", "B.txt"]);
        assert!(nav(&mut s, &InputEvent::Chord("Down".parse().unwrap())));
        assert_eq!(s.list.cursor, 1);
        assert_eq!(s.form.fields[0].value(), Some("B.txt"));
        // Typing edits the second name.
        let _ = s.form.apply(&InputEvent::Chord("End".parse().unwrap()));
        let _ = s.form.apply(&InputEvent::Text("x".into()));
        s.store();
        assert_eq!(s.names[1], "B.txtx");
        s.select(0);
        assert_eq!(s.form.fields[0].value(), Some("A.txt"));
        assert_eq!(s.names, ["A.txt", "B.txtx"]);
    }
}
