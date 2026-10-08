//! `soyuz-app` library half: everything except process startup lives here so pure logic
//! modules can be unit-tested and reused (the `soyuz` binary is a thin `main.rs`).
#![forbid(unsafe_code)]

pub mod app;
pub mod theme;
pub mod ui;

pub use app::SoyuzApp;
