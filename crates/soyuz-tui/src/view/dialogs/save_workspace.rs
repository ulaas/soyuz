//! The view of the "Save workspace as…" dialog (`TuiDialog::SaveWorkspace`): what saving
//! keeps, the name field, the tab / group count and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::save_workspace::State;

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let (tabs, groups) = (core.ws.tabs.len(), core.ws.groups.len());
    let count = if groups == 0 {
        tr!("workspaces-tabs", count = tabs)
    } else {
        tr!("workspaces-tabs-and-groups", tabs = tabs, groups = groups)
    };
    DialogVm {
        body: vec![tr!("workspaces-save-intro").into(), count],
        hints: tr!("tui-workspaces-save-hints").into(),
        width: 64,
        ..DialogVm::from_form(tr!("workspaces-save-title"), &s.form)
    }
}
