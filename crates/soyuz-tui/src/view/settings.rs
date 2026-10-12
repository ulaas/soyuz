//! The view of the settings editor and its key bindings page, a full-screen overlay (Phase T6):
//! drawn into `Regions::area` (drawn after the compare screen, `HitTarget::Overlay` first).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-settings-editor (tables and secrets: W3) fills it
//! (the model's fields, `prepare`, the drawing).

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsVm {}

impl SettingsVm {
    pub fn from_core(_core: &AppCore, _input: &InputState, _regions: &Regions) -> SettingsVm {
        SettingsVm::default()
    }
}

/// It is drawn this frame.
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// Step 1 of a frame (`view::prepare`): what the view needs from `&mut AppCore`.
pub fn prepare(_core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_settings(_f: &mut Frame<'_>, _area: Rect, _m: &SettingsVm, _theme: &TuiTheme, _hits: &mut Hits) {}
