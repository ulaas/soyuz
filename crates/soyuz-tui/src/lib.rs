//! The Soyuz terminal UI (`soyuz --tui`, roadmap Phase T): a ratatui frontend that drives
//! the same `soyuz_appcore::core::AppCore` as the egui GUI.
//!
//! Phase T1 builds the shell: the terminal host, the key adapter, the frame loop and a
//! minimal pane view. Browsing parity follows in T2.
//!
//! Entry point: [`run`] (`soyuz --tui [PATH]`, wired in `soyuz-app`'s `main.rs`).
#![deny(unsafe_code)]

pub mod app;
pub mod dialog;
pub mod host;
pub mod keys;
pub mod overlay;
pub mod signals;
pub mod term;
pub mod view;

pub use app::run;
