//! The terminal UI end to end, without a terminal: the real frame loop (`run_loop`) on
//! ratatui's `TestBackend`, with a scripted event source that waits for the app to reach
//! a state before it sends the next key. Browses a temporary folder (down, up, into a
//! subfolder and back), opens the delete confirmation and cancels it, then quits with
//! Ctrl+Q and checks that the terminal UI's own session file was written.
//!
//! The core keeps its settings and session in the config directory, which follows the
//! executable in portable mode. So the test re-runs itself from a copy of this test
//! binary placed in a temporary directory next to a `portable` marker: nothing outside
//! that directory is read or written, and raw mode is never entered.

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

/// Set in the re-run child: the folder to browse.
const CHILD: &str = "SOYUZ_HEADLESS_TUI_DATA";
const TEST: &str = "tui_browses_confirms_and_quits";

#[test]
fn tui_browses_confirms_and_quits() {
    if let Some(data) = std::env::var_os(CHILD) {
        run_tui(PathBuf::from(data));
        return;
    }
    let tmp = tempfile::tempdir().expect("temp dir");
    let exe = std::env::current_exe().expect("test binary");
    let copy = tmp.path().join(exe.file_name().expect("exe name"));
    std::fs::copy(&exe, &copy).expect("copy the test binary");
    std::fs::write(tmp.path().join(config::PORTABLE_MARKER), b"").expect("portable marker");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(data.join("sub")).unwrap();
    std::fs::write(data.join("sub").join("inner.txt"), b"i").unwrap();
    std::fs::write(data.join("a.txt"), b"a").unwrap();
    std::fs::write(data.join("b.txt"), b"bb").unwrap();

    let out = Command::new(&copy)
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, &data)
        .output()
        .expect("run the copied test binary");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "headless TUI run failed:\n{log}");
    assert!(log.contains("headless tui: ok"), "the child did not run the test:\n{log}");
    assert!(data.join("a.txt").is_file(), "the cancelled delete kept a.txt");
    let cfg = tmp.path().join(config::PORTABLE_DATA_DIR);
    assert!(cfg.join("session-tui.toml").is_file(), "the TUI saved its own session");
    assert!(!cfg.join("session.toml").exists(), "the GUI session is untouched");
}

type Check = Box<dyn Fn(&App) -> bool>;

/// Wait until `when` holds, then send `send`.
struct Step {
    what: &'static str,
    when: Check,
    send: Event,
}

struct Script {
    steps: VecDeque<Step>,
    since: Instant,
}

impl EventSource for Script {
    fn next(&mut self, app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent> {
        let wait = timeout.min(Duration::from_millis(10));
        let Some(step) = self.steps.front() else {
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

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn labels(app: &App) -> Vec<String> {
    app.view().map(|v| v.rows.iter().map(|r| r.label()).collect()).unwrap_or_default()
}

fn cursor(app: &App) -> Option<String> {
    app.view()?.rows.iter().find(|r| r.cursor).map(|r| r.label())
}

fn at(app: &App, dir: &Path) -> bool {
    app.core.ws.pane().location == soyuz_core::Location::Dir(dir.to_path_buf())
}

fn run_tui(data: PathBuf) {
    // Never fall back to the real config directory.
    let dir = config::config_dir();
    let root = data.parent().expect("temp root");
    assert!(dir.ends_with(config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());
    assert_eq!(
        dir.parent().and_then(|d| d.canonicalize().ok()),
        root.canonicalize().ok(),
        "config dir {} is outside the temp root",
        dir.display()
    );

    let mut c = Config::default();
    c.general.show_hidden = false;
    c.general.confirm_delete = true;
    c.updates.weekly_check = false;
    c.cli.server_enabled = false;
    c.search.index.enabled = false;
    let startup = Startup { open: Some(data.clone()), ..Default::default() };
    let mut app = App::new(c, startup);
    assert_eq!(app.core.frontend_kind, soyuz_appcore::core::FrontendKind::Tui);

    let sub = data.join("sub");
    let d1 = data.clone();
    let d2 = data.clone();
    let steps = vec![
        Step {
            what: "the listing",
            when: Box::new(move |a| at(a, &d1) && labels(a) == ["sub/", "a.txt", "b.txt"]),
            send: key(KeyCode::Down),
        },
        Step {
            what: "the cursor on a.txt",
            when: Box::new(|a| cursor(a).as_deref() == Some("a.txt")),
            send: key(KeyCode::Up),
        },
        Step {
            what: "the cursor on sub/",
            when: Box::new(|a| cursor(a).as_deref() == Some("sub/")),
            send: key(KeyCode::Enter),
        },
        Step {
            what: "the subfolder",
            when: Box::new(move |a| at(a, &sub) && labels(a) == ["inner.txt"]),
            send: key(KeyCode::Backspace),
        },
        Step {
            what: "back in the folder with the cursor on sub/",
            when: Box::new(move |a| at(a, &d2) && labels(a).len() == 3 && cursor(a).as_deref() == Some("sub/")),
            send: key(KeyCode::Down),
        },
        Step {
            what: "the cursor on a.txt again",
            when: Box::new(|a| cursor(a).as_deref() == Some("a.txt")),
            send: key(KeyCode::Delete),
        },
        Step {
            what: "the delete confirmation",
            when: Box::new(|a| {
                a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.title.contains("a.txt"))
                    && a.key_context() == soyuz_core::config::KeyContext::Dialog
            }),
            send: key(KeyCode::Esc),
        },
        Step {
            what: "the confirmation to close",
            when: Box::new(|a| a.view().is_some_and(|v| v.dialog.is_none()) && !a.core.dialogs.is_open()),
            send: Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        },
    ];
    let mut source = Script { steps: steps.into(), since: Instant::now() };
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    let mut io = NullIo::default();
    let end = run_loop(&mut app, &mut terminal, &mut source, &mut io).expect("the loop runs");
    assert_eq!(end, LoopEnd::Quit);
    assert!(source.steps.is_empty(), "every step ran");
    assert!(data.join("a.txt").is_file(), "Esc cancelled the delete");
    // The window title went out as OSC 2 after a frame.
    let written = String::from_utf8_lossy(&io.written);
    assert!(written.contains("\x1b]2;data"), "{written:?}");
    // The last frame shows the listing and the status line.
    let buf = terminal.backend().buffer().clone();
    let screen: String =
        (0..24).map(|y| (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect();
    assert!(screen.contains("a.txt") && screen.contains("3 items"), "{screen}");
    app.finish();
    assert!(config::config_dir().join("session-tui.toml").is_file(), "session-tui.toml written");
    println!("headless tui: ok");
}
