//! The view of the dialog for naming a recipe (`TuiDialog::RecipeName`): what a recipe
//! is, the name field and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::recipe_name::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm {
        body: vec![tr!("ai-save-recipe-tip").into()],
        hints: tr!("tui-ai-recipe-hints").into(),
        width: 60,
        ..DialogVm::from_form(tr!("ai-save-recipe"), &s.form)
    }
}
