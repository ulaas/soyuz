//! The view of open with (`FrontendCmd::OpenWith`): "Looking up applications…", the
//! empty state, or the applications as a list; then the program field and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::open_with as model;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::open_with::State;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let (body, list) = match s.apps.get() {
        None => (vec![tr!("dialogs-openwith-looking").to_string()], Vec::new()),
        Some(l) if l.is_empty() => (vec![tr!("dialogs-openwith-none").to_string()], Vec::new()),
        Some(l) => (Vec::new(), l.iter().map(|a| ListRow::new(safe(&model::app_label(a)))).collect()),
    };
    DialogVm {
        body,
        list,
        list_state: s.list.clone(),
        hints: tr!("tui-requests-openwith-hints").into(),
        width: 64,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
