//! Headless tests of T5: the AI assistant panel (W2-ai-panel). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! The model is the scripted fake provider (`SOYUZ_AI_FAKE`, transcripts in
//! `tests/fixtures/ai/`), passed to the sandboxed child with [`common::relaunch_env`]. The
//! panel never touches the disk itself: approved plans run as jobs, undo goes through the
//! jobs history.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{Event, KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::ai::agent::Finish;
use soyuz_appcore::ai::model::Route;
use soyuz_appcore::ai::state::{CardState, ChatItem, PendingTask, TaskEnd, UndoState};
use soyuz_core::config::Config;
use soyuz_tui::app::{App, EventSource, LoopEnd, NullIo, TuiEvent, run_loop};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;

use crate::common::{self, Run, Step, at, ctrl, key, type_text};

/// A transcript of `tests/fixtures/ai`.
pub(super) fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("ai").join(name)
}

/// Run test `name` with the fake model playing transcript `transcript`.
pub(super) fn relaunch_fake(module: &str, name: &str, transcript: &str, setup: impl FnOnce(&Path)) -> Run {
    let path = fixture(transcript);
    common::relaunch_env(module, name, &[("SOYUZ_AI_FAKE", OsStr::new(&path))], setup)
}

/// Test settings: the assistant may work in the sandbox (its default scope is the home
/// folder).
pub(super) fn ai_config(data: &Path) -> Config {
    let mut c = common::test_config();
    c.ai.enabled = true;
    c.ai.scope.allowed_roots = vec![data.to_path_buf()];
    c
}

/// The panel's body and footer in the last frame, one line each.
pub(super) fn panel_text(a: &App) -> String {
    a.view()
        .map(|v| v.ai.lines.iter().chain(&v.ai.footer).map(|l| l.text()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

/// A plan card of the running task waits for an answer and is drawn.
fn card_waits(a: &App) -> bool {
    let Some(r) = &a.core.ai.running else { return false };
    let waits = a
        .core
        .ai
        .chat
        .iter()
        .any(|c| matches!(c, ChatItem::Plan(p) if p.task == r.task && p.state == CardState::Pending));
    waits && a.view().is_some_and(|v| v.ai.lines.iter().any(|l| l.text().contains("[Approve]")))
}

/// The end of the last task.
fn task_end(a: &App) -> Option<&TaskEnd> {
    a.core.ai.chat.iter().rev().find_map(|c| match c {
        ChatItem::TaskEnd(t) => Some(t.as_ref()),
        _ => None,
    })
}

fn ended(a: &App) -> bool {
    a.core.ai.running.is_none() && task_end(a).is_some()
}

fn a_and_moved(data: &Path) {
    std::fs::write(data.join("a.txt"), b"a").unwrap();
    std::fs::create_dir_all(data.join("Moved")).unwrap();
}

/// Ctrl+I, type `prompt`, Enter.
fn ask(steps: &mut Vec<Step>, data: &Path, prompt: &str) {
    let d = data.to_path_buf();
    steps.push(Step::new("the listing", move |a| at(a, &d) && !common::labels(a).is_empty(), ctrl('i')));
    steps.push(Step::new(
        "the panel has the keyboard",
        |a| a.focus() == TuiFocus::Ai,
        key(KeyCode::Char(prompt.chars().next().unwrap())),
    ));
    type_text(steps, &prompt[1..]);
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
}

#[test]
fn approve_runs_a_job_and_undo_puts_it_back() {
    match relaunch_fake(module_path!(), "approve_runs_a_job_and_undo_puts_it_back", "move.toml", a_and_moved) {
        Run::Child(data) => approve_undo(data),
        Run::Parent(r) => {
            assert!(r.data.join("a.txt").exists(), "undone");
            assert!(!r.data.join("Moved").join("a.txt").exists());
        }
    }
}

fn approve_undo(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    ask(&mut steps, &data, "move it");
    steps.push(Step::new("a plan card waits", card_waits, key(KeyCode::Tab)));
    steps.push(Step::new("the cards have the keyboard", |a| a.focus() == TuiFocus::Ai, key(KeyCode::Char('a'))));
    let d = data.clone();
    steps.push(Step::new(
        "the job moved the file and the task ended",
        move |a| {
            d.join("Moved").join("a.txt").exists()
                && !d.join("a.txt").exists()
                && ended(a)
                && common::wait_jobs_idle(a)
                && panel_text(a).contains("[Undo everything the assistant did]")
        },
        key(KeyCode::Char('u')),
    ));
    let d = data.clone();
    steps.push(Step::new(
        "undone",
        move |a| {
            d.join("a.txt").exists()
                && task_end(a).is_some_and(|t| matches!(t.undo, UndoState::Done(_)))
                && common::wait_jobs_idle(a)
        },
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 120, 30);
    let t = task_end(&app).expect("a task end");
    assert!(matches!(t.reason, Finish::Done), "{:?}", t.reason);
    assert_eq!(t.executed.len(), 1, "one approved plan ran");
    assert!(!app.core.fileops.jobs.history().is_empty(), "the move is in the history");
    app.finish();
    common::ok();
}

#[test]
fn reject_runs_no_job() {
    match relaunch_fake(module_path!(), "reject_runs_no_job", "move.toml", a_and_moved) {
        Run::Child(data) => reject(data),
        Run::Parent(r) => assert!(r.data.join("a.txt").exists()),
    }
}

fn reject(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    ask(&mut steps, &data, "move it");
    steps.push(Step::new("a plan card waits", card_waits, key(KeyCode::Tab)));
    steps.push(Step::new("the cards", |_| true, key(KeyCode::Char('r'))));
    steps.push(Step::new("the task ended", ended, ctrl('q')));
    common::run_script(&mut app, steps, 120, 30);
    assert!(data.join("a.txt").exists() && !data.join("Moved").join("a.txt").exists());
    assert!(app.core.fileops.jobs.history().is_empty(), "no job ran");
    assert!(app.core.ai.chat.iter().any(|c| matches!(c, ChatItem::Plan(p) if p.state == CardState::Rejected)));
    assert!(task_end(&app).is_some_and(|t| t.executed.is_empty()));
    app.finish();
    common::ok();
}

#[test]
fn cost_question_go_then_esc_stops_the_task() {
    match relaunch_fake(module_path!(), "cost_question_go_then_esc_stops_the_task", "move.toml", a_and_moved) {
        Run::Child(data) => cost_and_stop(data),
        Run::Parent(r) => assert!(r.data.join("a.txt").exists(), "stopped before anything ran"),
    }
}

fn cost_and_stop(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    // A big task waits for its cost to be confirmed (the fake model never asks itself).
    app.core.ai.pending = Some(PendingTask {
        prompt: "move it".into(),
        route: Route::Strong,
        mentions: Vec::new(),
        attachments: Default::default(),
    });
    app.core.ai.chat.push(ChatItem::Confirm { estimate: 1.25, model: "big-model".into(), pending: 1 });
    app.core.ai.panel.open = true;
    app.core.ai.panel.focus_composer = true;
    let steps = vec![
        Step::new(
            "the cost question",
            |a| a.focus() == TuiFocus::Ai && panel_text(a).contains("[Start]") && panel_text(a).contains("1.25"),
            key(KeyCode::Tab),
        ),
        Step::new("the cards", |_| true, key(KeyCode::Enter)),
        Step::new("the task started and its plan waits", card_waits, key(KeyCode::Esc)),
        Step::new(
            "stopped",
            |a| ended(a) && task_end(a).is_some_and(|t| matches!(t.reason, Finish::Cancelled)),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 120, 30);
    assert!(app.core.ai.pending.is_none());
    assert!(!app.core.ai.chat.iter().any(|c| matches!(c, ChatItem::Confirm { .. })), "answered");
    assert!(app.core.fileops.jobs.history().is_empty());
    app.finish();
    common::ok();
}

#[test]
fn edit_names_replans_with_the_new_name() {
    match relaunch_fake(module_path!(), "edit_names_replans_with_the_new_name", "rename.toml", |d| {
        std::fs::write(d.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => edit_names(data),
        Run::Parent(r) => {
            assert!(r.data.join("c.txt").exists(), "renamed to the edited name");
            assert!(!r.data.join("a.txt").exists() && !r.data.join("b.txt").exists());
        }
    }
}

fn edit_names(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    ask(&mut steps, &data, "rename it");
    steps.push(Step::new("a rename card waits", card_waits, key(KeyCode::Tab)));
    steps.push(Step::new("the cards", |_| true, key(KeyCode::Char('e'))));
    steps.push(Step::new(
        "the name table",
        |a| common::dialog_title(a).as_deref() == Some("Edit the new names"),
        key(KeyCode::Backspace),
    ));
    for _ in 0..4 {
        steps.push(Step::new("erasing", |_| true, key(KeyCode::Backspace)));
    }
    type_text(&mut steps, "c.txt");
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new(
        "a new card with the edited name waits",
        |a| {
            common::dialog_title(a).is_none()
                && card_waits(a)
                && a.core.ai.chat.iter().any(|c| matches!(c, ChatItem::Plan(p) if p.state == CardState::Edited))
                && panel_text(a).contains("c.txt")
        },
        key(KeyCode::Char('a')),
    ));
    let d = data.clone();
    steps.push(Step::new("renamed", move |a| d.join("c.txt").exists() && ended(a), ctrl('q')));
    common::run_script(&mut app, steps, 120, 30);
    app.finish();
    common::ok();
}

// ---- mouse ----------------------------------------------------------------------------

/// What a [`DynStep`] sends (`None`: not yet).
pub(super) type SendFn = Box<dyn Fn(&App) -> Option<Event>>;

/// A step whose event is computed from the last frame (click targets move with the
/// content): sent once `send` returns one.
pub(super) struct DynStep {
    pub what: &'static str,
    pub send: SendFn,
}

impl DynStep {
    pub fn new(what: &'static str, send: impl Fn(&App) -> Option<Event> + 'static) -> DynStep {
        DynStep { what, send: Box::new(send) }
    }

    /// `ev` once `when` holds.
    pub fn when(what: &'static str, when: impl Fn(&App) -> bool + 'static, ev: Event) -> DynStep {
        DynStep::new(what, move |a| when(a).then(|| ev.clone()))
    }
}

struct DynScript {
    steps: VecDeque<DynStep>,
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
        if let Some(ev) = (step.send)(app) {
            self.steps.pop_front();
            self.since = Instant::now();
            return vec![TuiEvent::Term(ev)];
        }
        assert!(self.since.elapsed() < Duration::from_secs(30), "timed out waiting for {}", step.what);
        let _ = redraw.recv_timeout(wait);
        Vec::new()
    }
}

/// [`common::run_script`] with [`DynStep`]s.
pub(super) fn run_dyn(app: &mut App, steps: Vec<DynStep>, w: u16, h: u16) {
    let mut source = DynScript { steps: steps.into(), since: Instant::now() };
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    let mut io = NullIo::default();
    let end = run_loop(app, &mut terminal, &mut source, &mut io).expect("the loop runs");
    assert_eq!(end, LoopEnd::Quit);
    assert!(source.steps.is_empty(), "every step ran");
}

/// A left click on the first target `want` accepts, in the last frame.
pub(super) fn click_on(a: &App, want: impl Fn(HitTarget) -> bool) -> Option<Event> {
    let (r, _) = a.hits().iter().filter(|(r, t)| want(*t) && a.hits().at(r.x, r.y) == Some(*t)).last()?;
    Some(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: r.x,
        row: r.y,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }))
}

#[test]
fn mouse_approves_a_card_and_switches_tabs() {
    match relaunch_fake(module_path!(), "mouse_approves_a_card_and_switches_tabs", "move.toml", a_and_moved) {
        Run::Child(data) => mouse(data),
        Run::Parent(r) => assert!(r.data.join("Moved").join("a.txt").exists()),
    }
}

fn mouse(data: PathBuf) {
    use soyuz_tui::hits::{AiButton, CardButton};
    let mut app = common::new_app(ai_config(&data), &data);
    let d = data.clone();
    let steps = vec![
        DynStep::when("the listing", move |a| at(a, &d), ctrl('i')),
        DynStep::when("the panel", |a| a.focus() == TuiFocus::Ai, key(KeyCode::Char('m'))),
        DynStep::when("typed", |_| true, key(KeyCode::Char('v'))),
        DynStep::new("the composer", |a| click_on(a, |t| t == HitTarget::AiComposer)),
        DynStep::when("send", |a| a.focus() == TuiFocus::Ai, key(KeyCode::Enter)),
        DynStep::new("Approve", |a| {
            if !card_waits(a) {
                return None;
            }
            click_on(a, |t| matches!(t, HitTarget::AiCardButton { button: CardButton::Approve, .. }))
        }),
        DynStep::new("the recipes tab", |a| {
            if !ended(a) {
                return None;
            }
            click_on(a, |t| t == HitTarget::AiTab(1))
        }),
        DynStep::new("close", |a| {
            if a.core.ai.panel.tab != soyuz_appcore::ai::state::PanelTab::Recipes {
                return None;
            }
            click_on(a, |t| t == HitTarget::AiButton(AiButton::Close))
        }),
        DynStep::when("closed", |a| !a.core.ai.panel.open && a.focus() == TuiFocus::List, ctrl('q')),
    ];
    run_dyn(&mut app, steps, 120, 30);
    assert!(data.join("Moved").join("a.txt").exists());
    app.finish();
    common::ok();
}

// ---- small screens --------------------------------------------------------------------

#[test]
fn tiny_sizes_with_the_panel_open() {
    match relaunch_fake(module_path!(), "tiny_sizes_with_the_panel_open", "move.toml", a_and_moved) {
        Run::Child(data) => tiny(data),
        Run::Parent(_) => {}
    }
}

fn tiny(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    // A chat with every kind of item, then frames at every small size.
    let mut steps = Vec::new();
    ask(&mut steps, &data, "move it");
    steps.push(Step::new("a plan card waits", card_waits, ctrl('q')));
    common::run_script(&mut app, steps, 120, 30);
    app.core.ai.chat.push(ChatItem::Notice { text: "日本語 notice \u{1b}[2J".into(), warn: true });
    app.core.ai.chat.push(ChatItem::Confirm { estimate: 2.0, model: "m".into(), pending: 1 });
    app.core.ai.panel.composer = "/or".into();
    app.core.ai.panel.open = true;
    app.core.ai.panel.focus_composer = true;
    let mut io = NullIo::default();
    for w in (0..=40).step_by(3) {
        for h in 0..=14 {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            app.frame(&mut terminal, &[], &mut io).expect("a frame");
            let screen = common::screen(&terminal);
            assert!(!screen.contains('\u{1b}'), "escape sequences are made safe");
        }
    }
    app.core.ai.stop();
    app.finish();
    common::ok();
}
