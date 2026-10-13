//! The view of the "Manage workspaces" dialog (`TuiDialog::ManageWorkspaces`): one row per
//! saved workspace (name and tab count, the hotkey on the right; the workspace last opened
//! or saved in the accent colour), the slot note, then the buttons; in edit mode the name
//! and hotkey fields under the list.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::workspaces::store_snapshot;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::manage_workspaces::{DELETE, EDIT, OPEN, State, UP};
use crate::theme::Role;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let list = store_snapshot();
    let mut d = DialogVm { width: 76, ..DialogVm::from_form(tr!("workspaces-manage-title"), &s.form) };
    if list.is_empty() {
        d.body.push(tr!("workspaces-manage-empty").into());
    }
    let last = core.workspaces.last.as_deref();
    d.list = list
        .iter()
        .map(|(name, hotkey, tabs)| ListRow {
            text: format!("{}  {}", safe(name), tr!("workspaces-tabs", count = *tabs)),
            right: safe(hotkey),
            role: (last == Some(name.as_str())).then_some(Role::Accent),
        })
        .collect();
    // A row deleted since the last key: the cursor stays inside the list.
    d.list_state = s.list.clone();
    d.list_state.clamp(list.len(), usize::from(super::MAX_LIST_ROWS));
    // The buttons that need a row follow the store as it is now (commands applied since
    // the last key).
    let up = list.len() > 1 && d.list_state.cursor > 0;
    if s.editing.is_none() {
        for (i, b) in d.buttons.iter_mut().enumerate() {
            b.enabled = match i {
                OPEN | EDIT | DELETE => !list.is_empty(),
                UP => up,
                _ => true,
            };
        }
        if !list.is_empty() {
            d.body.push(tr!("workspaces-manage-slots-note").into());
        }
    }
    d.hints =
        if s.editing.is_some() { tr!("tui-workspaces-edit-hints") } else { tr!("tui-workspaces-manage-hints") }.into();
    d
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::layout::Hits;
    use crate::theme::TuiTheme;
    use crate::view::dialogs::draw_dialog;

    fn rows(n: usize) -> DialogVm {
        let s = crate::input::dialogs::manage_workspaces::state();
        let mut d = DialogVm { width: 76, ..DialogVm::from_form("Workspaces", &s.form) };
        d.list = (0..n)
            .map(|i| ListRow { text: format!("Space {i}  3 tabs"), right: "Ctrl+Alt+1".into(), role: None })
            .collect();
        d.body.push("The first nine are also …".into());
        d.hints = "Enter open · F2 edit".into();
        d
    }

    #[test]
    fn zero_one_and_twelve_rows_draw_at_every_size() {
        for n in [0, 1, 12] {
            let d = rows(n);
            for (w, h) in [(20, 8), (120, 40), (0, 0), (3, 2)] {
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = Hits::default();
                t.draw(|f| draw_dialog(f, f.area(), &d, &TuiTheme::plain(), &mut hits)).unwrap();
                if (w, h) == (120, 40) && n == 12 {
                    let buf = t.backend().buffer();
                    let all: String =
                        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|p| buf[p].symbol().to_string()).collect();
                    assert!(all.contains("Space 9") && all.contains("Ctrl+Alt+1"), "ten rows shown");
                }
            }
        }
    }
}
