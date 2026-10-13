//! Headless tests of T5: the embedded terminal pane (W2-terminal-pane). Each test runs in its own
//! sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! These start a real shell in a real PTY (ConPTY on Windows, openpty on unix): `cmd.exe`
//! on Windows, `/bin/sh` elsewhere, without the shell integration, in the sandbox's data
//! folder. Nothing the shell runs leaves that folder.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_core::config::Config;
use soyuz_tui::app::{App, EventSource, LoopEnd, NullIo, TuiEvent, run_loop};
use soyuz_tui::layout::HitTarget;

use crate::common::{self, Run, Step, alt, ctrl, key, key_mods};

/// Settings with a predictable shell.
fn config() -> Config {
    let mut c = common::test_config();
    c.terminal.shell = if cfg!(windows) { "cmd.exe".into() } else { "/bin/sh".into() };
    c.terminal.args = Vec::new();
    c.terminal.shell_integration = false;
    c.terminal.follow_navigation = false;
    c.terminal.follow_shell = false;
    c
}

/// ToggleTerminal (Ctrl+`).
fn toggle_terminal() -> Event {
    key_mods(KeyCode::Char('`'), KeyModifiers::CONTROL)
}

/// The text of the grid drawn in the last frame.
fn grid(a: &App) -> String {
    a.view().and_then(|v| v.terminal.grid.as_ref()).map(|g| g.text()).unwrap_or_default()
}

/// The active session's shell waits at an empty prompt.
fn at_prompt(a: &App) -> bool {
    let t = &a.core.ui.terminal;
    let owner = soyuz_appcore::models::terminal::current_owner(&a.core);
    t.active_id(owner).and_then(|id| t.session(id)).and_then(|s| s.term.as_ref()).is_some_and(|t| t.at_prompt())
}

/// The pane is drawn and has the keyboard.
fn focused(a: &App) -> bool {
    a.view().is_some_and(|v| v.terminal.visible && v.terminal.focused)
}

/// Steps that type `text` and press Enter.
fn type_line(steps: &mut Vec<Step>, text: &str) {
    common::type_text(steps, text);
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
}

/// `echo` in a real shell shows in the grid; `exit` ends the session, which closes the
/// pane and gives the keyboard back.
#[test]
fn tui_terminal_echo_then_exit_closes() {
    let Run::Child(data) = common::relaunch(module_path!(), "tui_terminal_echo_then_exit_closes", |_| {}) else {
        return;
    };
    let mut app = common::new_app(config(), &data);
    let mut steps = vec![Step::new("the listing", |_| true, toggle_terminal())];
    steps.push(Step::new("a prompt", |a: &App| focused(a) && at_prompt(a), key(KeyCode::Char('e'))));
    type_line(&mut steps, "cho t5-hi");
    steps.push(Step::new(
        "the output",
        |a: &App| grid(a).lines().filter(|l| l.trim_end().ends_with("t5-hi")).count() >= 2 && at_prompt(a),
        key(KeyCode::Char('e')),
    ));
    type_line(&mut steps, "xit");
    steps.push(Step::new(
        "the session ended",
        |a: &App| a.core.ui.terminal.sessions.is_empty() && !a.core.ui.bottom.terminal_shown(),
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 100, 30);
    assert!(!app.core.ui.terminal.focused, "the keyboard went back to the files");
    common::ok();
}

/// Typed keys reach the shell (its line editor echoes them); Esc at an empty prompt gives
/// the keyboard back; FocusTerminal (Alt+T) takes it again; the passthrough Alt+J leaves
/// for the jobs panel.
#[test]
fn tui_terminal_typing_esc_and_passthrough() {
    let Run::Child(data) = common::relaunch(module_path!(), "tui_terminal_typing_esc_and_passthrough", |_| {}) else {
        return;
    };
    let mut app = common::new_app(config(), &data);
    let mut steps = vec![Step::new("the listing", |_| true, toggle_terminal())];
    steps.push(Step::new("a prompt", |a: &App| focused(a) && at_prompt(a), key(KeyCode::Char('q'))));
    common::type_text(&mut steps, "wxz");
    steps.push(Step::new("the echo", |a: &App| grid(a).contains("qwxz"), key(KeyCode::Backspace)));
    for _ in 0..3 {
        steps.push(Step::new("erasing", |_| true, key(KeyCode::Backspace)));
    }
    // Esc in the middle of a line goes to the shell; at an empty prompt it leaves.
    steps.push(Step::new("an empty prompt", |a: &App| !grid(a).contains("qw") && at_prompt(a), key(KeyCode::Esc)));
    steps.push(Step::new(
        "the files have the keyboard",
        |a: &App| !a.core.ui.terminal.focused && a.view().is_some_and(|v| v.terminal.visible && !v.terminal.focused),
        alt('t'),
    ));
    steps.push(Step::new("focused again", focused, alt('j')));
    steps.push(Step::new(
        "the jobs panel",
        |a: &App| a.core.ui.bottom.activity_shown() && a.view().is_some_and(|v| !v.terminal.visible),
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

/// A program's OSC 52 clipboard write (`terminal.osc52 = "ask"`) shows the prompt bar;
/// `a` allows it once: the text reaches the clipboard and the bar closes.
#[test]
fn tui_terminal_osc52_bar_copies() {
    let Run::Child(data) = common::relaunch(module_path!(), "tui_terminal_osc52_bar_copies", |_| {}) else {
        return;
    };
    let mut app = common::new_app(config(), &data);
    // What `models::terminal::pump` records for a shell's OSC 52 write under "ask".
    app.core.ui.terminal.clip_request = Some("t5 secret".into());
    let steps = vec![
        Step::new("the listing", |_| true, toggle_terminal()),
        Step::new(
            "the bar",
            |a: &App| focused(a) && a.view().is_some_and(|v| v.terminal.clip.as_deref() == Some("t5 secret")),
            key(KeyCode::Char('x')),
        ),
        // Other keys wait for the answer.
        Step::new("still asking", |a: &App| a.core.ui.terminal.clip_request.is_some(), key(KeyCode::Char('a'))),
        Step::new(
            "copied",
            |a: &App| {
                a.core.ui.terminal.clip_request.is_none()
                    && a.host.last_copy().as_deref() == Some("t5 secret")
                    && a.view().is_some_and(|v| v.terminal.clip.is_none())
            },
            alt('j'),
        ),
        Step::new("left", |a: &App| a.core.ui.bottom.activity_shown(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    assert!(!app.core.ui.terminal.clip_allow_session, "allowed once, not for the session");
    common::ok();
}

/// A tiny screen with the pane open: nothing panics, the pane still has the keyboard.
#[test]
fn tui_terminal_tiny_screen() {
    let Run::Child(data) = common::relaunch(module_path!(), "tui_terminal_tiny_screen", |_| {}) else {
        return;
    };
    let mut app = common::new_app(config(), &data);
    let steps = vec![
        Step::new("the listing", |_| true, toggle_terminal()),
        Step::new("focused", |a: &App| a.core.ui.terminal.focused, alt('j')),
        Step::new("left", |a: &App| !a.core.ui.bottom.terminal_shown(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 12, 5);
    common::ok();
}

/// A step whose event is built when it is sent (from the last frame's click targets).
struct Dyn {
    what: &'static str,
    when: Box<dyn Fn(&App) -> bool>,
    send: Box<dyn Fn(&App) -> Event>,
}

fn dyn_step(what: &'static str, when: impl Fn(&App) -> bool + 'static, send: impl Fn(&App) -> Event + 'static) -> Dyn {
    Dyn { what, when: Box::new(when), send: Box::new(send) }
}

struct DynScript {
    steps: VecDeque<Dyn>,
    since: Instant,
}

impl EventSource for DynScript {
    fn next(&mut self, app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent> {
        let wait = timeout.min(Duration::from_millis(10));
        let Some(step) = self.steps.front() else {
            assert!(self.since.elapsed() < Duration::from_secs(60), "the app did not quit after the last step");
            let _ = redraw.recv_timeout(wait);
            return Vec::new();
        };
        if (step.when)(app) {
            let step = self.steps.pop_front().expect("step");
            self.since = Instant::now();
            return vec![TuiEvent::Term((step.send)(app))];
        }
        assert!(self.since.elapsed() < Duration::from_secs(30), "timed out waiting for {}", step.what);
        let _ = redraw.recv_timeout(wait);
        Vec::new()
    }
}

/// The centre of the first click target `want` accepts, in the last frame.
fn target(a: &App, want: impl Fn(HitTarget) -> bool) -> Option<(u16, u16)> {
    a.hits().iter().find(|(_, t)| want(*t)).map(|(r, _)| (r.x + r.width / 2, r.y + r.height / 2))
}

fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> Event {
    Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE })
}

/// A press of `button` on the first target `want` accepts.
fn press(a: &App, button: MouseButton, want: impl Fn(HitTarget) -> bool) -> Event {
    mouse(MouseEventKind::Down(button), target(a, want).expect("the click target"))
}

fn tabs(a: &App) -> Vec<u64> {
    a.view().map(|v| v.terminal.tabs.iter().map(|t| t.id).collect()).unwrap_or_default()
}

/// The strip and the grid by mouse: `[+]` starts a session, a tab's close mark closes
/// it, a right click opens the session menu (Rename: the name is edited in place), a
/// press on the grid takes the keyboard and a press elsewhere gives it back.
#[test]
fn tui_terminal_mouse() {
    let Run::Child(data) = common::relaunch(module_path!(), "tui_terminal_mouse", |_| {}) else {
        return;
    };
    let mut app = common::new_app(config(), &data);
    let mut steps = vec![
        dyn_step("the listing", |_| true, |_| toggle_terminal()),
        dyn_step(
            "one session",
            |a: &App| focused(a) && tabs(a).len() == 1,
            |a: &App| press(a, MouseButton::Left, |t| t == HitTarget::TermNew),
        ),
        dyn_step(
            "two sessions",
            |a: &App| tabs(a).len() == 2,
            |a: &App| {
                let second = tabs(a)[1];
                press(a, MouseButton::Left, move |t| t == HitTarget::TermTabClose(second))
            },
        ),
        dyn_step(
            "one again",
            |a: &App| tabs(a).len() == 1,
            |a: &App| press(a, MouseButton::Right, |t| matches!(t, HitTarget::TermTab(_))),
        ),
        dyn_step("the menu", |a: &App| a.view().is_some_and(|v| v.menu.menu.is_some()), |_| key(KeyCode::Home)),
        dyn_step("on Rename", |_| true, |_| key(KeyCode::Enter)),
    ];
    // The field starts with the old name, the cursor at its end: the typed text appends.
    steps.push(dyn_step(
        "renaming",
        |a: &App| a.view().is_some_and(|v| v.terminal.renaming.is_some()),
        |_| key(KeyCode::End),
    ));
    for c in "t5name".chars() {
        steps.push(dyn_step("typing", |_| true, move |_| key(KeyCode::Char(c))));
    }
    steps.push(dyn_step("typed", |_| true, |_| key(KeyCode::Enter)));
    steps.push(dyn_step(
        "renamed",
        |a: &App| {
            a.view().is_some_and(|v| v.terminal.renaming.is_none() && v.terminal.tabs[0].label.ends_with("t5name"))
        },
        |a: &App| {
            let (r, _) = *a
                .hits()
                .iter()
                .find(|(_, t)| matches!(t, HitTarget::PaneBackground { .. } | HitTarget::PaneRow { .. }))
                .expect("the pane");
            mouse(MouseEventKind::Down(MouseButton::Left), (r.x + 1, r.y))
        },
    ));
    steps.push(dyn_step(
        "the files have the keyboard",
        |a: &App| !a.core.ui.terminal.focused,
        |a: &App| press(a, MouseButton::Left, |t| matches!(t, HitTarget::TermCell { .. })),
    ));
    steps.push(dyn_step("focused by the click", |a: &App| a.core.ui.terminal.focused, |_| alt('j')));
    steps.push(dyn_step("left", |a: &App| !a.core.ui.terminal.focused, |_| ctrl('q')));
    let mut source = DynScript { steps: steps.into(), since: Instant::now() };
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("test terminal");
    let mut io = NullIo::default();
    let end = run_loop(&mut app, &mut terminal, &mut source, &mut io).expect("the loop runs");
    assert_eq!(end, LoopEnd::Quit);
    assert!(source.steps.is_empty(), "every step ran");
    common::ok();
}
