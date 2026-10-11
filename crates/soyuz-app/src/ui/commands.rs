//! Context-menu entries of the user-defined commands ([`crate::commands`]).

use egui::Ui;
use soyuz_core::config::{Action, CommandContext, Config};

use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::theme::icons::ph;
use crate::ui::widgets::{menu_item, menu_separator};

/// Context-menu entries for the commands that apply to `kinds`.
pub fn context_items(ui: &mut Ui, config: &Config, cmds: &mut Vec<Cmd>, pane: PaneId, kinds: &[CommandContext]) {
    let list = crate::commands::context_entries(config, kinds);
    if list.is_empty() {
        return;
    }
    menu_separator(ui);
    for c in list {
        let glyph = if c.terminal { ph::TERMINAL } else { ph::LIGHTNING };
        let hint = (!c.hotkey.trim().is_empty()).then_some(c.hotkey.as_str());
        if menu_item(ui, Some(glyph), &c.name, hint, true).clicked() {
            cmds.push(Cmd::PaneAction(pane, Action::CustomCommand(c.name.clone())));
            ui.close();
        }
    }
}
