//! Frontend-agnostic state models (Phase T0): the state and rules behind panels, toasts
//! and other UI parts, without drawing. `ui/` draws them for the GUI.

pub mod archive;
pub mod context_menu;
pub mod devices;
pub mod dialogs;
pub mod frontend;
pub mod jobs_rows;
pub mod panels;
pub mod portal;
pub mod preview;
pub mod preview_hex;
pub mod preview_info;
pub mod preview_load;
pub mod properties;
pub mod quicklook;
pub mod remote;
pub mod requests;
pub mod search;
pub mod settings;
pub mod sidebar;
pub mod smart;
pub mod terminal;
pub mod toasts;
pub mod tools;
pub mod trash;
pub mod tree;
pub mod windows;
pub mod workspaces;
