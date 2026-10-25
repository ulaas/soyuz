//! Shared panels that live in one window at a time: the editors, the assistant panel, the
//! drop shelf and the tool windows (treemap, compare, duplicates…). Each stays one
//! instance per process (its state is not per window); [`Owners`] says which window draws
//! it, like `WorkspacesState::player_window` does for the media player.
//!
//! * Opening one from a window ([`claim`]) shows it there; invoking it from another window
//!   moves it there ([`toggle_closes`]: a toggle in a window that doesn't show the panel
//!   brings it over instead of closing it). Requests from outside a window (an MCP
//!   approval) open it in the focused window ([`summon`]).
//! * When its window closes, the panel goes to the main window ([`Owners::hand_over`]);
//!   when the main window closes and another takes over, the panels of both end up in the
//!   new main window. Unsaved editor documents simply move along (nothing is closed, so
//!   nothing is asked); quitting still asks (`editors::intercept_close`).
//! * Keys reach a panel only in its own window ([`here`]).
//! * The session remembers the editors' window (`WindowSession::editors`).
//! * `[workspace] panels_follow_window = false` keeps them all in the main window.

use std::cell::Cell;

use crate::core::AppCore;
use crate::state::workspace_windows::WindowId;

/// A shared panel that lives in one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Editors,
    Assistant,
    Shelf,
    /// The tool windows (all of them together, with their backdrop).
    Tools,
}

/// The window of every shared panel (the main window until one is opened elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owners {
    pub editors: WindowId,
    pub assistant: WindowId,
    pub shelf: WindowId,
    pub tools: WindowId,
}

impl Default for Owners {
    fn default() -> Self {
        Owners { editors: WindowId::MAIN, assistant: WindowId::MAIN, shelf: WindowId::MAIN, tools: WindowId::MAIN }
    }
}

impl Owners {
    pub fn get(&self, p: Panel) -> WindowId {
        match p {
            Panel::Editors => self.editors,
            Panel::Assistant => self.assistant,
            Panel::Shelf => self.shelf,
            Panel::Tools => self.tools,
        }
    }

    fn slot(&mut self, p: Panel) -> &mut WindowId {
        match p {
            Panel::Editors => &mut self.editors,
            Panel::Assistant => &mut self.assistant,
            Panel::Shelf => &mut self.shelf,
            Panel::Tools => &mut self.tools,
        }
    }

    /// Window `gone` closed (or was taken over): what it showed goes to `heir`.
    pub fn hand_over(&mut self, gone: WindowId, heir: WindowId) {
        for p in [Panel::Editors, Panel::Assistant, Panel::Shelf, Panel::Tools] {
            let s = self.slot(p);
            if *s == gone {
                *s = heir;
            }
        }
    }
}

/// The window that shows `p` (the main window when its window is gone).
pub fn owner(app: &AppCore, p: Panel) -> WindowId {
    let o = app.workspaces.panels.get(p);
    if super::windows::exists(app, o) { o } else { WindowId::MAIN }
}

/// This frame's window shows `p` (and takes its keys).
pub fn here(app: &AppCore, p: Panel) -> bool {
    owner(app, p) == app.workspaces.current
}

/// Show `p` in the current window (opened or invoked from it). Returns `true` when it
/// moved from another window.
pub fn claim(app: &mut AppCore, p: Panel) -> bool {
    move_to(app, p, app.workspaces.current)
}

/// Show `p` in window `id` (the main window with `[workspace] panels_follow_window =
/// false`); a panel that moves asks for the keyboard in its new window.
pub fn move_to(app: &mut AppCore, p: Panel, id: WindowId) -> bool {
    let id = if app.config.workspace.panels_follow_window { id } else { WindowId::MAIN };
    let was = owner(app, p);
    *app.workspaces.panels.slot(p) = id;
    if was == id {
        return false;
    }
    match p {
        Panel::Editors => {
            if app.editors.open
                && let Some(d) = app.editors.active_doc_mut()
            {
                d.request_focus();
            }
        }
        Panel::Assistant => {
            if app.ai.panel.open {
                app.ai.panel.focus_composer = true;
            }
        }
        Panel::Shelf | Panel::Tools => {}
    }
    true
}

/// A toggle of `p` (shown: `open`) in the current window: `true` when it should close (it
/// is shown here). Otherwise `p` now belongs to this window (moved from another, or about
/// to open) and stays / becomes open.
pub fn toggle_closes(app: &mut AppCore, p: Panel, open: bool) -> bool {
    if open && (here(app, p) || !app.config.workspace.panels_follow_window) {
        return true;
    }
    claim(app, p);
    false
}

/// `p` is about to open without a window asking (an MCP request): a closed panel opens in
/// the focused window, an open one stays where it is.
pub fn summon(app: &mut AppCore, p: Panel, open: bool) {
    if !open {
        let id = super::windows::focused(app);
        move_to(app, p, id);
    }
}

thread_local! {
    static CURRENT: Cell<WindowId> = const { Cell::new(WindowId::MAIN) };
}

/// Mirror of `app.workspaces.current` for code without the app (the tool queue,
/// `models::tools::queue`).
pub(crate) fn note_current(id: WindowId) {
    CURRENT.set(id);
}

/// The window being handled right now (see [`note_current`]).
pub fn current_window() -> WindowId {
    CURRENT.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_over_moves_only_the_gone_windows_panels() {
        let (a, b) = (WindowId(701), WindowId(702));
        let mut o = Owners { editors: a, assistant: b, shelf: a, tools: WindowId::MAIN };
        o.hand_over(a, WindowId::MAIN);
        assert_eq!(o, Owners { editors: WindowId::MAIN, assistant: b, shelf: WindowId::MAIN, tools: WindowId::MAIN });
        o.hand_over(b, a);
        assert_eq!(o.get(Panel::Assistant), a);
    }
}
