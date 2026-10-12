//! The inline rename field on a file row (F2) — Phase T3.
//!
//! The rename itself is the core's `fileops.rename` (`InlineRename`); the TUI keeps the
//! field's cursor in `crate::input::rename::State`. [`decorate`] puts an [`EditVm`] on the
//! row being renamed (`Row::edit`) after the panes' models are built, and `draw_pane`
//! calls [`draw_edit`] for such a row. Stubs until T3 fills them.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use super::pane::PaneVm;
use crate::input::InputState;
use crate::layout::Hits;
use crate::theme::TuiTheme;
use crate::widgets::LineEdit;

/// A row's inline edit field as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditVm {
    pub text: String,
    /// Cursor (and scroll) of the field.
    pub edit: LineEdit,
    /// Why the name cannot be used, if it cannot.
    pub error: Option<String>,
}

/// Put the rename field on its row (`ViewModel::from_core`).
pub fn decorate(_core: &AppCore, _input: &InputState, _panes: &mut [PaneVm]) {}

/// Draw the field of a row into `row` (the whole row; `prefix` cells of mark and icon
/// come first), recording a `RenameField` target.
pub fn draw_edit(_buf: &mut Buffer, _row: Rect, _prefix: u16, _e: &EditVm, _theme: &TuiTheme, _hits: &mut Hits) {}
