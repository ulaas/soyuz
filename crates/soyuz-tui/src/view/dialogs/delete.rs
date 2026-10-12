//! The view of the delete / purge / empty-trash confirmation
//! (`DialogModel::DeleteConfirm`): the model's message, the first five names (and how
//! many more), Cancel and the confirm button, drawn as an alert.

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;

/// Names listed at most (the rest is "and N more"), like the GUI.
pub const SHOWN: usize = 5;

/// What the dialog shows.
pub fn vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    let DialogModel::DeleteConfirm(d) = m else { return crate::dialog::fallback(m) };
    let (msg, _, _) = d.texts();
    let mut body = vec![safe(&msg)];
    let names = d.names();
    if !names.is_empty() {
        body.push(String::new());
    }
    body.extend(names.iter().take(SHOWN).map(|n| format!("  {}", safe(n))));
    if names.len() > SHOWN {
        body.push(format!("  {}", tr!("dialogs-delete-and-more", count = names.len() - SHOWN)));
    }
    DialogVm {
        body,
        hints: tr!("tui-dialogs-hint-confirm").to_string(),
        alert: true,
        width: 64,
        ..DialogVm::from_form(m.title(), &ui.form)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use soyuz_appcore::models::dialogs::DeleteConfirmModel;
    use soyuz_jobs::DeleteMode;

    use super::*;

    #[test]
    fn lists_five_names_and_the_rest() {
        let paths: Vec<PathBuf> = (0..8).map(|i| PathBuf::from(format!("f{i}\x07.txt"))).collect();
        let m: DialogModel = DeleteConfirmModel::new(paths, DeleteMode::Permanent).into();
        let d = vm(&m, &crate::input::dialogs::delete::ui(&m));
        assert!(d.title.contains('8'), "{}", d.title);
        let names = d.body.iter().filter(|l| l.starts_with("  f")).count();
        assert_eq!(names, SHOWN);
        assert!(d.body.iter().any(|l| l.contains("3 more")), "{:?}", d.body);
        assert!(!d.body.concat().chars().any(char::is_control));
        assert_eq!(d.buttons.len(), 2);
        assert!(d.buttons[1].default && d.buttons[1].focused);
        let m: DialogModel = DeleteConfirmModel::empty_trash(2).into();
        let d = vm(&m, &crate::input::dialogs::delete::ui(&m));
        assert_eq!(d.body.len(), 1);
    }
}
