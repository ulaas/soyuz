//! The view of the smart folder editor (`TuiDialog::Smart`): how the query will be
//! answered (live from the index, not covered, the index is off), the query cheat sheet
//! (cut first on short screens), the fields, the validation message and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::smart::{coverage_text, query_help};
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::smart::{State, folder};
use crate::view::text;

/// Cells of the example column of the cheat sheet.
const EXAMPLE_COLS: usize = 36;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let f = folder(s);
    let mut body = Vec::new();
    if let Some(c) = coverage_text(&f.query, f.include_hidden) {
        body.push(c.to_string());
    }
    body.push(tr!("tui-locations-smart-examples").to_string());
    for (example, what) in query_help() {
        let ex = text::truncate_end(example, EXAMPLE_COLS);
        let pad = EXAMPLE_COLS.saturating_sub(text::width(&ex));
        body.push(format!("{ex}{} {what}", " ".repeat(pad)));
    }
    let title = if s.index.is_some() { tr!("views-smart-dialog-edit") } else { tr!("views-smart-dialog-new") };
    let mut vm = DialogVm {
        body: body.iter().map(|l| safe(l)).collect(),
        hints: tr!("tui-locations-smart-hints").into(),
        width: 86,
        ..DialogVm::from_form(title, &s.form)
    };
    if vm.error.is_none() {
        vm.error = crate::input::dialogs::smart::problem(s);
    }
    vm
}
