//! New links: the core's "new link" prompt (`DialogModel::NewLink`) and the link dialog
//! the core requests for the selection (`FrontendCmd::MetaDialog(MetaDialog::Link)`,
//! the GUI's `dialogs::links`).
//!
//! The prompt mirrors the GUI's `ui/dialogs/new_item.rs`: the kind (symbolic link, hard
//! link, junction on Windows; Left / Right pick, a click on the focused group picks the
//! next), the target (focused first; the name follows it until the name is edited) and
//! the name, then Cancel / Create. Enter creates when `LinkModel::create` accepts the
//! form (its error shows under the fields once a target is typed), Esc cancels.
//!
//! The link dialog for the selection is still a stub: [`open`] shows the "not available
//! in the terminal UI yet" toast (see the shape of a kind file in `super`).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::dialogs::{DialogModel, LinkChoice, LinkModel};
use soyuz_appcore::models::properties::LinkKind;
use soyuz_appcore::tr;

use super::prompt::{buttons, click, set_text};
use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::app::App;
use crate::widgets::{Field, Form, FormOutcome};

// ---- the core prompt (`DialogModel::NewLink`) -----------------------------------------

/// Field indexes of the prompt's form.
pub const KIND: usize = 0;
pub const TARGET: usize = 1;
pub const NAME: usize = 2;

/// The kinds offered on this platform, in the radio group's order.
pub fn kinds() -> Vec<LinkChoice> {
    LinkChoice::offered().collect()
}

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(m: &DialogModel) -> ModelUi {
    let DialogModel::NewLink(l) = m else { return ModelUi::default() };
    let kinds = kinds();
    let labels = kinds.iter().map(|k| k.label().to_string()).collect();
    let selected = kinds.iter().position(|k| *k == l.kind).unwrap_or(0);
    let mut form = Form::new(
        vec![
            Field::radio(tr!("tui-dialogs-link-kind"), labels, selected),
            Field::text(tr!("dialogs-link-target"), l.target.clone()),
            Field::text(tr!("dialogs-link-name"), l.name.clone()),
        ],
        buttons(&[tr!("common-cancel"), tr!("dialogs-link-create")]),
    )
    .default_button(1);
    form.focus = TARGET;
    form.error = error(l);
    ModelUi { form, ..ModelUi::default() }
}

/// Why the link cannot be created, once a target is typed (like the GUI).
fn error(l: &LinkModel) -> Option<String> {
    if l.target.trim().is_empty() { None } else { l.error() }
}

fn on_form(m: &mut DialogModel, ui: &mut ModelUi, out: FormOutcome) -> Option<Outcome> {
    let DialogModel::NewLink(l) = m else { return None };
    match out {
        FormOutcome::Changed(KIND) => {
            if let Some(k) = ui.form.fields[KIND].selected().and_then(|i| kinds().get(i).copied()) {
                l.kind = k;
            }
        }
        FormOutcome::Changed(TARGET) => {
            l.target = ui.form.fields[TARGET].value().unwrap_or_default().to_string();
            l.target_edited();
            if ui.form.fields[NAME].value() != Some(l.name.as_str()) {
                let name = l.name.clone();
                set_text(&mut ui.form, NAME, &name);
            }
        }
        FormOutcome::Changed(_) => {
            l.name = ui.form.fields[NAME].value().unwrap_or_default().to_string();
            l.name_edited();
        }
        FormOutcome::Pressed(1) => {
            return match l.create() {
                Some(c) => Some(Outcome::Close(Some(c))),
                None => {
                    // Say why (also before a target was typed).
                    ui.form.error = l.error();
                    None
                }
            };
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => return Some(Outcome::Close(None)),
        FormOutcome::Consumed | FormOutcome::Ignored => return None,
    }
    ui.form.error = error(l);
    None
}

/// A key press or paste on the prompt.
pub fn on_model_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(m, ui, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event on the prompt.
pub fn on_model_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let out = click(&mut ui.form, e);
    on_form(m, ui, out).unwrap_or(Outcome::Keep)
}

/// The prompt went away by itself.
pub fn model_stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

// ---- the terminal UI dialog (`MetaDialog::Link`) ---------------------------------------

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
}

/// Open the link dialog of kind `kind` for the focused pane's selection.
pub fn open(app: &mut App, kind: LinkKind) {
    let _ = kind;
    app.unavailable(tr!("tui-feature-links"));
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    super::form_input(&mut s.form, i)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_appcore::ops::fileops::FileOpCmd;

    use super::*;

    fn chord(s: &str) -> DialogInput {
        DialogInput {
            key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
            events: vec![InputEvent::Chord(s.parse().unwrap())],
        }
    }

    fn text(s: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Text(s.into())] }
    }

    #[test]
    fn kind_target_name_and_create() {
        let dir = std::env::temp_dir();
        let mut m: DialogModel = LinkModel::new(dir, None, LinkChoice::Symlink, vec!["taken".into()]).into();
        let mut u = ui(&m);
        assert_eq!(u.form.focused_field(), Some(TARGET));
        assert!(u.form.error.is_none(), "no error before a target is typed");
        assert!(matches!(on_model_input(&mut m, &mut u, &chord("Enter")), Outcome::Keep));
        assert!(u.form.error.is_some(), "Create says why it cannot");
        on_model_input(&mut m, &mut u, &text("some/where/file.txt"));
        let name = u.form.fields[NAME].value().unwrap().to_string();
        assert!(!name.is_empty(), "the name follows the target");
        assert!(matches!(&m, DialogModel::NewLink(l) if l.name == name));
        // Shift+Tab to the kind, Right picks a hard link.
        on_model_input(&mut m, &mut u, &chord("Shift+Tab"));
        on_model_input(&mut m, &mut u, &chord("Right"));
        assert!(matches!(&m, DialogModel::NewLink(l) if l.kind == LinkChoice::Hardlink));
        // A taken name is an error; a free one creates.
        on_model_input(&mut m, &mut u, &chord("Tab"));
        on_model_input(&mut m, &mut u, &chord("Tab"));
        assert_eq!(u.form.focused_field(), Some(NAME));
        for _ in 0..name.chars().count() {
            on_model_input(&mut m, &mut u, &chord("Backspace"));
        }
        on_model_input(&mut m, &mut u, &text("TAKEN"));
        assert!(u.form.error.is_some());
        on_model_input(&mut m, &mut u, &chord("Backspace"));
        match on_model_input(&mut m, &mut u, &chord("Enter")) {
            Outcome::Close(Some(Cmd::FileOp(FileOpCmd::CreateLink {
                name,
                kind: LinkChoice::Hardlink,
                target,
                ..
            }))) => {
                assert_eq!(name, "TAKE");
                assert_eq!(target, PathBuf::from("some/where/file.txt"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn esc_cancels() {
        let mut m: DialogModel = LinkModel::new(std::env::temp_dir(), None, LinkChoice::Symlink, Vec::new()).into();
        let mut u = ui(&m);
        assert!(matches!(on_model_input(&mut m, &mut u, &chord("Esc")), Outcome::Close(None)));
    }
}
