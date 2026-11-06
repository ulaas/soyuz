//! Changing and saving the active configuration: live apply now, `config.toml` written
//! shortly after (debounced), merged into the user's file so comments survive.

use std::path::Path;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError, bounded};
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
    /// `[monitor]`: the system monitor.
    Monitor,
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
    pub const ALL: [Section; 19] = [
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
        Section::Monitor,
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
            Section::Monitor => "monitor",
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
                "open",
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
            Section::Monitor => &["monitor"],
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
                c.open = d.open;
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
            Section::Monitor => c.monitor = d.monitor,
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
    /// The configuration as the file last had it: what changed since is what a reload
    /// must not lose ([`config::rebase`]).
    pub baseline: Option<Config>,
    /// A write could not happen (the file does not load, or the disk said no); the changes
    /// are still in memory and a successful reload schedules the write again.
    pub held: bool,
    /// The write running on a worker.
    pub(crate) in_flight: Option<SaveInFlight>,
}

/// A `config.toml` write on a worker, and the configuration it writes.
#[derive(Clone, Debug)]
pub(crate) struct SaveInFlight {
    rx: Receiver<SaveOutcome>,
    config: Config,
}

#[derive(Debug)]
pub(crate) enum SaveOutcome {
    Written(String),
    /// The file was left alone: it could not be read, or the merge failed.
    Refused(String),
    WriteFailed(String),
}

impl ConfigSave {
    pub fn schedule(&mut self) {
        self.due = Some(Instant::now() + SAVE_DELAY);
    }

    /// Settings changed that are not in the file yet.
    pub fn pending(&self) -> bool {
        self.due.is_some() || self.held
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

/// How often a frame looks for the worker's answer (a write takes milliseconds).
const SAVE_POLL: Duration = Duration::from_millis(20);

/// Write `config.toml` on a worker once the debounce delay has passed.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    if settle(app) {
        host.redraw_after(SAVE_POLL);
        return;
    }
    if let Some(due) = app.config_save.due {
        let now = Instant::now();
        if now >= due {
            save_in_background(app);
            host.redraw_after(SAVE_POLL);
        } else {
            host.redraw_after(due - now);
        }
    }
}

/// Take the worker's answer if it is there. Returns whether a write is still running.
pub fn settle(app: &mut AppCore) -> bool {
    let Some(flight) = &app.config_save.in_flight else { return false };
    let outcome = match flight.rx.try_recv() {
        Ok(outcome) => outcome,
        Err(TryRecvError::Empty) => return true,
        Err(TryRecvError::Disconnected) => SaveOutcome::WriteFailed("the save worker stopped".into()),
    };
    if let Some(flight) = app.config_save.in_flight.take() {
        finish(app, flight.config, outcome);
    }
    false
}

/// Wait for a write running on the worker.
pub fn wait_for_save(app: &mut AppCore) {
    if let Some(flight) = app.config_save.in_flight.take() {
        let outcome = flight.rx.recv().unwrap_or_else(|_| SaveOutcome::WriteFailed("the save worker stopped".into()));
        finish(app, flight.config, outcome);
    }
}

/// Start writing a pending save now, on a worker (closing the settings window). One write
/// runs at a time; a change made meanwhile waits for it.
pub fn save_in_background(app: &mut AppCore) {
    if app.config_save.in_flight.is_some() || app.config_save.due.take().is_none() {
        return;
    }
    if refused_while_broken(app) {
        return;
    }
    let (path, snapshot) = (app.config_watch.path().to_path_buf(), app.config.clone());
    let (tx, rx) = bounded(1);
    let (worker_path, worker_config) = (path.clone(), snapshot.clone());
    let spawned = std::thread::Builder::new().name("soyuz-config-save".into()).spawn(move || {
        let _ = tx.send(write_config(&worker_path, &worker_config));
    });
    match spawned {
        Ok(_) => app.config_save.in_flight = Some(SaveInFlight { rx, config: snapshot }),
        Err(e) => {
            log::warn!("cannot spawn the config save thread: {e}");
            let outcome = write_config(&path, &snapshot);
            finish(app, snapshot, outcome);
        }
    }
}

/// Write `config.toml` now if a save is pending, and return when it is on disk (app exit,
/// handing the terminal to an editor).
pub fn flush(app: &mut AppCore) {
    wait_for_save(app);
    if app.config_save.due.take().is_none() || refused_while_broken(app) {
        return;
    }
    let snapshot = app.config.clone();
    let outcome = write_config(app.config_watch.path(), &snapshot);
    finish(app, snapshot, outcome);
}

/// The active configuration did not come from the file: writing it would replace the
/// user's settings with defaults. Wait for a reload that works.
fn refused_while_broken(app: &mut AppCore) -> bool {
    if app.config_watch.load_error.is_some() {
        refuse(app, None);
        return true;
    }
    false
}

/// Merge `config` into the user's file, so comments and formatting survive (toml_edit), and
/// write it. A file that cannot be read stays as it is.
fn write_config(path: &Path, config: &Config) -> SaveOutcome {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return SaveOutcome::Refused(e.to_string()),
    };
    let text = match config::to_string_preserving(&existing, config) {
        Ok(text) => text,
        Err(e) => return SaveOutcome::Refused(e.to_string()),
    };
    if let Some(dir) = path.parent() {
        let _ = soyuz_fs::create_dir_all(dir);
    }
    match soyuz_fs::write_atomic(path, text.as_bytes()) {
        Ok(()) => SaveOutcome::Written(text),
        Err(e) => SaveOutcome::WriteFailed(e.to_string()),
    }
}

fn finish(app: &mut AppCore, written: Config, outcome: SaveOutcome) {
    match outcome {
        SaveOutcome::Written(text) => {
            app.config_watch.last_text = Some(text);
            app.config_save.flashed = Some(Instant::now());
            app.config_save.baseline = Some(written);
            app.config_save.held = false;
        }
        SaveOutcome::Refused(why) => refuse(app, Some(why)),
        SaveOutcome::WriteFailed(e) => {
            app.config_save.held = true;
            app.ui.toasts.push(Toast::error(tr!("settings-save-failed")).body(e));
        }
    }
}

/// `config.toml` was left alone because it does not load: say so, keep the change in memory.
fn refuse(app: &mut AppCore, why: Option<String>) {
    app.config_save.held = true;
    let body = match why {
        Some(why) => format!("{}\n{why}", tr!("settings-save-refused-body")),
        None => tr!("settings-save-refused-body").to_string(),
    };
    app.ui.toasts.push(
        Toast::error(tr!("settings-save-refused"))
            .body(body)
            .action(tr!("services-config-edit"), config::Action::EditConfigFile),
    );
}
