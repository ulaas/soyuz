//! Frontend-agnostic state models (Phase T0): the state and rules behind panels, toasts
//! and other UI parts, without drawing. `ui/` draws them for the GUI.

pub mod frontend;
pub mod panels;
pub mod properties;
pub mod quicklook;
pub mod settings;
pub mod terminal;
pub mod toasts;
pub mod tools;
pub mod trash;
