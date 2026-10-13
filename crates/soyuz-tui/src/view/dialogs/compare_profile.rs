//! The view of the dialog for creating, editing, renaming and deleting a sync profile
//! (`TuiDialog::CompareProfile`): the title names the profile; New / Edit show the form
//! (name, folders, method, sync mode, schedule and its fields), the picked schedule's
//! explanation above it and the folder completions of a focused folder field as a list;
//! the delete confirmation is drawn as an alert.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::state::crumbs;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::compare_profile::{Mode, State};
use crate::widgets::{ListRow, SelectList};

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let Some(mode) = &s.mode else { return DialogVm::default() };
    match mode {
        Mode::Delete(name) => DialogVm {
            body: vec![safe(tr!("tui-cmpprof-delete-body"))],
            hints: tr!("tui-cmpprof-hints-delete").into(),
            alert: true,
            width: 64,
            ..DialogVm::from_form(&tr!("tui-cmpprof-title-delete", name = name.clone()), &s.form)
        },
        Mode::Rename(name) => DialogVm {
            hints: tr!("tui-cmpprof-hints-rename").into(),
            width: 64,
            ..DialogVm::from_form(&tr!("tui-cmpprof-title-rename", name = name.clone()), &s.form)
        },
        Mode::New | Mode::Edit(_) => {
            let title = match mode {
                Mode::Edit(name) => tr!("tui-cmpprof-title-edit", name = name.clone()),
                _ => tr!("tui-cmpprof-title-new").to_string(),
            };
            let body = s.kind().note().map(|n| vec![safe(n)]).unwrap_or_default();
            // The folder completions already listed (the input asked for them).
            let names = s
                .folder_focus()
                .map(|i| {
                    let text = s.form.fields[i].value().unwrap_or_default();
                    crumbs::completion_cached(&core.services, text).names
                })
                .unwrap_or_default();
            let mut list_state = SelectList::new();
            list_state.select(s.pick, names.len());
            DialogVm {
                body,
                list: names.into_iter().map(|n| ListRow::new(safe(&n))).collect(),
                list_state,
                hints: tr!("tui-cmpprof-hints-form").into(),
                width: 84,
                ..DialogVm::from_form(&title, &s.form)
            }
        }
    }
}
