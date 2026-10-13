//! The view of the table editor (`TuiDialog::TableEdit`): the elements with their labels,
//! the buttons, a question (remove, the key of a new element) or the open element form.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::settings_schema as schema;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::table_edit::{Mode, State, detail, label};
use crate::theme::Role;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let Some(draft) = &s.draft else { return DialogVm::default() };
    if let Some(row) = &s.row {
        return super::table_row::row_vm(core, draft, row);
    }
    let mut d = DialogVm::from_form(&s.title, &s.form);
    d.width = 76;
    match &s.mode {
        Mode::List => {
            let n = draft.table_len(&s.table);
            d.list = if n == 0 {
                vec![ListRow { role: Some(Role::Muted), ..ListRow::new(tr!("tui-tables-empty")) }]
            } else {
                (0..n)
                    .map(|i| ListRow {
                        text: crate::dialog::safe(&label(draft, &s.table, i)),
                        right: crate::dialog::safe(&detail(draft, &s.table, i)),
                        role: None,
                    })
                    .collect()
            };
            d.list_state = s.list.clone();
            let doc = schema::row(&s.table).map(|r| r.doc.clone()).unwrap_or_default();
            d.body = if doc.is_empty() { Vec::new() } else { vec![doc] };
            d.hints = if s.keyed { tr!("tui-tables-hint-map") } else { tr!("tui-tables-hint") }.into();
        }
        Mode::Name => {
            d.body = vec![tr!("tui-tables-key-body").to_string()];
            d.hints = tr!("tui-tables-key-hint").into();
        }
        Mode::Remove(i) => {
            d.body = vec![tr!("tui-tables-remove-confirm", name = crate::dialog::safe(&label(draft, &s.table, *i)))];
            d.alert = true;
        }
    }
    d
}
