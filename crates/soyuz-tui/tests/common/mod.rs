//! Shared harness of the headless TUI tests: the relaunch into a portable sandbox, a
//! scripted event source, and helpers to read the app's state and screen.
//!
//! The core keeps its settings and session in the config directory, which follows the
//! executable in portable mode. So each test re-runs itself ([`relaunch`]) from a copy of
//! the test binary placed in a temporary directory next to a `portable` marker: nothing
//! outside that directory is read or written, and raw mode is never entered.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::core::Startup;
use soyuz_core::config::{self, Config};
use soyuz_tui::app::{App, EventSource, LoopEnd, NullIo, TuiEvent, run_loop};

/// Set in the re-run child: the folder the test works in.
pub const CHILD: &str = "SOYUZ_HEADLESS_TUI_DATA";
/// Printed by the child when its checks passed ([`ok`]).
pub const OK: &str = "headless tui: ok";

/// Which side of the relaunch this is.
pub enum Run {
    /// In the sandboxed child: run the test on this data folder, then call [`ok`].
    Child(PathBuf),
    /// In the parent, after the child succeeded: check what it left on disk.
    Parent(Relaunched),
}

/// The finished child run.
pub struct Relaunched {
    /// The sandbox (deleted on drop).
    pub tmp: tempfile::TempDir,
    /// The data folder the child worked in (`<sandbox>/data`).
    pub data: PathBuf,
    /// The child's stdout + stderr.
    pub log: String,
}

impl Relaunched {
    /// The child's config directory (portable mode).
    pub fn config_dir(&self) -> PathBuf {
        self.tmp.path().join(config::PORTABLE_DATA_DIR)
    }
}

/// Run test `name` (in module `module`, pass `module_path!()`) in a sandboxed copy of
/// this test binary. In the parent: creates `<sandbox>/data`, lets `setup` fill it, runs
/// the child and asserts it passed. In the child: checks the sandbox and returns the data
/// folder.
pub fn relaunch(module: &str, name: &str, setup: impl FnOnce(&Path)) -> Run {
    if let Some(data) = std::env::var_os(CHILD) {
        let data = PathBuf::from(data);
        check_portable(&data);
        return Run::Child(data);
    }
    // libtest names a test by its path without the crate.
    let path = match module.split_once("::") {
        Some((_, rest)) => format!("{rest}::{name}"),
        None => name.to_string(),
    };
    let tmp = tempfile::tempdir().expect("temp dir");
    let exe = std::env::current_exe().expect("test binary");
    let copy = tmp.path().join(exe.file_name().expect("exe name"));
    std::fs::copy(&exe, &copy).expect("copy the test binary");
    std::fs::write(tmp.path().join(config::PORTABLE_MARKER), b"").expect("portable marker");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).expect("data folder");
    setup(&data);
    let out = Command::new(&copy)
        .args(["--exact", &path, "--nocapture", "--test-threads=1"])
        .env(CHILD, &data)
        .output()
        .expect("run the copied test binary");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "headless TUI run of {path} failed:\n{log}");
    assert!(log.contains(OK), "the child did not run {path}:\n{log}");
    Run::Parent(Relaunched { tmp, data, log })
}

/// The child passed its checks.
pub fn ok() {
    println!("{OK}");
}

/// Never fall back to the real config directory.
fn check_portable(data: &Path) {
    let dir = config::config_dir();
    let root = data.parent().expect("temp root");
    assert!(dir.ends_with(config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());
    assert_eq!(
        dir.parent().and_then(|d| d.canonicalize().ok()),
        root.canonicalize().ok(),
        "config dir {} is outside the temp root",
        dir.display()
    );
}

/// Settings for tests: nothing that reaches the network, the index or the CLI server.
pub fn test_config() -> Config {
    let mut c = Config::default();
    c.general.show_hidden = false;
    c.general.confirm_delete = true;
    c.updates.weekly_check = false;
    c.cli.server_enabled = false;
    c.search.index.enabled = false;
    // Fixed screen coordinates in the tests assume no sidebar; the sidebar tests turn it on.
    c.layout.sidebar = false;
    // Screen text in the tests is ASCII (`/` folders, `>` separators, `v` / `>` chevrons);
    // `headless::theme` covers the Unicode default.
    c.tui.icons = soyuz_core::config::TuiIcons::Ascii;
    c
}

/// A terminal UI opened on `dir`.
pub fn new_app(c: Config, dir: &Path) -> App {
    let startup = Startup { open: Some(dir.to_path_buf()), ..Default::default() };
    let app = App::new(c, startup);
    assert_eq!(app.core.frontend_kind, soyuz_appcore::core::FrontendKind::Tui);
    app
}

pub type Check = Box<dyn Fn(&App) -> bool>;

/// Wait until `when` holds, then send `send`.
pub struct Step {
    pub what: &'static str,
    pub when: Check,
    pub send: Event,
}

impl Step {
    pub fn new(what: &'static str, when: impl Fn(&App) -> bool + 'static, send: Event) -> Step {
        Step { what, when: Box::new(when), send }
    }
}

/// An event source that sends each step's event once its condition holds (30 s timeout
/// per step; after the last step the app must quit within 60 s).
pub struct Script {
    pub steps: VecDeque<Step>,
    pub since: Instant,
}

impl Script {
    pub fn new(steps: Vec<Step>) -> Script {
        Script { steps: steps.into(), since: Instant::now() }
    }
}

impl EventSource for Script {
    fn next(&mut self, app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent> {
        let wait = timeout.min(Duration::from_millis(10));
        let Some(step) = self.steps.front() else {
            // The last step was sent (usually a quit): the loop must end by itself.
            assert!(self.since.elapsed() < Duration::from_secs(60), "the app did not quit after the last step");
            let _ = redraw.recv_timeout(wait);
            return Vec::new();
        };
        if (step.when)(app) {
            let step = self.steps.pop_front().expect("step");
            self.since = Instant::now();
            return vec![TuiEvent::Term(step.send)];
        }
        assert!(self.since.elapsed() < Duration::from_secs(30), "timed out waiting for {}", step.what);
        let _ = redraw.recv_timeout(wait);
        Vec::new()
    }
}

/// Run the frame loop on a `w` × `h` test terminal until the script quits the app (its
/// last step should send Ctrl+Q). Asserts every step ran.
pub fn run_script(app: &mut App, steps: Vec<Step>, w: u16, h: u16) -> (Terminal<TestBackend>, NullIo) {
    let mut source = Script::new(steps);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    let mut io = NullIo::default();
    let end = run_loop(app, &mut terminal, &mut source, &mut io).expect("the loop runs");
    assert_eq!(end, LoopEnd::Quit);
    assert!(source.steps.is_empty(), "every step ran");
    (terminal, io)
}

pub fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

pub fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// A key with modifiers.
pub fn key_mods(code: KeyCode, mods: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, mods))
}

pub fn alt(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT))
}

pub fn shift(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::SHIFT))
}

/// Steps that type `text`, one character per frame (no condition).
pub fn type_text(steps: &mut Vec<Step>, text: &str) {
    for c in text.chars() {
        steps.push(Step::new("typing", |_| true, key(KeyCode::Char(c))));
    }
}

/// The title of the dialog drawn in the last frame (either stack), if one is open.
pub fn dialog_title(app: &App) -> Option<String> {
    app.view()?.dialog.as_ref().map(|d| d.title.clone())
}

/// No job is running or waiting (finished ones may still be listed).
pub fn wait_jobs_idle(app: &App) -> bool {
    app.core.fileops.jobs.pending().next().is_none()
}

/// The texts of the toasts drawn in the last frame (titles and bodies).
pub fn toasts(app: &App) -> Vec<String> {
    app.view()
        .map(|v| v.toasts.lines.iter().flat_map(|t| std::iter::once(t.title.clone()).chain(t.body.clone())).collect())
        .unwrap_or_default()
}

/// Labels of the focused pane's rows in the last frame.
pub fn labels(app: &App) -> Vec<String> {
    app.view().and_then(|v| v.focused_pane()).map(|p| p.rows().map(|r| r.label()).collect()).unwrap_or_default()
}

/// Label of the focused pane's cursor row in the last frame.
pub fn cursor(app: &App) -> Option<String> {
    app.view()?.focused_pane()?.rows().find(|r| r.cursor).map(|r| r.label())
}

/// The focused pane is at `dir`.
pub fn at(app: &App, dir: &Path) -> bool {
    app.core.ws.pane().location == soyuz_core::Location::Dir(dir.to_path_buf())
}

/// The last frame as text, one line per row.
pub fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let area = buf.area;
    (0..area.height)
        .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n")
        .collect()
}
