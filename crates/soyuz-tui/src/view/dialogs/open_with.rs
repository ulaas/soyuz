//! The view of open with (`FrontendCmd::OpenWith`): "Looking up applications…", the
//! empty state, or the applications as a list; then the program field and the buttons.
//! Browse replaces it with the folder, its folders and programs, and the query.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::open_with as model;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::open_with::{Browser, State};
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    if let Some(b) = &s.browser {
        return browser_vm(b);
    }
    let (body, list) = if !s.preselected {
        (vec![tr!("dialogs-openwith-looking").to_string()], Vec::new())
    } else if s.listed.is_empty() {
        (vec![tr!("dialogs-openwith-none").to_string()], Vec::new())
    } else {
        (Vec::new(), s.listed.iter().map(|a| ListRow::new(safe(&app_row(a)))).collect())
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

/// A row: the application, and the folder of a program picked by path.
fn app_row(app: &soyuz_fs::AppInfo) -> String {
    let label = model::app_label(app);
    match app.exec.parent().filter(|_| model::is_by_path(app)) {
        Some(dir) => format!("{label}  {}", dir.display()),
        None => label,
    }
}

fn browser_vm(b: &Browser) -> DialogVm {
    let mut body = vec![safe(&b.files.dir().display().to_string())];
    if b.files.loading() {
        body.push(tr!("common-loading").to_string());
    } else if let Some(e) = b.files.error() {
        body.push(safe(e));
    } else if b.files.is_empty() {
        body.push(tr!("app-picker-no-items").to_string());
    }
    let sep = std::path::MAIN_SEPARATOR;
    let list = b
        .files
        .rows()
        .map(|e| ListRow::new(safe(&if e.is_dir { format!("{}{sep}", e.name) } else { e.name.clone() })))
        .collect();
    DialogVm {
        body,
        list,
        list_state: b.list.clone(),
        hints: tr!("tui-requests-openwith-browse-hints").into(),
        width: 72,
        ..DialogVm::from_form(tr!("dialogs-openwith-browse-title"), &b.form)
    }
}
