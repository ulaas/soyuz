//! Changing and saving the active configuration: live apply now, `config.toml` written
//! shortly after (debounced), merged into the user's file so comments survive.

use std::time::{Duration, Instant};

use soyuz_core::config::{self, Config};

use crate::core::AppCore;
use crate::host::Host;
use crate::models::toasts::Toast;

/// A page of the settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Appearance,
    Fonts,
    Views,
    Behaviour,
    Jobs,
    Terminal,
    /// `[tui]`: the terminal UI (`soyuz --tui`), not the embedded terminal panel.
    TerminalUi,
    Preview,
    Editor,
    Keyboard,
    Commands,
    Bookmarks,
    Scripts,
    Automations,
    Ai,
    Advanced,
    About,
}

impl Section {
    pub const ALL: [Section; 18] = [
        Section::General,
        Section::Appearance,
        Section::Fonts,
        Section::Views,
        Section::Behaviour,
        Section::Jobs,
        Section::Terminal,
        Section::TerminalUi,
        Section::Preview,
        Section::Editor,
        Section::Keyboard,
        Section::Commands,
        Section::Bookmarks,
        Section::Scripts,
        Section::Automations,
        Section::Ai,
        Section::Advanced,
        Section::About,
    ];

    /// Stable, untranslated id (`general`, `appearance`, …): dev scripts and the Fluent
    /// keys `settings-section-<id>` / `…-desc` use it.
    pub fn id(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::Fonts => "fonts",
            Section::Views => "views",
            Section::Behaviour => "behaviour",
            Section::Jobs => "jobs",
            Section::Terminal => "terminal",
            Section::TerminalUi => "terminal-ui",
            Section::Preview => "preview",
            Section::Editor => "editor",
            Section::Keyboard => "keyboard",
            Section::Commands => "commands",
            Section::Bookmarks => "bookmarks",
            Section::Scripts => "scripts",
            Section::Automations => "automations",
            Section::Ai => "ai",
            Section::Advanced => "advanced",
            Section::About => "about",
        }
    }

    pub fn title(self) -> &'static str {
        // i18n-dynamic: settings-section-
        crate::i18n::t_dyn(&format!("settings-section-{}", self.id()))
    }

    /// English title (dev scripts select sections by it whatever the UI language).
    pub fn english_title(self) -> &'static str {
        crate::i18n::t_in(&format!("settings-section-{}", self.id()), soyuz_core::Locale::EnUs)
    }

    pub fn description(self) -> &'static str {
        // i18n-dynamic: settings-section-
        crate::i18n::t_dyn(&format!("settings-section-{}-desc", self.id()))
    }

    /// The config paths this page edits: whole tables (`"general"`, `"tui"`, `"bookmarks"`,
    /// sub-tables included) or single keys a page borrows from another page's table
    /// (`"general.default_view"` on Views). Every table of `Config` belongs to exactly one
    /// page; Advanced holds the tables no other page shows. Mirrors [`Section::reset`]
    /// (checked by tests): resetting these paths, minus [`Section::kept`], is that reset.
    pub fn tables(self) -> &'static [&'static str] {
        match self {
            Section::General => &["general"],
            Section::Appearance => &["appearance", "theme"],
            Section::Fonts => &["fonts"],
            Section::Views => &[
                "columns",
                "layout",
                "sidebar",
                "tags",
                "toolbar",
                "context_menu",
                "general.default_view",
                "general.natural_sort",
                "general.case_insensitive_sort",
                "general.folders_first",
            ],
            Section::Behaviour => &[
                "behaviour",
                "general.single_click_open",
                "general.confirm_delete",
                "general.confirm_permanent_delete",
                "general.double_click_speed_ms",
                "general.open_folders_in",
            ],
            Section::Jobs => &["jobs", "sync"],
            Section::Terminal => &["terminal"],
            Section::TerminalUi => &["tui"],
            Section::Preview => &["preview", "media", "git", "search"],
            Section::Editor => &["editor"],
            Section::Keyboard => &["keys"],
            Section::Commands => &["custom_commands"],
            Section::Bookmarks => &["bookmarks"],
            Section::Scripts => &["scripting"],
            Section::Automations => &["automation", "cli"],
            Section::Ai => &["ai"],
            Section::Advanced => &["workspace", "archive", "performance", "remote", "linux"],
            Section::About => &["updates"],
        }
    }

    /// User data inside [`Section::tables`] that a section reset keeps (pinned Portal
    /// folders, saved searches, automation rules, AI profiles and MCP connections).
    pub fn kept(self) -> &'static [&'static str] {
        match self {
            Section::Views => &["sidebar.portal"],
            Section::Preview => &["search.saved"],
            Section::Automations => &["automation.rules"],
            Section::Ai => &["ai.profiles", "ai.mcp_clients", "ai.mcp_servers"],
            _ => &[],
        }
    }

    /// Reset this section of `c` to defaults.
    pub fn reset(self, c: &mut Config) {
        let d = Config::default();
        match self {
            Section::General => c.general = d.general,
            Section::Appearance => {
                c.appearance = d.appearance;
                c.theme = d.theme;
            }
            Section::Fonts => c.fonts = d.fonts,
            Section::Views => {
                c.columns = d.columns;
                c.layout = d.layout;
                // Pinned Portal folders are the user's data, not layout: keep them.
                let portal = std::mem::take(&mut c.sidebar.portal);
                c.sidebar = d.sidebar;
                c.tags = d.tags;
                c.sidebar.portal = portal;
                c.toolbar = d.toolbar;
                c.context_menu = d.context_menu;
                c.general.default_view = d.general.default_view;
                c.general.natural_sort = d.general.natural_sort;
                c.general.case_insensitive_sort = d.general.case_insensitive_sort;
                c.general.folders_first = d.general.folders_first;
            }
            Section::Behaviour => {
                c.behaviour = d.behaviour;
                c.general.single_click_open = d.general.single_click_open;
                c.general.confirm_delete = d.general.confirm_delete;
                c.general.confirm_permanent_delete = d.general.confirm_permanent_delete;
                c.general.double_click_speed_ms = d.general.double_click_speed_ms;
                c.general.open_folders_in = d.general.open_folders_in;
            }
            Section::Jobs => {
                c.jobs = d.jobs;
                c.sync = d.sync;
            }
            Section::Terminal => c.terminal = d.terminal,
            Section::TerminalUi => c.tui = d.tui,
            Section::Preview => {
                c.preview = d.preview;
                c.media = d.media;
                c.git = d.git;
                let saved = std::mem::take(&mut c.search.saved);
                c.search = d.search;
                c.search.saved = saved;
            }
            Section::Editor => c.editor = d.editor,
            Section::Keyboard => c.keys = d.keys,
            Section::Commands => c.custom_commands = d.custom_commands,
            Section::Bookmarks => c.bookmarks = d.bookmarks,
            Section::Scripts => c.scripting = d.scripting,
            Section::Automations => {
                // Keep the rules; reset the switches.
                let rules = std::mem::take(&mut c.automation.rules);
                c.automation = d.automation;
                c.automation.rules = rules;
                c.cli = d.cli;
            }
            Section::Ai => {
                // Keep the user's provider profiles and MCP connections; reset everything else.
                let profiles = std::mem::take(&mut c.ai.profiles);
                let clients = std::mem::take(&mut c.ai.mcp_clients);
                let servers = std::mem::take(&mut c.ai.mcp_servers);
                c.ai = d.ai;
                c.ai.profiles = profiles;
                c.ai.mcp_clients = clients;
                c.ai.mcp_servers = servers;
            }
            Section::Advanced => *c = d,
            Section::About => c.updates = d.updates,
        }
    }
}

/// Whether every word of `query` (case-insensitive) occurs in `texts`; an empty query
/// matches everything. The settings search of both frontends.
pub fn matches(query: &str, texts: &[&str]) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let hay = texts.join(" ").to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// How long a change waits before `config.toml` is written.
pub const SAVE_DELAY: Duration = Duration::from_millis(700);

/// Pending and last write of `config.toml`.
#[derive(Clone, Debug, Default)]
pub struct ConfigSave {
    /// A write is scheduled for this time.
    pub due: Option<Instant>,
    /// When the last write finished (the settings window flashes "Saved").
    pub flashed: Option<Instant>,
}

impl ConfigSave {
    pub fn schedule(&mut self) {
        self.due = Some(Instant::now() + SAVE_DELAY);
    }
}

/// Change the active configuration (UI zoom keys, quick toggles, AI folder access, …):
/// applies live and writes `config.toml` shortly after.
pub fn update_config(app: &mut AppCore, host: &dyn Host, edit: impl FnOnce(&mut Config)) {
    let mut c = app.config.clone();
    edit(&mut c);
    if c == app.config {
        return;
    }
    crate::services::config_watch::apply_config(app, host, c);
    app.config_save.schedule();
    host.redraw_after(SAVE_DELAY);
}

/// Write `config.toml` once the debounce delay has passed.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    if let Some(due) = app.config_save.due {
        let now = Instant::now();
        if now >= due {
            flush(app);
        } else {
            host.redraw_after(due - now);
        }
    }
}

/// Write `config.toml` now if a save is pending (closing settings, app exit).
pub fn flush(app: &mut AppCore) {
    if app.config_save.due.take().is_none() {
        return;
    }
    let path = app.config_watch.path().to_path_buf();
    // Merge into the user's file so comments and formatting survive (toml_edit).
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    match config::to_string_preserving(&existing, &app.config) {
        Ok(text) => {
            if let Some(dir) = path.parent() {
                let _ = soyuz_fs::create_dir_all(dir);
            }
            match soyuz_fs::write_atomic(&path, text.as_bytes()) {
                Ok(()) => {
                    app.config_watch.last_text = Some(text);
                    app.config_save.flashed = Some(Instant::now());
                }
                Err(e) => {
                    app.ui.toasts.push(Toast::error(tr!("settings-save-failed")).body(e.to_string()));
                }
            }
        }
        Err(e) => {
            app.ui.toasts.push(Toast::error(tr!("settings-save-failed")).body(e.to_string()));
        }
    }
}
