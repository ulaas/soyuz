//! The dialog for the JSON arguments of an AI plan card (`TuiDialog::PlanJson`): the
//! GUI's JSON editor (`ui/ai/cards.rs::editor`) for any tool of the assistant or of an
//! external MCP client, as a multi-line text area
//! ([`crate::input::ai::text_area::TextArea`], no wrapping, scrolls both ways).
//!
//! Keys in the text: typing, paste, Enter (a new line keeping the indentation), the
//! arrows, Home / End, PageUp / PageDown; Ctrl+S or Alt+S applies, Ctrl+E (or F4) edits
//! the text in `$EDITOR` (`input::ai::arm` runs the suspend: a dialog callback has no
//! `App`), Tab goes to the buttons ("Use these changes", "Edit in $EDITOR", "Cancel"), Esc
//! cancels. A click in the text puts the cursor there, the wheel scrolls it.
//!
//! Applying parses the text and checks it against the tool's argument schema
//! (`soyuz_appcore::ai::panel::edit::args_from_json`); an error shows under the text and
//! the dialog stays open. A valid edit goes back as `Decision::Edit(args)`: the agent (or
//! the MCP server) dry-runs the edited call and proposes a new card. The dialog closes by
//! itself when the card stops waiting.

use soyuz_appcore::ai::TaskId;
use soyuz_appcore::ai::agent::Decision;
use soyuz_appcore::ai::panel::edit;
use soyuz_appcore::ai::state::{CardState, ChatItem, PlanCard};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::prompt::left_click;
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::input::ai::Sel;
use crate::input::ai::text_area::{TextArea, clean};
use crate::layout::HitTarget;
use crate::view::text;
use crate::widgets::{Button, EditOutcome, Form, FormOutcome};

/// The dialog's preferred width (borders included).
pub const WIDTH: u16 = 90;
/// Text rows at most.
pub const MAX_ROWS: usize = 18;
/// The screen size assumed before the first frame told it.
const SCREEN: (u16, u16) = (100, 30);
/// Lines the wheel scrolls.
const WHEEL_ROWS: i64 = 3;

/// Button 0: use the changes; 1: edit in `$EDITOR`; 2: cancel.
const APPLY: usize = 0;
const EXTERNAL: usize = 1;
const CANCEL: usize = 2;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// The card (`Sel::Plan` or `Sel::Mcp`; `None` only in a default state).
    pub card: Option<Sel>,
    pub tool: String,
    /// The arguments as edited.
    pub text: String,
    pub area: TextArea,
    /// The buttons and the validation message.
    pub form: Form,
    /// The keyboard is in the text (else on the buttons).
    pub in_text: bool,
    /// The last frame's screen size (`input::ai::arm`; `(0, 0)` before the first one).
    pub screen: (u16, u16),
    /// "Edit in $EDITOR" was asked: `input::ai::arm` writes the text out and suspends.
    pub editor: bool,
}

impl State {
    /// Replace the text (the editor's result), cursor at the start.
    pub fn set_text(&mut self, text: String) {
        self.text = clean(&text);
        self.area = TextArea::new();
        self.form.error = None;
        self.in_text = true;
        self.follow();
    }

    /// Keep the cursor in the view.
    fn follow(&mut self) {
        let (w, h) = dims(self.screen, self.form.error.is_some());
        self.area.scroll_to_cursor(&self.text, None, w, h);
    }
}

/// The text area's size for a `screen`: width (cells) and rows.
pub fn dims(screen: (u16, u16), error: bool) -> (usize, usize) {
    let (sw, sh) = if screen == (0, 0) { SCREEN } else { screen };
    let width = sw.saturating_sub(4).clamp(1, WIDTH).min(sw);
    let inner = usize::from(width.saturating_sub(2)).max(1);
    let hints = text::wrap(&hints(), inner).len();
    // Borders, the gap after the text, the error, the buttons, the gap and the hints.
    let other = 2 + 1 + usize::from(error) + 1 + 1 + hints;
    (inner, usize::from(sh).saturating_sub(other).clamp(1, MAX_ROWS))
}

/// The key hints.
pub fn hints() -> String {
    tr!("tui-ai-json-hints").into()
}

fn form() -> Form {
    Form::new(
        Vec::new(),
        vec![
            Button::new(tr!("ai-card-use-edit")),
            Button::new(tr!("tui-ai-json-external")),
            Button::new(tr!("common-cancel")),
        ],
    )
    .default_button(APPLY)
}

/// The dialog's state for `card` with `text` (and an error to show).
pub fn state(sel: Sel, card: &PlanCard, text: String, error: Option<String>) -> State {
    let mut form = form();
    form.error = error;
    let mut s = State {
        card: Some(sel),
        tool: card.tool.clone(),
        text: clean(&text),
        area: TextArea::new(),
        form,
        in_text: true,
        screen: (0, 0),
        editor: false,
    };
    s.follow();
    s
}

/// Open the dialog for `card` (`sel`: where it lives) with its current arguments.
pub fn open(app: &mut App, sel: Sel, card: &PlanCard) {
    open_with(app, sel, card, edit::card_args_json(card), None);
}

/// Open the dialog for `card` with `text` and an error to show (an edit from `$EDITOR`
/// that did not validate).
pub fn open_with(app: &mut App, sel: Sel, card: &PlanCard, text: String, error: Option<String>) {
    let area = app.regions().area;
    let mut s = state(sel, card, text, error);
    s.screen = (area.width, area.height);
    s.follow();
    app.push_dialog(TuiDialog::PlanJson(s));
}

/// The card as it is now, while it still waits for an answer.
pub fn waiting_card(core: &AppCore, sel: Sel) -> Option<PlanCard> {
    match sel {
        Sel::Plan { id, task } => {
            let live = core.ai.running.as_ref().is_some_and(|r| r.task.0 == task);
            core.ai
                .chat
                .iter()
                .find_map(|c| match c {
                    ChatItem::Plan(p) if p.id == id && p.task.0 == task => Some(p.as_ref().clone()),
                    _ => None,
                })
                .filter(|p| live && p.state == CardState::Pending)
        }
        Sel::Mcp(id) => core.services.mcp.approvals.lock().card(id).filter(|c| c.is_pending()).map(|c| c.card.clone()),
        _ => None,
    }
}

/// Send edited arguments for the card (`Decision::Edit`).
pub fn decide(core: &mut AppCore, sel: Sel, args: serde_json::Value) {
    match sel {
        Sel::Plan { id, task } => core.ai.decide(id, TaskId(task), Decision::Edit(args)),
        Sel::Mcp(id) => core.services.mcp.approvals.lock().decide(id, Decision::Edit(args)),
        _ => {}
    }
}

/// Validate the text and send it. Returns whether it was sent (the dialog closes); an
/// error stays in the dialog.
pub fn submit(core: &mut AppCore, s: &mut State) -> bool {
    let Some(sel) = s.card else { return true };
    let Some(card) = waiting_card(core, sel) else { return true };
    match edit::args_from_json(&card, &s.text) {
        Ok(args) => {
            decide(core, sel, args);
            true
        }
        Err(e) => {
            s.form.error = Some(e);
            s.in_text = true;
            s.follow();
            false
        }
    }
}

fn apply_now(core: &mut AppCore, s: &mut State) -> Outcome {
    if submit(core, s) { Outcome::Close(None) } else { Outcome::Keep }
}

/// Ask `input::ai::arm` for `$EDITOR` (and a frame right away).
fn external(core: &AppCore, s: &mut State) {
    s.editor = true;
    (core.wake)();
}

fn press(core: &mut AppCore, s: &mut State, b: usize) -> Outcome {
    match b {
        APPLY => apply_now(core, s),
        EXTERNAL => {
            external(core, s);
            Outcome::Keep
        }
        CANCEL => Outcome::Close(None),
        _ => Outcome::Keep,
    }
}

fn ctrl_or_alt(ev: &InputEvent, key: KeyName) -> bool {
    matches!(ev, InputEvent::Chord(c) if c.key == key && (c.mods == Mods::CTRL || c.mods == Mods::CMD || c.mods == Mods::ALT))
}

/// Enter in the text: a new line with the current line's indentation.
fn enter(s: &mut State) {
    let at = s.area.cursor();
    let start = s.text[..at].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = s.text[start..at].chars().take_while(|c| *c == ' ').collect();
    s.area.insert(&mut s.text, &format!("\n{indent}"));
}

fn text_key(core: &mut AppCore, s: &mut State, ev: &InputEvent) -> Option<Outcome> {
    let (_, rows) = dims(s.screen, s.form.error.is_some());
    let before = s.text.clone();
    match s.area.apply(&mut s.text, ev, None) {
        EditOutcome::Submit => enter(s),
        EditOutcome::Cancel => return Some(Outcome::Close(None)),
        EditOutcome::Tab => {
            s.in_text = false;
            s.form.focus_button(APPLY);
        }
        EditOutcome::PageUp => {
            s.area.move_rows(&s.text, None, -(rows as i64));
        }
        EditOutcome::PageDown => {
            s.area.move_rows(&s.text, None, rows as i64);
        }
        EditOutcome::ShiftTab | EditOutcome::Up | EditOutcome::Down | EditOutcome::Consumed => {}
        EditOutcome::Ignored => {
            if matches!(ev, InputEvent::Chord(c) if c.key == KeyName::F4 && c.mods.is_empty()) {
                external(core, s);
            }
        }
    }
    if s.text != before {
        s.form.error = None;
    }
    s.follow();
    None
}

fn button_key(core: &mut AppCore, s: &mut State, ev: &InputEvent) -> Option<Outcome> {
    let first = s.form.focused_button() == Some(APPLY);
    let last = s.form.focused_button() == Some(CANCEL);
    if let InputEvent::Chord(c) = ev
        && c.key == KeyName::Tab
        && c.mods.difference(Mods::SHIFT).is_empty()
    {
        let back = c.mods.contains(Mods::SHIFT);
        if (back && first) || (!back && last) {
            s.in_text = true;
            return None;
        }
    }
    match s.form.apply(ev) {
        FormOutcome::Pressed(b) => Some(press(core, s, b)),
        FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        if ctrl_or_alt(ev, KeyName::S) {
            return apply_now(core, s);
        }
        if ctrl_or_alt(ev, KeyName::E) {
            external(core, s);
            return Outcome::Keep;
        }
        let out = if s.in_text { text_key(core, s, ev) } else { button_key(core, s, ev) };
        if let Some(o) = out {
            return o;
        }
    }
    Outcome::Keep
}

/// Where the text area's first cell is on the screen (the dialog is centred like
/// `view::dialogs::draw_dialog` does).
fn text_x(s: &State) -> u16 {
    let (sw, _) = if s.screen == (0, 0) { SCREEN } else { s.screen };
    let width = sw.saturating_sub(4).clamp(1, WIDTH).min(sw);
    (sw - width) / 2 + 1
}

/// A mouse event: a click in the text puts the cursor there, the wheel scrolls it, the
/// buttons press.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    use crossterm::event::MouseEventKind;
    if let (true, Some(HitTarget::DialogText(row))) = (left_click(m), m.target) {
        let col = m.event.column.saturating_sub(text_x(s));
        s.area.click(&s.text, None, usize::from(row), usize::from(col));
        s.in_text = true;
        return Outcome::Keep;
    }
    let rows = match m.event.kind {
        MouseEventKind::ScrollUp => -WHEEL_ROWS,
        MouseEventKind::ScrollDown => WHEEL_ROWS,
        _ => 0,
    };
    if rows != 0 {
        s.area.move_rows(&s.text, None, rows);
        s.follow();
        return Outcome::Keep;
    }
    match super::confirm::click(&mut s.form, m) {
        Some(FormOutcome::Pressed(b)) => press(core, s, b),
        _ => Outcome::Keep,
    }
}

/// The dialog went away by itself: the card no longer waits.
pub fn stale(core: &AppCore, s: &State) -> bool {
    s.card.is_none_or(|sel| waiting_card(core, sel).is_none())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use soyuz_ai::tools::{Plan, PlanAction, Risk};
    use soyuz_appcore::ai::state::CardOrigin;

    use super::*;

    fn card() -> PlanCard {
        PlanCard {
            id: 1,
            task: TaskId(5),
            tool: "move".into(),
            args: json!({"sources": ["a.txt"], "dest_dir": "Moved"}),
            plan: Plan {
                tool: "move".into(),
                summary: "Move".into(),
                risk: Risk::Write,
                items: Vec::new(),
                total_bytes: 0,
                total_entries: 0,
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

    fn ev(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    #[test]
    fn enter_keeps_the_indentation() {
        let c = card();
        let mut s = state(Sel::Plan { id: 1, task: 5 }, &c, edit::card_args_json(&c), None);
        assert!(s.text.contains('\n'), "pretty JSON");
        assert_eq!(s.form.buttons.len(), 3);
        // The end of the second line (`  "dest_dir": …`), then Enter.
        s.area.set_cursor(&s.text, 0);
        for k in ["Down", "End"] {
            let _ = s.area.apply(&mut s.text, &ev(k), None);
        }
        enter(&mut s);
        let line = s.text[..s.area.cursor()].rsplit('\n').next().unwrap().to_string();
        assert_eq!(line, "  ", "the new line is indented like the one before");
    }

    #[test]
    fn dims_fit_the_screen() {
        let (w, h) = dims((120, 40), false);
        assert_eq!(w, usize::from(WIDTH) - 2);
        assert_eq!(h, MAX_ROWS);
        let (w, h) = dims((20, 8), true);
        assert!(w <= 18 && h >= 1);
        assert_eq!(dims((0, 0), false), dims(SCREEN, false));
        for sw in 0..30 {
            for sh in 0..12 {
                let (w, h) = dims((sw, sh), sh % 2 == 0);
                assert!(w >= 1 && h >= 1);
            }
        }
    }
}
