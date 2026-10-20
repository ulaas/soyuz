//! The dialog for editing an AI plan card's items and destination (`TuiDialog::PlanForm`):
//! the GUI's form editor (`ui/ai/cards.rs::editor`) for `copy`, `move`, `compress`,
//! `trash`, `delete_permanent`, `tag` and `extract` plans (`soyuz_appcore::ai::panel::
//! edit::form`). The list shows each item with a check box: Insert (or a click) leaves it
//! in or out of the call, Space too when the plan has no destination field; Up / Down /
//! PageUp / PageDown move. The field edits the destination. "Use these changes" sends the
//! edited arguments as `Decision::Edit` (`edit::form_args`: at least one item, a
//! destination); the agent or the MCP server plans the call again and proposes a new
//! card. Esc / Cancel keep the card as it is; the dialog closes by itself when the card
//! stops waiting.

use soyuz_appcore::ai::TaskId;
use soyuz_appcore::ai::agent::Decision;
use soyuz_appcore::ai::panel::edit;
use soyuz_appcore::ai::state::{CardState, PlanCard, PlanForm};
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
    /// The items, whether each stays, and the destination (the field holds its text).
    pub plan: PlanForm,
    pub list: SelectList,
    /// Field: the destination (when the tool has one); buttons: use, cancel.
    pub form: Form,
}

/// Open the dialog for `card` (`sel`: where it lives); `false` when the card has no form.
pub fn open(app: &mut App, sel: Sel, card: &PlanCard) -> bool {
    match state(sel, card) {
        Some(s) => {
            app.push_dialog(TuiDialog::PlanForm(s));
            true
        }
        None => false,
    }
}

/// The dialog's state for `card` (`None`: the tool has no form).
pub fn state(sel: Sel, card: &PlanCard) -> Option<State> {
    let plan = edit::form(card)?;
    let fields = match &plan.dest {
        Some((_, d)) => vec![Field::text(tr!("ai-card-form-dest"), d.clone())],
        None => Vec::new(),
    };
    let form = Form::new(fields, vec![Button::new(tr!("ai-card-use-edit")), Button::new(tr!("common-cancel"))])
        .default_button(0);
    Some(State { card: Some(sel), plan, list: SelectList::new(), form })
}

impl State {
    fn rows(&self) -> usize {
        self.plan.list.as_ref().map_or(0, |(_, items)| items.len())
    }

    /// Leave item `i` in or out.
    pub fn toggle(&mut self, i: usize) {
        if let Some((_, items)) = &mut self.plan.list
            && let Some(item) = items.get_mut(i)
        {
            item.1 = !item.1;
            self.form.error = None;
        }
    }

    /// The field's text into the destination.
    fn store(&mut self) {
        if let Some((_, d)) = &mut self.plan.dest {
            *d = self.form.fields.first().and_then(|f| f.value()).unwrap_or_default().to_string();
        }
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

/// Send the edited arguments (`Decision::Edit`), or show why not.
fn submit(core: &mut AppCore, s: &mut State) -> Outcome {
    s.store();
    let Some(sel) = s.card else { return Outcome::Close(None) };
    let Some(c) = card(core, sel) else { return Outcome::Close(None) };
    match edit::form_args(&c, &s.plan) {
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

/// Up / Down / PageUp / PageDown move between the items; Insert (Space without a
/// destination field) leaves the cursor's item in or out.
fn nav(s: &mut State, ev: &InputEvent) -> bool {
    let InputEvent::Chord(c) = ev else { return false };
    if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT) || s.rows() == 0 {
        return false;
    }
    let toggle = c.key == KeyName::Insert || (c.key == KeyName::Space && s.form.fields.is_empty());
    if toggle {
        let i = s.list.cursor;
        s.toggle(i);
        return true;
    }
    let keys = [KeyName::ArrowUp, KeyName::ArrowDown, KeyName::PageUp, KeyName::PageDown];
    if !keys.contains(&c.key) {
        return false;
    }
    s.list.apply(ev, s.rows(), LIST_PAGE);
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

/// A mouse event: a click on a row leaves it in or out, the wheel scrolls, the form's
/// field and buttons.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    if let (true, Some(HitTarget::DialogRow(r))) = (left_click(m), m.target) {
        s.list.select(r, s.rows());
        s.toggle(r);
        return Outcome::Keep;
    }
    let n = s.rows();
    if wheel(&mut s.list, m, n) {
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
    use soyuz_ai::tools::{Plan, PlanAction, Risk};
    use soyuz_appcore::ai::state::CardOrigin;

    use super::*;

    fn card(tool: &str, args: serde_json::Value) -> PlanCard {
        PlanCard {
            id: 1,
            task: TaskId(5),
            tool: tool.into(),
            args,
            plan: Plan {
                tool: tool.into(),
                summary: "Move 2".into(),
                risk: Risk::Write,
                items: Vec::new(),
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
    fn insert_toggles_and_the_field_holds_the_destination() {
        let c = card("move", json!({"sources": ["d/a.pdf", "d/b.pdf"], "dest_dir": "d/Docs"}));
        let mut s = state(Sel::Plan { id: 1, task: 5 }, &c).expect("a form");
        assert_eq!(s.form.fields[0].value(), Some("d/Docs"));
        assert!(nav(&mut s, &InputEvent::Chord("Insert".parse().unwrap())));
        assert!(nav(&mut s, &InputEvent::Chord("Down".parse().unwrap())));
        assert_eq!(s.list.cursor, 1);
        let items = &s.plan.list.as_ref().unwrap().1;
        assert_eq!(items, &[(PathBuf::from("d/a.pdf"), false), (PathBuf::from("d/b.pdf"), true)]);
        // Space types into the destination field here, it does not toggle.
        assert!(!nav(&mut s, &InputEvent::Chord("Space".parse().unwrap())));
        let _ = s.form.apply(&InputEvent::Text("2".into()));
        s.store();
        assert_eq!(s.plan.dest.as_ref().unwrap().1, "d/Docs2");
        // Without a destination (trash) Space toggles.
        let mut t = state(Sel::Plan { id: 1, task: 5 }, &card("trash", json!({"paths": ["d/x"]}))).unwrap();
        assert!(nav(&mut t, &InputEvent::Chord("Space".parse().unwrap())));
        assert!(!t.plan.list.as_ref().unwrap().1[0].1);
        assert!(state(Sel::Plan { id: 1, task: 5 }, &card("set_attributes", json!({}))).is_none());
    }
}
