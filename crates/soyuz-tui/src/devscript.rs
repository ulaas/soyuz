//! `SOYUZ_SCRIPT` in the terminal UI (Phase T8): the core runs the script
//! (`soyuz_appcore::devscript::Runner`, `App::script`); this module feeds it the
//! frontend's facts ([`facts`]) and carries out the frontend steps:
//! - `keys:<spec>`: crossterm events for the next frame (`crate::keys::to_crossterm`;
//!   `{Paste:x}` → `Event::Paste`);
//! - `shot:<file>`: the next frame drawn (`App::last_buffer`) saved by `crate::shot`, the
//!   format by extension (`.txt`, `.snap`, `.svg`);
//! - `size:<w>x<h>`: resizes the screen in headless mode ([`HeadlessBackend`]); a real
//!   terminal ignores it with a warning;
//! - the GUI's pointer, window and native steps are skipped with a warning (once per verb).
//!
//! [`pump`] runs once per loop iteration, after the frame (`app::run_loop`). The script
//! comes from `SOYUZ_SCRIPT` in `crate::run` / [`run_headless`] only, or from
//! `App::with_script` in tests.
//!
//! ## Headless mode
//!
//! `SOYUZ_TUI_HEADLESS=<w>x<h>` (a debug hook, like the GUI's `SOYUZ_WINDOW_POS`) runs the
//! session without a terminal ([`run_headless`], from `crate::launch`): the frame loop on
//! an in-memory screen of that size, driven by the script alone, for screenshots and the
//! smoke test of the shipped binary. It touches no console or terminal at all (no console
//! window, no keyboard protocol or graphics query, no raw mode, no input reader, no
//! signal handlers, no panic hook that restores a terminal), pins the key map overlay to
//! the defaults (`WT_SESSION` is ignored), the colours to truecolor (unless
//! `[tui] color_mode` names a mode) and `[tui] images` to off, and quits once the script
//! is done (`quit`, or no steps left).

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Size};
use soyuz_appcore::core::Startup;
use soyuz_appcore::devscript::{FrontStep, KeyTok, Runner, ScriptFacts};
use soyuz_appcore::host::{Host as _, WindowCmd, WindowId};
use soyuz_core::config::{Config, TuiColorMode, TuiImages};

use crate::app::{App, EventSource, LoopEnd, NullIo, TuiEvent, run_loop};
use crate::input::TuiFocus;

/// What the script's checks need from the terminal UI (`dialog=`, `settings=`, `focus=`,
/// `screen~`).
pub fn facts(app: &App) -> ScriptFacts {
    let focus = app.focus();
    ScriptFacts {
        dialog_open: app.view().is_some_and(|v| v.dialog.is_some()),
        settings_open: app.view().is_some_and(|v| v.settings.visible),
        focus: Some(focus_name(focus)),
        screen_text: app.last_buffer().map(|b| crate::snapshot::lines(b).join("\n")),
    }
}

/// `TuiFocus` by name, as `expect:focus=<name>` writes it (`List`, `Dialog`, …).
pub fn focus_name(focus: TuiFocus) -> String {
    format!("{focus:?}")
}

/// Run the script's due steps; synthetic input goes to `events` (the next frame's).
pub fn pump(app: &mut App, events: &mut Vec<Event>) {
    // A `shot:` waits for the frame after it: that frame was just drawn.
    if let Some(path) = app.pending_shot.take() {
        save_shot(app, &path);
    }
    let Some(mut runner) = app.script.take() else {
        end_headless(app, true);
        return;
    };
    loop {
        // Fresh facts for every call: a step may have changed what they report.
        let facts = facts(app);
        let Some(step) = runner.pump(&mut app.core, &app.host, &facts) else { break };
        let yields = step.yields();
        handle(app, step, events);
        if yields {
            break;
        }
    }
    let done = runner.is_done();
    app.script = Some(runner);
    end_headless(app, done);
}

/// One frontend step.
fn handle(app: &mut App, step: FrontStep, events: &mut Vec<Event>) {
    match step {
        FrontStep::Keys(keys) => {
            events.extend(key_events(&keys));
            app.host.redraw();
        }
        FrontStep::Shot(path) => {
            app.pending_shot = Some(path);
            app.host.redraw();
        }
        FrontStep::Size { w, h } => {
            let size =
                (w.round().clamp(1.0, f32::from(u16::MAX)) as u16, h.round().clamp(1.0, f32::from(u16::MAX)) as u16);
            if HEADLESS.load(Ordering::Relaxed) {
                *lock(&PENDING_SIZE) = Some(size);
                app.host.redraw();
            } else {
                warn_once(
                    "size",
                    "SOYUZ_SCRIPT: size needs headless mode (SOYUZ_TUI_HEADLESS); a real terminal keeps its size",
                );
            }
        }
        FrontStep::Other { verb, .. } => {
            let msg = format!("SOYUZ_SCRIPT: {verb} is not a terminal UI step (GUI only)");
            warn_once(&verb, &msg);
        }
    }
}

/// `keys:` tokens → crossterm events.
pub fn key_events(keys: &[KeyTok]) -> Vec<Event> {
    keys.iter().flat_map(crate::keys::to_crossterm).collect()
}

/// Save the last frame drawn to `path` (its folder is created).
fn save_shot(app: &App, path: &Path) {
    let Some(buf) = app.last_buffer() else {
        log::warn!("SOYUZ_SCRIPT: cannot save {}: no frame drawn", path.display());
        return;
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    match crate::shot::write(path, buf) {
        Ok(()) => log::info!("SOYUZ_SCRIPT: saved {}", path.display()),
        Err(e) => log::warn!("SOYUZ_SCRIPT: cannot save {}: {e}", path.display()),
    }
}

/// Verbs already warned about (each warns once per run).
static WARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn warn_once(key: &str, msg: &str) {
    let mut warned = lock(&WARNED);
    if !warned.iter().any(|k| k == key) {
        warned.push(key.to_string());
        log::warn!("{msg}");
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- Headless mode ----

/// The environment variable of headless mode: `<width>x<height>` in cells.
pub const HEADLESS_ENV: &str = "SOYUZ_TUI_HEADLESS";
/// How long a headless run may go on after its script is done (a quit held up by a
/// prompt, e.g. "Quit while jobs run?") before it is ended from outside.
pub const HEADLESS_LINGER: Duration = Duration::from_secs(120);

/// A [`HeadlessBackend`] exists in this process (`size:` resizes it).
static HEADLESS: AtomicBool = AtomicBool::new(false);
/// The size a `size:` step asked for, applied by the [`HeadlessBackend`].
static PENDING_SIZE: Mutex<Option<(u16, u16)>> = Mutex::new(None);
/// The headless run asked to quit already.
static ENDED: AtomicBool = AtomicBool::new(false);

/// `<width>x<height>` (`100x30`) → the screen size of headless mode (each at least 1).
pub fn parse_size(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<u16>().ok()?, h.trim().parse::<u16>().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// The headless screen size from [`HEADLESS_ENV`] (`env` reads a variable), if it is set
/// and valid. An invalid value is ignored with a warning.
pub fn headless_size(env: impl Fn(&str) -> Option<OsString>) -> Option<(u16, u16)> {
    let v = env(HEADLESS_ENV)?;
    let size = v.to_str().and_then(parse_size);
    if size.is_none() {
        log::warn!("{HEADLESS_ENV}={v:?} is not <width>x<height>; starting normally");
    }
    size
}

/// Headless mode quits once the script is `done` (or there is none): a close request,
/// as the script's own `quit` makes. Waits for a pending `shot:` first.
fn end_headless(app: &mut App, done: bool) {
    if !done || app.pending_shot.is_some() || !HEADLESS.load(Ordering::Relaxed) || ENDED.swap(true, Ordering::Relaxed) {
        return;
    }
    if !app.core.workspaces.quitting {
        log::info!("SOYUZ_SCRIPT: done, quitting (headless)");
    }
    app.core.workspaces.quitting = true;
    app.host.window(WindowId::Root, WindowCmd::Close);
}

/// An in-memory screen (ratatui's `TestBackend`) whose size `size:` steps change.
pub struct HeadlessBackend {
    inner: TestBackend,
}

impl HeadlessBackend {
    /// A `w` × `h` screen; from now on `size:` steps resize it.
    pub fn new(w: u16, h: u16) -> HeadlessBackend {
        HEADLESS.store(true, Ordering::Relaxed);
        HeadlessBackend { inner: TestBackend::new(w, h) }
    }

    /// What is on the screen.
    pub fn buffer(&self) -> &Buffer {
        self.inner.buffer()
    }

    /// Apply a pending `size:` (ratatui asks [`Backend::size`], then clears and draws).
    fn apply(&mut self) {
        if let Some((w, h)) = lock(&PENDING_SIZE).take() {
            self.inner.resize(w, h);
        }
    }
}

impl Backend for HeadlessBackend {
    type Error = std::convert::Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.apply();
        self.inner.draw(content)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.apply();
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.apply();
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        match *lock(&PENDING_SIZE) {
            Some((w, h)) => Ok(Size::new(w, h)),
            None => self.inner.size(),
        }
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.apply();
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// The event source of headless mode: no input, only the core's wake-ups and the loop's
/// deadlines. Ends the run ([`TuiEvent::Terminate`], after saving) when the app is still
/// up [`HEADLESS_LINGER`] after its script was done.
#[derive(Default)]
pub struct HeadlessSource {
    done_since: Option<Instant>,
}

impl EventSource for HeadlessSource {
    fn next(&mut self, app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent> {
        if app.script().is_none_or(Runner::is_done) {
            if self.done_since.get_or_insert_with(Instant::now).elapsed() >= HEADLESS_LINGER {
                log::warn!("SOYUZ_SCRIPT: the headless run did not quit after its script; ending it");
                return vec![TuiEvent::Terminate];
            }
        } else {
            self.done_since = None;
        }
        let _ = redraw.recv_timeout(timeout);
        Vec::new()
    }
}

/// `SOYUZ_TUI_HEADLESS=<w>x<h>`: run the terminal UI for `startup` on a `w` × `h`
/// in-memory screen, driven by `SOYUZ_SCRIPT` (see the module docs), and return the exit
/// code. The caller set up logging and claimed the crash journal; nothing here touches a
/// console or terminal.
pub fn run_headless(mut config: Config, startup: Startup, screen_reader: bool, (w, h): (u16, u16)) -> ExitCode {
    soyuz_fs::register_ui_thread();
    // The same keys everywhere (`WT_SESSION` and the keyboard protocol are not asked).
    let overlay = crate::overlay::Overlay::Defaults;
    log::info!("tui: headless {w}x{h}, key map overlay {overlay:?}");
    soyuz_appcore::keys::set_keymap_overlay(overlay.function());
    // Nothing read from the environment: explicit colours, no images.
    if config.tui.color_mode == TuiColorMode::Auto {
        config.tui.color_mode = TuiColorMode::Truecolor;
    }
    config.tui.images = TuiImages::Off;
    let mut terminal = match Terminal::new(HeadlessBackend::new(w, h)) {
        Ok(t) => t,
        Err(e) => match e {},
    };
    let mut app = App::new(config, startup);
    app.sr_override = screen_reader;
    app.script = Runner::from_env();
    if app.script.is_none() {
        log::warn!("tui: headless mode without {}: nothing to do", soyuz_appcore::devscript::ENV);
    }
    let mut io_ = NullIo::default();
    let result = run_loop(&mut app, &mut terminal, &mut HeadlessSource::default(), &mut io_);
    if let Ok(LoopEnd::Quit) | Err(_) = result {
        app.finish();
    }
    let failed = app.script().map_or(0, |r| r.failures().len());
    drop(app);
    match result {
        Ok(_) if failed > 0 => {
            log::warn!("SOYUZ_SCRIPT: {failed} expectation(s) failed");
            ExitCode::FAILURE
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("tui: {e}");
            let _ = io::Write::write_all(&mut io::stderr(), format!("soyuz: {e}\n").as_bytes());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_sizes() {
        assert_eq!(parse_size("100x30"), Some((100, 30)));
        assert_eq!(parse_size(" 80 X 24 "), Some((80, 24)));
        assert_eq!(parse_size("0x24"), None);
        assert_eq!(parse_size("80"), None);
        assert_eq!(parse_size("ax24"), None);
        assert_eq!(parse_size("70000x24"), None);
        let env = |v: &'static str| move |k: &str| (k == HEADLESS_ENV).then(|| OsString::from(v));
        assert_eq!(headless_size(env("60x20")), Some((60, 20)));
        assert_eq!(headless_size(env("big")), None);
        assert_eq!(headless_size(|_| None), None);
    }
}
