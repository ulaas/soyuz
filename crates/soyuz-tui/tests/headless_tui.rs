//! The terminal UI end to end, without a terminal: the real frame loop (`run_loop`) on
//! ratatui's `TestBackend`, driven by scripted events, each test in its own portable
//! sandbox (`common::relaunch`).
//!
//! One test binary for every headless test; each step adds its `#[test]`s to its own file
//! under `tests/headless/` (Phase T3 / T4 files are pre-created, one per part).

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
    /// A terminal paste in the file list runs Paste.
    mod paste;
    /// T2.6: sidebar.
    mod sidebar;
    /// T2.8: status line, filter, toasts, borders.
    mod status;
    /// T2.4: tabs, dual and quad panes.
    mod tabs;
    /// T2.2: theme, colours, icons.
    mod theme;
    // ---- Phase T3 ----
    /// T3: batch rename.
    mod batch;
    /// T3: the clipboard (OSC 52, files).
    mod clipboard;
    /// T3: the dialog stacks (core prompts and terminal UI dialogs).
    mod dialogs;
    /// T3: the jobs panel.
    mod jobs;
    /// T3: the context menu.
    mod menu;
    /// T3: copy, move, trash, new items, links through the jobs engine.
    mod ops;
    /// T3: properties and attributes.
    mod props;
    /// T3: interrupted jobs and crash reports.
    mod recovery;
    /// T3: inline rename.
    mod rename;
    /// T3: confirm, select by pattern, columns, open with.
    mod requests;
    // ---- Phase T4 ----
    /// T4: the editors (and quitting with unsaved changes).
    mod editor;
    /// T4: grid and gallery views.
    mod grid;
    /// T4: images (`[tui] images`).
    mod images;
    /// T4: the preview pane.
    mod preview;
    /// T4: Quick Look.
    mod quicklook;
    // ---- Integration ----
    /// T3 + T4 end to end: copy / paste with a conflict, the jobs history, rename,
    /// preview, Quick Look, the editor.
    mod e2e;
}
