//! `AppCore`: the frontend-agnostic half of the application (Phase T0). It owns the
//! configuration, the workspace (tabs → panes), the background services, view memory,
//! the jobs engine, models of every panel and prompt, and the command queue. It never
//! names a toolkit type: side effects go through [`Host`], and requests only a frontend
//! can carry out (open its settings window, show a GUI dialog, …) are queued on
//! [`AppCore::frontend`].
//!
//! A frontend (`app::SoyuzApp` for egui, `soyuz_tui` for the terminal) owns an `AppCore` and runs
//! the phases below once per frame, interleaving its own pumps where noted:
//!
//! 1. [`AppCore::begin_frame`]: per-frame resets of the services.
//! 2. *(frontend: font / theme bookkeeping)*
//! 3. [`AppCore::pump_services`]: mailbox, crash reports, devices, properties, watcher,
//!    palette (config hot reload), terminal, search.
//! 4. *(frontend: tool windows, portal)*
//! 5. [`AppCore::pump_jobs`]: job prompts, finished jobs, pending renames.
//! 6. *(frontend: external drops)*
//! 7. [`AppCore::pump_background`]: MCP bridge, assistant, automation, updates,
//!    FileManager1, remotes, OS clipboard.
//! 8. [`AppCore::apply_cmds`] (the frontend drains [`AppCore::frontend`] after each one).
//! 9. Input: the frontend turns its events into an [`InputFrame`] and calls
//!    [`AppCore::input`] (plus `actions::execute` for keys its own widgets route).
//! 10. `apply_cmds` again, then the frontend draws (widgets push [`Cmd`]s and call
//!     core functions directly), then `ops::fileops::end_frame`.
//! 11. `apply_cmds` again, [`AppCore::update_title`].
//! 12. [`AppCore::end_frame`]: periodic session save.
//!
//! Outside the frame: [`AppCore::handle_close_request`] (every frame, also while the
//! window is minimized), [`AppCore::save_all`] (periodic and on exit), and
//! [`AppCore::on_exit`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_core::config::{self, Config, KeyContext, Keymap};
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::host::{Host, Wake};
use crate::keys::input::InputFrame;
use crate::models::panels::BottomPanel;
use crate::models::sidebar::SidebarState;
use crate::models::toasts::Toasts;
use crate::ops;
use crate::services::Services;
use crate::services::preview_gfx::Uploader;
use crate::state::pane::Pane;
use crate::state::session::{FavoriteEntry, Session};
use crate::state::view_memory::{ViewMemory, ViewState};
use crate::state::workspace::{Tab, Workspace};

/// Where keyboard input goes when no text field has focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Sidebar,
}

/// Address bar (breadcrumb ↔ path editor) state.
#[derive(Clone, Debug, Default)]
pub struct AddressState {
    pub editing: bool,
    pub text: String,
    pub request_focus: bool,
    pub completion: Option<usize>,
}

/// Window-level view state shared by every frontend (what is shown, what has focus).
pub struct UiState {
    pub sidebar: SidebarState,
    pub toasts: Toasts,
    pub show_hidden: bool,
    pub show_system: bool,
    pub checkbox_mode: bool,
    pub sidebar_visible: bool,
    pub status_visible: bool,
    pub focus: Focus,
    pub address: AddressState,
    /// Focus the filter box on the next frame.
    pub filter_focus: bool,
    pub volumes: Vec<VolumeInfo>,
    pub cloud: Vec<(String, PathBuf)>,
    pub bottom: BottomPanel,
    pub theme: crate::host::ThemePref,
    pub reduced_motion: bool,
    pub title: String,
    pub terminal: crate::models::terminal::TerminalPanel,
    /// The preview / info pane (open, width, mode); the frontend keeps what it renders.
    pub preview: crate::models::preview::PreviewModel,
    pub quicklook: crate::models::quicklook::QuickLook,
    pub search: crate::models::search::SearchState,
    /// Show the welcome / onboarding card (first run, or `Action::Welcome`).
    pub welcome: bool,
    /// The welcome card was dismissed at least once (persisted in the session).
    pub welcome_seen: bool,
    /// Folder tree sidebar.
    pub tree: crate::models::tree::TreeState,
    /// The frontend's settings window is open (kept up to date by the frontend).
    pub settings_open: bool,
}

/// Which frontend drives the core. The terminal UI keeps its own session
/// (`session-tui.toml`), never owns `org.freedesktop.FileManager1`, and actions that
/// would start another GUI window or an elevated copy of Soyuz say "not available in
/// the terminal UI" instead (see [`AppCore::tui_unavailable`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrontendKind {
    /// The egui window (`soyuz`).
    #[default]
    Gui,
    /// The terminal UI (`soyuz --tui`).
    Tui,
}

/// The frontend of this process (set by [`AppCore::new`]); for services without an
/// `AppCore` at hand (`services::crash::restart_in_safe_mode`).
static PROCESS_FRONTEND: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The frontend kind of the last [`AppCore`] built in this process.
pub fn process_frontend() -> FrontendKind {
    match PROCESS_FRONTEND.load(std::sync::atomic::Ordering::Relaxed) {
        1 => FrontendKind::Tui,
        _ => FrontendKind::Gui,
    }
}

/// Session file of a frontend: the GUI and the terminal UI keep separate tabs.
pub fn session_file_name(kind: FrontendKind) -> &'static str {
    match kind {
        FrontendKind::Gui => "session.toml",
        FrontendKind::Tui => "session-tui.toml",
    }
}

/// Command-line startup options (see `main.rs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Startup {
    /// Folder to open (a file opens its parent with the file under the cursor).
    pub open: Option<PathBuf>,
    /// Secondary window: no session restore / save.
    pub secondary: bool,
    /// `--safe-mode`: safe defaults (see `services::crash::safe_config`), no session
    /// restore or save, settings saved to `safe-mode.toml` instead of `config.toml`.
    pub safe_mode: bool,
    /// The frontend that runs this core.
    pub frontend: FrontendKind,
}

/// Window-close state: closing with pending jobs asks first (they would be cancelled).
#[derive(Default)]
pub struct QuitState {
    /// The user confirmed: let the next close request through.
    pub allowed: bool,
    /// Close as soon as no job is pending ("Quit when finished").
    pub when_idle: bool,
    /// The confirmation dialog is on screen (reset when the dialog is dropped).
    pub prompting: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The frontend-agnostic application state (see the module docs for the frame phases).
pub struct AppCore {
    pub config: Config,
    pub keymap: Keymap,
    pub ws: Workspace,
    pub services: Services,
    pub views: ViewMemory,
    pub ui: UiState,
    pub cmds: Vec<Cmd>,
    /// Background file operations: jobs engine, clipboard, drag and drop, prompts.
    pub fileops: crate::ops::fileops::FileOps,
    /// The frontend's renderer (`wgpu` / `glow`), passed on to windows it spawns.
    pub renderer: &'static str,
    /// Default view for folders without memory (from the config).
    pub default_view: ViewState,
    session_path: PathBuf,
    views_path: PathBuf,
    last_session: Option<String>,
    last_session_check: Instant,
    /// Command palette / go-to-folder state and persistent palette memory.
    pub palette: crate::palette::PaletteState,
    /// `config.toml` hot reload.
    pub config_watch: crate::services::config_watch::ConfigWatch,
    /// Pending multi-key sequence / mark.
    pub keyseq: crate::keys::sequence::Sequencer,
    /// A secondary window (`--new-window`): never restores or overwrites the session.
    pub secondary: bool,
    /// Close-request handling while jobs are pending.
    pub quit: QuitState,
    /// Built-in text / hex / image editors (documents; the frontend draws them).
    pub editors: crate::editors::Editors,
    /// The in-app assistant (AI panel, agent tasks, highlights).
    pub ai: crate::ai::AiState,
    /// Saved workspaces, the other windows of this process, group editing.
    pub workspaces: crate::models::workspaces::WorkspacesState,
    /// Scripts, automation rules and the CLI pipe.
    pub automation: crate::automation::AutomationState,
    /// Asks the frontend for another frame (background threads keep a clone).
    pub wake: Wake,
    /// Debounced writes of `config.toml`.
    pub config_save: crate::models::settings::ConfigSave,
    /// Requests for the frontend (open its settings window, …), drained every frame.
    pub frontend: Vec<crate::models::frontend::FrontendCmd>,
    /// Prompts raised by core logic (job prompts, delete confirmations, approvals, …),
    /// drawn by the frontend (`models::dialogs`).
    pub dialogs: crate::models::dialogs::DialogStack,
    /// The frontend that drives this core (from [`Startup::frontend`]).
    pub frontend_kind: FrontendKind,
    /// The frontend's image uploader (the one passed to [`AppCore::new`]), for core code
    /// that decodes images outside the services that got it at construction (previews,
    /// editors): the GUI's turns pixels into egui textures, the terminal UI's keeps none
    /// (`services::preview_gfx::no_upload`) until it draws images itself.
    pub uploader: Uploader,
}

impl AppCore {
    /// Build the core from a loaded configuration. Needs no window: `wake` asks the
    /// frontend for a frame, `uploader` turns decoded images into its textures (use
    /// `host::no_wake` / `services::preview_gfx::no_upload` without a frontend).
    pub fn new(config: Config, mut startup: Startup, wake: Wake, uploader: Uploader) -> AppCore {
        startup.secondary |= startup.safe_mode;
        PROCESS_FRONTEND.store(
            match startup.frontend {
                FrontendKind::Gui => 0,
                FrontendKind::Tui => 1,
            },
            std::sync::atomic::Ordering::Relaxed,
        );
        apply_core_settings(&config);
        let (keymap, warnings) = crate::keys::build_keymap(&config);
        for w in warnings {
            log::warn!("{w}");
        }
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let thumbs =
            crate::services::thumbs::Thumbs::new(wake.clone(), uploader.clone(), cores.saturating_sub(1).clamp(1, 4));
        let mut services =
            Services::new(wake.clone(), thumbs, Duration::from_millis(config.behaviour.watch_debounce_ms.max(50)));
        services.watcher.set_poll_interval(Duration::from_millis(config.behaviour.watch_poll_interval_ms));
        let dir = config::config_dir();
        let session_path = dir.join(session_file_name(startup.frontend));
        let views_path = dir.join("views.toml");
        let views =
            if config.behaviour.remember_folder_views { ViewMemory::load(&views_path) } else { ViewMemory::default() };
        // Small persistent UI state (welcome card, shelf) is read even when tab
        // restoration is off.
        let stored = Session::load(&session_path);
        let welcome_seen = stored.as_ref().is_some_and(|s| s.welcome_seen);
        let shelf = stored.as_ref().map(|s| s.shelf.clone()).unwrap_or_default();
        let session = if config.behaviour.restore_session && !startup.secondary { stored } else { None };
        let default_view = ViewState::from_config(&config);

        let mut sidebar = SidebarState::new();
        if let Some(favs) = session.as_ref().and_then(|s| s.favorites.clone()) {
            sidebar.favorites = favs;
            sidebar.favorites_customized = true;
        }
        if let Some(s) = &session {
            sidebar.collapsed = s.collapsed_sections.iter().cloned().collect();
        }
        let theme_pref = session
            .as_ref()
            .and_then(|s| s.theme.as_deref())
            .map(crate::host::ThemePref::from_config)
            .unwrap_or_else(|| crate::host::ThemePref::from_config(&config.appearance.theme));

        let mut ui = UiState {
            sidebar,
            toasts: Toasts::default(),
            show_hidden: session.as_ref().and_then(|s| s.show_hidden).unwrap_or(config.general.show_hidden),
            show_system: config.general.show_system,
            checkbox_mode: session.as_ref().is_some_and(|s| s.checkbox_mode),
            sidebar_visible: session.as_ref().and_then(|s| s.sidebar_visible).unwrap_or(config.layout.sidebar),
            status_visible: config.layout.status_bar,
            focus: Focus::List,
            address: AddressState::default(),
            filter_focus: false,
            volumes: Vec::new(),
            cloud: Vec::new(),
            bottom: BottomPanel {
                height: config.layout.terminal_height.clamp(120.0, 720.0),
                activity_placement: config.layout.activity_panel,
                ..Default::default()
            },
            theme: theme_pref,
            reduced_motion: config.appearance.reduced_motion,
            title: String::new(),
            terminal: Default::default(),
            preview: crate::models::preview::PreviewModel::new(config.layout.preview_pane),
            quicklook: Default::default(),
            search: crate::models::search::SearchState::new(&config),
            welcome: !welcome_seen,
            welcome_seen,
            tree: Default::default(),
            settings_open: false,
        };
        if let Some(s) = &session {
            ui.tree.restore_expanded(s.tree_expanded.iter().cloned());
        }
        crate::models::tools::restore_shelf(shelf);

        let opened = startup.open.as_deref().and_then(startup_target);
        let start = match &opened {
            Some((loc, _)) if session.is_none() => loc.clone(),
            _ => start_location(&config),
        };
        let fileops = crate::ops::fileops::FileOps::new(wake.clone(), &config);
        let mut app = AppCore {
            ws: Workspace::new(Tab::new(Pane::new(start.clone(), default_view.clone()))),
            fileops,
            config,
            keymap,
            services,
            views,
            ui,
            cmds: Vec::new(),
            renderer: "",
            default_view,
            session_path,
            views_path,
            last_session: None,
            last_session_check: Instant::now(),
            palette: crate::palette::PaletteState::new(dir.join("palette.toml")),
            config_watch: crate::services::config_watch::ConfigWatch::start(
                wake.clone(),
                if startup.safe_mode { crate::services::crash::safe_config_path() } else { config::default_path() },
            ),
            keyseq: Default::default(),
            secondary: startup.secondary,
            quit: QuitState::default(),
            editors: crate::editors::Editors::new(uploader.clone()),
            ai: crate::ai::AiState::new(wake.clone()),
            wake,
            config_save: Default::default(),
            frontend: Vec::new(),
            dialogs: Default::default(),
            workspaces: crate::models::workspaces::WorkspacesState::load(),
            automation: Default::default(),
            frontend_kind: startup.frontend,
            uploader,
        };
        crate::models::workspaces::bind_hotkeys(&mut app);
        let restored = session.as_ref().is_some_and(|s| app.restore_tabs(s));
        if let Some(s) = &session
            && app.config.workspace.restore_windows
        {
            app.workspaces.restore = s.windows.clone();
        }
        if let Some(s) = &session {
            crate::editors::restore(&mut app, &s.editors);
        }
        if restored {
            if let Some((loc, select)) = opened {
                ops::open_in_new_tab(&mut app, loc, true);
                app.ws.pane_mut().pending_cursor = select;
            }
        } else {
            let view = app.view_for(&start);
            app.ws.tabs[0].panes[0].view = view;
            if let Some((_, select)) = opened {
                app.ws.tabs[0].panes[0].pending_cursor = select;
            }
            if app.config.layout.dual_pane {
                let p2 = Pane::new(start.clone(), app.view_for(&start));
                let t = &mut app.ws.tabs[0];
                t.panes.push(p2);
                t.dual = true;
                t.orientation = app.config.layout.dual_pane_orientation;
            }
        }
        app.last_session = Some(app.session().to_toml());
        app
    }

    /// The remembered (or default) view for a location.
    pub fn view_for(&self, loc: &Location) -> ViewState {
        if self.config.behaviour.remember_folder_views {
            self.views.lookup(loc, &self.default_view)
        } else {
            self.default_view.clone()
        }
    }

    fn restore_tabs(&mut self, s: &Session) -> bool {
        let win = crate::state::session::WindowSession {
            active_tab: s.active_tab,
            tabs: s.tabs.clone(),
            groups: s.groups.clone(),
            ..Default::default()
        };
        // Tabs (up to four panes each, pinned first), groups and layouts.
        match crate::models::workspaces::build_workspace(self, &win, false) {
            Some(ws) => {
                self.ws = ws;
                true
            }
            None => false,
        }
    }

    /// Snapshot of the current session.
    pub fn session(&self) -> Session {
        let main = crate::models::workspaces::window_session(&self.ws, false);
        Session {
            active_tab: main.active_tab,
            tabs: main.tabs,
            groups: main.groups,
            windows: crate::models::windows::session_windows(self),
            favorites: self.ui.sidebar.favorites_customized.then(|| self.ui.sidebar.favorites.clone()),
            show_hidden: Some(self.ui.show_hidden),
            theme: Some(theme_name(self.ui.theme).to_string()),
            collapsed_sections: self.ui.sidebar.collapsed.iter().cloned().collect(),
            sidebar_visible: Some(self.ui.sidebar_visible),
            checkbox_mode: self.ui.checkbox_mode,
            welcome_seen: self.ui.welcome_seen,
            shelf: crate::models::tools::shelf_paths(),
            tree_expanded: self.ui.tree.session_expanded(),
            editors: crate::editors::session_paths(self),
            ..Session::default()
        }
    }

    /// Persist the session and view memory if they changed.
    pub fn save_state(&mut self) {
        if !self.secondary {
            let text = self.session().to_toml();
            if self.last_session.as_deref() != Some(text.as_str()) {
                // Written by the store worker (in order, atomically; flushed on exit).
                crate::services::store_file::save_async(self.session_path.clone(), text.clone().into_bytes());
                self.last_session = Some(text);
            }
        }
        if self.views.dirty && self.config.behaviour.remember_folder_views {
            crate::services::store_file::save_async(self.views_path.clone(), self.views.to_toml().into_bytes());
            self.views.dirty = false;
        }
    }

    /// Phase 1: per-frame resets of the services.
    pub fn begin_frame(&mut self) {
        self.services.thumbs.begin_frame();
    }

    /// Phase 3: results of background work (lists, watcher, devices, …).
    pub fn pump_services(&mut self, host: &dyn Host) {
        ops::drain_mailbox(self, host);
        crate::services::crash::pump(self, host);
        crate::models::devices::pump(self);
        crate::models::properties::pump(self, host);
        ops::drain_watcher(self);
        crate::palette::begin_frame(self, host);
        crate::models::terminal::pump(self, host);
        crate::models::search::pump(self, host);
    }

    /// Phase 5: the jobs engine (prompts, finished jobs, pending renames).
    pub fn pump_jobs(&mut self, host: &dyn Host) {
        ops::fileops::begin_frame(self, host);
    }

    /// Phase 7: integrations (MCP, assistant, automation, updates, D-Bus, remotes, OS
    /// clipboard).
    pub fn pump_background(&mut self, host: &dyn Host) {
        crate::services::mcp_bridge::pump(self, host);
        crate::ai::pump(self, host);
        crate::automation::pump(self, host);
        crate::services::updates::pump(self);
        crate::services::filemanager1::pump(self, host);
        crate::models::remote::pump(self, host);
        crate::services::os_clipboard::pump(self);
    }

    /// Phase 9: the keyboard / mouse-button input the frontend did not consume, for the
    /// key context it determined.
    pub fn input(&mut self, host: &dyn Host, frame: &mut InputFrame, kctx: KeyContext, text_focused: bool) {
        crate::keys::input::handle(self, host, frame, kctx, text_focused);
    }

    /// Apply queued commands (commands may queue more; bounded to avoid loops). Requests
    /// for the frontend stay queued on [`Self::frontend`]; the GUI uses its own loop that
    /// drains them after every command (`SoyuzApp::apply_cmds`).
    pub fn apply_cmds(&mut self, host: &dyn Host) {
        for _ in 0..CMD_ROUNDS {
            if self.cmds.is_empty() {
                break;
            }
            for c in std::mem::take(&mut self.cmds) {
                ops::apply(self, c, host);
            }
        }
    }

    /// Before drawing the panes (phase 10): start the visible panes' listings lazily and
    /// refresh their visible order (filter, hidden / system files, sort).
    pub fn prepare_visible_panes(&mut self) {
        let show_hidden = self.ui.show_hidden;
        let show_system = self.ui.show_system;
        let tab = self.ws.tab_mut();
        let shown = tab.shown();
        let focused = tab.focused;
        for (i, pane) in tab.panes.iter_mut().enumerate() {
            let visible = if shown >= 2 { i < shown } else { i == focused };
            if visible {
                ops::ensure_started(pane, &mut self.services);
            }
            pane.refresh_visible(show_hidden, show_system);
        }
    }

    /// Phase 11: the window title follows the focused pane.
    pub fn update_title(&mut self, host: &dyn Host) {
        let pane = self.ws.pane();
        let t = format!("{} — Soyuz", crate::state::crumbs::location_title(&pane.location));
        if t != self.ui.title {
            host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Title(t.clone()));
            self.ui.title = t;
        }
    }

    /// Phase 12: cheap periodic check: persist when the session text changed (every
    /// ~30 s, plus [`Self::save_all`] on exit).
    pub fn end_frame(&mut self) {
        if self.last_session_check.elapsed() > Duration::from_secs(30) {
            self.last_session_check = Instant::now();
            self.save_state();
        }
    }

    /// Window close requests (`close_requested`: the frontend's window got one this
    /// frame): with pending jobs, cancel the close and ask first (quitting cancels the
    /// jobs). Runs every frame, also while the window is minimized.
    pub fn handle_close_request(&mut self, host: &dyn Host, close_requested: bool) {
        use std::sync::atomic::Ordering;
        // Other windows open: closing the main window closes just this window.
        if crate::models::windows::intercept_main_close(self, host, close_requested) {
            self.close_cancelled();
            return;
        }
        if crate::editors::intercept_close(self, host, close_requested) {
            self.close_cancelled();
            return;
        }
        let pending = self.fileops.jobs.pending().count();
        if self.quit.when_idle && pending == 0 {
            self.quit.when_idle = false;
            self.quit.allowed = true;
            host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Close);
            return;
        }
        if self.quit.when_idle {
            host.redraw_after(Duration::from_millis(250));
        }
        if !close_requested || self.quit.allowed || pending == 0 {
            return;
        }
        host.window(crate::host::WindowId::Current, crate::host::WindowCmd::CancelClose);
        self.close_cancelled();
        if !self.quit.prompting.swap(true, Ordering::SeqCst) {
            host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Minimized(false));
            host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Focus);
            self.dialogs.push(crate::models::dialogs::QuitModel::new(pending, self.quit.prompting.clone()));
        }
    }

    /// A window close was cancelled (to ask first): an "Install & restart" that started
    /// it no longer restarts; the update stays installed for the next start.
    fn close_cancelled(&mut self) {
        if crate::services::updates::cancel_restart() {
            self.workspaces.quitting = false;
            self.ui.toasts.push(crate::models::toasts::Toast::info(tr!("app-restart-cancelled")));
        }
    }

    /// The user answered the close confirmation.
    pub fn confirm_quit(&mut self, host: &dyn Host, when_idle: bool) {
        if when_idle {
            self.quit.when_idle = true;
            self.ui.toasts.push(crate::models::toasts::Toast::info(tr!("app-quit-when-finished")));
            host.redraw();
        } else {
            self.quit.allowed = true;
            host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Close);
        }
    }

    /// Write everything that is saved lazily: session, view memory, pending
    /// `config.toml` save, palette memory, tags / recent / folder colours, and wait for the
    /// store worker (periodically and on exit).
    pub fn save_all(&mut self) {
        self.save_state();
        crate::models::settings::flush(self);
        self.palette.save_now();
        // Debounced tag / recent changes are queued now and written before we return
        // (frontends call this on exit too), in order, on the single store worker.
        crate::services::tags::flush();
        crate::services::recent::flush();
        crate::services::folder_colors::flush();
        if !crate::services::store_file::flush(Duration::from_secs(3)) {
            log::warn!("some settings stores were not written in time");
        }
    }

    /// The process is exiting (the close went through).
    pub fn on_exit(&mut self) {
        // An "Install & restart" now restarts.
        crate::services::updates::arm_restart();
    }

    /// For actions the terminal UI cannot carry out (another GUI window, an elevated
    /// copy of Soyuz, …): in the terminal UI, says so with a toast and returns `true`;
    /// `false` in the GUI.
    pub fn tui_unavailable(&mut self) -> bool {
        if self.frontend_kind != FrontendKind::Tui {
            return false;
        }
        self.ui.toasts.push(crate::models::toasts::Toast::info(tr!("tui-not-available")));
        true
    }

    /// A core prompt ([`Self::dialogs`]) is open. Frontends add their own dialogs.
    pub fn prompt_open(&self) -> bool {
        self.dialogs.is_open()
    }
}

/// How many rounds of commands one `apply_cmds` runs (commands may queue more).
pub const CMD_ROUNDS: usize = 8;

pub fn theme_name(p: crate::host::ThemePref) -> &'static str {
    p.name()
}

/// Load `config.toml` (defaults when missing or invalid; problems are logged).
pub fn load_config() -> Config {
    let path = config::default_path();
    match config::migrate_file(&path) {
        Ok(config::MigrateOutcome::Migrated { from, to, backup }) => {
            log::info!("{}: migrated config schema {from} → {to} (backup: {})", path.display(), backup.display());
        }
        Ok(config::MigrateOutcome::UpToDate) => {}
        Err(e) => log::warn!("{}: config migration skipped: {e}", path.display()),
    }
    match config::load_or_default(&path) {
        Ok(loaded) => {
            for w in &loaded.warnings {
                log::warn!("{}: {w}", path.display());
            }
            loaded.config
        }
        Err(e) => {
            log::error!("{}: {e}; using defaults", path.display());
            Config::default()
        }
    }
}

/// Process-wide settings read by core services (startup and every config change; the
/// GUI's `app::apply_live_settings` adds its own).
pub fn apply_core_settings(c: &Config) {
    crate::i18n::apply_setting(&c.general.language);
    crate::services::thumbs::set_pdf(c.appearance.pdf_thumbnails, &c.preview.pdfium_dir);
    crate::services::thumbs::set_os_cache(c.appearance.os_thumbnail_cache);
    crate::services::thumbs::set_write_os_cache(c.appearance.write_os_thumbnails);
    crate::services::lister::set_dot_hidden(c.general.honour_dot_hidden);
    crate::ai::state::set_rules(&c.ai);
    crate::services::tags::set_custom_names(&c.tags);
    crate::services::tags::set_sync_xdg_tags(c.tags.sync_xdg_tags);
    crate::models::devices::set_show_gtk_bookmarks(c.sidebar.gtk_bookmarks);
    if let Some(w) = crate::services::preview_text::set_syntax_theme(&c.preview.syntax_theme) {
        log::warn!("{w}");
    }
    crate::services::thumbs::set_budget_bytes(c.preview.thumb_cache_mib.saturating_mul(1024 * 1024));
    crate::services::preview_cache::set_budget_bytes(c.preview.preview_cache_mib.saturating_mul(1024 * 1024));
    crate::services::timeline::set_options(crate::services::timeline::TimelineOpts::from_config(c));
}

/// The location (and entry to put the cursor on) for a command-line path: a folder
/// opens itself, a file opens its parent with the file under the cursor.
pub fn startup_target(path: &std::path::Path) -> Option<(Location, Option<String>)> {
    // Desktop files pass `%U`: besides `file://`, launchers hand over `sftp://`, `smb://` …
    if crate::services::remote::is_remote_path(path) {
        return match crate::services::remote::normalize(Location::Remote(path.to_string_lossy().into_owned())) {
            Ok(loc) => Some((loc, None)),
            Err(e) => {
                log::warn!("{}: {e}; opening the start folder instead", path.display());
                None
            }
        };
    }
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().ok()?.join(path) };
    match std::fs::metadata(&abs) {
        Ok(m) if m.is_dir() => Some((Location::Dir(abs), None)),
        Ok(_) => {
            let parent = abs.parent()?.to_path_buf();
            let name = abs.file_name()?.to_string_lossy().into_owned();
            Some((Location::Dir(parent), Some(name)))
        }
        Err(e) => {
            log::warn!("{}: {e}; opening the start folder instead", abs.display());
            None
        }
    }
}

/// `[general] start_location`: `home` | `last` (session handles it) | a path.
fn start_location(c: &Config) -> Location {
    let s = c.general.start_location.trim();
    match s.to_ascii_lowercase().as_str() {
        "" | "home" | "last" => Location::Dir(soyuz_fs::home()),
        _ => Location::parse(s).unwrap_or_else(|| Location::Dir(soyuz_fs::home())),
    }
}

/// Default favorites: Home plus the standard user folders that exist.
pub fn default_favorites() -> Vec<FavoriteEntry> {
    crate::models::sidebar::standard_places()
        .into_iter()
        .map(|p| FavoriteEntry { name: p.label, path: p.path })
        .collect()
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn remote_uris_open_as_remote_locations() {
        let (loc, select) = startup_target(std::path::Path::new("sftp://host/srv/data")).expect("remote target");
        assert!(matches!(&loc, Location::Remote(s) if s.starts_with("sftp://host")), "{loc:?}");
        assert!(select.is_none());
    }
}
