//! The view of the embedded terminal pane (Phase T5, `[tui] shell_mode = "pane"`): drawn into
//! `Regions::bottom` (shared with the jobs panel by tab).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-terminal-pane fills it (the model's fields,
//! `prepare`, the drawing).

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalVm {}

impl TerminalVm {
    pub fn from_core(_core: &AppCore, _input: &InputState, _regions: &Regions) -> TerminalVm {
        TerminalVm::default()
    }
}

/// It is drawn this frame.
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// Rows the pane wants (`[tui] terminal_rows`, clamped); `Facts::bottom_rows` while
/// [`shown`].
pub fn rows(core: &AppCore) -> u16 {
    core.config.tui.terminal_rows_clamped()
}

/// Step 1 of a frame (`view::prepare`): what the view needs from `&mut AppCore`.
pub fn prepare(core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {
    // No view yet: the core's keyboard focus on the terminal is given back at once (the
    // reset `unsupported.rs` did before Phase T5), with a "not available" toast.
    if soyuz_appcore::models::terminal::owns_keyboard(core) {
        core.ui.terminal.focused = false;
        core.tui_unavailable();
    }
}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_terminal(_f: &mut Frame<'_>, _area: Rect, _m: &TerminalVm, _theme: &TuiTheme, _hits: &mut Hits) {}
