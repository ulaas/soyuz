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
//!   `lang=<tag>` (used by the keyboard-only test, `tests/keyboard_only.rs`);
//! * `mark:<label>` — log a marker line (`SOYUZ_SCRIPT: mark <label>`), e.g. where the
//!   frame-budget test starts counting;
//! * `settings:<section>` — open the settings window on a section (`settings:appearance`;
//!   `settings:close` closes it);
//! * `smart:<name>` — open a smart folder (`smart:` alone: the new smart folder dialog);
//! * `keys:<spec>` — synthetic keyboard input on the next frame: plain text is typed,
//!   `{Enter}`, `{Ctrl+S}`, `{Shift+F3}`, `{Copy}`, `{Cut}`, `{Paste:text}` are keys;
//! * `edit:<path>` — open a file in the built-in editor; `editor:<command>` — drive it
//!   (see `editors::devscript`);
//! * `ai:<prompt>` — send a prompt to the assistant; `ai:!approve` approves every
//!   pending plan, `ai:!smart <text>` opens smart search with `text`, `ai:!save <name>`
//!   saves the last task as a recipe, `ai:!tab chat|recipes|audit` switches the panel page,
//!   `ai:!mcp-approve` approves every pending MCP request card;
//! * `ws:<verb> [arg]` — workspaces (F7): `group`, `add <tab>`, `collapse`, `color <0-7>`,
//!   `save <name>`, `open <n>`, `layout single|dual|stacked|quad`, `sync`, `tear <tab>`,
//!   `move <tab> <window>`, `close <window>`, `merge`, `select <tab>`;
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

/// Remember when `main` started (startup-time measurement, R5).
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
    if !app.devscript.events.is_empty() {
        let events = std::mem::take(&mut app.devscript.events);
        ctx.input_mut(|i| i.events.extend(events));
    }
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
    if app.devscript.frame_log {
        app.devscript.frame_start = Some(Instant::now());
    }
}

/// Run due steps. Called once per frame.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    if app.devscript.frame_log {
        app.devscript.frames += 1;
        let ms = app.devscript.frame_start.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
        log::info!("frame {} ui {ms:.2} ms", app.devscript.frames);
    }
    if let Some(path) = app.devscript.pending_shot.clone() {
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        match image {
            Some(img) => {
                save_png(&path, &img);
                app.devscript.pending_shot = None;
            }
            None => {
                ctx.request_repaint();
                return;
            }
        }
    }
    loop {
        if let Some(t) = app.devscript.resume_at {
            let now = Instant::now();
            if now < t {
                ctx.request_repaint_after(t - now);
                return;
            }
            app.devscript.resume_at = None;
        }
        let Some(step) = app.devscript.steps.pop_front() else { return };
        log::info!("SOYUZ_SCRIPT: {step}");
        let (verb, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
        match verb {
            "wait" => {
                let ms = arg.trim().parse::<u64>().unwrap_or(500);
                app.devscript.resume_at = Some(Instant::now() + Duration::from_millis(ms));
            }
            "shot" => {
                app.devscript.pending_shot = Some(PathBuf::from(arg.trim()));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                ctx.request_repaint();
                return;
            }
            "cd" => {
                let pane = app.ws.pane().id;
                app.cmds.push(Cmd::Navigate { pane, location: Location::Dir(PathBuf::from(arg.trim())), select: None });
            }
            "select" => {
                let pane = app.ws.pane_mut();
                if let Some(pos) = pane.visible_pos(arg.trim()) {
                    pane.move_cursor(pos, false, false);
                }
            }
            "theme" => {
                app.ui.theme = crate::theme::preference_from_config(arg);
                ctx.set_theme(app.ui.theme);
            }
            "config" => match edited(&app.config, arg) {
                Ok(c) => crate::services::config_watch::apply_config(app, ctx, c),
                Err(e) => log::warn!("SOYUZ_SCRIPT {step}: {e}"),
            },
            "size" => {
                if let Some((w, h)) = arg.split_once('x')
                    && let (Ok(w), Ok(h)) = (w.trim().parse::<f32>(), h.trim().parse::<f32>())
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                }
            }
            "welcome" => app.ui.welcome = arg.trim() != "off",
            "mark" => log::info!("SOYUZ_SCRIPT: mark {}", arg.trim()),
            "expect" => match expect(app, arg.trim()) {
                Ok(()) => log::info!("SOYUZ_SCRIPT: expect ok {}", arg.trim()),
                Err(actual) => log::warn!("SOYUZ_SCRIPT: expect FAILED {} (actual {actual})", arg.trim()),
            },
            "scroll" => {
                // Absolute scroll offset of the focused pane (perf probes).
                let y = arg.trim().parse::<f32>().unwrap_or(0.0);
                app.ws.pane_mut().restore_scroll = Some(y);
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
                app.workspaces.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            "ws" => crate::ui::workspaces::devscript(app, ctx, arg),
            "smart" => {
                // `smart:<name>` opens a smart folder (F5); `smart:` opens the "new" dialog.
                if arg.trim().is_empty() {
                    app.cmds.push(Cmd::Smart(crate::ui::smart::SmartCmd::Edit(None)));
                } else if let Some(index) = crate::ui::smart::folders().iter().position(|f| f.name == arg.trim()) {
                    app.cmds.push(Cmd::Smart(crate::ui::smart::SmartCmd::Open { index, new_tab: false }));
                }
            }
            "edit" | "editor" => crate::editors::devscript(app, ctx, verb, arg),
            "keys" => {
                app.devscript.events.extend(key_events(arg));
                ctx.request_repaint();
                return;
            }
            "ai" => crate::ai::state::script(app, ctx, arg),
            _ => match step.parse::<Action>() {
                Ok(a) => app.cmds.push(Cmd::Action(a)),
                Err(e) => log::warn!("SOYUZ_SCRIPT: {e}"),
            },
        }
        ctx.request_repaint();
    }
}

/// One `expect:` check; `Err(actual value)` when it does not hold.
fn expect(app: &SoyuzApp, check: &str) -> Result<(), String> {
    let (what, want) = check.split_once('=').unwrap_or((check, ""));
    let open = |b: bool| if b { "open" } else { "closed" }.to_string();
    let actual = match what.trim() {
        "cwd" => {
            let loc = app.ws.pane().location.display();
            let name = std::path::Path::new(&loc).file_name().map(|n| n.to_string_lossy().into_owned());
            name.unwrap_or(loc)
        }
        "cursor" => {
            let pane = app.ws.pane();
            pane.cursor_pos().and_then(|p| pane.entry_at(p)).map(|e| e.name.clone()).unwrap_or_default()
        }
        "settings" => open(app.settings.is_open()),
        "palette" => open(app.palette.open),
        "dialog" => open(app.ui.dialogs.is_open()),
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

fn save_png(path: &std::path::Path, img: &egui::ColorImage) {
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
