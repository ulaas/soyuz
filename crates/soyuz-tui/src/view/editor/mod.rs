//! The built-in text and hex editors (`soyuz_appcore::editors`) — Phase T4.
//!
//! Stub until T4 fills it: [`placement`] decides whether and where the editor panel goes
//! (`Regions::editor`: `[editor] dock`, maximized), [`prepare`] runs with the core
//! mutable, [`EditorVm::from_core`] builds the model and [`draw_editor`] draws it,
//! recording `EditorTab`, `EditorClose` and `EditorCell` targets (`crate::input::editor`
//! handles them and the keys while `TuiFocus::Editor`). Submodules per editor kind go in
//! this folder.

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;
use soyuz_core::config::EditorDock;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// Where the open editor panel goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorPlacement {
    pub dock: EditorDock,
    /// It fills the panes' area.
    pub maximized: bool,
}

/// The editor panel as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorVm {
    /// The panel is drawn.
    pub visible: bool,
    /// It has the keyboard.
    pub focused: bool,
}

/// Where the editor panel goes, `None` while it is closed (T4: `core.editors.open`,
/// `core.editors.maximized`, `[editor] dock`).
pub fn placement(_core: &AppCore) -> Option<EditorPlacement> {
    None
}

/// Step 1 of a frame (`view::prepare`).
pub fn prepare(_core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {}

impl EditorVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> EditorVm {
        EditorVm { visible: placement(core).is_some(), focused: input.editor.focused }
    }
}

/// Draw the panel into `area` (`Regions::editor`).
pub fn draw_editor(_f: &mut Frame<'_>, area: Rect, m: &EditorVm, _theme: &TuiTheme, _hits: &mut Hits) {
    if area.is_empty() || !m.visible {
        // T4: the editor.
    }
}
