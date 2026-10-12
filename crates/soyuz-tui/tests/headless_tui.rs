//! The terminal UI end to end, without a terminal: the real frame loop (`run_loop`) on
//! ratatui's `TestBackend`, driven by scripted events, each test in its own portable
//! sandbox (`common::relaunch`).
//!
//! One test binary for every headless test; each Phase T2 step adds its `#[test]`s to its
//! own file under `tests/headless/`.

mod common;

mod headless {
    /// Phase T1: browse, confirm, quit.
    mod browse;
    /// T2.1: `[tui]` settings.
    mod config;
    /// T2.5: breadcrumbs and the address field.
    mod crumbs;
    /// T2.9: mouse.
    mod mouse;
    /// T2.7: command palette.
    mod palette;
    /// T2.3: pane views.
    mod panes;
    /// T2.6: sidebar.
    mod sidebar;
    /// T2.8: status line, filter, toasts, borders.
    mod status;
    /// T2.4: tabs, dual and quad panes.
    mod tabs;
    /// T2.2: theme, colours, icons.
    mod theme;
}
