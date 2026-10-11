//! Frontend-agnostic state models (Phase T0): the state and rules behind panels, toasts
//! and other UI parts, without drawing. `ui/` draws them for the GUI.

pub mod devices;
pub mod frontend;
pub mod portal;
pub mod settings;
pub mod sidebar;
pub mod toasts;
pub mod tree;
pub mod windows;
pub mod workspaces;
