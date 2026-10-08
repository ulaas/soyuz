//! Config hot reload (PLAN §3): a `soyuz_fs::Watcher` on the config directory (editors
//! often replace the file, so the folder is watched, not the file). On a change to
//! `config.toml` the file is parsed on the watcher thread and the result is handed to
//! the UI, which applies it live or shows the parse error (with line / column) and keeps
//! the previous configuration.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use soyuz_core::config::{self, Config, ConfigError, Loaded};
use soyuz_fs::{WatchKind, Watcher};

use crate::app::SoyuzApp;
use crate::state::view_memory::ViewState;
use crate::ui::toasts::Toast;

/// Debounce for config changes (well inside the 1 s budget).
const DEBOUNCE: Duration = Duration::from_millis(120);

/// A reload result plus the raw text it came from.
pub struct Reloaded {
    pub result: Result<Loaded, ConfigError>,
    pub text: Option<String>,
}

pub struct ConfigWatch {
    rx: Option<Receiver<Reloaded>>,
    /// Text of the last file we wrote ourselves (or loaded), to ignore our own saves.
    pub last_text: Option<String>,
    path: PathBuf,
}

impl ConfigWatch {
    /// Start watching `path`'s directory (created when missing). Never fails: without a
    /// watcher the app simply has no hot reload.
    pub fn start(ctx: &egui::Context, path: PathBuf) -> Self {
        let last_text = std::fs::read_to_string(&path).ok();
        let (tx, rx) = unbounded::<Reloaded>();
        let ctx = ctx.clone();
        let file = path.clone();
        let spawned = std::thread::Builder::new().name("soyuz-config-watch".into()).spawn(move || {
            let Some(dir) = file.parent().map(Path::to_path_buf) else { return };
            let _ = soyuz_fs::create_dir_all(&dir);
            let (ev_tx, ev_rx) = unbounded();
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            let mut watcher = match Watcher::new(ev_tx, DEBOUNCE, wake) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("config hot reload unavailable: {e}");
                    return;
                }
            };
            if let Err(e) = watcher.watch(&dir, false) {
                log::warn!("cannot watch {}: {e}", dir.display());
                return;
            }
            let name = file.file_name().map(|n| n.to_os_string());
            while let Ok(ev) = ev_rx.recv() {
                let relevant = match &ev.kind {
                    WatchKind::Changed(paths) => paths.iter().any(|p| p.file_name().map(|n| n.to_os_string()) == name),
                    WatchKind::Rescan => true,
                    WatchKind::Lost => false,
                };
                if !relevant {
                    continue;
                }
                let reloaded = read(&file);
                if tx.send(reloaded).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        let rx = match spawned {
            Ok(_) => Some(rx),
            Err(e) => {
                log::warn!("cannot spawn the config watcher: {e}");
                None
            }
        };
        ConfigWatch { rx, last_text, path }
    }

    /// A watcher that never reports (tests).
    pub fn disabled(path: PathBuf) -> Self {
        ConfigWatch { rx: None, last_text: None, path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Latest pending reload (older ones are superseded).
    pub fn poll(&mut self) -> Option<Reloaded> {
        let rx = self.rx.as_ref()?;
        let mut last = None;
        while let Ok(r) = rx.try_recv() {
            last = Some(r);
        }
        last
    }
}

/// Read and parse the config file (missing file = defaults).
pub fn read(path: &Path) -> Reloaded {
    match std::fs::read_to_string(path) {
        Ok(text) => Reloaded { result: config::parse_str(&text), text: Some(text) },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Reloaded { result: Ok(Loaded { config: Config::default(), warnings: Vec::new() }), text: None }
        }
        Err(_) => Reloaded { result: config::load(path), text: None },
    }
}

/// Per frame: apply a pending external edit.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    let Some(r) = app.config_watch.poll() else { return };
    if r.text.is_some() && r.text == app.config_watch.last_text {
        return; // our own save (or no real change)
    }
    app.config_watch.last_text = r.text;
    handle(app, ctx, r.result, false);
}

/// Apply a parse result: live-apply a good config, or toast the error and keep the old one.
pub fn handle(app: &mut SoyuzApp, ctx: &egui::Context, result: Result<Loaded, ConfigError>, explicit: bool) {
    match result {
        Ok(loaded) => {
            let changed = loaded.config != app.config;
            if changed {
                apply_config(app, ctx, loaded.config);
            }
            if !loaded.warnings.is_empty() {
                app.ui
                    .toasts
                    .push(Toast::warning("Configuration reloaded with warnings").body(loaded.warnings.join("\n")));
            } else if changed || explicit {
                app.ui.toasts.push(Toast::success("Configuration reloaded"));
            }
        }
        Err(e) => {
            let title = match &e {
                ConfigError::Parse { line, col, .. } if *line > 0 => {
                    format!("config.toml: error at line {line}, column {col}")
                }
                _ => "config.toml could not be loaded".to_string(),
            };
            let body = match &e {
                ConfigError::Parse { msg, .. } => format!("{msg}\nThe previous settings stay active."),
                other => format!("{other}\nThe previous settings stay active."),
            };
            app.ui
                .toasts
                .push(Toast::error(title).body(body).action("Edit config", soyuz_core::config::Action::EditConfigFile));
        }
    }
}

/// `ReloadConfig`: re-read the file now.
pub fn reload_now(app: &mut SoyuzApp, ctx: &egui::Context) {
    let r = read(app.config_watch.path());
    app.config_watch.last_text = r.text.clone();
    handle(app, ctx, r.result, true);
}

/// Make `new` the active configuration, applying everything that is not read live each
/// frame: theme / fonts / scale, key map, default view, visibility toggles.
pub fn apply_config(app: &mut SoyuzApp, ctx: &egui::Context, new: Config) {
    let old = std::mem::replace(&mut app.config, new);
    let c = &app.config;
    if old.appearance != c.appearance || old.theme != c.theme {
        for w in crate::theme::install(ctx, &c.appearance, &c.theme.colors) {
            log::warn!("{w}");
        }
        let scale = if c.appearance.ui_scale > 0.25 { c.appearance.ui_scale.clamp(0.5, 3.0) } else { 1.0 };
        if (ctx.zoom_factor() - scale).abs() > f32::EPSILON {
            ctx.set_zoom_factor(scale);
        }
        app.ui.reduced_motion = c.appearance.reduced_motion;
        if old.appearance.theme != c.appearance.theme {
            app.ui.theme = crate::theme::preference_from_config(&c.appearance.theme);
        }
        ctx.set_theme(app.ui.theme);
    }
    let (km, warnings) = config::keymap(c);
    for w in warnings {
        log::warn!("{w}");
    }
    app.keymap = km;
    app.keyseq.reset();
    crate::app::apply_live_settings(ctx, c);
    if old.layout.terminal_height != c.layout.terminal_height {
        // The panel remembers a size the user dragged; the setting is the default height.
        app.ui.bottom.height = c.layout.terminal_height.clamp(120.0, 720.0);
        app.ui.bottom.reset_height = true;
    }
    app.ui.bottom.set_activity_placement(c.layout.activity_panel);
    if old.general != c.general
        || old.columns != c.columns
        || old.appearance.thumbnail_size != c.appearance.thumbnail_size
    {
        app.default_view = ViewState::from_config(c);
    }
    let mut relist = false;
    if old.general.show_hidden != c.general.show_hidden {
        app.ui.show_hidden = c.general.show_hidden;
        relist = true;
    }
    if old.general.show_system != c.general.show_system {
        app.ui.show_system = c.general.show_system;
        relist = true;
    }
    if old.layout.sidebar != c.layout.sidebar {
        app.ui.sidebar_visible = c.layout.sidebar;
    }
    if old.layout.status_bar != c.layout.status_bar {
        app.ui.status_visible = c.layout.status_bar;
    }
    if old.layout.preview_pane != c.layout.preview_pane && app.ui.preview.open != c.layout.preview_pane {
        app.ui.preview.toggle();
    }
    if relist || old.general.folders_first != c.general.folders_first {
        for t in &mut app.ws.tabs {
            for p in &mut t.panes {
                p.dirty = true;
            }
        }
    }
}
