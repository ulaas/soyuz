//! `SoyuzApp`: the eframe application. It owns the frontend-agnostic [`AppCore`]
//! (configuration, workspace, services, jobs, models) and the egui-only [`GuiState`]
//! (GUI dialogs, preview renderer, settings window, window chrome, debug hooks). One
//! frame runs the core phases (see `core.rs`) with the GUI pumps in between: drain the
//! mailbox + watcher → dispatch keys → draw the shell (widgets push [`Cmd`]s) → apply
//! commands.

use std::time::Duration;

use eframe::CreationContext;
use egui::TextEdit;
use soyuz_core::config::{Config, KeyContext};

use crate::keys;
use crate::theme;
use crate::ui::dialogs::Dialogs;
use crate::ui::shell;
use crate::{actions, ops};

pub use crate::core::{
    AddressState, AppCore, Focus, QuitState, Startup, UiState, default_favorites, load_config, startup_target,
    theme_name,
};

/// State only the egui frontend has.
pub struct GuiState {
    /// GUI-only dialogs (core prompts live on `AppCore::dialogs`; both draw as one stack).
    pub dialogs: Dialogs,
    /// What the preview pane renders (open / width / mode are on `core.ui.preview`).
    pub preview: crate::ui::preview::PreviewPane,
    /// Settings window.
    pub settings: crate::ui::settings::SettingsState,
    /// Window frame currently in effect (`[appearance] custom_titlebar`).
    pub chrome: crate::ui::titlebar::Chrome,
    /// `SOYUZ_SCRIPT` automation / `SOYUZ_FRAME_LOG` (debug hooks).
    pub devscript: crate::devscript::DevScript,
}

/// The egui application: the core plus the GUI's own state. Fields are explicit (no
/// `Deref` to the core) so GUI code can borrow both halves at once.
pub struct SoyuzApp {
    pub core: AppCore,
    pub gui: GuiState,
}

impl SoyuzApp {
    /// A modal dialog is open: a core prompt (`core.dialogs`) or a GUI-only dialog.
    pub fn dialog_open(&self) -> bool {
        self.core.dialogs.is_open() || self.gui.dialogs.is_open()
    }

    /// Construct from the configuration loaded by [`load_config`] (the window
    /// attributes in `main.rs` are derived from the same config).
    pub fn new(cc: &CreationContext<'_>, config: Config, renderer: &'static str, startup: Startup) -> Self {
        let transparent = crate::ui::native_chrome::backdrop_active();
        crate::ui::native_chrome::init(cc, &config, transparent);
        crate::ui::dnd::linux_dnd::init(cc);
        crate::ui::preview::video_gpu::init(cc);
        soyuz_fs::register_ui_thread();
        Self::with_startup(&cc.egui_ctx, config, renderer, startup)
    }

    /// Construct with an explicit config (also used by tests).
    pub fn with_config(ctx: &egui::Context, config: Config, renderer: &'static str) -> Self {
        Self::with_startup(ctx, config, renderer, Startup::default())
    }

    /// Construct with an explicit config and command-line startup options.
    pub fn with_startup(ctx: &egui::Context, config: Config, renderer: &'static str, startup: Startup) -> Self {
        egui_extras::install_image_loaders(ctx);
        // Ctrl +/-/0 are our zoom actions, not egui's global UI zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        apply_live_settings(ctx, &config);
        for w in theme::install(ctx, &config.appearance, &config.fonts, &config.theme.colors) {
            log::warn!("{w}");
        }
        let mut core = AppCore::new(config, startup, crate::host::egui_wake(ctx), crate::ui::gfx::egui_uploader(ctx));
        core.renderer = renderer;
        ctx.set_theme(crate::host::to_egui_theme(core.ui.theme));
        let gui = GuiState {
            dialogs: Dialogs::default(),
            preview: Default::default(),
            settings: Default::default(),
            chrome: crate::ui::titlebar::Chrome::for_config(core.config.appearance.custom_titlebar),
            devscript: crate::devscript::DevScript::from_env(),
        };
        SoyuzApp { core, gui }
    }

    /// Keyboard context + whether a text field owns the keyboard.
    fn key_context(&self, ctx: &egui::Context) -> (KeyContext, bool) {
        if self.dialog_open() {
            return (KeyContext::Dialog, true);
        }
        let text_focused = ctx.memory(|m| m.focused()).is_some_and(|id| TextEdit::load_state(ctx, id).is_some());
        if text_focused {
            (KeyContext::Omnibar, true)
        } else if crate::ui::preview::video::owns_keyboard(self, ctx) {
            // The preview pane's player (its surface or a control) has focus: the
            // transport keys shadow the global chords, which still fall through.
            (KeyContext::Player, false)
        } else if self.core.ui.focus == Focus::List {
            (KeyContext::FileList, false)
        } else if self.core.ui.focus == Focus::Sidebar {
            (KeyContext::Sidebar, false)
        } else {
            (KeyContext::Global, false)
        }
    }

    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        if self.dialog_open()
            || crate::palette::owns_keyboard(&self.core)
            || crate::ui::widgets::dropdown::any_open(ctx)
            || crate::models::tools::any_window_open()
        {
            return; // the modal (or an open dropdown / tool window) owns the keyboard
        }
        if crate::ui::quicklook::handle_keys(self, ctx) {
            return;
        }
        if crate::models::terminal::owns_keyboard(&self.core) {
            let actions =
                crate::ui::panels::terminal::dispatch_keys(ctx, &self.core.keymap, self.core.config.keys.cmd_is_ctrl);
            for a in actions {
                actions::execute(&mut self.core, a, &crate::host::EguiHost::new(ctx));
            }
            return;
        }
        if crate::ui::editors::owns_keyboard(self, ctx) {
            crate::ui::editors::handle_keys(self, ctx);
            return;
        }
        let (kctx, text_focused) = self.key_context(ctx);
        if kctx == KeyContext::FileList && !text_focused {
            // Ctrl+V with only files on the OS clipboard produces no egui event: the
            // fallback reads raw key state. Feed it this frame's egui Ctrl+V so it can
            // de-duplicate, and drop the egui event when the fallback already pasted.
            let cmd_is_ctrl = self.core.config.keys.cmd_is_ctrl;
            let is_v = |c: &soyuz_core::config::KeyChord| {
                c.key == soyuz_core::config::KeyName::V && c.mods.contains(soyuz_core::config::Mods::CTRL)
            };
            let mut synth: Vec<keys::Dispatched> = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|ev| crate::ui::keys::event_chord(ev, i.modifiers, cmd_is_ctrl))
                    .filter(|c| is_v(c))
                    .filter_map(|c| {
                        self.core.keymap.lookup(kctx, c).map(|a| keys::Dispatched { action: a.clone(), chord: c })
                    })
                    .collect()
            });
            let had_egui = !synth.is_empty();
            crate::ui::keys::paste_key_fallback(ctx, &self.core.keymap, &mut synth);
            if had_egui && synth.is_empty() {
                ctx.input_mut(|i| {
                    let m = i.modifiers;
                    i.events.retain(|ev| !crate::ui::keys::event_chord(ev, m, cmd_is_ctrl).is_some_and(|c| is_v(&c)));
                });
            }
            if !had_egui {
                for d in synth {
                    actions::execute(&mut self.core, d.action, &crate::host::EguiHost::new(ctx));
                }
            }
        }
        let mut frame = crate::ui::input::frame(ctx, self.core.config.keys.cmd_is_ctrl);
        self.core.input(&crate::host::EguiHost::new(ctx), &mut frame, kctx, text_focused);
        crate::ui::input::strip(ctx, &frame, self.core.config.keys.cmd_is_ctrl);
    }

    /// Apply queued commands (commands may queue more; bounded to avoid loops). The
    /// core's requests for the frontend are carried out after every command.
    pub fn apply_cmds(&mut self, host: &crate::host::EguiHost) {
        for _ in 0..crate::core::CMD_ROUNDS {
            if self.core.cmds.is_empty() {
                break;
            }
            let cmds = std::mem::take(&mut self.core.cmds);
            for c in cmds {
                ops::apply(&mut self.core, c, host);
                crate::ui::frontend::drain(self, host);
            }
        }
        crate::ui::frontend::drain(self, host);
    }
}

/// Settings applied to egui itself, plus the core's process-wide settings (startup and
/// every config change).
pub fn apply_live_settings(ctx: &egui::Context, c: &Config) {
    crate::core::apply_core_settings(c);
    let delay = f64::from(c.general.double_click_speed_ms.clamp(100, 2000)) / 1000.0;
    ctx.options_mut(|o| o.input_options.max_double_click_delay = delay);
    crate::ui::views::set_display_options(c.general.show_extensions, c.appearance.icon_style);
    crate::ui::views::icon_pack::apply(ctx, &c.appearance.icon_pack);
    crate::ui::native::set_enabled(c.appearance.native_dialogs);
    for w in crate::ui::context_menu::set_hidden(&c.context_menu.hide) {
        log::warn!("{w}");
    }
    crate::ui::looks::apply_settings(c);
}

impl SoyuzApp {
    /// One frame of the main window (eframe's `App::ui`). Core phases (`core.rs`) in
    /// order, with the GUI's own pumps where they always ran.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let host = crate::host::EguiHost::new(&ctx);
        crate::ui::native::begin_frame();
        crate::devscript::begin(self);
        crate::devscript::inject(self, &ctx);
        crate::ui::workspaces::windows::begin_main_frame(self, &ctx);
        self.core.begin_frame();
        crate::theme::fonts::begin_frame(&ctx);
        self.core.pump_services(&host);
        crate::ui::tools::begin_frame(self, &ctx);
        crate::ui::portal::begin_frame(self, &ctx);
        self.core.pump_jobs(&host);
        crate::ui::dnd::external_drops(self, &ctx);
        self.core.pump_background(&host);
        crate::ui::ai::escape_stops_task(self, &ctx);
        self.apply_cmds(&host);
        self.handle_input(&ctx);
        self.apply_cmds(&host);
        crate::theme::library::sync(&ctx, &self.core.config, crate::host::to_egui_theme(self.core.ui.theme));
        crate::ui::titlebar::sync(&ctx, self);
        crate::ui::native_chrome::sync_theme(&ctx);
        shell::show(ui, self);
        // Overlays live in the focused window (the main one unless another has focus).
        let overlays = crate::models::workspaces::overlays_here(&self.core);
        if overlays {
            crate::ui::palette::show(&ctx, self);
            crate::ui::settings::show(&ctx, self);
        }
        crate::ui::titlebar::frame(&ctx, self);
        if overlays {
            crate::ui::dnd::end_frame(self, &ctx);
            ops::fileops::end_frame(&mut self.core, &host);
        }
        crate::devscript::pump(self, &ctx);
        self.apply_cmds(&host);
        self.core.update_title(&host);
        crate::ui::a11y::window_name(&ctx, &self.core.ui.title);
        // The other in-process windows, drawn as immediate viewports.
        crate::ui::workspaces::windows::show(self, &ctx);
        crate::ui::native::popup::end_frame();
        crate::ui::native::end_frame(&ctx);
        self.core.end_frame();
    }
}

impl SoyuzApp {
    /// The close went through: an "Install & restart" now restarts.
    fn exit(&mut self) {
        self.core.on_exit();
        crate::ui::dnd::linux_dnd::shutdown();
    }
}

impl eframe::App for SoyuzApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        self.core.handle_close_request(&crate::host::EguiHost::new(ctx), close_requested);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        crate::ui::dnd::dragout::raw_input(ctx, raw_input);
        crate::devscript::raw_input(self, raw_input);
        crate::devscript::raw_input_hook(self, raw_input);
    }

    #[cfg(not(feature = "glow"))]
    fn on_exit(&mut self) {
        self.exit();
    }

    // eframe's `glow` feature adds the GL context to the signature.
    #[cfg(feature = "glow")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.exit();
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.core.save_all();
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
