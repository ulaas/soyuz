//! Debug / automation hook (like `SOYUZ_THROTTLE_MBPS` and `SOYUZ_OPEN_TOOL`):
//! `SOYUZ_SCRIPT` runs a list of steps at startup, separated by `;`, and can save
//! screenshots of the window. Used for visual regression passes and docs screenshots.
//!
//! Steps:
//! * `wait:<ms>` — let the UI settle;
//! * `shot:<file.png>` — screenshot of this window only;
//! * `cd:<path>` — navigate the focused pane; `select:<name>` — select one item;
//! * `theme:dark|light|system`;
//! * `config:<dotted.key>=<toml value>` — change a setting live (`config:appearance.density="touch"`);
//! * `scroll:<px>` — scroll the focused pane to an offset;
//! * `size:<w>x<h>` — resize the window; `welcome:on|off`; `appearance` (Settings →
//!   Appearance); `quit`;
//! * `expect:<check>` — log `SOYUZ_SCRIPT: expect ok <check>` or `expect FAILED <check>
//!   (actual …)`; checks: `cwd=<folder name>`, `cursor=<name>`, `settings=open|closed`,
//!   `palette=open|closed`, `dialog=open|closed`, `exists=<path>`, `missing=<path>`,
//!   `lang=<tag>`, `has=<name>` (the focused pane shows it) (used by the keyboard-only test, `tests/keyboard_only.rs`);
//! * `until:<path>` — pause the script until `<path>` exists (at most 120 s); external
//!   drivers such as `scripts/uia-audit.ps1` use it to hold a state while they inspect
//!   the window;
//! * `toast:<kind>|<title>[|<body>[|undo]]` — show a toast for 30 s (`info`, `success`,
//!   `warning`, `error`; `undo` adds an Undo action), for toast screenshots;
//! * `hover:<x>,<y>` — move egui's pointer to that window position (no OS input);
//! * `panes:<label>` — log the panes of the active tab (location, status, entry and
//!   visible counts, visible names);
//! * `mark:<label>` — log a marker line (`SOYUZ_SCRIPT: mark <label>`), e.g. where the
//!   frame-budget test starts counting;
//! * `settings:<section>` — open the settings window on a section (`settings:appearance`;
//!   `settings:close` closes it);
//! * `dropdown:<key>[=query]` — open the dropdown with that key (settings rows use
//!   `settings.<id>`, e.g. `dropdown:settings.font.ui=segoe`) with `query` in its search;
//! * `smart:<name>` — open a smart folder (`smart:` alone: the new smart folder dialog);
//! * `keys:<spec>` — synthetic keyboard input on the next frame: plain text is typed,
//!   `{Enter}`, `{Ctrl+S}`, `{Shift+F3}`, `{Copy}`, `{Cut}`, `{Paste:text}` are keys;
//! * `edit:<path>` — open a file in the built-in editor; `editor:<command>` — drive it
//!   (see `editors::devscript`);
//! * `ai:<prompt>` — send a prompt to the assistant; `ai:!approve` approves every
//!   pending plan, `ai:!smart <text>` opens smart search with `text`, `ai:!save <name>`
//!   saves the last task as a recipe, `ai:!tab chat|recipes|audit` switches the panel page,
//!   `ai:!mcp-approve` approves every pending MCP request card;
//! * `ws:<verb> [arg]` — workspaces: `group`, `add <tab>`, `collapse`, `color <0-7>`,
//!   `save <name>`, `open <n>`, `layout single|dual|stacked|quad`, `sync`, `tear <tab>`,
//!   `move <tab> <window>`, `close <window>`, `merge`, `select <tab>`;
//! * `click:<x>,<y>[,right]` — synthetic egui click at a point of the main window;
//!   `press:<x>,<y>` / `move:<x>,<y>` / `release:<x>,<y>` — its parts, chained for a
//!   synthetic drag (`press:300,140;move:320,150;move:900,400;release:900,400`);
//! * `native:list` / `native:move <x>,<y>` / `native:moveby <dx>,<dy>` — native dialog / popup
//!   windows (`ui::native`): log their handles and rects, move the dialog (physical pixels);
//!   `keys:` goes to the newest native window while one is open;
//! * anything else is an action name (`view_grid`, `toggle_dual_pane`, `settings`, …).
//!
//! `SOYUZ_WINDOW_POS=x,y` (read in `main.rs`) opens the window at that position without
//! activating it, e.g. `-4000,-4000` keeps scripted screenshot sessions off-screen.
//!
//! `SOYUZ_FRAME_LOG=1` logs a line per rendered frame with the time spent in `ui()`
//! (to check that an idle window does not repaint and that big folders stay fast).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_core::config::{Action, Config};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;

static PROCESS_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
static STARTUP_FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Remember when `main` started (startup-time measurement).
pub fn mark_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

/// Milliseconds since [`mark_process_start`] (0 when it was not called).
pub fn since_start_ms() -> f64 {
    PROCESS_START.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0)
}

/// Log a startup milestone (`startup: <what> at <ms> ms`).
pub fn startup_mark(what: &str) {
    if PROCESS_START.get().is_some() {
        log::info!("startup: {what} at {:.1} ms", since_start_ms());
    }
}

#[derive(Default)]
pub struct DevScript {
    steps: VecDeque<String>,
    resume_at: Option<Instant>,
    pending_shot: Option<PathBuf>,
    frame_log: bool,
    frames: u64,
    frame_start: Option<Instant>,
    /// Synthetic input for the next frame (`keys:`).
    events: Vec<egui::Event>,
    /// When the current `until:` step started waiting.
    until_since: Option<Instant>,
    /// Synthetic pointer position (`hover:`), fed through [`raw_input_hook`].
    pointer: Option<egui::Pos2>,
    pointer_sent: Option<egui::Pos2>,
    /// Synthetic pointer input, fed before the next frame (`click:`).
    raw: Vec<egui::Event>,
}

impl DevScript {
    pub fn from_env() -> Self {
        let steps = std::env::var("SOYUZ_SCRIPT")
            .map(|s| s.split(';').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
            .unwrap_or_default();
        DevScript { steps, frame_log: std::env::var("SOYUZ_FRAME_LOG").is_ok_and(|v| v == "1"), ..Default::default() }
    }
}

/// Start of a frame: inject queued synthetic input.
pub fn inject(app: &mut SoyuzApp, ctx: &egui::Context) {
    if !app.gui.devscript.events.is_empty() {
        let events = std::mem::take(&mut app.gui.devscript.events);
        ctx.input_mut(|i| i.events.extend(events));
    }
}

/// eframe's raw-input hook: keep the synthetic `hover:` pointer in place (before egui
/// reads the input, so hover state follows it).
pub fn raw_input_hook(app: &mut SoyuzApp, raw: &mut egui::RawInput) {
    if let Some(pos) = app.gui.devscript.pointer {
        raw.events.retain(|e| !matches!(e, egui::Event::PointerGone | egui::Event::PointerMoved(_)));
        // Moved once: repeating it every frame would count as movement (no tooltips).
        if app.gui.devscript.pointer_sent != Some(pos) {
            raw.events.push(egui::Event::PointerMoved(pos));
            app.gui.devscript.pointer_sent = Some(pos);
        }
    }
}

/// Before a frame of the main window: queued synthetic pointer input (egui needs it in
/// the raw input to compute hover and clicks).
pub fn raw_input(app: &mut SoyuzApp, raw: &mut egui::RawInput) {
    raw.events.append(&mut app.gui.devscript.raw);
}

/// `keys:` spec → egui events.
fn key_events(spec: &str) -> Vec<egui::Event> {
    let mut out = Vec::new();
    let mut rest = spec;
    while !rest.is_empty() {
        if let Some(body) = rest.strip_prefix('{')
            && let Some(end) = body.find('}')
        {
            let tok = &body[..end];
            rest = &body[end + 1..];
            if tok == "Copy" {
                out.push(egui::Event::Copy);
            } else if tok == "Cut" {
                out.push(egui::Event::Cut);
            } else if let Some(t) = tok.strip_prefix("Paste:") {
                out.push(egui::Event::Paste(t.to_string()));
            } else {
                let mut m = egui::Modifiers::NONE;
                let mut key = None;
                for part in tok.split('+') {
                    match part {
                        "Ctrl" => {
                            m.ctrl = true;
                            m.command = true;
                        }
                        "Shift" => m.shift = true,
                        "Alt" => m.alt = true,
                        k => key = egui::Key::from_name(k),
                    }
                }
                if let Some(key) = key {
                    out.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
                    out.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m });
                }
            }
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        rest = &rest[c.len_utf8()..];
        out.push(egui::Event::Text(c.to_string()));
    }
    out
}

/// Start of a frame (frame-time logging).
pub fn begin(app: &mut SoyuzApp) {
    match STARTUP_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) {
        0 => startup_mark("first frame (ui start)"),
        // The second `ui()` starts after the first frame was painted and presented.
        1 => startup_mark("first frame presented"),
        _ => {}
    }
    if app.gui.devscript.frame_log {
        app.gui.devscript.frame_start = Some(Instant::now());
    }
}

/// Run due steps. Called once per frame.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    if app.gui.devscript.frame_log {
        app.gui.devscript.frames += 1;
        let ms = app.gui.devscript.frame_start.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
        log::info!("frame {} ui {ms:.2} ms", app.gui.devscript.frames);
    }
    if let Some(path) = app.gui.devscript.pending_shot.clone() {
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        match image {
            Some(img) => {
                save_png(&path, &img);
                app.gui.devscript.pending_shot = None;
            }
            None => {
                ctx.request_repaint();
                return;
            }
        }
    }
    loop {
        if let Some(t) = app.gui.devscript.resume_at {
            let now = Instant::now();
            if now < t {
                ctx.request_repaint_after(t - now);
                return;
            }
            app.gui.devscript.resume_at = None;
        }
        let Some(step) = app.gui.devscript.steps.pop_front() else { return };
        if app.gui.devscript.until_since.is_none() {
            log::info!("SOYUZ_SCRIPT: {step}");
        }
        let (verb, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
        match verb {
            "wait" => {
                let ms = arg.trim().parse::<u64>().unwrap_or(500);
                app.gui.devscript.resume_at = Some(Instant::now() + Duration::from_millis(ms));
            }
            "until" => {
                let since = *app.gui.devscript.until_since.get_or_insert_with(Instant::now);
                if !std::path::Path::new(arg.trim()).exists() && since.elapsed() < Duration::from_secs(120) {
                    app.gui.devscript.steps.push_front(step.clone());
                    app.gui.devscript.resume_at = Some(Instant::now() + Duration::from_millis(100));
                } else {
                    app.gui.devscript.until_since = None;
                }
            }
            "shot" => {
                app.gui.devscript.pending_shot = Some(PathBuf::from(arg.trim()));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                ctx.request_repaint();
                return;
            }
            "cd" => {
                let pane = app.core.ws.pane().id;
                app.core.cmds.push(Cmd::Navigate {
                    pane,
                    location: Location::Dir(PathBuf::from(arg.trim())),
                    select: None,
                });
            }
            "select" => {
                let pane = app.core.ws.pane_mut();
                if let Some(pos) = pane.visible_pos(arg.trim()) {
                    pane.move_cursor(pos, false, false);
                }
            }
            "theme" => {
                app.core.ui.theme = crate::host::ThemePref::from_config(arg);
                ctx.set_theme(app.core.ui.theme);
            }
            "config" => match edited(&app.core.config, arg) {
                Ok(c) => crate::services::config_watch::apply_config(&mut app.core, ctx, c),
                Err(e) => log::warn!("SOYUZ_SCRIPT {step}: {e}"),
            },
            "size" => {
                if let Some((w, h)) = arg.split_once('x')
                    && let (Ok(w), Ok(h)) = (w.trim().parse::<f32>(), h.trim().parse::<f32>())
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                }
            }
            "welcome" => app.core.ui.welcome = arg.trim() != "off",
            "mark" => log::info!("SOYUZ_SCRIPT: mark {}", arg.trim()),
            "panes" => log_panes(app, arg.trim()),
            "hover" => {
                // `hover:<x>,<y>`: synthetic pointer position (logical points), in-app only.
                if let Some((x, y)) = arg.split_once(',')
                    && let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>())
                {
                    app.gui.devscript.pointer = Some(egui::pos2(x, y));
                }
            }
            "toast" => {
                app.core.ui.toasts.push(script_toast(arg));
            }
            "expect" => match expect(app, arg.trim()) {
                Ok(()) => log::info!("SOYUZ_SCRIPT: expect ok {}", arg.trim()),
                Err(actual) => log::warn!("SOYUZ_SCRIPT: expect FAILED {} (actual {actual})", arg.trim()),
            },
            "scroll" => {
                // Absolute scroll offset of the focused pane (perf probes).
                let y = arg.trim().parse::<f32>().unwrap_or(0.0);
                app.core.ws.pane_mut().restore_scroll = Some(y);
            }
            "export_picker" => {
                crate::ui::settings::open(app, Some(crate::ui::settings::Section::Advanced));
                crate::ui::settings::start_export(app);
            }
            "settings" => {
                // `settings:appearance` opens the window on that section.
                let want = arg.trim().to_ascii_lowercase();
                if want == "close" {
                    crate::ui::settings::close(app);
                    ctx.request_repaint();
                    continue;
                }
                let sec = crate::ui::settings::Section::ALL
                    .into_iter()
                    .find(|s| s.english_title().to_ascii_lowercase().starts_with(&want));
                crate::ui::settings::open(app, sec);
            }
            "appearance" => crate::ui::settings::open(app, Some(crate::ui::settings::Section::Appearance)),
            "quit" => {
                app.core.workspaces.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            "ws" => crate::models::workspaces::devscript(&mut app.core, arg),
            "smart" => {
                // `smart:<name>` opens a smart folder; `smart:` opens the "new" dialog.
                if arg.trim().is_empty() {
                    app.core.cmds.push(Cmd::Smart(crate::ui::smart::SmartCmd::Edit(None)));
                } else if let Some(index) = crate::ui::smart::folders().iter().position(|f| f.name == arg.trim()) {
                    app.core.cmds.push(Cmd::Smart(crate::ui::smart::SmartCmd::Open { index, new_tab: false }));
                }
            }
            "edit" | "editor" => crate::editors::devscript(&mut app.core, ctx, verb, arg),
            "keys" => {
                // An open native dialog / popup window has the keyboard.
                let evs = key_events(arg);
                if !crate::ui::native::inject(&evs) {
                    app.gui.devscript.events.extend(evs);
                }
                ctx.request_repaint();
                return;
            }
            "dropdown" => {
                // `dropdown:<key>[=query]`: open a dropdown (and type into its search).
                let (key, query) = arg.split_once('=').unwrap_or((arg, ""));
                crate::ui::widgets::dropdown::request_open(ctx, key.trim(), query);
            }
            "ai" => crate::ai::state::script(&mut app.core, ctx, arg),
            "native" => crate::ui::native::devscript(ctx, arg),
            "click" | "press" | "move" | "release" => {
                // `click:<x>,<y>[,right]` — synthetic egui pointer click (points), spread
                // over three frames (move, press, release) like a real one.
                let parts: Vec<&str> = arg.split(',').map(str::trim).collect();
                let x = parts.first().and_then(|v| v.parse::<f32>().ok());
                let y = parts.get(1).and_then(|v| v.parse::<f32>().ok());
                if let (Some(x), Some(y)) = (x, y) {
                    let pos = egui::pos2(x, y);
                    let right = parts.get(2) == Some(&"right");
                    let button = if right { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                    let ev =
                        |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE };
                    match verb {
                        "click" => {
                            app.gui.devscript.raw.push(egui::Event::PointerMoved(pos));
                            app.gui.devscript.steps.push_front(format!("release:{arg}"));
                            app.gui.devscript.steps.push_front(format!("press:{arg}"));
                        }
                        "press" => app.gui.devscript.raw.push(ev(true)),
                        "move" => app.gui.devscript.raw.push(egui::Event::PointerMoved(pos)),
                        _ => app.gui.devscript.raw.push(ev(false)),
                    }
                }
                ctx.request_repaint();
                return;
            }
            _ => match step.parse::<Action>() {
                Ok(a) => app.core.cmds.push(Cmd::Action(a)),
                Err(e) => log::warn!("SOYUZ_SCRIPT: {e}"),
            },
        }
        ctx.request_repaint();
    }
}

/// `panes:<label>` step: log every pane of the active tab of each window (location, listing state,
/// entry / visible counts and the visible names) for scripted refresh checks.
fn log_panes(app: &SoyuzApp, label: &str) {
    let secondary = app.core.workspaces.windows.iter().map(|w| w.ws.tab());
    for (i, p) in std::iter::once(app.core.ws.tab()).chain(secondary).flat_map(|t| t.panes.iter()).enumerate() {
        let names: Vec<&str> = p.visible.iter().filter_map(|&j| p.entries.get(j)).map(|e| e.name.as_str()).collect();
        log::info!(
            "SOYUZ_SCRIPT: panes {label} #{i} {} status={:?} refreshing={} entries={} visible={} [{}]",
            p.location.display(),
            p.status,
            p.refreshing,
            p.entries.len(),
            p.visible.len(),
            names.join(", ")
        );
    }
}

/// `toast:` step → a toast that stays for 30 s.
fn script_toast(arg: &str) -> crate::models::toasts::Toast {
    use crate::models::toasts::{Toast, ToastKind};
    let mut parts = arg.split('|');
    let kind = match parts.next().unwrap_or("").trim() {
        "success" => ToastKind::Success,
        "warning" => ToastKind::Warning,
        "error" => ToastKind::Error,
        _ => ToastKind::Info,
    };
    let mut t = Toast::new(kind, parts.next().unwrap_or("Toast").trim());
    t.duration = Some(30.0);
    if let Some(b) = parts.next().map(str::trim).filter(|b| !b.is_empty()) {
        t = t.body(b.replace("\\n", "\n"));
    }
    if parts.next().is_some_and(|a| a.trim() == "undo") {
        t = t.action(tr!("common-undo"), Action::Undo);
    }
    t
}

/// One `expect:` check; `Err(actual value)` when it does not hold.
fn expect(app: &SoyuzApp, check: &str) -> Result<(), String> {
    let (what, want) = check.split_once('=').unwrap_or((check, ""));
    let open = |b: bool| if b { "open" } else { "closed" }.to_string();
    let actual = match what.trim() {
        "cwd" => {
            let loc = app.core.ws.pane().location.display();
            let name = std::path::Path::new(&loc).file_name().map(|n| n.to_string_lossy().into_owned());
            name.unwrap_or(loc)
        }
        "cursor" => {
            let pane = app.core.ws.pane();
            pane.cursor_pos().and_then(|p| pane.entry_at(p)).map(|e| e.name.clone()).unwrap_or_default()
        }
        "settings" => open(app.gui.settings.is_open()),
        "palette" => open(app.core.palette.open),
        "dialog" => open(app.dialog_open()),
        // Debug hook only: a one-off metadata probe on the UI thread is fine here.
        "exists" => {
            if std::path::Path::new(want).exists() {
                want.to_string()
            } else {
                "absent".into()
            }
        }
        "missing" => {
            if std::path::Path::new(want).exists() {
                "present".into()
            } else {
                want.to_string()
            }
        }
        "lang" => crate::i18n::current().tag().to_string(),
        // `has=<name>`: the focused pane shows an entry with that name.
        "has" => {
            let pane = app.core.ws.pane();
            if pane.visible_pos(want).is_some() { want.to_string() } else { "absent".into() }
        }
        other => return Err(format!("unknown check {other}")),
    };
    if actual == want { Ok(()) } else { Err(actual) }
}

/// `config` with one dotted key set to a TOML value.
fn edited(config: &Config, assignment: &str) -> Result<Config, String> {
    let (key, value) = assignment.split_once('=').ok_or("expected key=value")?;
    let mut root: toml::Table =
        toml::from_str(&soyuz_core::config::to_pretty_string(config).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let value: toml::Value = toml::from_str::<toml::Table>(&format!("v = {value}"))
        .map_err(|e| e.to_string())?
        .remove("v")
        .ok_or("no value")?;
    let parts: Vec<&str> = key.trim().split('.').collect();
    let mut t = &mut root;
    for p in &parts[..parts.len() - 1] {
        t = t.get_mut(*p).and_then(toml::Value::as_table_mut).ok_or_else(|| format!("no table {p}"))?;
    }
    t.insert(parts[parts.len() - 1].to_string(), value);
    soyuz_core::config::parse_str(&toml::to_string(&root).map_err(|e| e.to_string())?)
        .map(|l| l.config)
        .map_err(|e| e.to_string())
}

pub(crate) fn save_png(path: &std::path::Path, img: &egui::ColorImage) {
    let [w, h] = img.size;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    match image::save_buffer(path, &bytes, w as u32, h as u32, image::ColorType::Rgba8) {
        Ok(()) => log::info!("SOYUZ_SCRIPT: saved {}", path.display()),
        Err(e) => log::warn!("SOYUZ_SCRIPT: cannot save {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_config_edit() {
        let c = edited(&Config::default(), "appearance.density=\"touch\"").unwrap();
        assert_eq!(c.appearance.density, soyuz_core::config::Density::Touch);
        let c = edited(&c, "appearance.high_contrast=true").unwrap();
        assert!(c.appearance.high_contrast);
        assert!(edited(&c, "nope.x=1").is_err());
    }
}
