//! `soyuz-app` library half: everything except process startup lives here so pure logic
//! modules can be unit-tested and reused (the `soyuz` binary is a thin `main.rs`).
//!
//! Architecture: `core::AppCore` owns the config, the `state::workspace` (tabs → panes),
//! `services` (lister pool, watcher, thumbnails, places), the jobs engine and the
//! frontend-agnostic models; `app::SoyuzApp` (the egui frontend) holds an `AppCore` plus
//! the GUI-only `app::GuiState` and runs the core's frame phases (see `core.rs`).
//! Widgets push `cmd::Cmd`s; `ops` applies them; `actions::execute` is the single
//! dispatcher for key bindings, menus and toolbar buttons.
// `unsafe` is denied everywhere except the raw OS clipboard shim
// (`services::os_clipboard::windows`), which opts in locally.
#![deny(unsafe_code)]

#[macro_use]
pub mod i18n;

pub mod actions;
pub mod ai;
pub mod app;
pub mod app_icon;
pub mod automation;
pub mod cmd;
pub mod color;
pub mod commands;
pub mod core;
pub mod devscript;
pub mod editors;
pub mod geom;
pub mod host;
pub mod icon;
pub mod keys;
pub mod models;
pub mod ops;
pub mod palette;
pub mod rename;
pub mod services;
pub mod state;
pub mod theme;
pub mod ui;

pub use app::SoyuzApp;

/// The git commit this binary is built from (short hash, `-dirty` with uncommitted
/// changes, `unknown` outside a checkout); stamped by `build.rs`.
pub const GIT_SHA: &str = env!("SOYUZ_GIT_SHA");
