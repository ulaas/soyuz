//! The view of the column chooser (`FrontendCmd::ChooseColumns`): one `[x]` / `[ ]` row
//! per column (Name greyed: it always shows), then Done.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::columns;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::columns::State;
use crate::theme::Role;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let id = s.pane.unwrap_or_else(|| core.ws.pane().id);
    let pane = core.ws.find_pane(id);
    let list = s
        .cols
        .iter()
        .map(|&col| {
            let on = pane.is_some_and(|p| columns::is_visible(p, col));
            let mut row =
                ListRow::new(format!("[{}] {}", if on { 'x' } else { ' ' }, crate::dialog::safe(&columns::label(col))));
            if !columns::can_toggle(col) {
                row.role = Some(Role::Disabled);
            }
            row
        })
        .collect();
    DialogVm {
        list,
        list_state: s.list.clone(),
        hints: tr!("tui-requests-columns-hints").into(),
        width: 44,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
