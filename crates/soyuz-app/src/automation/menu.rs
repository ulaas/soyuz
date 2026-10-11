//! Script actions in the right-click menu and the command palette.

use std::path::PathBuf;

use soyuz_core::config::{Action, CommandContext};
use soyuz_script::{ActionCtx, MenuContext};

/// Palette entries: every script action that wants to be in the palette.
pub fn palette_actions() -> Vec<Action> {
    let Some(set) = super::scripts::current() else { return Vec::new() };
    set.actions().filter(|(_, _, a)| a.palette).map(|(id, ..)| Action::RunScript(id)).collect()
}

/// Palette detail line of a script action.
pub fn palette_detail(id: &str) -> Option<String> {
    let set = super::scripts::current()?;
    let (sc, a) = set.action(id)?;
    let desc = if a.description.trim().is_empty() {
        tr!("automation-script-action").to_string()
    } else {
        a.description.clone()
    };
    Some(format!("{desc} · {}", sc.file))
}

/// One script action in the right-click menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextEntry {
    pub id: String,
    pub title: String,
    /// Hotkey hint (may be blank).
    pub hotkey: String,
}

/// Right-click entries of script actions whose conditions hold for the selection.
/// `selected` are the selected paths (empty for the folder background).
pub fn context_entries(kinds: &[CommandContext], dir: Option<PathBuf>, selected: &[PathBuf]) -> Vec<ContextEntry> {
    let Some(set) = super::scripts::current() else { return Vec::new() };
    let kinds: Vec<MenuContext> = kinds
        .iter()
        .map(|k| match k {
            CommandContext::File => MenuContext::File,
            CommandContext::Dir => MenuContext::Dir,
            CommandContext::Background => MenuContext::Background,
        })
        .collect();
    let exts: Vec<String> = selected
        .iter()
        .filter_map(|p| p.file_name())
        .filter_map(|n| soyuz_script::api::name_ext(&n.to_string_lossy()).map(str::to_ascii_lowercase))
        .collect();
    let ctx = ActionCtx { dir, focused: selected.first().cloned(), selected: selected.to_vec(), other_dir: None };
    set.actions()
        .filter(|(_, _, a)| a.menu)
        .filter(|(id, ..)| set.applies(id, &ctx, &kinds, &exts))
        .map(|(id, _, a)| ContextEntry { id, title: a.title.clone(), hotkey: a.hotkey.clone() })
        .collect()
}
