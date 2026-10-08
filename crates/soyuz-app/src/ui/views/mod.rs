//! Pane content views. `flow` renders the details, grid, gallery and list views (all
//! virtualized, painter-only rows with one interaction surface, so 100k entries stay
//! smooth); `miller` composes flow columns; `computer` is the drives page.

pub mod columns;
pub mod computer;
pub mod flow;
pub mod icons;
pub mod layout;
pub mod miller;

use soyuz_core::config::{Config, Keymap};
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::services::Services;

/// Everything a view needs besides the pane itself.
pub struct ViewCtx<'a> {
    pub config: &'a Config,
    pub keymap: &'a Keymap,
    pub services: &'a mut Services,
    pub cmds: &'a mut Vec<Cmd>,
    /// This pane is the focused pane of the active tab.
    pub focused: bool,
    /// Keyboard focus is in the file list (draws the cursor ring).
    pub list_focus: bool,
    pub checkbox_mode: bool,
    pub show_hidden: bool,
    pub show_system: bool,
    pub volumes: &'a [VolumeInfo],
    /// Dual pane is available (enables "open in other pane").
    pub dual: bool,
}
