//! The view of the drop shelf dialog (`TuiDialog::Shelf`): one row per item (a mark, its
//! name; on the right its size and folder, or "Missing"), what copy / move here would
//! take and where, then the buttons. An empty shelf says how to fill it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::tools::with_shelf;
use soyuz_appcore::tr;
use soyuz_core::{SizeStyle, format_size};

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::shelf::{State, destination};
use crate::theme::Role;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let mut d = DialogVm { width: 80, ..DialogVm::from_form(tr!("tools-shelf-title"), &s.form) };
    let rows: Vec<ListRow> = with_shelf(|sh| {
        sh.items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let mark = if s.marked.contains(&i) { "[x] " } else { "[ ] " };
                let name = safe(&soyuz_core::entry::display_name(&item.path));
                let folder = item.path.parent().map(soyuz_core::entry::display_name).unwrap_or_default();
                let right = if item.missing {
                    tr!("tools-shelf-missing").to_string()
                } else {
                    match &item.entry {
                        Some(e) if !e.is_dir_like() => {
                            tr!("tools-shelf-item-sub", size = format_size(e.size, SizeStyle::Binary), folder = &folder)
                        }
                        _ => folder,
                    }
                };
                ListRow {
                    text: format!("{mark}{name}"),
                    right: safe(&right),
                    role: item.missing.then_some(Role::Muted),
                }
            })
            .collect()
    });
    if rows.is_empty() {
        d.body = vec![tr!("tools-shelf-empty-title").into(), tr!("tui-shelf-empty-body").into()];
    } else {
        let n = if s.marked.is_empty() { rows.len() } else { s.marked.len() };
        let dest = destination(core).map(|p| safe(&soyuz_core::entry::display_name(&p))).unwrap_or_else(|| "—".into());
        let which = if s.marked.is_empty() { "all" } else { "selected" };
        d.body.push(tr!("tools-shelf-footer", which = which, count = n, dest = dest));
    }
    d.list = rows;
    d.list_state = s.list.clone();
    d.hints = tr!("tui-shelf-hints").into();
    d
}
