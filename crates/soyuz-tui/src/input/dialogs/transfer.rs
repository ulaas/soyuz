//! "Copy to" / "Move to" with a destination field (`DialogModel::Transfer`), like the
//! GUI's `ui/dialogs/new_item.rs`, plus folder completion like the address bar
//! (`soyuz_appcore::state::crumbs`): [`prepare`] lists the subfolders matching the typed
//! name every frame (`ModelUi::names`, shown under the message); Up / Down pick one,
//! Tab accepts the picked one (else the first), a click accepts it too. Enter copies /
//! moves to the typed folder (`TransferModel::confirm`; nothing happens while the field
//! is empty), Esc cancels. Conflicts follow `[behaviour] paste_conflict` like a paste.

use soyuz_appcore::models::dialogs::{DialogModel, TransferModel};
use soyuz_appcore::services::Services;
use soyuz_appcore::state::crumbs;
use soyuz_appcore::tr;
use soyuz_core::config::KeyName;

use super::prompt::{buttons, click, is_key, left_click, set_text, wheel};
use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::layout::HitTarget;
use crate::widgets::{Field, Form, FormOutcome, SelectList};

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(m: &DialogModel) -> ModelUi {
    let DialogModel::Transfer(t) = m else { return ModelUi::default() };
    let go = if t.mv { tr!("common-move") } else { tr!("common-copy") };
    let form = Form::new(
        vec![Field::text(tr!("dialogs-transfer-dest"), t.dest.clone())],
        buttons(&[tr!("common-cancel"), go]),
    )
    .default_button(1);
    ModelUi { form, ..ModelUi::default() }
}

/// Every frame while the dialog is on top: the completions of the typed folder (this
/// also asks for its listing, off the UI thread).
pub fn prepare(services: &mut Services, t: &TransferModel, ui: &mut ModelUi) {
    let c = crumbs::completion(services, &t.dest);
    if c.names != ui.names {
        ui.names = c.names;
        ui.list = SelectList::default();
    }
}

/// Accept completion `i` into the field and the model.
fn accept(t: &mut TransferModel, ui: &mut ModelUi, i: usize) {
    let Some(name) = ui.names.get(i).cloned() else { return };
    crumbs::accept(&mut t.dest, &name);
    set_text(&mut ui.form, 0, &t.dest);
    ui.names.clear();
    ui.list = SelectList::default();
}

fn on_form(t: &mut TransferModel, ui: &mut ModelUi, out: FormOutcome) -> Option<Outcome> {
    match out {
        FormOutcome::Changed(_) => {
            t.dest = ui.form.fields[0].value().unwrap_or_default().to_string();
            None
        }
        FormOutcome::Pressed(1) => t.confirm().map(|c| Outcome::Close(Some(c))),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Consumed | FormOutcome::Ignored => None,
    }
}

/// A key press or paste.
pub fn on_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    let DialogModel::Transfer(t) = m else { return Outcome::Keep };
    for ev in &i.events {
        let n = ui.names.len();
        if ui.form.focused_field() == Some(0) && n > 0 {
            if is_key(ev, KeyName::Tab) {
                accept(t, ui, ui.list.cursor);
                continue;
            }
            // Up / Down pick a completion (wrapping, like the address bar).
            if is_key(ev, KeyName::ArrowDown) {
                ui.list.select((ui.list.cursor + 1) % n, n);
                continue;
            }
            if is_key(ev, KeyName::ArrowUp) {
                ui.list.select((ui.list.cursor + n - 1) % n, n);
                continue;
            }
        }
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(t, ui, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event: a click on a completion accepts it.
pub fn on_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let DialogModel::Transfer(t) = m else { return Outcome::Keep };
    if let (true, Some(HitTarget::DialogRow(r))) = (left_click(e), e.target) {
        accept(t, ui, r);
        ui.form.focus = 0;
        return Outcome::Keep;
    }
    if wheel(&mut ui.list, e, ui.names.len()) {
        return Outcome::Keep;
    }
    let out = click(&mut ui.form, e);
    on_form(t, ui, out).unwrap_or(Outcome::Keep)
}

/// The prompt went away by itself.
pub fn stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_appcore::ops::fileops::FileOpCmd;
    use soyuz_appcore::ops::fileops::drop::DropEffect;

    use super::*;

    fn chord(s: &str) -> DialogInput {
        DialogInput {
            key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
            events: vec![InputEvent::Chord(s.parse().unwrap())],
        }
    }

    #[test]
    fn completes_and_confirms() {
        let start = std::env::temp_dir();
        let mut m: DialogModel = TransferModel::new(vec![PathBuf::from("a.txt")], start.clone(), true).into();
        let mut u = ui(&m);
        assert_eq!(u.form.focused_field(), Some(0));
        let sep = std::path::MAIN_SEPARATOR;
        let base = start.display().to_string().trim_end_matches(['/', '\\']).to_string();
        // Completions as `prepare` finds them: Down picks the second, Tab takes it.
        let DialogModel::Transfer(t) = &mut m else { panic!() };
        t.dest = format!("{base}{sep}d");
        set_text(&mut u.form, 0, &t.dest);
        u.names = vec!["docs".into(), "downloads".into()];
        on_input(&mut m, &mut u, &chord("Down"));
        assert_eq!(u.list.cursor, 1);
        on_input(&mut m, &mut u, &chord("Up"));
        on_input(&mut m, &mut u, &chord("Up"));
        assert_eq!(u.list.cursor, 1, "wraps");
        on_input(&mut m, &mut u, &chord("Tab"));
        let want = format!("{base}{sep}downloads{sep}");
        assert_eq!(u.form.fields[0].value(), Some(want.as_str()));
        assert!(matches!(&m, DialogModel::Transfer(t) if t.dest == want));
        // Without completions Tab moves on to the buttons.
        on_input(&mut m, &mut u, &chord("Tab"));
        assert_eq!(u.form.focused_button(), Some(0));
        on_input(&mut m, &mut u, &chord("Shift+Tab"));
        match on_input(&mut m, &mut u, &chord("Enter")) {
            Outcome::Close(Some(Cmd::FileOp(FileOpCmd::Transfer { dest, effect: DropEffect::Move, .. }))) => {
                assert!(dest.ends_with("downloads"), "{dest:?}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_destination_waits_and_esc_cancels() {
        let mut m: DialogModel = TransferModel::new(vec![PathBuf::from("a")], PathBuf::new(), false).into();
        let mut u = ui(&m);
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Keep));
        assert!(matches!(on_input(&mut m, &mut u, &chord("Esc")), Outcome::Close(None)));
    }
}
