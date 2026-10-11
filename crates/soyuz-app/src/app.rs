//! `SoyuzApp`: the eframe application. Owns the configuration, the workspace (tabs →
//! panes), the background services, view memory and transient UI state. One frame:
//! drain the mailbox + watcher → dispatch keys → draw the shell (widgets push [`Cmd`]s)
//! → apply commands.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::CreationContext;
use egui::{TextEdit, ThemePreference};
use soyuz_core::Location;
use soyuz_core::config::{self, Config, KeyContext, Keymap};
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::keys;
use crate::services::Services;
use crate::state::pane::Pane;
use crate::state::session::{FavoriteEntry, Session};
use crate::state::view_memory::{ViewMemory, ViewState};
use crate::state::workspace::{Tab, Workspace};
use crate::theme;
use crate::ui::dialogs::Dialogs;
use crate::ui::panels::BottomPanel;
use crate::ui::shell;
use crate::ui::sidebar::SidebarState;
use crate::ui::toasts::Toasts;
use crate::ui::views::layout::NavDir;
use crate::{actions, ops};
use soyuz_core::ViewMode;

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

/// Transient, window-level UI state.
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
    pub dialogs: Dialogs,
    pub bottom: BottomPanel,
    pub theme: ThemePreference,
    pub reduced_motion: bool,
    pub title: String,
    pub terminal: crate::ui::panels::terminal::TerminalPanel,
    pub preview: crate::ui::preview::PreviewPane,
    pub quicklook: crate::ui::quicklook::QuickLook,
    pub search: crate::ui::search::SearchUi,
    /// Show the welcome / onboarding card (first run, or `Action::Welcome`).
    pub welcome: bool,
    /// The welcome card was dismissed at least once (persisted in the session).
    pub welcome_seen: bool,
    /// Window frame currently in effect (`[appearance] custom_titlebar`).
    pub chrome: crate::ui::titlebar::Chrome,
    /// Folder tree sidebar.
    pub tree: crate::ui::tree::TreeState,
}

pub struct SoyuzApp {
    pub config: Config,
    pub keymap: Keymap,
    pub ws: Workspace,
    pub services: Services,
    pub views: ViewMemory,
    pub ui: UiState,
    pub cmds: Vec<Cmd>,
    /// Background file operations: jobs engine, clipboard, drag and drop, prompts.
    pub fileops: crate::ops::fileops::FileOps,
    pub renderer: &'static str,
    /// Default view for folders without memory (from the config).
    pub default_view: ViewState,
    session_path: PathBuf,
    views_path: PathBuf,
    last_session: Option<String>,
    last_session_check: Instant,
    /// Command palette / go-to-folder state and persistent palette memory.
    pub palette: crate::palette::PaletteState,
    /// Settings window.
    pub settings: crate::ui::settings::SettingsState,
    /// `config.toml` hot reload.
    pub config_watch: crate::services::config_watch::ConfigWatch,
    /// Pending multi-key sequence / mark.
    pub keyseq: crate::keys::sequence::Sequencer,
    /// `SOYUZ_SCRIPT` automation / `SOYUZ_FRAME_LOG` (debug hooks).
    pub devscript: crate::devscript::DevScript,
    /// A secondary window (`--new-window`): never restores or overwrites the session.
    pub secondary: bool,
    /// Close-request handling while jobs are pending.
    pub quit: QuitState,
    /// Built-in text / hex / image editors.
    pub editors: crate::editors::Editors,
    /// The in-app assistant (AI panel, agent tasks, highlights).
    pub ai: crate::ai::AiState,
    /// Saved workspaces, the other windows of this process, group editing.
    pub workspaces: crate::ui::workspaces::WorkspacesState,
    /// Scripts, automation rules and the CLI pipe.
    pub automation: crate::automation::AutomationState,
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

impl SoyuzApp {
    /// Construct from the configuration loaded by [`load_config`] (the window
    /// attributes in `main.rs` are derived from the same config).
    pub fn new(cc: &CreationContext<'_>, config: Config, renderer: &'static str, startup: Startup) -> Self {
        let transparent = crate::ui::native_chrome::backdrop_active();
        crate::ui::native_chrome::init(cc, &config, transparent);
        crate::ops::fileops::linux_dnd::init(cc);
        soyuz_fs::register_ui_thread();
        Self::with_startup(&cc.egui_ctx, config, renderer, startup)
    }

    /// Construct with an explicit config (also used by tests).
    pub fn with_config(ctx: &egui::Context, config: Config, renderer: &'static str) -> Self {
        Self::with_startup(ctx, config, renderer, Startup::default())
    }

    /// Construct with an explicit config and command-line startup options.
    pub fn with_startup(ctx: &egui::Context, config: Config, renderer: &'static str, mut startup: Startup) -> Self {
        startup.secondary |= startup.safe_mode;
        egui_extras::install_image_loaders(ctx);
        // Ctrl +/-/0 are our zoom actions, not egui's global UI zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        apply_live_settings(ctx, &config);
        for w in theme::install(ctx, &config.appearance, &config.fonts, &config.theme.colors) {
            log::warn!("{w}");
        }
        let (keymap, warnings) = config::keymap(&config);
        for w in warnings {
            log::warn!("{w}");
        }
        let mut services = Services::new(ctx, Duration::from_millis(config.behaviour.watch_debounce_ms.max(50)));
        services.watcher.set_poll_interval(Duration::from_millis(config.behaviour.watch_poll_interval_ms));
        let dir = config::config_dir();
        let session_path = dir.join("session.toml");
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
            .map(theme::preference_from_config)
            .unwrap_or_else(|| theme::preference_from_config(&config.appearance.theme));
        ctx.set_theme(theme_pref);

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
            dialogs: Dialogs::default(),
            bottom: BottomPanel {
                height: config.layout.terminal_height.clamp(120.0, 720.0),
                activity_placement: config.layout.activity_panel,
                ..Default::default()
            },
            theme: theme_pref,
            reduced_motion: config.appearance.reduced_motion,
            title: String::new(),
            terminal: Default::default(),
            preview: crate::ui::preview::PreviewPane::new(config.layout.preview_pane),
            quicklook: Default::default(),
            search: crate::ui::search::SearchUi::new(&config),
            welcome: !welcome_seen,
            welcome_seen,
            chrome: crate::ui::titlebar::Chrome::for_config(config.appearance.custom_titlebar),
            tree: Default::default(),
        };
        if let Some(s) = &session {
            ui.tree.restore_expanded(s.tree_expanded.iter().cloned());
        }
        crate::ui::tools::restore_shelf(shelf);

        let opened = startup.open.as_deref().and_then(startup_target);
        let start = match &opened {
            Some((loc, _)) if session.is_none() => loc.clone(),
            _ => start_location(&config),
        };
        let fileops = crate::ops::fileops::FileOps::new(ctx, &config);
        let mut app = SoyuzApp {
            ws: Workspace::new(Tab::new(Pane::new(start.clone(), default_view.clone()))),
            fileops,
            config,
            keymap,
            services,
            views,
            ui,
            cmds: Vec::new(),
            renderer,
            default_view,
            session_path,
            views_path,
            last_session: None,
            last_session_check: Instant::now(),
            palette: crate::palette::PaletteState::new(dir.join("palette.toml")),
            settings: Default::default(),
            config_watch: crate::services::config_watch::ConfigWatch::start(
                ctx,
                if startup.safe_mode { crate::services::crash::safe_config_path() } else { config::default_path() },
            ),
            keyseq: Default::default(),
            devscript: crate::devscript::DevScript::from_env(),
            secondary: startup.secondary,
            quit: QuitState::default(),
            editors: Default::default(),
            ai: crate::ai::AiState::new(ctx),
            workspaces: crate::ui::workspaces::WorkspacesState::load(),
            automation: Default::default(),
        };
        crate::ui::workspaces::bind_hotkeys(&mut app);
        let restored = session.as_ref().is_some_and(|s| app.restore_tabs(s));
        if let Some(s) = &session
            && app.config.workspace.restore_windows
        {
            app.workspaces.restore = s.windows.clone();
        }
        if let Some(s) = &session {
            crate::editors::restore(&mut app, ctx, &s.editors);
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
        match crate::ui::workspaces::build_workspace(self, &win, false) {
            Some(ws) => {
                self.ws = ws;
                true
            }
            None => false,
        }
    }

    /// Snapshot of the current session.
    pub fn session(&self) -> Session {
        let main = crate::ui::workspaces::window_session(&self.ws, false);
        Session {
            active_tab: main.active_tab,
            tabs: main.tabs,
            groups: main.groups,
            windows: crate::ui::workspaces::windows::session_windows(self),
            favorites: self.ui.sidebar.favorites_customized.then(|| self.ui.sidebar.favorites.clone()),
            show_hidden: Some(self.ui.show_hidden),
            theme: Some(theme_name(self.ui.theme).to_string()),
            collapsed_sections: self.ui.sidebar.collapsed.iter().cloned().collect(),
            sidebar_visible: Some(self.ui.sidebar_visible),
            checkbox_mode: self.ui.checkbox_mode,
            welcome_seen: self.ui.welcome_seen,
            shelf: crate::ui::tools::shelf_paths(),
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

    /// Keyboard context + whether a text field owns the keyboard.
    fn key_context(&self, ctx: &egui::Context) -> (KeyContext, bool) {
        if self.ui.dialogs.is_open() {
            return (KeyContext::Dialog, true);
        }
        let text_focused = ctx.memory(|m| m.focused()).is_some_and(|id| TextEdit::load_state(ctx, id).is_some());
        if text_focused {
            (KeyContext::Omnibar, true)
        } else if self.ui.focus == Focus::List {
            (KeyContext::FileList, false)
        } else if self.ui.focus == Focus::Sidebar {
            (KeyContext::Sidebar, false)
        } else {
            (KeyContext::Global, false)
        }
    }

    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        if self.ui.dialogs.is_open()
            || crate::palette::owns_keyboard(self)
            || crate::ui::widgets::dropdown::any_open(ctx)
            || crate::ui::tools::any_window_open()
        {
            return; // the modal (or an open dropdown / tool window) owns the keyboard
        }
        if crate::ui::quicklook::handle_keys(self, ctx) {
            return;
        }
        if crate::ui::panels::terminal::owns_keyboard(self) {
            let actions = crate::ui::panels::terminal::dispatch_keys(ctx, &self.keymap, self.config.keys.cmd_is_ctrl);
            for a in actions {
                actions::execute(self, a, ctx);
            }
            return;
        }
        if crate::editors::owns_keyboard(self, ctx) {
            crate::editors::handle_keys(self, ctx);
            return;
        }
        let (kctx, text_focused) = self.key_context(ctx);
        if kctx == KeyContext::FileList && !text_focused {
            // Ctrl+V with only files on the OS clipboard produces no egui event: the
            // fallback reads raw key state. Feed it this frame's egui Ctrl+V so it can
            // de-duplicate, and drop the egui event when the fallback already pasted.
            let cmd_is_ctrl = self.config.keys.cmd_is_ctrl;
            let is_v = |c: &soyuz_core::config::KeyChord| {
                c.key == soyuz_core::config::KeyName::V && c.mods.contains(soyuz_core::config::Mods::CTRL)
            };
            let mut synth: Vec<keys::Dispatched> = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|ev| keys::event_chord(ev, i.modifiers, cmd_is_ctrl))
                    .filter(|c| is_v(c))
                    .filter_map(|c| {
                        self.keymap.lookup(kctx, c).map(|a| keys::Dispatched { action: a.clone(), chord: c })
                    })
                    .collect()
            });
            let had_egui = !synth.is_empty();
            crate::services::os_clipboard::paste_key_fallback(ctx, &self.keymap, &mut synth);
            if had_egui && synth.is_empty() {
                ctx.input_mut(|i| {
                    let m = i.modifiers;
                    i.events.retain(|ev| !keys::event_chord(ev, m, cmd_is_ctrl).is_some_and(|c| is_v(&c)));
                });
            }
            if !had_egui {
                for d in synth {
                    actions::execute(self, d.action, ctx);
                }
            }
        }
        keys::sequence::dispatch_all(self, ctx, kctx, text_focused);
        // Mouse back/forward buttons.
        let (back, fwd) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1),
                i.pointer.button_pressed(egui::PointerButton::Extra2),
            )
        });
        if back {
            actions::execute(self, config::Action::Back, ctx);
        }
        if fwd {
            actions::execute(self, config::Action::Forward, ctx);
        }
        if kctx == KeyContext::FileList && !text_focused {
            // Unbound Tab switches panes in dual mode instead of moving egui focus.
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) && self.ws.tab().dual {
                actions::execute(self, config::Action::FocusOtherPane, ctx);
            }
            // Left/Right: horizontal movement in the icon/list views, Miller in/out.
            for (key, dir) in [(egui::Key::ArrowLeft, NavDir::Left), (egui::Key::ArrowRight, NavDir::Right)] {
                let (plain, shift) = ctx.input_mut(|i| {
                    (i.consume_key(egui::Modifiers::NONE, key), i.consume_key(egui::Modifiers::SHIFT, key))
                });
                if !plain && !shift {
                    continue;
                }
                match self.ws.pane().view.mode {
                    ViewMode::Grid | ViewMode::Gallery | ViewMode::List => actions::cursor(self, dir, shift, false),
                    ViewMode::Miller if dir == NavDir::Left => actions::execute(self, config::Action::Up, ctx),
                    ViewMode::Miller => {
                        let pane = self.ws.pane();
                        let target = pane.cursor.as_deref().and_then(|c| pane.entry(c)).filter(|e| e.is_dir_like());
                        if let Some(e) = target {
                            let key = e.name.clone();
                            let id = pane.id;
                            ops::open_entry(self, id, &key);
                        }
                    }
                    ViewMode::Details => {}
                }
            }
            let typed = keys::typed_text(ctx);
            if !typed.is_empty() {
                ops::type_ahead(self, &typed);
            }
        }
    }

    /// Apply queued commands (commands may queue more; bounded to avoid loops).
    pub fn apply_cmds(&mut self, ctx: &egui::Context) {
        for _ in 0..8 {
            if self.cmds.is_empty() {
                break;
            }
            let cmds = std::mem::take(&mut self.cmds);
            for c in cmds {
                ops::apply(self, c, ctx);
            }
        }
    }

    /// Window close requests: with pending jobs, cancel the close and ask first (quitting
    /// cancels the jobs). Runs in `logic`, so it also works while the window is minimized.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        // Other windows open: closing the main window closes just this window.
        if crate::ui::workspaces::windows::intercept_main_close(self, ctx) {
            self.close_cancelled();
            return;
        }
        if crate::editors::intercept_close(self, ctx) {
            self.close_cancelled();
            return;
        }
        let pending = self.fileops.jobs.pending().count();
        if self.quit.when_idle && pending == 0 {
            self.quit.when_idle = false;
            self.quit.allowed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.quit.when_idle {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if !ctx.input(|i| i.viewport().close_requested()) || self.quit.allowed || pending == 0 {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.close_cancelled();
        if !self.quit.prompting.swap(true, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.ui.dialogs.push(crate::ui::dialogs::quit::QuitDialog::new(pending, self.quit.prompting.clone()));
        }
    }

    /// A window close was cancelled (to ask first): an "Install & restart" that started
    /// it no longer restarts; the update stays installed for the next start.
    fn close_cancelled(&mut self) {
        if crate::services::updates::cancel_restart() {
            self.workspaces.quitting = false;
            self.ui.toasts.push(crate::ui::toasts::Toast::info(tr!("app-restart-cancelled")));
        }
    }

    /// The user answered the close confirmation.
    pub fn confirm_quit(&mut self, ctx: &egui::Context, when_idle: bool) {
        if when_idle {
            self.quit.when_idle = true;
            self.ui.toasts.push(crate::ui::toasts::Toast::info(tr!("app-quit-when-finished")));
            ctx.request_repaint();
        } else {
            self.quit.allowed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let pane = self.ws.pane();
        let t = format!("{} — Soyuz", crate::state::crumbs::location_title(&pane.location));
        if t != self.ui.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.ui.title = t;
        }
        crate::ui::a11y::window_name(ctx, &self.ui.title);
    }
}

pub fn theme_name(p: ThemePreference) -> &'static str {
    match p {
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "light",
        ThemePreference::System => "system",
    }
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

/// Settings applied to egui itself (startup and every config change).
pub fn apply_live_settings(ctx: &egui::Context, c: &Config) {
    crate::i18n::apply_setting(&c.general.language);
    let delay = f64::from(c.general.double_click_speed_ms.clamp(100, 2000)) / 1000.0;
    ctx.options_mut(|o| o.input_options.max_double_click_delay = delay);
    crate::ui::views::set_display_options(c.general.show_extensions, c.appearance.icon_style);
    crate::ui::views::icon_pack::apply(ctx, &c.appearance.icon_pack);
    crate::services::thumbs::set_pdf(c.appearance.pdf_thumbnails, &c.preview.pdfium_dir);
    crate::services::thumbs::set_os_cache(c.appearance.os_thumbnail_cache);
    crate::services::thumbs::set_write_os_cache(c.appearance.write_os_thumbnails);
    crate::services::lister::set_dot_hidden(c.general.honour_dot_hidden);
    crate::ai::state::set_rules(&c.ai);
    crate::ui::native::set_enabled(c.appearance.native_dialogs);
    crate::services::tags::set_custom_names(&c.tags);
    crate::services::tags::set_sync_xdg_tags(c.tags.sync_xdg_tags);
    crate::ui::sidebar::set_show_gtk_bookmarks(c.sidebar.gtk_bookmarks);
    for w in crate::ui::context_menu::set_hidden(&c.context_menu.hide) {
        log::warn!("{w}");
    }
    if let Some(w) = crate::services::preview_text::set_syntax_theme(&c.preview.syntax_theme) {
        log::warn!("{w}");
    }
    crate::services::thumbs::set_budget_bytes(c.preview.thumb_cache_mib.saturating_mul(1024 * 1024));
    crate::ui::looks::apply_settings(c);
    crate::services::preview_cache::set_budget_bytes(c.preview.preview_cache_mib.saturating_mul(1024 * 1024));
    crate::services::timeline::set_options(crate::services::timeline::TimelineOpts::from_config(c));
}

/// The location (and entry to put the cursor on) for a command-line path: a folder
/// opens itself, a file opens its parent with the file under the cursor.
pub fn startup_target(path: &std::path::Path) -> Option<(Location, Option<String>)> {
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
    crate::ui::sidebar::standard_places().into_iter().map(|p| FavoriteEntry { name: p.label, path: p.path }).collect()
}

impl SoyuzApp {
    /// One frame of the main window (eframe's `App::ui`).
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        crate::ui::native::begin_frame();
        crate::devscript::begin(self);
        crate::devscript::inject(self, &ctx);
        crate::ui::workspaces::windows::begin_main_frame(self, &ctx);
        self.services.thumbs.begin_frame();
        crate::theme::fonts::begin_frame(&ctx);
        ops::drain_mailbox(self, &ctx);
        crate::services::crash::pump(self, &ctx);
        crate::ui::sidebar::pump(self);
        crate::ui::dialogs::properties::pump(self, &ctx);
        ops::drain_watcher(self);
        crate::palette::begin_frame(self, &ctx);
        crate::ui::panels::terminal::pump(self, &ctx);
        crate::ui::search::pump(self, &ctx);
        crate::ui::tools::begin_frame(self, &ctx);
        crate::ui::portal::begin_frame(self, &ctx);
        ops::fileops::begin_frame(self, &ctx);
        crate::services::mcp_bridge::pump(self, &ctx);
        crate::ai::pump(self, &ctx);
        crate::automation::pump(self, &ctx);
        crate::services::updates::pump(self, &ctx);
        crate::services::filemanager1::pump(self, &ctx);
        crate::ui::remote::pump(self, &ctx);
        crate::services::os_clipboard::pump(self);
        self.apply_cmds(&ctx);
        self.handle_input(&ctx);
        self.apply_cmds(&ctx);
        crate::theme::library::sync(&ctx, &self.config, self.ui.theme);
        crate::ui::looks::begin_frame(&ctx);
        crate::ui::titlebar::sync(&ctx, self);
        crate::ui::native_chrome::sync_theme(&ctx);
        shell::show(ui, self);
        // Overlays live in the focused window (the main one unless another has focus).
        let overlays = crate::ui::workspaces::overlays_here(self);
        if overlays {
            crate::palette::show(&ctx, self);
            crate::ui::settings::show(&ctx, self);
        }
        crate::ui::titlebar::frame(&ctx, self);
        if overlays {
            ops::fileops::end_frame(self, &ctx);
        }
        crate::devscript::pump(self, &ctx);
        self.apply_cmds(&ctx);
        self.update_title(&ctx);
        // The other in-process windows, drawn as immediate viewports.
        crate::ui::workspaces::windows::show(self, &ctx);
        crate::ui::native::popup::end_frame();
        crate::ui::native::end_frame(&ctx);

        // Cheap periodic check: persist when the session text changed (every ~30 s,
        // plus eframe's own `save` on exit).
        if self.last_session_check.elapsed() > Duration::from_secs(30) {
            self.last_session_check = Instant::now();
            self.save_state();
        }
    }
}

impl eframe::App for SoyuzApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_close_request(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        crate::ops::fileops::dragout::raw_input(ctx, raw_input);
        crate::devscript::raw_input(self, raw_input);
        crate::devscript::raw_input_hook(self, raw_input);
    }

    fn on_exit(&mut self) {
        // The close went through: an "Install & restart" now restarts.
        crate::services::updates::arm_restart();
        crate::ops::fileops::linux_dnd::shutdown();
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_state();
        crate::ui::settings::save_now(self);
        self.palette.save_now();
        // Debounced tag / recent changes are queued now and written before we return
        // (eframe calls this on exit too), in order, on the single store worker.
        crate::services::tags::flush();
        crate::services::recent::flush();
        crate::services::folder_colors::flush();
        if !crate::services::store_file::flush(Duration::from_secs(3)) {
            log::warn!("some settings stores were not written in time");
        }
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if crate::ui::native_chrome::backdrop_active() {
            [0.0; 4] // the system backdrop shows through
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }
}
