//! Saved process filters (`FrontendCmd::MonitorFilters`, `f` / `F` on the Processes page):
//! one dialog with two modes. The list applies (Enter), deletes (Delete) and reorders
//! (Shift+Up / Down) the saved filters; `n` / `e` (`F2`) open the editor in place, a form of
//! name, filter and regex mode, checked like the core checks it before it is saved.

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::monitor::{MonitorCmd, saved_filters};
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::config::monitor::SavedProcFilter;
use soyuz_core::config::{KeyName, Mods};

use crate::app::App;
use crate::input::dialogs::confirm::{ListClick, click, list_mouse};
use crate::input::dialogs::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::widgets::form::FieldKind;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// The editor of one entry.
#[derive(Debug)]
pub struct FilterEdit {
    /// The name of the entry being changed; `None` for a new one.
    pub old: Option<String>,
    pub form: Form,
}

/// The saved filters dialog's state; the entries themselves are the config's.
#[derive(Debug)]
pub struct FiltersState {
    pub pane: PaneId,
    pub list: SelectList,
    /// Apply / Close under the list.
    pub form: Form,
    pub edit: Option<FilterEdit>,
    /// Opened to save the filter box (`F`): saving or cancelling the editor closes the dialog.
    pub save_only: bool,
}

/// Open the dialog; `new`: straight into the editor, holding the pane's filter box.
pub fn open(app: &mut App, pane: PaneId, new: bool) {
    let form =
        Form::new(Vec::new(), vec![Button::new(tr!("monitor-saved-filter-apply")), Button::new(tr!("common-close"))])
            .default_button(0);
    let mut s = FiltersState { pane, list: SelectList::new(), form, edit: None, save_only: new };
    if new {
        let (query, regex) = app
            .core
            .ui
            .monitor
            .peek(pane)
            .map(|v| (v.filter_text.trim().to_string(), v.filter_regex))
            .unwrap_or_default();
        let list = &app.core.config.monitor.saved_filters;
        let base: String = query.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
        let name = saved_filters::suggest_name(list, &base);
        s.edit = Some(editor(None, &SavedProcFilter { name, query, regex }));
    }
    app.push_dialog(TuiDialog::MonitorFilters(s));
}

fn editor(old: Option<String>, f: &SavedProcFilter) -> FilterEdit {
    let form = Form::new(
        vec![
            Field::text(tr!("monitor-saved-filter-name"), f.name.as_str()),
            Field::text(tr!("monitor-saved-filter-query"), f.query.as_str()),
            Field::check(tr!("monitor-gui-regex"), f.regex),
        ],
        vec![Button::new(tr!("common-save")), Button::new(tr!("common-cancel"))],
    )
    .default_button(0);
    FilterEdit { old, form }
}

/// The entry the editor's form describes.
pub fn draft(form: &Form) -> SavedProcFilter {
    let text = |i: usize| match form.fields.get(i).map(|f| &f.kind) {
        Some(FieldKind::Text { text, .. }) => text.clone(),
        _ => String::new(),
    };
    let regex = matches!(form.fields.get(2).map(|f| &f.kind), Some(FieldKind::Check(true)));
    SavedProcFilter { name: text(0), query: text(1), regex }
}

/// The dialog went away by itself: its pane closed.
pub fn gone(core: &AppCore, s: &FiltersState) -> bool {
    core.ws.find_pane(s.pane).is_none()
}

fn push(core: &mut AppCore, pane: PaneId, cmd: MonitorCmd) {
    core.cmds.push(Cmd::Monitor(pane, cmd));
}

/// Leave the editor: back to the list, or out of the dialog when it only saves.
fn leave_editor(s: &mut FiltersState) -> Outcome {
    s.edit = None;
    if s.save_only { Outcome::Close(None) } else { Outcome::Keep }
}

fn edit_outcome(core: &mut AppCore, s: &mut FiltersState, o: FormOutcome) -> Option<Outcome> {
    let edit = s.edit.as_mut()?;
    match o {
        FormOutcome::Pressed(0) => {
            let entry = draft(&edit.form);
            let mut list = core.config.monitor.saved_filters.clone();
            match saved_filters::save(&mut list, edit.old.as_deref(), entry.clone()) {
                Ok(i) => {
                    push(core, s.pane, MonitorCmd::SaveFilter { old: edit.old.clone(), entry });
                    s.list.cursor = i;
                    Some(leave_editor(s))
                }
                Err(e) => {
                    edit.form.error = Some(e.message());
                    Some(Outcome::Keep)
                }
            }
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(leave_editor(s)),
        FormOutcome::Changed(_) => {
            edit.form.error = None;
            None
        }
        _ => None,
    }
}

fn list_outcome(core: &mut AppCore, s: &FiltersState, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            let name = core.config.monitor.saved_filters.get(s.list.cursor).map(|f| f.name.clone())?;
            push(core, s.pane, MonitorCmd::ApplySavedFilter(name));
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste in the dialog.
pub fn input(core: &mut AppCore, s: &mut FiltersState, i: &DialogInput) -> Outcome {
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        if let Some(edit) = s.edit.as_mut() {
            let o = edit.form.apply(ev);
            if let Some(out @ Outcome::Close(_)) = edit_outcome(core, s, o) {
                return out;
            }
            continue;
        }
        let list = core.config.monitor.saved_filters.clone();
        let cur = list.get(s.list.cursor);
        if let InputEvent::Chord(c) = ev {
            let plain = c.mods.is_empty();
            match c.key {
                KeyName::N if plain => {
                    let name = saved_filters::suggest_name(&list, "");
                    s.edit = Some(editor(None, &SavedProcFilter { name, ..Default::default() }));
                    continue;
                }
                KeyName::E | KeyName::F2 if plain => {
                    if let Some(f) = cur {
                        s.edit = Some(editor(Some(f.name.clone()), f));
                    }
                    continue;
                }
                KeyName::Delete if plain => {
                    if let Some(f) = cur {
                        push(core, s.pane, MonitorCmd::DeleteSavedFilter(f.name.clone()));
                        s.list.cursor = s.list.cursor.min(list.len().saturating_sub(2));
                    }
                    continue;
                }
                KeyName::ArrowUp | KeyName::ArrowDown if c.mods == Mods::SHIFT => {
                    let to = if c.key == KeyName::ArrowUp {
                        s.list.cursor.checked_sub(1)
                    } else {
                        Some(s.list.cursor + 1).filter(|t| *t < list.len())
                    };
                    if let (Some(f), Some(to)) = (cur, to) {
                        push(core, s.pane, MonitorCmd::MoveSavedFilter(f.name.clone(), to));
                        s.list.cursor = to;
                    }
                    continue;
                }
                _ => {}
            }
        }
        if s.list.apply(ev, list.len(), height) {
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = list_outcome(core, s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event in the dialog: a double click on a row applies it.
pub fn mouse(core: &mut AppCore, s: &mut FiltersState, e: &DialogMouse) -> Outcome {
    if let Some(edit) = s.edit.as_mut() {
        return match click(&mut edit.form, e) {
            Some(o) => edit_outcome(core, s, o).unwrap_or(Outcome::Keep),
            None => Outcome::Keep,
        };
    }
    let n = core.config.monitor.saved_filters.len();
    if let Some(ListClick::Row { again: true, .. }) = list_mouse(&mut s.list, n, e) {
        return list_outcome(core, s, FormOutcome::Pressed(0)).unwrap_or(Outcome::Keep);
    }
    match click(&mut s.form, e) {
        Some(o) => list_outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_round_trips_an_entry() {
        let f = SavedProcFilter { name: "Busy".into(), query: "cpu>5".into(), regex: true };
        assert_eq!(draft(&editor(None, &f).form), f);
    }
}
