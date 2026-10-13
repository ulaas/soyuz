//! The view of an element form of the table editor (`TuiDialog::TableRow`, also drawn
//! inside `TuiDialog::TableEdit`): the description of the field under the cursor, the
//! fields with their values, the editor of the current field, its error, the buttons.
//! A list editor, shortcut recorder or secret dialog opened from the form is drawn
//! instead while it is open.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::settings_draft::Draft;
use soyuz_appcore::models::settings_schema::{FieldKind, FieldRow};
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::table_row::{Row, State, Sub, element_name, fields};
use crate::input::settings::format_float;
use crate::theme::Role;
use crate::widgets::ListRow;

/// What the dialog shows (standalone).
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    match (&s.draft, &s.row) {
        (Some(d), Some(row)) => row_vm(core, d, row),
        _ => DialogVm::default(),
    }
}

/// The description of field `r`: its doc, then range / values / what to do.
fn description(r: &FieldRow) -> Vec<String> {
    let mut out = Vec::new();
    if !r.doc.is_empty() {
        out.push(r.doc.clone());
    }
    let fact = match &r.kind {
        FieldKind::Int { min: Some(lo), max: Some(hi) } => {
            Some(tr!("tui-settings-range", min = lo.to_string(), max = hi.to_string()))
        }
        FieldKind::Float { min: Some(lo), max: Some(hi) } => {
            Some(tr!("tui-settings-range", min = format_float(*lo), max = format_float(*hi)))
        }
        FieldKind::Table { .. } | FieldKind::Opaque => Some(tr!("tui-tables-opaque").to_string()),
        FieldKind::EnumList { .. } | FieldKind::TextList | FieldKind::Map => {
            Some(tr!("tui-settings-list-desc").to_string())
        }
        FieldKind::Chord => Some(tr!("tui-tables-chord").to_string()),
        _ if r.secret_ref => Some(tr!("tui-tables-secret-ref").to_string()),
        _ => None,
    };
    out.extend(fact);
    out
}

/// The element form `row` of `draft`.
pub fn row_vm(core: &AppCore, draft: &Draft, row: &Row) -> DialogVm {
    match &row.sub {
        Some(Sub::List(s)) => return super::list_edit::vm(core, s),
        Some(Sub::Keybind(s)) => return super::keybind::vm(core, s),
        Some(Sub::Secret(s)) => return super::secret_set::vm(core, s),
        None => {}
    }
    let list = fields(&row.element)
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let shown = FieldRow { path: row.path(draft, r), ..r.clone() };
            let (value, _) = crate::view::settings::value_text(draft, &shown);
            let failed = row.error.as_ref().is_some_and(|(e, _)| *e == i);
            ListRow { text: r.name.clone(), right: value, role: if failed { Some(Role::Danger) } else { None } }
        })
        .collect();
    let name = element_name(draft, &row.table, row.index);
    let title = if name.is_empty() {
        tr!("tui-tables-row-title", table = row.table.as_str(), name = (row.index + 1).to_string())
    } else {
        tr!("tui-tables-row-title", table = row.table.as_str(), name = crate::dialog::safe(&name))
    };
    let mut d = DialogVm::from_form(&title, &row.form);
    d.body = row.field().map(description).unwrap_or_default();
    d.list = list;
    d.list_state = row.list.clone();
    d.error = row.error.as_ref().map(|(_, e)| e.clone()).or(d.error);
    d.hints = tr!("tui-tables-row-hint").into();
    d.width = 76;
    d
}
