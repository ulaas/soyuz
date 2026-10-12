//! `soyuz-appcore`: the frontend-agnostic half of the Soyuz application.
//!
//! Architecture: `core::AppCore` owns the config, the `state::workspace` (tabs → panes),
//! `services` (lister pool, watcher, thumbnails, places), the jobs engine and the
//! frontend-agnostic models. A frontend (`soyuz-app`'s egui `SoyuzApp`, a terminal UI
//! later) holds an `AppCore` and runs its frame phases (see `core.rs`), talking to it
//! through `host::Host`. Widgets push `cmd::Cmd`s; `ops` applies them;
//! `actions::execute` is the single dispatcher for key bindings, menus and toolbar buttons.
//!
//! This crate must never depend on a GUI toolkit (egui, eframe, epaint, winit, wgpu):
//! `tests/boundary.rs` checks the resolved dependency graph.
// `unsafe` is denied everywhere except the raw OS clipboard shim
// (`services::os_clipboard::windows`), which opts in locally.
#![deny(unsafe_code)]

#[macro_use]
pub mod i18n;

pub mod actions;
pub mod ai;
pub mod automation;
pub mod cmd;
pub mod color;
pub mod commands;
pub mod core;
pub mod editors;
pub mod file_class;
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

/// The git commit this binary is built from (short hash, `-dirty` with uncommitted
/// changes, `unknown` outside a checkout); stamped by `build.rs`.
pub const GIT_SHA: &str = env!("SOYUZ_GIT_SHA");
