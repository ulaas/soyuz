//! Debug / automation hook (like `SOYUZ_THROTTLE_MBPS` and `SOYUZ_OPEN_TOOL`):
//! `SOYUZ_SCRIPT` runs a list of steps at startup, separated by `;`, and can save
//! screenshots of the window. Used for visual regression passes and docs screenshots.
//!
//! The core runs the script (`soyuz_appcore::devscript::Runner`, which documents the core
//! steps: `wait`, `until`, `wait_for`, `cd`, `select`, `theme`, `config`, `scroll`,
//! `welcome`, `expect`, `toast`, `panes`, `mark`, `smart`, `edit` / `editor`, `ai`, `ws`,
//! `quit`, action names). This module carries out the GUI's steps:
//! * `shot:<file.png>` — screenshot of this window only;
//! * `size:<w>x<h>` — resize the window; `appearance` (Settings → Appearance);
//! * `keys:<spec>` — synthetic keyboard input on the next frame: plain text is typed,
//!   `{Enter}`, `{Ctrl+S}`, `{Shift+F3}`, `{Copy}`, `{Cut}`, `{Paste:text}` are keys
//!   (used by the keyboard-only test, `tests/keyboard_only.rs`);
//! * `hover:<x>,<y>` — move egui's pointer to that window position (no OS input);
//! * `settings:<section>` — open the settings window on a section (`settings:appearance`;
//!   `settings:close` closes it);
//! * `dropdown:<key>[=query]` — open the dropdown with that key (settings rows use
//!   `settings.<id>`, e.g. `dropdown:settings.font.ui=segoe`) with `query` in its search;
//! * `export_picker` — the settings export picker;
//! * `click:<x>,<y>[,right]` — synthetic egui click at a point of the main window;
//!   `press:<x>,<y>` / `move:<x>,<y>` / `release:<x>,<y>` — its parts, chained for a
//!   synthetic drag (`press:300,140;move:320,150;move:900,400;release:900,400`);
//! * `native:list` / `native:move <x>,<y>` / `native:moveby <dx>,<dy>` / `native:hover <x>,<y>[,<n>]` — native dialog / popup
//!   windows (`ui::native`): log their handles and rects, move the dialog (physical pixels);
//!   `keys:` goes to the newest native window while one is open.
//!
//! `SOYUZ_WINDOW_POS=x,y` (read in `main.rs`) opens the window at that position without
//! activating it, e.g. `-4000,-4000` keeps scripted screenshot sessions off-screen.
//!
//! `SOYUZ_FRAME_LOG=1` logs a line per rendered frame with the time spent in `ui()`
//! (to check that an idle window does not repaint and that big folders stay fast).

use std::path::PathBuf;
use std::time::Instant;

use soyuz_appcore::devscript::{FrontStep, KeyTok, Runner, ScriptFacts};
use soyuz_core::config::Mods;

use crate::app::SoyuzApp;

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

/// Where the screenshot callback leaves the image for [`pump`].
type ShotSlot = std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>;

#[derive(Default)]
pub struct DevScript {
    /// The script (`None` without `SOYUZ_SCRIPT`); taken out of the app while it runs.
    runner: Option<Runner>,
    /// The `shot:` being taken: where it goes, and the image once egui hands it over
    /// (`Context::request_screenshot` calls back after the frame is read).
    pending_shot: Option<(PathBuf, ShotSlot)>,
    frame_log: bool,
    frames: u64,
    frame_start: Option<Instant>,
    /// Synthetic input for the next frame (`keys:`).
    events: Vec<egui::Event>,
    /// Synthetic pointer position (`hover:`), fed through [`raw_input_hook`].
    pointer: Option<egui::Pos2>,
    pointer_sent: Option<egui::Pos2>,
    /// Synthetic pointer input, fed before the next frame (`click:`).
    raw: Vec<egui::Event>,
}

impl DevScript {
    pub fn from_env() -> Self {
        DevScript {
            runner: Runner::from_env(),
            frame_log: std::env::var("SOYUZ_FRAME_LOG").is_ok_and(|v| v == "1"),
            ..Default::default()
        }
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

/// `keys:` tokens → egui events (a chord is a press and a release; Ctrl also sets
/// `command`, like egui-winit off macOS).
fn key_events(keys: &[KeyTok]) -> Vec<egui::Event> {
    let mut out = Vec::new();
    for k in keys {
        match k {
            KeyTok::Text(c) => out.push(egui::Event::Text(c.to_string())),
            KeyTok::Copy => out.push(egui::Event::Copy),
            KeyTok::Cut => out.push(egui::Event::Cut),
            KeyTok::Paste(t) => out.push(egui::Event::Paste(t.clone())),
            KeyTok::Chord(chord) => {
                let Some(key) =
                    egui::Key::ALL.iter().copied().find(|&k| crate::ui::keys::key_name(k) == Some(chord.key))
                else {
                    continue;
                };
                let mut m = egui::Modifiers::NONE;
                if chord.mods.contains(Mods::CTRL) {
                    m.ctrl = true;
                    m.command = true;
                }
                if chord.mods.contains(Mods::CMD) {
                    m.mac_cmd = true;
                    m.command = true;
                }
                m.shift = chord.mods.contains(Mods::SHIFT);
                m.alt = chord.mods.contains(Mods::ALT);
                out.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
                out.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m });
            }
        }
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

/// What the script's checks need from the GUI.
fn facts(app: &SoyuzApp) -> ScriptFacts {
    ScriptFacts {
        dialog_open: app.dialog_open(),
        settings_open: app.gui.settings.is_open(),
        focus: None,
        screen_text: None,
    }
}

/// Run due steps. Called once per frame.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    if app.gui.devscript.frame_log {
        app.gui.devscript.frames += 1;
        let ms = app.gui.devscript.frame_start.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
        log::info!("frame {} ui {ms:.2} ms", app.gui.devscript.frames);
    }
    if let Some((path, slot)) = &app.gui.devscript.pending_shot {
        let image = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        match image {
            Some(img) => {
                save_png(path, &img);
                app.gui.devscript.pending_shot = None;
            }
            None => {
                ctx.request_repaint();
                return;
            }
        }
    }
    let Some(mut runner) = app.gui.devscript.runner.take() else { return };
    let host = crate::host::EguiHost::new(ctx);
    // Facts again after every frontend step: it may have opened settings or a dialog.
    loop {
        let facts = facts(app);
        let Some(step) = runner.pump(&mut app.core, &host, &facts) else { break };
        let yields = step.yields();
        handle(app, &mut runner, ctx, step);
        ctx.request_repaint();
        if yields {
            break;
        }
    }
    app.gui.devscript.runner = Some(runner);
}

/// One GUI step.
fn handle(app: &mut SoyuzApp, runner: &mut Runner, ctx: &egui::Context, step: FrontStep) {
    match step {
        FrontStep::Shot(path) => {
            let slot = ShotSlot::default();
            let (out, wake) = (slot.clone(), ctx.clone());
            ctx.request_screenshot(move |img| {
                *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(img);
                wake.request_repaint();
            });
            app.gui.devscript.pending_shot = Some((path, slot));
        }
        FrontStep::Keys(keys) => {
            // An open native dialog / popup window has the keyboard.
            let evs = key_events(&keys);
            if !crate::ui::native::inject(&evs) {
                app.gui.devscript.events.extend(evs);
            }
        }
        FrontStep::Size { w, h } => ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h))),
        FrontStep::Other { verb, arg } => match verb.as_str() {
            "hover" => {
                // `hover:<x>,<y>`: synthetic pointer position (logical points), in-app only.
                if let Some((x, y)) = arg.split_once(',')
                    && let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>())
                {
                    app.gui.devscript.pointer = Some(egui::pos2(x, y));
                }
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
                    return;
                }
                let sec = crate::ui::settings::Section::ALL
                    .into_iter()
                    .find(|s| s.english_title().to_ascii_lowercase().starts_with(&want));
                crate::ui::settings::open(app, sec);
            }
            "appearance" => crate::ui::settings::open(app, Some(crate::ui::settings::Section::Appearance)),
            "dropdown" => {
                // `dropdown:<key>[=query]`: open a dropdown (and type into its search).
                let (key, query) = arg.split_once('=').unwrap_or((&arg, ""));
                crate::ui::widgets::dropdown::request_open(ctx, key.trim(), query);
            }
            "native" => crate::ui::native::devscript(ctx, &arg),
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
                    match verb.as_str() {
                        "click" => {
                            app.gui.devscript.raw.push(egui::Event::PointerMoved(pos));
                            runner.push_front(format!("release:{arg}"));
                            runner.push_front(format!("press:{arg}"));
                        }
                        "press" => app.gui.devscript.raw.push(ev(true)),
                        "move" => app.gui.devscript.raw.push(egui::Event::PointerMoved(pos)),
                        _ => app.gui.devscript.raw.push(ev(false)),
                    }
                }
            }
            other => log::warn!("SOYUZ_SCRIPT: {other} is not a GUI step"),
        },
    }
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
    use soyuz_appcore::devscript::parse_keys;

    /// The `keys:` conversion before Phase T8 (spec → egui events in one pass), kept as
    /// the oracle for [`key_events`] over [`parse_keys`].
    fn old_key_events(spec: &str) -> Vec<egui::Event> {
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
                        out.push(egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: m,
                        });
                        out.push(egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: m,
                        });
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

    #[test]
    fn key_events_match_the_old_conversion() {
        let corpus = [
            // Every spec of `tests/keyboard_only.rs`.
            "{Enter}",
            "{Backspace}",
            "{Ctrl+D}",
            "{F2}",
            "renamed{Enter}",
            "{Shift+Delete}",
            "{Ctrl+Comma}",
            "{Escape}",
            "{Ctrl+Shift+P}",
            "new folder",
            "yeni klas",
            // Clipboard events, Unicode text, unknown keys, odd braces.
            "{Copy}{Cut}{Paste:x}",
            "{Paste:a b}",
            "ğüşİı日本語 é",
            "{Nope}",
            "{Ctrl+Nope}",
            "{}",
            "{a",
            "x}y",
            "{Alt+F4}{Shift+Tab}{Ctrl+,}{Ctrl+Alt+Shift+Z}",
            "{a}{Space}{Home}{End}{PageUp}{PageDown}{Up}{Down}{Left}{Right}{Insert}{Tab}",
            "{Ctrl+1}{F12}{Minus}{Plus}{Slash}{Backslash}{Semicolon}{Quote}{Backtick}",
        ];
        for spec in corpus {
            assert_eq!(key_events(&parse_keys(spec)), old_key_events(spec), "keys:{spec}");
        }
        // Ctrl sets `command` too.
        match key_events(&parse_keys("{Ctrl+S}")).first() {
            Some(egui::Event::Key { modifiers, .. }) => assert!(modifiers.ctrl && modifiers.command),
            other => panic!("not a key: {other:?}"),
        }
    }
}
