//! The view of Quick share (`TuiDialog::Share`): the address (or why there is none),
//! what to do with it, the QR code in half blocks (the text area), the other addresses
//! as a list, what is shared and what happened so far.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::share::State;
use crate::widgets::ListRow;

/// Quiet zone around the code (modules; the spec asks for 4, scanners manage with 2).
const QUIET: usize = 2;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let m = &s.model;
    let mut body = Vec::new();
    if let Some(problem) = m.problem() {
        body.push(problem);
    }
    let url = m.url().filter(|_| m.running());
    if let Some(u) = &url {
        body.push(format!("{}: {u}", tr!("tools-share-address")));
        body.push(tr!("tools-share-intro").into());
    }
    let shared: Vec<String> = m.items.iter().take(3).map(|p| safe(&soyuz_core::entry::display_name(p))).collect();
    let more = m.items.len().saturating_sub(3);
    body.push(format!(
        "{}: {}{}",
        tr!("tools-share-shared"),
        shared.join(", "),
        if more > 0 { format!(" +{more}") } else { String::new() }
    ));
    body.push(m.status());
    let text = m.qr().map(|q| q.half_blocks(QUIET)).unwrap_or_default();
    let list = if m.addresses.len() > 1 && url.is_some() {
        m.addresses.iter().map(|a| ListRow::new(a.to_string())).collect()
    } else {
        Vec::new()
    };
    DialogVm {
        body,
        list,
        list_state: s.list.clone(),
        text,
        hints: tr!("tui-share-hints").into(),
        width: 76,
        ..DialogVm::from_form(tr!("tools-share-title"), &s.form)
    }
}
