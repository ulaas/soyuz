//! The Soyuz terminal UI (`soyuz --tui`, roadmap Phase T): a ratatui frontend that drives
//! the same `soyuz_appcore::core::AppCore` as the egui GUI.
//!
//! Phase T1 built the shell: the terminal host, the key adapter, the frame loop and a
//! minimal pane view. Phase T2 brings browsing parity on the skeleton of T2.0:
//! `layout` (screen regions, small-terminal rules), `hits` (click targets), `theme`
//! (colour roles), `icons`, `view/*` (one model + draw function per part), `input/*`
//! (focus and key routing), `widgets` (the line editor) and `unsupported` (features
//! without a view yet).
//!
//! Entry point: [`run`] (`soyuz --tui [PATH]`, wired in `soyuz-app`'s `main.rs`).
#![deny(unsafe_code)]

pub mod app;
pub mod dialog;
pub mod hits;
pub mod host;
pub mod icons;
pub mod input;
pub mod keys;
pub mod layout;
pub mod overlay;
pub mod signals;
pub mod term;
pub mod theme;
pub mod unsupported;
pub mod view;
pub mod widgets;

pub use app::run;
