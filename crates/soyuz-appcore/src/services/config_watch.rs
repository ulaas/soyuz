//! Config hot reload: a `soyuz_fs::Watcher` on the config directory (editors
//! often replace the file, so the folder is watched, not the file). On a change to
//! `config.toml` the file is parsed on the watcher thread and the result is handed to
//! the UI, which applies it live or shows the parse error (with line / column) and keeps
//! the previous configuration.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use soyuz_core::config::{self, Config, ConfigError, Loaded};
use soyuz_fs::{WatchKind, Watcher};

use crate::core::AppCore;
use crate::host::Host;
use crate::models::toasts::Toast;
use crate::state::view_memory::ViewState;

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
    /// Why the file on disk does not load (at startup or at the last reload). While set,
    /// the active configuration did not come from the file, so saving is refused
    /// ([`crate::models::settings::flush`]) until a reload succeeds.
    pub load_error: Option<ConfigError>,
    path: PathBuf,
}

impl ConfigWatch {
    /// Start watching `path`'s directory (created when missing). Never fails: without a
    /// watcher the app simply has no hot reload.
    pub fn start(wake: crate::host::Wake, path: PathBuf) -> Self {
        let first = read(&path);
        let (last_text, load_error) = (first.text, first.result.err());
        let (tx, rx) = unbounded::<Reloaded>();
        let file = path.clone();
        let spawned = std::thread::Builder::new().name("soyuz-config-watch".into()).spawn(move || {
            let Some(dir) = file.parent().map(Path::to_path_buf) else { return };
            let _ = soyuz_fs::create_dir_all(&dir);
            let (ev_tx, ev_rx) = unbounded();
            let mut watcher = match Watcher::new(ev_tx, DEBOUNCE, crate::host::no_wake()) {
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
                wake();
            }
        });
        let rx = match spawned {
            Ok(_) => Some(rx),
            Err(e) => {
                log::warn!("cannot spawn the config watcher: {e}");
                None
            }
        };
        ConfigWatch { rx, last_text, load_error, path }
    }

    /// A watcher that never reports (tests).
    pub fn disabled(path: PathBuf) -> Self {
        ConfigWatch { rx: None, last_text: None, load_error: None, path }
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
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    let Some(r) = app.config_watch.poll() else { return };
    if r.text.is_some() && r.text == app.config_watch.last_text {
        return; // our own save (or no real change)
    }
    app.config_watch.last_text = r.text;
    handle(app, host, r.result, false);
}

/// Apply a parse result: live-apply a good config, or toast the error and keep the old one.
/// Settings not saved yet are kept on top of the reloaded file, and go out with the next
/// write (a write refused while the file did not load is scheduled again here).
pub fn handle(app: &mut AppCore, host: &dyn Host, result: Result<Loaded, ConfigError>, explicit: bool) {
    match result {
        Ok(loaded) => {
            app.config_watch.load_error = None;
            let new = match (app.config_save.pending(), &app.config_save.baseline) {
                (true, Some(base)) => config::rebase(base, &app.config, &loaded.config).unwrap_or_else(|e| {
                    log::warn!("cannot keep unsaved settings over the reloaded config: {e}");
                    loaded.config.clone()
                }),
                _ => loaded.config.clone(),
            };
            app.config_save.baseline = Some(loaded.config);
            if std::mem::take(&mut app.config_save.held) {
                app.config_save.schedule();
                host.redraw_after(crate::models::settings::SAVE_DELAY);
            }
            let changed = new != app.config;
            if changed {
                apply_config(app, host, new);
            }
            if !loaded.warnings.is_empty() {
                app.ui
                    .toasts
                    .push(Toast::warning(tr!("services-config-reloaded-warnings")).body(loaded.warnings.join("\n")));
            } else if changed || explicit {
                app.ui.toasts.push(Toast::success(tr!("services-config-reloaded")));
            }
        }
        Err(e) => {
            app.ui.toasts.push(error_toast(&e));
            app.config_watch.load_error = Some(e);
        }
    }
}

/// The toast for a `config.toml` that does not load.
pub fn error_toast(e: &ConfigError) -> Toast {
    let title = match e {
        ConfigError::Parse { line, col, .. } if *line > 0 => {
            tr!("services-config-error-at", line = line.to_string(), col = col.to_string())
        }
        _ => tr!("services-config-load-failed").to_string(),
    };
    let body = match e {
        ConfigError::Parse { msg, .. } => tr!("services-config-error-body", error = msg),
        other => tr!("services-config-error-body", error = other.to_string()),
    };
    Toast::error(title).body(body).action(tr!("services-config-edit"), soyuz_core::config::Action::EditConfigFile)
}

/// `ReloadConfig`: re-read the file now.
pub fn reload_now(app: &mut AppCore, host: &dyn Host) {
    let r = read(app.config_watch.path());
    app.config_watch.last_text = r.text.clone();
    handle(app, host, r.result, true);
}

/// Make `new` the active configuration, applying everything that is not read live each
/// frame: theme / fonts / scale, key map, default view, visibility toggles.
pub fn apply_config(app: &mut AppCore, host: &dyn Host, new: Config) {
    let old = std::mem::replace(&mut app.config, new);
    let c = &app.config;
    host.config_changed(&old, c);
    if old.appearance != c.appearance || old.fonts != c.fonts || old.theme != c.theme {
        app.ui.reduced_motion = c.appearance.reduced_motion;
        app.ui.toasts.set_time_scale(c.appearance.toast_time_scale);
        if old.appearance.theme != c.appearance.theme {
            app.ui.theme = crate::host::ThemePref::from_config(&c.appearance.theme);
        }
        host.set_theme(app.ui.theme);
    }
    if old.archive != c.archive {
        app.fileops.jobs.reconfigure(c);
    }
    if old.jobs != c.jobs {
        // Copy options (buffer, verify, reflink, preserve) apply to the next job; the
        // queue settings (parallel jobs, lanes, apply-to-all, history) right away.
        app.fileops.jobs.reconfigure(c);
    }
    let (km, warnings) = crate::keys::build_keymap(c);
    for w in warnings {
        log::warn!("{w}");
    }
    app.keymap = km;
    crate::models::workspaces::bind_hotkeys_into(&mut app.keymap, &app.workspaces.store);
    app.keyseq.reset();
    // The bottom panel and the preview pane are per window: apply to every window.
    let height =
        (old.layout.terminal_height != c.layout.terminal_height).then(|| c.layout.terminal_height.clamp(120.0, 720.0));
    let placement = (app.ui.bottom.activity_placement, c.layout.activity_panel);
    let preview = (old.layout.preview_pane != c.layout.preview_pane).then_some(c.layout.preview_pane);
    crate::models::windows::for_each_window(app, |app| {
        if let Some(h) = height {
            // The panel remembers a size the user dragged; the setting is the default height.
            app.ui.bottom.height = h;
            app.ui.bottom.reset_height = true;
        }
        // The placement is shared: move each window's panel from the old place.
        app.ui.bottom.activity_placement = placement.0;
        app.ui.bottom.set_activity_placement(placement.1);
        if let Some(open) = preview
            && app.ui.preview.open != open
        {
            app.ui.preview.toggle();
        }
    });
    let c = &app.config;
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
    if relist || old.general.folders_first != c.general.folders_first {
        for ws in crate::models::windows::all_workspaces_mut(app) {
            for t in &mut ws.tabs {
                for p in &mut t.panes {
                    p.dirty = true;
                }
            }
        }
    }
}
