//! The Soyuz terminal UI (`soyuz-tui`, `soyuz --tui`; roadmap Phase T): a ratatui frontend that drives
//! the same `soyuz_appcore::core::AppCore` as the egui GUI.
//!
//! Phase T1 built the shell: the terminal host, the key adapter, the frame loop and a
//! minimal pane view. Phase T2 brings browsing parity on the skeleton of T2.0:
//! `layout` (screen regions, small-terminal rules), `hits` (click targets), `theme`
//! (colour roles), `icons`, `view/*` (one model + draw function per part), `input/*`
//! (focus and key routing) and `widgets` (the line editor).
//!
//! Phases T3 / T4 add `widgets::{form, menu, select_list}`, the dialog framework
//! (`input::dialogs`, `view::dialogs`: both dialog stacks, one file per kind; shared
//! helpers in `dialog`), the panels (`view::{jobs, preview, editor, quicklook, menu,
//! rename}` with their `input::*` modules), grid and gallery views, `frontend` (the
//! core's `FrontendCmd` requests), `clipboard` (`[tui] osc52`) and `images`
//! (`[tui] images`).
//!
//! Phases T5 / T6 add the embedded terminal, the search bar, the AI panel, the compare
//! screen and the settings editor (`view::{terminal, search, ai, compare, settings}` with
//! their `input::*` modules), the location dialogs (smart folders, remotes, archives),
//! `suspend` (leave the screen for a shell or an editor) and `tools` (the core's tool
//! queue).
//!
//! Entry points: [`launch()`] (console, configuration, then [`run`]) and
//! [`launch::init_logging`], shared by this crate's `soyuz-tui [PATH]` binary
//! (`src/main.rs`) and `soyuz --tui [PATH]` (`soyuz-app`'s `main.rs`).
#![deny(unsafe_code)]

pub mod app;
pub mod borders;
pub mod clipboard;
pub mod dialog;
pub mod frontend;
pub mod hits;
pub mod host;
pub mod icons;
pub mod images;
pub mod input;
pub mod keys;
pub mod launch;
pub mod layout;
pub mod overlay;
pub mod signals;
pub mod suspend;
pub mod term;
pub mod theme;
pub mod tools;
pub mod view;
pub mod widgets;

pub use app::run;
pub use launch::launch;
