//! The preview pane: text, code, markdown, hex, images, archive listings and metadata —
//! Phase T4.
//!
//! Stub until T4 fills it: [`shown`] decides whether the pane takes room
//! (`Regions::preview`, `[tui] preview_ratio`), [`prepare`] runs with the core mutable
//! (requests, image protocols via `crate::images`), [`PreviewVm::from_core`] builds the
//! model and [`draw_preview`] draws it, recording `PreviewScroll` and `ArchiveRow`
//! targets (`crate::input::preview` handles them, the `PreviewScroll*` / `PreviewPage*`
//! actions and the keys while `TuiFocus::Preview`).

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// The preview pane as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewVm {
    /// The pane is drawn.
    pub visible: bool,
    /// It has the keyboard.
    pub focused: bool,
}

/// Whether the preview pane is open (T4: `core.ui.preview.open`).
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// Step 1 of a frame (`view::prepare`).
pub fn prepare(_core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {}

impl PreviewVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> PreviewVm {
        PreviewVm { visible: shown(core), focused: input.preview.focused }
    }
}

/// Draw the pane into `area` (`Regions::preview`).
pub fn draw_preview(_f: &mut Frame<'_>, area: Rect, m: &PreviewVm, _theme: &TuiTheme, _hits: &mut Hits) {
    if area.is_empty() || !m.visible {
        // T4: the pane.
    }
}
