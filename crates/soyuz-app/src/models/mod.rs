//! Frontend-agnostic state models (Phase T0): the state and rules behind panels, toasts
//! and other UI parts, without drawing. `ui/` draws them for the GUI.

pub mod archive;
pub mod frontend;
pub mod remote;
pub mod search;
pub mod settings;
pub mod smart;
pub mod toasts;
