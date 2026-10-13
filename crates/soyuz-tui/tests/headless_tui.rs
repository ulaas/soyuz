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
    /// Colour tags and git status in the list (the core's decoration pumps).
    mod tags;
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
    // ---- Phase T5 + T6 ----
    /// T5: the AI assistant panel.
    mod ai;
    /// T5: the multi-line AI composer and JSON argument edits.
    mod ai_composer;
    /// T5: archive dialogs.
    mod archive;
    /// T5: the folder compare screen.
    mod compare;
    /// T5: editing and scheduling sync profiles.
    mod compare_profiles;
    /// T6: the key bindings page.
    mod keys_editor;
    /// T5: MCP approval cards in the AI panel.
    mod mcp_panel;
    /// T5 + T6 polish (wave 4): toasts under the search bar, hints, middle click,
    /// completion, token generation, field clicks.
    mod polish;
    /// T5: remote locations.
    mod remote;
    /// T5: the search bar.
    mod search;
    /// T6: the settings editor.
    mod settings;
    /// T6: arrays / maps of tables and secrets in the settings editor.
    mod settings_tables;
    /// T5: smart folders.
    mod smart;
    /// T5 / T6: suspend to a shell or a text editor.
    mod suspend;
    /// T5: the embedded terminal pane.
    mod terminal;
    // ---- Phase T7 ----
    /// T7 (agent B): device rows and menus in the sidebar, unlocking, the read-only offer.
    mod devices;
    /// T7 (agent C): the duplicate finder screen.
    mod dupes;
    /// T7 (places): the Trash and Computer pages.
    mod places;
    /// T7 (agent A): the drop shelf dialog.
    mod shelf;
    /// T7 (agent A): split and join files.
    mod split;
    /// T7 (agent C): the disk usage treemap screen.
    mod treemap;
    /// T7 (agent B): previous versions (and from Properties).
    mod versions;
    /// T7 (agent A): workspaces (save as, manage, hotkeys), tab history, the close-window path and About.
    mod workspaces;
    // ---- Integration ----
    /// T3 + T4 end to end: copy / paste with a conflict, the jobs history, rename,
    /// preview, Quick Look, the editor.
    mod e2e;
}
