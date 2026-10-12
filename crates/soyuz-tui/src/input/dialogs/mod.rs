//! Dialogs: keys, mouse and the dialog stack of the terminal UI.
//!
//! Two stacks are open at once, like in the GUI (`soyuz-app/src/ui/dialogs/mod.rs`):
//! - the core's prompts (`AppCore::dialogs`, a `DialogModel` each: job prompts, delete,
//!   quit, transfer, new link, MCP / script approval);
//! - the terminal UI's own dialogs ([`TuiDialogs`], a [`TuiDialog`] each), opened for the
//!   core's `FrontendCmd` requests (`crate::frontend`): confirm, select by pattern,
//!   columns, open with, links, batch rename, attributes, properties, recovery, crash
//!   reports, unsaved changes; Phase T5 / T6 the location dialogs (smart folders,
//!   remotes, archives), compare conflicts and sync profiles, AI plan edits and recipe
//!   names, key recording, list / table / secret edits of the settings editor and the
//!   search bar's size filter and save prompt.
//!
//! Both are stamped by `soyuz_appcore::models::dialogs::next_seq`, so [`top`] picks the
//! most recently opened dialog of either kind; it owns the keyboard (`TuiFocus::Dialog`)
//! and the mouse. [`remove_stale`] closes dialogs that went away by themselves.
//!
//! One file per dialog kind, here (state, keys, mouse) and in `crate::view::dialogs`
//! (the [`crate::view::dialogs::DialogVm`] it shows). Core prompts keep their state in the
//! model; what the terminal UI adds (focus, text cursors, list scroll) is a [`ModelUi`]
//! created by the kind's `ui` function when the model reaches the top. Every core prompt
//! and every request the GUI answers with a dialog has its real view here (shared
//! helpers: `crate::dialog`); the requests that still get a "not available" toast are
//! listed in `crate::frontend`. Each kind file has the same shape:
//!
//! | Core prompts | Terminal UI dialogs |
//! |---|---|
//! | `ui(&DialogModel) -> ModelUi` | `State` (`Debug`), `open(&mut App, …)` |
//! | `on_input(&mut DialogModel, &mut ModelUi, &DialogInput) -> Outcome` | `on_input(&mut AppCore, &mut State, &DialogInput) -> Outcome` |
//! | `on_mouse(&mut DialogModel, &mut ModelUi, &DialogMouse) -> Outcome` | `on_mouse(&mut AppCore, &mut State, &DialogMouse) -> Outcome` |
//! | `stale(&mut DialogModel) -> bool` | `stale(&AppCore, &State) -> bool` |

pub mod approval_mcp;
pub mod approval_script;
pub mod archive;
pub mod attributes;
pub mod batch_rename;
pub mod columns;
pub mod compare_conflict;
pub mod compare_profile;
pub mod confirm;
pub mod crash_reports;
pub mod delete;
pub mod keybind;
pub mod link;
pub mod list_edit;
pub mod open_with;
pub mod pattern;
pub mod plan_json;
pub mod plan_names;
pub mod prompt;
pub mod properties;
pub mod quit;
pub mod recipe_name;
pub mod recovery;
pub mod remote;
pub mod save_search;
pub mod search_size;
pub mod secret_set;
pub mod smart;
pub mod table_edit;
pub mod table_row;
pub mod transfer;
pub mod unsaved;

use crossterm::event::{Event, KeyEvent, KeyEventKind, MouseEvent};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::dialogs::{DialogModel, next_seq};

use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::{Form, FormOutcome, SelectList};

/// What an input did to the top-most dialog.
#[derive(Debug)]
pub enum Outcome {
    /// It stays open.
    Keep,
    /// Close it, after queuing the command, if any (core prompts: `DialogStack::close_top`,
    /// which dismisses an unanswered model).
    Close(Option<Cmd>),
}

/// One key press or paste for a dialog: the raw key and the same input as core events (chord, then typed text) for the
/// widgets (`Form`, `LineEdit`, `SelectList`).
#[derive(Clone, Debug)]
pub struct DialogInput {
    /// The key press (`None` for a paste).
    pub key: Option<KeyEvent>,
    pub events: Vec<InputEvent>,
}

/// A mouse event for a dialog, with the click target under it (from the last frame).
#[derive(Clone, Copy, Debug)]
pub struct DialogMouse {
    pub event: MouseEvent,
    pub target: Option<HitTarget>,
}

/// What the terminal UI keeps for the open core prompt (the model keeps the answer).
#[derive(Clone, Debug, Default)]
pub struct ModelUi {
    /// The model's ordering stamp (`DialogStack::top_seq`).
    pub seq: u64,
    pub form: Form,
    pub list: SelectList,
    /// The kind's page (job prompts: `1` while the conflict's "More…" options show).
    pub page: usize,
    /// Folder completions of a destination field ("Copy to" / "Move to"), refreshed
    /// every frame by [`prepare`].
    pub names: Vec<String>,
}

/// A terminal UI dialog (opened for a `FrontendCmd`), its state in its kind's file.
#[derive(Debug)]
pub enum TuiDialog {
    Confirm(confirm::State),
    Pattern(pattern::State),
    Columns(columns::State),
    OpenWith(open_with::State),
    Link(link::State),
    BatchRename(batch_rename::State),
    Attributes(attributes::State),
    Properties(properties::State),
    Recovery(recovery::State),
    CrashReports(crash_reports::State),
    Unsaved(unsaved::State),
    // ---- Phase T5 + T6 ----
    Smart(smart::State),
    Remote(remote::State),
    Archive(archive::State),
    CompareConflict(compare_conflict::State),
    PlanNames(plan_names::State),
    RecipeName(recipe_name::State),
    Keybind(keybind::State),
    ListEdit(list_edit::State),
    TableEdit(table_edit::State),
    TableRow(table_row::State),
    SecretSet(secret_set::State),
    CompareProfile(compare_profile::State),
    PlanJson(plan_json::State),
    SearchSize(search_size::State),
    SaveSearch(save_search::State),
}

impl TuiDialog {
    fn on_input(&mut self, core: &mut AppCore, i: &DialogInput) -> Outcome {
        match self {
            TuiDialog::Confirm(s) => confirm::on_input(core, s, i),
            TuiDialog::Pattern(s) => pattern::on_input(core, s, i),
            TuiDialog::Columns(s) => columns::on_input(core, s, i),
            TuiDialog::OpenWith(s) => open_with::on_input(core, s, i),
            TuiDialog::Link(s) => link::on_input(core, s, i),
            TuiDialog::BatchRename(s) => batch_rename::on_input(core, s, i),
            TuiDialog::Attributes(s) => attributes::on_input(core, s, i),
            TuiDialog::Properties(s) => properties::on_input(core, s, i),
            TuiDialog::Recovery(s) => recovery::on_input(core, s, i),
            TuiDialog::CrashReports(s) => crash_reports::on_input(core, s, i),
            TuiDialog::Unsaved(s) => unsaved::on_input(core, s, i),
            TuiDialog::Smart(s) => smart::on_input(core, s, i),
            TuiDialog::Remote(s) => remote::on_input(core, s, i),
            TuiDialog::Archive(s) => archive::on_input(core, s, i),
            TuiDialog::CompareConflict(s) => compare_conflict::on_input(core, s, i),
            TuiDialog::PlanNames(s) => plan_names::on_input(core, s, i),
            TuiDialog::RecipeName(s) => recipe_name::on_input(core, s, i),
            TuiDialog::Keybind(s) => keybind::on_input(core, s, i),
            TuiDialog::ListEdit(s) => list_edit::on_input(core, s, i),
            TuiDialog::TableEdit(s) => table_edit::on_input(core, s, i),
            TuiDialog::TableRow(s) => table_row::on_input(core, s, i),
            TuiDialog::SecretSet(s) => secret_set::on_input(core, s, i),
            TuiDialog::CompareProfile(s) => compare_profile::on_input(core, s, i),
            TuiDialog::PlanJson(s) => plan_json::on_input(core, s, i),
            TuiDialog::SearchSize(s) => search_size::on_input(core, s, i),
            TuiDialog::SaveSearch(s) => save_search::on_input(core, s, i),
        }
    }

    fn on_mouse(&mut self, core: &mut AppCore, m: &DialogMouse) -> Outcome {
        match self {
            TuiDialog::Confirm(s) => confirm::on_mouse(core, s, m),
            TuiDialog::Pattern(s) => pattern::on_mouse(core, s, m),
            TuiDialog::Columns(s) => columns::on_mouse(core, s, m),
            TuiDialog::OpenWith(s) => open_with::on_mouse(core, s, m),
            TuiDialog::Link(s) => link::on_mouse(core, s, m),
            TuiDialog::BatchRename(s) => batch_rename::on_mouse(core, s, m),
            TuiDialog::Attributes(s) => attributes::on_mouse(core, s, m),
            TuiDialog::Properties(s) => properties::on_mouse(core, s, m),
            TuiDialog::Recovery(s) => recovery::on_mouse(core, s, m),
            TuiDialog::CrashReports(s) => crash_reports::on_mouse(core, s, m),
            TuiDialog::Unsaved(s) => unsaved::on_mouse(core, s, m),
            TuiDialog::Smart(s) => smart::on_mouse(core, s, m),
            TuiDialog::Remote(s) => remote::on_mouse(core, s, m),
            TuiDialog::Archive(s) => archive::on_mouse(core, s, m),
            TuiDialog::CompareConflict(s) => compare_conflict::on_mouse(core, s, m),
            TuiDialog::PlanNames(s) => plan_names::on_mouse(core, s, m),
            TuiDialog::RecipeName(s) => recipe_name::on_mouse(core, s, m),
            TuiDialog::Keybind(s) => keybind::on_mouse(core, s, m),
            TuiDialog::ListEdit(s) => list_edit::on_mouse(core, s, m),
            TuiDialog::TableEdit(s) => table_edit::on_mouse(core, s, m),
            TuiDialog::TableRow(s) => table_row::on_mouse(core, s, m),
            TuiDialog::SecretSet(s) => secret_set::on_mouse(core, s, m),
            TuiDialog::CompareProfile(s) => compare_profile::on_mouse(core, s, m),
            TuiDialog::PlanJson(s) => plan_json::on_mouse(core, s, m),
            TuiDialog::SearchSize(s) => search_size::on_mouse(core, s, m),
            TuiDialog::SaveSearch(s) => save_search::on_mouse(core, s, m),
        }
    }

    fn stale(&self, core: &AppCore) -> bool {
        match self {
            TuiDialog::Confirm(s) => confirm::stale(core, s),
            TuiDialog::Pattern(s) => pattern::stale(core, s),
            TuiDialog::Columns(s) => columns::stale(core, s),
            TuiDialog::OpenWith(s) => open_with::stale(core, s),
            TuiDialog::Link(s) => link::stale(core, s),
            TuiDialog::BatchRename(s) => batch_rename::stale(core, s),
            TuiDialog::Attributes(s) => attributes::stale(core, s),
            TuiDialog::Properties(s) => properties::stale(core, s),
            TuiDialog::Recovery(s) => recovery::stale(core, s),
            TuiDialog::CrashReports(s) => crash_reports::stale(core, s),
            TuiDialog::Unsaved(s) => unsaved::stale(core, s),
            TuiDialog::Smart(s) => smart::stale(core, s),
            TuiDialog::Remote(s) => remote::stale(core, s),
            TuiDialog::Archive(s) => archive::stale(core, s),
            TuiDialog::CompareConflict(s) => compare_conflict::stale(core, s),
            TuiDialog::PlanNames(s) => plan_names::stale(core, s),
            TuiDialog::RecipeName(s) => recipe_name::stale(core, s),
            TuiDialog::Keybind(s) => keybind::stale(core, s),
            TuiDialog::ListEdit(s) => list_edit::stale(core, s),
            TuiDialog::TableEdit(s) => table_edit::stale(core, s),
            TuiDialog::TableRow(s) => table_row::stale(core, s),
            TuiDialog::SecretSet(s) => secret_set::stale(core, s),
            TuiDialog::CompareProfile(s) => compare_profile::stale(core, s),
            TuiDialog::PlanJson(s) => plan_json::stale(core, s),
            TuiDialog::SearchSize(s) => search_size::stale(core, s),
            TuiDialog::SaveSearch(s) => save_search::stale(core, s),
        }
    }
}

/// The terminal UI's dialogs with their ordering stamp ([`next_seq`]), top-most last.
#[derive(Debug, Default)]
pub struct TuiDialogs {
    stack: Vec<(u64, TuiDialog)>,
}

impl TuiDialogs {
    pub fn push(&mut self, d: TuiDialog) {
        self.stack.push((next_seq(), d));
    }

    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn top(&self) -> Option<&TuiDialog> {
        self.stack.last().map(|(_, d)| d)
    }

    pub fn top_mut(&mut self) -> Option<&mut TuiDialog> {
        self.stack.last_mut().map(|(_, d)| d)
    }

    pub fn top_seq(&self) -> Option<u64> {
        self.stack.last().map(|(s, _)| *s)
    }

    /// Close the top-most dialog.
    pub fn pop(&mut self) -> Option<TuiDialog> {
        self.stack.pop().map(|(_, d)| d)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TuiDialog> {
        self.stack.iter().map(|(_, d)| d)
    }
}

/// What the dialogs keep between frames (`InputState::dialogs`).
#[derive(Debug, Default)]
pub struct State {
    pub tui: TuiDialogs,
    /// The terminal UI's part of the open core prompt (reset when another one is on top).
    pub model: Option<ModelUi>,
}

/// Which stack the top-most dialog is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Top {
    /// The core's `DialogStack`.
    Model,
    /// [`TuiDialogs`].
    Tui,
}

/// The top-most open dialog: the most recently opened of either stack (the GUI's rule).
pub fn top(core: &AppCore, st: &State) -> Option<Top> {
    order(core.dialogs.top_seq(), st.tui.top_seq())
}

/// Which stack is on top, from the stamps of their top-most dialogs.
pub fn order(model: Option<u64>, tui: Option<u64>) -> Option<Top> {
    match (model, tui) {
        (None, None) => None,
        (Some(_), None) => Some(Top::Model),
        (None, Some(_)) => Some(Top::Tui),
        (Some(m), Some(t)) => Some(if m > t { Top::Model } else { Top::Tui }),
    }
}

/// Step 1 of a frame (`view::prepare`): the [`ModelUi`] of a core prompt that just
/// reached the top exists before its first draw; a destination field's completions are
/// requested and refreshed (`transfer::prepare`).
pub fn prepare(core: &mut AppCore, st: &mut State) {
    if top(core, st) == Some(Top::Model)
        && let (Some(seq), Some(m)) = (core.dialogs.top_seq(), core.dialogs.top())
    {
        let ui = model_ui(&mut st.model, seq, m);
        if let DialogModel::Transfer(t) = m {
            transfer::prepare(&mut core.services, t, ui);
        }
    }
}

/// A form's mouse (the terminal UI dialogs): a click on a field focuses (a checkbox toggles)
/// it, on a button presses it; a click on a list row moves `list`'s cursor; the wheel
/// scrolls `list`.
pub fn form_mouse(form: &mut Form, list: Option<(&mut SelectList, usize)>, m: &DialogMouse) -> FormOutcome {
    use crossterm::event::{MouseButton, MouseEventKind};
    match m.event.kind {
        MouseEventKind::Down(MouseButton::Left) => match m.target {
            Some(HitTarget::DialogField(i)) => form.click_field(i),
            Some(HitTarget::DialogButton(i)) => form.click_button(i),
            Some(HitTarget::DialogRow(i)) => {
                if let Some((l, len)) = list {
                    l.select(i, len);
                }
                FormOutcome::Consumed
            }
            _ => FormOutcome::Ignored,
        },
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            if let Some((l, len)) = list {
                let rows = if m.event.kind == MouseEventKind::ScrollUp { -3 } else { 3 };
                l.wheel(rows, len, usize::from(crate::view::dialogs::MAX_LIST_ROWS));
            }
            FormOutcome::Consumed
        }
        _ => FormOutcome::Ignored,
    }
}

/// Any dialog is open.
pub fn is_open(core: &AppCore, st: &State) -> bool {
    core.dialogs.is_open() || st.tui.is_open()
}

/// The [`ModelUi`] of the top-most core prompt (made by the kind's `ui` when it is new).
fn model_ui<'a>(slot: &'a mut Option<ModelUi>, seq: u64, m: &DialogModel) -> &'a mut ModelUi {
    if slot.as_ref().is_none_or(|u| u.seq != seq) {
        let mut ui = match m {
            DialogModel::Prompt(_) => prompt::ui(m),
            DialogModel::DeleteConfirm(_) => delete::ui(m),
            DialogModel::Quit(_) => quit::ui(m),
            DialogModel::Transfer(_) => transfer::ui(m),
            DialogModel::NewLink(_) => link::ui(m),
            DialogModel::McpApproval(_) => approval_mcp::ui(m),
            DialogModel::ScriptApproval(_) => approval_script::ui(m),
        };
        ui.seq = seq;
        *slot = Some(ui);
    }
    slot.get_or_insert_with(ModelUi::default)
}

/// The [`DialogInput`] of a terminal event, if it is one for dialogs.
pub fn dialog_input(ev: &Event, cmd_is_ctrl: bool) -> Option<DialogInput> {
    let key = match ev {
        Event::Key(k) if k.kind == KeyEventKind::Release => return None,
        Event::Key(k) => Some(*k),
        Event::Paste(_) => None,
        _ => return None,
    };
    let events = crate::keys::frame(std::slice::from_ref(ev), cmd_is_ctrl, cfg!(windows)).events;
    Some(DialogInput { key, events })
}

/// Keys and pastes while a dialog is open: each goes to the top-most dialog of the
/// moment (one may close and uncover another between two keys).
pub fn route(app: &mut App, events: &[Event]) {
    let cmd_is_ctrl = app.core.config.keys.cmd_is_ctrl;
    for ev in events {
        let Some(input) = dialog_input(ev, cmd_is_ctrl) else { continue };
        let Some(which) = top(&app.core, &app.input.dialogs) else { break };
        let out = match which {
            Top::Model => {
                let Some(seq) = app.core.dialogs.top_seq() else { break };
                let Some(m) = app.core.dialogs.top_mut() else { break };
                let ui = model_ui(&mut app.input.dialogs.model, seq, m);
                match m {
                    DialogModel::Prompt(_) => prompt::on_input(m, ui, &input),
                    DialogModel::DeleteConfirm(_) => delete::on_input(m, ui, &input),
                    DialogModel::Quit(_) => quit::on_input(m, ui, &input),
                    DialogModel::Transfer(_) => transfer::on_input(m, ui, &input),
                    DialogModel::NewLink(_) => link::on_model_input(m, ui, &input),
                    DialogModel::McpApproval(_) => approval_mcp::on_input(m, ui, &input),
                    DialogModel::ScriptApproval(_) => approval_script::on_input(m, ui, &input),
                }
            }
            Top::Tui => {
                let Some(d) = app.input.dialogs.tui.top_mut() else { break };
                d.on_input(&mut app.core, &input)
            }
        };
        close_if(app, which, out);
    }
}

/// Mouse events while a dialog is open (`crate::input::mouse` hands them over).
pub fn mouse(app: &mut App, event: &MouseEvent) {
    let target = crate::input::mouse::hit(&app.hits, event.column, event.row).map(|(_, t)| t);
    let m = DialogMouse { event: *event, target };
    let Some(which) = top(&app.core, &app.input.dialogs) else { return };
    let out = match which {
        Top::Model => {
            let Some(seq) = app.core.dialogs.top_seq() else { return };
            let Some(model) = app.core.dialogs.top_mut() else { return };
            let ui = model_ui(&mut app.input.dialogs.model, seq, model);
            match model {
                DialogModel::Prompt(_) => prompt::on_mouse(model, ui, &m),
                DialogModel::DeleteConfirm(_) => delete::on_mouse(model, ui, &m),
                DialogModel::Quit(_) => quit::on_mouse(model, ui, &m),
                DialogModel::Transfer(_) => transfer::on_mouse(model, ui, &m),
                DialogModel::NewLink(_) => link::on_model_mouse(model, ui, &m),
                DialogModel::McpApproval(_) => approval_mcp::on_mouse(model, ui, &m),
                DialogModel::ScriptApproval(_) => approval_script::on_mouse(model, ui, &m),
            }
        }
        Top::Tui => {
            let Some(d) = app.input.dialogs.tui.top_mut() else { return };
            d.on_mouse(&mut app.core, &m)
        }
    };
    close_if(app, which, out);
}

fn close_if(app: &mut App, which: Top, out: Outcome) {
    let Outcome::Close(cmd) = out else { return };
    if let Some(c) = cmd {
        app.core.cmds.push(c);
    }
    match which {
        Top::Model => {
            app.core.dialogs.close_top();
            app.input.dialogs.model = None;
        }
        Top::Tui => {
            app.input.dialogs.tui.pop();
        }
    }
}

/// Close dialogs that went away by themselves: core prompts from the top down (a job
/// prompt answered elsewhere, an MCP request withdrawn), any terminal UI dialog whose
/// kind says so; the [`ModelUi`] of a closed prompt goes too.
pub fn remove_stale(app: &mut App) {
    while let Some(m) = app.core.dialogs.top_mut() {
        let gone = match m {
            DialogModel::Prompt(_) => prompt::stale(m),
            DialogModel::DeleteConfirm(_) => delete::stale(m),
            DialogModel::Quit(_) => quit::stale(m),
            DialogModel::Transfer(_) => transfer::stale(m),
            DialogModel::NewLink(_) => link::model_stale(m),
            DialogModel::McpApproval(_) => approval_mcp::stale(m),
            DialogModel::ScriptApproval(_) => approval_script::stale(m),
        };
        if !gone {
            break;
        }
        app.core.dialogs.close_top();
    }
    let core = &app.core;
    app.input.dialogs.tui.stack.retain(|(_, d)| !d.stale(core));
    if app.input.dialogs.model.as_ref().is_some_and(|u| Some(u.seq) != app.core.dialogs.top_seq()) {
        app.input.dialogs.model = None;
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::models::dialogs::DialogStack;

    use super::*;

    #[test]
    fn the_newest_dialog_of_either_stack_is_on_top() {
        let mut models = DialogStack::default();
        let mut st = State::default();
        let top_of = |models: &DialogStack, st: &State| order(models.top_seq(), st.tui.top_seq());
        assert_eq!(top_of(&models, &st), None);
        st.tui.push(TuiDialog::Confirm(confirm::State::default()));
        assert_eq!(top_of(&models, &st), Some(Top::Tui));
        let open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        models.push(soyuz_appcore::models::dialogs::QuitModel::new(1, open));
        assert_eq!(top_of(&models, &st), Some(Top::Model));
        st.tui.push(TuiDialog::Pattern(pattern::State::default()));
        assert_eq!(top_of(&models, &st), Some(Top::Tui));
        st.tui.pop();
        assert_eq!(top_of(&models, &st), Some(Top::Model));
        models.close_top();
        assert_eq!(top_of(&models, &st), Some(Top::Tui));
        assert_eq!(st.tui.len(), 1);
    }

    #[test]
    fn paste_and_keys_become_dialog_input() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let k = Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let i = dialog_input(&k, true).unwrap();
        assert!(i.key.is_some());
        assert!(i.events.iter().any(|e| matches!(e, InputEvent::Text(t) if t == "y")), "{i:?}");
        let p = dialog_input(&Event::Paste("a b".into()), true).unwrap();
        assert!(p.key.is_none());
        assert!(dialog_input(&Event::FocusGained, true).is_none());
        let mut release = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(dialog_input(&Event::Key(release), true).is_none());
    }
}
