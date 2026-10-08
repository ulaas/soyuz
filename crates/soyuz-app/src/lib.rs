//! `soyuz-app` library half: everything except process startup lives here so pure logic
//! modules can be unit-tested and reused (the `soyuz` binary is a thin `main.rs`).
//!
//! Architecture (PLAN §2.8, §3): `app::SoyuzApp` owns the config, the `state::workspace`
//! (tabs → panes), `services` (lister pool, watcher, thumbnails, places) and UI state.
//! Widgets push `cmd::Cmd`s; `ops` applies them; `actions::execute` is the single
//! dispatcher for key bindings, menus and toolbar buttons.
// `unsafe` is denied everywhere except the raw OS clipboard shim
// (`services::os_clipboard::windows`), which opts in locally.
#![deny(unsafe_code)]

pub mod actions;
pub mod app;
pub mod cmd;
pub mod keys;
pub mod ops;
pub mod rename;
pub mod services;
pub mod state;
pub mod theme;
pub mod ui;

pub use app::SoyuzApp;
