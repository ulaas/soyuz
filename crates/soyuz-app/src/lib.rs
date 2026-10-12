//! `soyuz-app`: the egui frontend of Soyuz (the `soyuz` binary is a thin `main.rs`).
//!
//! Everything frontend-agnostic lives in `soyuz-appcore` (`AppCore`, models, commands,
//! services, i18n) and is re-exported here under the same module paths, so GUI code keeps
//! writing `crate::models::…`. This crate adds the egui side: `app::SoyuzApp` (an `AppCore`
//! plus the GUI-only `app::GuiState`), `ui`, `theme` and the egui [`host::EguiHost`].
// `unsafe` is denied in the whole GUI crate.
#![deny(unsafe_code)]

// `tr!` (exported by `soyuz_appcore::i18n`).
#[macro_use]
extern crate soyuz_appcore;

pub use soyuz_appcore::{
    GIT_SHA, actions, ai, automation, cmd, color, commands, core, editors, geom, i18n, icon, keys, models, ops,
    palette, rename, services, state,
};

pub mod app;
pub mod app_icon;
pub mod devscript;
pub mod host;
pub mod theme;
pub mod ui;

pub use app::SoyuzApp;
