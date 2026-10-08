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
use crate::state::session::{FavoriteEntry, PaneSession, Session, TabSession};
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
    /// Command palette / go-to-folder state and persistent palette memory (M11).
    pub palette: crate::palette::PaletteState,
    /// Settings window (M11).
    pub settings: crate::ui::settings::SettingsState,
    /// `config.toml` hot reload (M11).
    pub config_watch: crate::services::config_watch::ConfigWatch,
    /// Pending multi-key sequence / mark (M11).
    pub keyseq: crate::keys::sequence::Sequencer,
}

impl SoyuzApp {
    pub fn new(cc: &CreationContext<'_>, renderer: &'static str) -> Self {
        let path = config::default_path();
        let config = match config::load_or_default(&path) {
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
        };
        Self::with_config(&cc.egui_ctx, config, renderer)
    }

    /// Construct with an explicit config (also used by tests).
    pub fn with_config(ctx: &egui::Context, config: Config, renderer: &'static str) -> Self {
        egui_extras::install_image_loaders(ctx);
        // Ctrl +/-/0 are our zoom actions, not egui's global UI zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        for w in theme::install(ctx, &config.appearance, &config.theme.colors) {
            log::warn!("{w}");
        }
        let (keymap, warnings) = config::keymap(&config);
        for w in warnings {
            log::warn!("{w}");
        }
        let services = Services::new(ctx, Duration::from_millis(config.behaviour.watch_debounce_ms.max(50)));
        let dir = config::config_dir();
        let session_path = dir.join("session.toml");
        let views_path = dir.join("views.toml");
        let views =
            if config.behaviour.remember_folder_views { ViewMemory::load(&views_path) } else { ViewMemory::default() };
        let session = if config.behaviour.restore_session { Session::load(&session_path) } else { None };
        let default_view = ViewState::from_config(&config);

        let mut sidebar = SidebarState::new();
        if let Some(favs) = session.as_ref().and_then(|s| s.favorites.clone()) {
            sidebar.favorites = favs;
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

        let ui = UiState {
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
            bottom: BottomPanel::default(),
            theme: theme_pref,
            reduced_motion: config.appearance.reduced_motion,
            title: String::new(),
            terminal: Default::default(),
            preview: crate::ui::preview::PreviewPane::new(config.layout.preview_pane),
            quicklook: Default::default(),
            search: crate::ui::search::SearchUi::new(&config),
        };

        let start = start_location(&config);
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
            config_watch: crate::services::config_watch::ConfigWatch::start(ctx, config::default_path()),
            keyseq: Default::default(),
        };
        let restored = session.as_ref().is_some_and(|s| app.restore_tabs(s));
        if !restored {
            let view = app.view_for(&start);
            app.ws.tabs[0].panes[0].view = view;
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
        let mut tabs = Vec::new();
        for ts in &s.tabs {
            let mut panes = Vec::new();
            for ps in &ts.panes {
                let Some(loc) = Location::parse(&ps.location) else { continue };
                let mut pane = Pane::new(loc.clone(), self.view_for(&loc));
                pane.pending_cursor = ps.cursor.clone();
                pane.pending_selection = ps.selection.clone();
                pane.restore_scroll = (ps.scroll > 0.0).then_some(ps.scroll);
                panes.push(pane);
            }
            if panes.is_empty() {
                continue;
            }
            let mut tab = Tab::new(panes.remove(0));
            tab.panes.extend(panes.into_iter().take(1));
            tab.dual = ts.dual && tab.panes.len() > 1;
            tab.focused = ts.focused.min(tab.panes.len() - 1);
            tab.orientation = ts.orientation;
            tab.split = ts.split.clamp(0.15, 0.85);
            tab.pinned = ts.pinned;
            tab.sync_nav = ts.sync_nav;
            tabs.push(tab);
        }
        if tabs.is_empty() {
            return false;
        }
        // Pinned tabs first.
        tabs.sort_by_key(|t| !t.pinned);
        self.ws.tabs = tabs;
        self.ws.active = s.active_tab.min(self.ws.tabs.len() - 1);
        true
    }

    /// Snapshot of the current session.
    pub fn session(&self) -> Session {
        let tabs = self
            .ws
            .tabs
            .iter()
            .map(|t| TabSession {
                pinned: t.pinned,
                dual: t.dual,
                orientation: t.orientation,
                split: t.split,
                focused: t.focused,
                sync_nav: t.sync_nav,
                panes: t
                    .panes
                    .iter()
                    .filter(|p| !matches!(p.location, Location::Search(_)))
                    .map(pane_session)
                    .collect(),
            })
            .collect();
        Session {
            active_tab: self.ws.active,
            tabs,
            favorites: self.ui.sidebar.favorites_customized.then(|| self.ui.sidebar.favorites.clone()),
            show_hidden: Some(self.ui.show_hidden),
            theme: Some(theme_name(self.ui.theme).to_string()),
            collapsed_sections: self.ui.sidebar.collapsed.iter().cloned().collect(),
            sidebar_visible: Some(self.ui.sidebar_visible),
            checkbox_mode: self.ui.checkbox_mode,
            ..Session::default()
        }
    }

    /// Persist the session and view memory if they changed.
    pub fn save_state(&mut self) {
        if self.config.behaviour.restore_session {
            let text = self.session().to_toml();
            if self.last_session.as_deref() != Some(text.as_str()) {
                match self.session().save(&self.session_path) {
                    Ok(()) => self.last_session = Some(text),
                    Err(e) => log::warn!("cannot save session: {e}"),
                }
            }
        }
        if self.views.dirty && self.config.behaviour.remember_folder_views {
            if let Some(dir) = self.views_path.parent() {
                let _ = soyuz_fs::create_dir_all(dir);
            }
            match soyuz_fs::write_atomic(&self.views_path, self.views.to_toml().as_bytes()) {
                Ok(()) => self.views.dirty = false,
                Err(e) => log::warn!("cannot save view memory: {e}"),
            }
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

    fn handle_input(&mut self, ctx: &egui::Context) {
        if self.ui.dialogs.is_open() || crate::palette::owns_keyboard(self) {
            return; // the modal owns the keyboard
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

    fn update_title(&mut self, ctx: &egui::Context) {
        let pane = self.ws.pane();
        let t = format!("{} — Soyuz", crate::state::crumbs::location_title(&pane.location));
        if t != self.ui.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.ui.title = t;
        }
    }
}

fn pane_session(p: &Pane) -> PaneSession {
    let mut selection: Vec<String> = p.selection.iter().map(|k| k.to_string()).collect();
    selection.truncate(500);
    PaneSession {
        location: p.location.display(),
        cursor: p.cursor.as_deref().map(str::to_string).or_else(|| p.pending_cursor.clone()),
        selection,
        scroll: p.scroll_y,
    }
}

pub fn theme_name(p: ThemePreference) -> &'static str {
    match p {
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "light",
        ThemePreference::System => "system",
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

impl eframe::App for SoyuzApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.services.thumbs.begin_frame();
        ops::drain_mailbox(self, &ctx);
        ops::drain_watcher(self);
        crate::palette::begin_frame(self, &ctx);
        crate::ui::panels::terminal::pump(self, &ctx);
        crate::ui::search::pump(self, &ctx);
        ops::fileops::begin_frame(self, &ctx);
        self.apply_cmds(&ctx);
        self.handle_input(&ctx);
        self.apply_cmds(&ctx);
        shell::show(ui, self);
        crate::palette::show(&ctx, self);
        crate::ui::settings::show(&ctx, self);
        ops::fileops::end_frame(self, &ctx);
        self.apply_cmds(&ctx);
        self.update_title(&ctx);
        // Cheap periodic check: persist when the session text changed (every ~30 s,
        // plus eframe's own `save` on exit).
        if self.last_session_check.elapsed() > Duration::from_secs(30) {
            self.last_session_check = Instant::now();
            self.save_state();
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_state();
        self.palette.save_now();
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}
