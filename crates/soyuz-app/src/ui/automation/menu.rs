//! Script actions in the right-click menu ([`crate::automation::menu`] picks them).

use std::path::PathBuf;

use egui::Ui;
use soyuz_core::config::{Action, CommandContext};

use crate::automation::menu::context_entries;
use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::theme::icons::ph;
use crate::ui::widgets::{menu_item, menu_separator};

/// Right-click entries of script actions whose conditions hold for the selection.
/// `selected` are the selected paths (empty for the folder background).
pub fn context_items(
    ui: &mut Ui,
    cmds: &mut Vec<Cmd>,
    pane: PaneId,
    kinds: &[CommandContext],
    dir: Option<PathBuf>,
    selected: &[PathBuf],
) {
    let items = context_entries(kinds, dir, selected);
    if items.is_empty() {
        return;
    }
    menu_separator(ui);
    for e in items {
        let hint = (!e.hotkey.trim().is_empty()).then_some(e.hotkey.as_str());
        if menu_item(ui, Some(ph::CODE), &e.title, hint, true).clicked() {
            cmds.push(Cmd::PaneAction(pane, Action::RunScript(e.id)));
            ui.close();
        }
    }
}
