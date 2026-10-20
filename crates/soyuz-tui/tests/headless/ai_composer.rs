//! Headless tests of T5: the multi-line AI composer, JSON argument edits (in the dialog and
//! in `$EDITOR`), the smart search card, the chips and their menus, and the panel's keys
//! from the file list (W3 AI composer + JSON edit). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! The model is the scripted fake provider (`SOYUZ_AI_FAKE`, transcripts in
//! `tests/fixtures/ai/`). No real editor ever starts: `App::child_runner` is a fake that
//! rewrites the scratch file, and `suspend::set_env_source` hides `$VISUAL`, `$EDITOR` and
//! `PATH`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::ai::PanelAct;
use soyuz_appcore::ai::agent::Finish;
use soyuz_appcore::ai::state::{CardState, ChatItem, TaskEnd};
use soyuz_appcore::cmd::Cmd;
use soyuz_core::config::{Action, AiAccess};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::suspend::{self, ChildSpec, Env};
use soyuz_tui::view::dialogs::DialogVm;

use super::ai::{DynStep, ai_config, click_on, panel_text, relaunch_fake, run_dyn};
use crate::common::{self, Run, Step, alt, at, ctrl, key, key_mods, type_text};

// ---- helpers --------------------------------------------------------------------------

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

fn edited(a: &App) -> bool {
    a.core.ai.chat.iter().any(|c| matches!(c, ChatItem::Plan(p) if p.state == CardState::Edited))
}

fn task_end(a: &App) -> Option<&TaskEnd> {
    a.core.ai.chat.iter().rev().find_map(|c| match c {
        ChatItem::TaskEnd(t) => Some(t.as_ref()),
        _ => None,
    })
}

fn ended(a: &App) -> bool {
    a.core.ai.running.is_none() && task_end(a).is_some()
}

/// The user's messages, in order.
fn user_texts(a: &App) -> Vec<String> {
    a.core
        .ai
        .chat
        .iter()
        .filter_map(|c| match c {
            ChatItem::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The dialog drawn in the last frame.
fn dialog(a: &App) -> Option<&DialogVm> {
    a.view()?.dialog.as_ref()
}

/// The JSON editor is open.
fn json_open(a: &App) -> bool {
    dialog(a).is_some_and(|d| d.title.starts_with("Edit the arguments"))
}

/// The JSON editor's line under the text cursor, as drawn.
fn cursor_line(a: &App) -> Option<String> {
    let d = dialog(a)?;
    let (row, _) = d.text_cursor?;
    d.text.get(usize::from(row)).cloned()
}

fn files(data: &Path) {
    std::fs::write(data.join("a.txt"), b"a").unwrap();
    std::fs::create_dir_all(data.join("Moved")).unwrap();
    std::fs::create_dir_all(data.join("Other")).unwrap();
}

/// Ctrl+I, type `prompt`, Enter (dynamic steps).
fn ask_dyn(steps: &mut Vec<DynStep>, data: &Path, prompt: &'static str) {
    let d = data.to_path_buf();
    steps.push(DynStep::when("the listing", move |a| at(a, &d) && !common::labels(a).is_empty(), ctrl('i')));
    let mut chars = prompt.chars();
    let first = chars.next().unwrap();
    steps.push(DynStep::when("the panel has the keyboard", |a| a.focus() == TuiFocus::Ai, key(KeyCode::Char(first))));
    for c in chars {
        steps.push(DynStep::when("typing", |_| true, key(KeyCode::Char(c))));
    }
    steps.push(DynStep::when("typed", |_| true, key(KeyCode::Enter)));
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

// ---- the multi-line composer ----------------------------------------------------------

#[test]
fn multi_line_send_keeps_newlines() {
    match relaunch_fake(module_path!(), "multi_line_send_keeps_newlines", "chat.toml", |d| {
        std::fs::write(d.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => multi_line(data),
        Run::Parent(_) => {}
    }
}

fn composer_rows(a: &App) -> usize {
    a.view().and_then(|v| v.ai.field.as_ref()).map_or(0, |f| f.lines.len())
}

fn multi_line(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let d = data.clone();
    let mut steps = vec![
        Step::new("the listing", move |a| at(a, &d) && !common::labels(a).is_empty(), ctrl('i')),
        Step::new("the panel has the keyboard", |a| a.focus() == TuiFocus::Ai, key(KeyCode::Char('o'))),
    ];
    type_text(&mut steps, "ne");
    // Alt+Enter, Ctrl+J and (with the kitty protocol) Shift+Enter start a new line.
    steps.push(Step::new("typed", |_| true, key_mods(KeyCode::Enter, KeyModifiers::ALT)));
    type_text(&mut steps, "two");
    steps.push(Step::new("typed", |_| true, ctrl('j')));
    type_text(&mut steps, "three");
    steps.push(Step::new("typed", |_| true, key_mods(KeyCode::Enter, KeyModifiers::SHIFT)));
    // A paste keeps its line breaks.
    steps.push(Step::new("typed", |_| true, Event::Paste("four\r\nfive".into())));
    steps.push(Step::new(
        "five lines in a five-row composer",
        |a| a.core.ai.panel.composer == "one\ntwo\nthree\nfour\nfive" && composer_rows(a) == 5,
        key(KeyCode::Up),
    ));
    // Up moves to the line above, keeping the column: typing lands at the end of `four`.
    steps.push(Step::new("up", |_| true, key(KeyCode::Char('!'))));
    steps.push(Step::new(
        "typed on line four",
        |a| a.core.ai.panel.composer == "one\ntwo\nthree\nfour!\nfive",
        key(KeyCode::Enter),
    ));
    steps.push(Step::new(
        "sent with its newlines, answered, and drawn line by line",
        |a| {
            let text = panel_text(a);
            ended(a)
                && a.core.ai.panel.composer.is_empty()
                && composer_rows(a) == 1
                && text.contains("Noted.")
                && text.lines().any(|l| l.trim_start() == "two")
                && text.lines().any(|l| l.trim_start() == "four!")
        },
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 120, 30);
    assert_eq!(user_texts(&app), ["one\ntwo\nthree\nfour!\nfive"]);
    app.finish();
    common::ok();
}

// ---- JSON edits -----------------------------------------------------------------------

/// Steps that put the JSON editor's cursor right after `Moved` on the `dest_dir` line.
fn to_moved(steps: &mut Vec<DynStep>) {
    for _ in 0..8 {
        steps.push(DynStep::new("down to dest_dir", |a| {
            let line = cursor_line(a)?;
            Some(key(if line.contains("\"dest_dir\"") { KeyCode::F(5) } else { KeyCode::Down }))
        }));
    }
    steps.push(DynStep::when(
        "on dest_dir",
        |a| cursor_line(a).is_some_and(|l| l.contains("dest_dir")),
        key(KeyCode::End),
    ));
    // Before the comma, if the line has one, then before the closing quote.
    steps.push(DynStep::new("before the comma", |a| {
        let line = cursor_line(a)?;
        Some(key(if line.trim_end().ends_with(',') { KeyCode::Left } else { KeyCode::F(5) }))
    }));
    steps.push(DynStep::when("before the quote", |_| true, key(KeyCode::Left)));
}

#[test]
fn json_edit_runs_the_edited_args() {
    match relaunch_fake(module_path!(), "json_edit_runs_the_edited_args", "move.toml", files) {
        Run::Child(data) => json_edit(data),
        Run::Parent(r) => {
            assert!(r.data.join("Other").join("a.txt").exists(), "moved where the edit said");
            assert!(!r.data.join("Moved").join("a.txt").exists());
        }
    }
}

fn json_edit(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    ask_dyn(&mut steps, &data, "move it");
    steps.push(DynStep::when("a plan card waits", card_waits, key(KeyCode::Tab)));
    steps.push(DynStep::when("the cards", |_| true, key(KeyCode::Char('j'))));
    to_moved(&mut steps);
    for _ in 0..5 {
        steps.push(DynStep::when("erasing Moved", json_open, key(KeyCode::Backspace)));
    }
    for c in "Other".chars() {
        steps.push(DynStep::when("typing", |_| true, key(KeyCode::Char(c))));
    }
    steps.push(DynStep::when("typed", |a| cursor_line(a).is_some_and(|l| l.contains("Other")), ctrl('s')));
    steps.push(DynStep::when(
        "a new card with the edited arguments waits",
        |a| dialog(a).is_none() && edited(a) && card_waits(a) && panel_text(a).contains("Other"),
        key(KeyCode::Char('a')),
    ));
    let d = data.clone();
    steps.push(DynStep::when(
        "moved",
        move |a| d.join("Other").join("a.txt").exists() && ended(a) && common::wait_jobs_idle(a),
        ctrl('q'),
    ));
    run_dyn(&mut app, steps, 120, 30);
    app.finish();
    common::ok();
}

/// The plan form (E on a move card) is open.
fn form_open(a: &App) -> bool {
    dialog(a).is_some_and(|d| d.title == soyuz_appcore::tr!("tui-ai-form-title"))
}

#[test]
fn form_edit_changes_the_destination() {
    match relaunch_fake(module_path!(), "form_edit_changes_the_destination", "move.toml", files) {
        Run::Child(data) => form_edit(data),
        Run::Parent(r) => {
            assert!(r.data.join("Other").join("a.txt").exists(), "moved where the form said");
            assert!(!r.data.join("Moved").join("a.txt").exists());
        }
    }
}

fn form_edit(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    ask_dyn(&mut steps, &data, "move it");
    steps.push(DynStep::when("a plan card waits", card_waits, key(KeyCode::Tab)));
    steps.push(DynStep::when("the cards", |_| true, key(KeyCode::Char('e'))));
    // The destination field ends in "Moved": erase it, type "Other".
    steps.push(DynStep::when("the form", form_open, key(KeyCode::End)));
    for _ in 0..5 {
        steps.push(DynStep::when("erasing Moved", form_open, key(KeyCode::Backspace)));
    }
    for c in "Other".chars() {
        steps.push(DynStep::when("typing", |_| true, key(KeyCode::Char(c))));
    }
    steps.push(DynStep::when("typed", form_open, key(KeyCode::Enter)));
    steps.push(DynStep::when(
        "a new card with the edited destination waits",
        |a| dialog(a).is_none() && edited(a) && card_waits(a) && panel_text(a).contains("Other"),
        key(KeyCode::Char('a')),
    ));
    let d = data.clone();
    steps.push(DynStep::when(
        "moved",
        move |a| d.join("Other").join("a.txt").exists() && ended(a) && common::wait_jobs_idle(a),
        ctrl('q'),
    ));
    run_dyn(&mut app, steps, 120, 30);
    app.finish();
    common::ok();
}

#[test]
fn palette_question_mark_asks_and_shows_the_plan() {
    match relaunch_fake(module_path!(), "palette_question_mark_asks_and_shows_the_plan", "move.toml", files) {
        Run::Child(data) => palette_ask(data),
        Run::Parent(r) => assert!(r.data.join("Moved").join("a.txt").exists(), "the approved plan ran"),
    }
}

fn palette_ask(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    let d = data.clone();
    steps.push(DynStep::when("the listing", move |a| at(a, &d) && !common::labels(a).is_empty(), ctrl('k')));
    steps.push(DynStep::when("the palette", |a| a.core.palette.open, key(KeyCode::Char('?'))));
    for c in " move it".chars() {
        steps.push(DynStep::when("typing", |_| true, key(KeyCode::Char(c))));
    }
    steps.push(DynStep::when(
        "the request is the first row",
        |a| {
            matches!(
                a.core.palette.results.first().map(|i| &i.exec),
                Some(soyuz_appcore::palette::Exec::Ask(t)) if t == "move it"
            )
        },
        key(KeyCode::Enter),
    ));
    steps.push(DynStep::when(
        "the panel shows the request and its plan",
        |a| !a.core.palette.open && user_texts(a) == ["move it"] && card_waits(a),
        key(KeyCode::Tab),
    ));
    steps.push(DynStep::when("the cards", |_| true, key(KeyCode::Char('a'))));
    let d = data.clone();
    steps.push(DynStep::when(
        "moved",
        move |a| d.join("Moved").join("a.txt").exists() && ended(a) && common::wait_jobs_idle(a),
        ctrl('q'),
    ));
    run_dyn(&mut app, steps, 120, 30);
    assert_eq!(app.core.palette.memory.asks, ["move it"], "remembered for the next time");
    app.finish();
    common::ok();
}

#[test]
fn invalid_json_stays_in_the_dialog() {
    match relaunch_fake(module_path!(), "invalid_json_stays_in_the_dialog", "move.toml", files) {
        Run::Child(data) => invalid_json(data),
        Run::Parent(r) => assert!(r.data.join("a.txt").exists(), "nothing ran"),
    }
}

fn invalid_json(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    ask_dyn(&mut steps, &data, "move it");
    steps.push(DynStep::when("a plan card waits", card_waits, key(KeyCode::Tab)));
    steps.push(DynStep::when("the cards", |_| true, key(KeyCode::Char('j'))));
    steps.push(DynStep::when("the JSON editor", json_open, key_mods(KeyCode::End, KeyModifiers::CONTROL)));
    steps.push(DynStep::when("at the end", |_| true, key(KeyCode::Char('x'))));
    steps.push(DynStep::when("broken", |_| true, ctrl('s')));
    steps.push(DynStep::when(
        "the error shows and the dialog stays",
        |a| json_open(a) && dialog(a).is_some_and(|d| d.error.is_some()) && card_waits(a),
        key(KeyCode::Esc),
    ));
    steps.push(DynStep::when(
        "cancelled: the card still waits",
        |a| dialog(a).is_none() && card_waits(a) && !edited(a),
        key(KeyCode::Char('r')),
    ));
    steps.push(DynStep::when("rejected", ended, ctrl('q')));
    run_dyn(&mut app, steps, 120, 30);
    assert!(app.core.fileops.jobs.history().is_empty(), "no job ran");
    app.finish();
    common::ok();
}

// ---- $EDITOR --------------------------------------------------------------------------

/// Every spec a fake editor was handed, and the text it saw first.
static SEEN: Mutex<Vec<ChildSpec>> = Mutex::new(Vec::new());
static FIRST: Mutex<String> = Mutex::new(String::new());

/// No `$VISUAL` / `$EDITOR`, an empty `PATH`; only an explicit path can be found.
fn isolated_env() -> Env {
    Env {
        visual: None,
        editor: None,
        path_dirs: Vec::new(),
        is_file: Box::new(Path::is_file),
        is_windows: cfg!(windows),
    }
}

/// This test binary: a file that exists, used as the "editor".
fn exe() -> String {
    std::env::current_exe().unwrap().to_string_lossy().into_owned()
}

fn scratch(spec: &ChildSpec) -> PathBuf {
    SEEN.lock().unwrap().push(spec.clone());
    PathBuf::from(spec.args.last().expect("the file to edit"))
}

/// A fake editor: sends the plan to `Other` instead of `Moved`.
fn edit_to_other(spec: &ChildSpec) -> io::Result<Option<i32>> {
    let path = scratch(spec);
    let text = std::fs::read_to_string(&path)?;
    std::fs::write(&path, text.replace("Moved", "Other"))?;
    Ok(Some(0))
}

/// A fake editor that breaks the JSON the first time and fixes it (to `Other`) the second.
fn break_then_fix(spec: &ChildSpec) -> io::Result<Option<i32>> {
    let path = scratch(spec);
    if SEEN.lock().unwrap().len() == 1 {
        *FIRST.lock().unwrap() = std::fs::read_to_string(&path)?;
        std::fs::write(&path, "{ not json")?;
    } else {
        std::fs::write(&path, FIRST.lock().unwrap().replace("Moved", "Other"))?;
    }
    Ok(Some(0))
}

fn editor_app(data: &Path, runner: suspend::ChildRunner) -> App {
    suspend::set_env_source(isolated_env);
    let mut c = ai_config(data);
    c.tui.editor = exe();
    let mut app = common::new_app(c, data);
    app.child_runner = runner;
    app
}

#[test]
fn editor_round_trip_replans() {
    match relaunch_fake(module_path!(), "editor_round_trip_replans", "move.toml", files) {
        Run::Child(data) => editor_round_trip(data),
        Run::Parent(r) => {
            assert!(r.data.join("Other").join("a.txt").exists());
            let tmp = r.config_dir().join("tmp");
            let left = std::fs::read_dir(&tmp).map(|d| d.count()).unwrap_or(0);
            assert_eq!(left, 0, "the scratch file is gone");
        }
    }
}

fn editor_round_trip(data: PathBuf) {
    let mut app = editor_app(&data, edit_to_other);
    let mut steps = Vec::new();
    ask(&mut steps, &data, "move it");
    steps.push(Step::new("a plan card waits", card_waits, key(KeyCode::Tab)));
    steps.push(Step::new("the cards", |_| true, key_mods(KeyCode::Char('E'), KeyModifiers::SHIFT)));
    steps.push(Step::new(
        "the editor ran and the edited plan waits",
        |a| SEEN.lock().unwrap().len() == 1 && edited(a) && card_waits(a) && panel_text(a).contains("Other"),
        key(KeyCode::Char('a')),
    ));
    let d = data.clone();
    steps.push(Step::new(
        "moved",
        move |a| d.join("Other").join("a.txt").exists() && ended(a) && common::wait_jobs_idle(a),
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 120, 30);
    let seen = SEEN.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let file = PathBuf::from(seen[0].args.last().unwrap());
    assert_eq!(file.extension().and_then(|e| e.to_str()), Some("json"));
    assert!(file.starts_with(soyuz_core::config::config_dir().join("tmp")), "{file:?}");
    assert!(!file.exists(), "read back and removed");
    app.finish();
    common::ok();
}

#[test]
fn editor_error_reopens_the_dialog() {
    match relaunch_fake(module_path!(), "editor_error_reopens_the_dialog", "move.toml", files) {
        Run::Child(data) => editor_error(data),
        Run::Parent(r) => assert!(r.data.join("Other").join("a.txt").exists()),
    }
}

fn editor_error(data: PathBuf) {
    let mut app = editor_app(&data, break_then_fix);
    let mut steps = Vec::new();
    ask(&mut steps, &data, "move it");
    steps.push(Step::new("a plan card waits", card_waits, key(KeyCode::Tab)));
    steps.push(Step::new("the cards", |_| true, key_mods(KeyCode::Char('E'), KeyModifiers::SHIFT)));
    // The broken edit comes back in the JSON editor with the error.
    steps.push(Step::new(
        "the JSON editor with the error",
        |a| {
            json_open(a)
                && dialog(a).is_some_and(|d| d.error.is_some() && d.text.iter().any(|l| l.contains("not json")))
                && !edited(a)
        },
        ctrl('e'),
    ));
    // Ctrl+E in the dialog: the editor again; this time it is valid and is sent.
    steps.push(Step::new(
        "the fixed edit was sent",
        |a| SEEN.lock().unwrap().len() == 2 && dialog(a).is_none() && edited(a) && card_waits(a),
        key(KeyCode::Char('a')),
    ));
    let d = data.clone();
    steps.push(Step::new(
        "moved",
        move |a| d.join("Other").join("a.txt").exists() && ended(a) && common::wait_jobs_idle(a),
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 120, 30);
    app.finish();
    common::ok();
}

// ---- smart search ---------------------------------------------------------------------

#[test]
fn smart_search_card_runs_and_asks() {
    match relaunch_fake(module_path!(), "smart_search_card_runs_and_asks", "chat.toml", |d| {
        std::fs::write(d.join("a.pdf"), b"%PDF").unwrap();
        std::fs::write(d.join("b.txt"), b"b").unwrap();
    }) {
        Run::Child(data) => smart(data),
        Run::Parent(_) => {}
    }
}

fn footer_text(a: &App) -> String {
    a.view().map(|v| v.ai.footer.iter().map(|l| l.text()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

fn smart(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let d = data.clone();
    let mut steps = vec![Step::new(
        "AiSmartSearch opened the card with the keyboard in it",
        move |a| {
            at(a, &d)
                && a.focus() == TuiFocus::Ai
                && a.core.ai.smart.open
                && footer_text(a).contains("Smart search")
                && a.view().is_some_and(|v| v.ai.footer_cursor.is_some())
        },
        key(KeyCode::Char('p')),
    )];
    type_text(&mut steps, "df");
    steps.push(Step::new(
        "the words became a filter",
        |a| a.core.ai.smart.query_text.contains("pdf") && footer_text(a).contains("pdf x]"),
        key_mods(KeyCode::Enter, KeyModifiers::ALT),
    ));
    steps.push(Step::new(
        "Alt+Enter asked the assistant",
        |a| ended(a) && user_texts(a).iter().any(|t| t.contains("pdf")) && a.core.ai.smart.open,
        key(KeyCode::Enter),
    ));
    steps.push(Step::new(
        "Enter ran the search in this folder",
        |a| {
            matches!(a.core.ws.pane().location, soyuz_core::Location::Search(_))
                && common::labels(a).iter().any(|l| l.contains("a.pdf"))
                && !common::labels(a).iter().any(|l| l.contains("b.txt"))
        },
        key(KeyCode::Esc),
    ));
    steps.push(Step::new("Esc closed the card", |a| !a.core.ai.smart.open && a.focus() == TuiFocus::Ai, alt('s')));
    steps.push(Step::new("Alt+S opened it again", |a| a.core.ai.smart.open, ctrl('q')));
    app.core.cmds.push(Cmd::Action(Action::AiSmartSearch));
    common::run_script(&mut app, steps, 120, 30);
    app.finish();
    common::ok();
}

// ---- chips and menus ------------------------------------------------------------------

#[test]
fn chips_open_menus() {
    match relaunch_fake(module_path!(), "chips_open_menus", "chat.toml", |d| {
        std::fs::write(d.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => chips(data),
        Run::Parent(_) => {}
    }
}

/// Index of the open chip menu's entry `label`.
fn menu_entry(a: &App, label: &str) -> Option<u16> {
    let menu = a.view()?.ai.menu.as_ref()?;
    menu.items
        .iter()
        .position(|i| matches!(i, soyuz_tui::widgets::MenuItem::Entry(e) if e.label == label))
        .map(|i| i as u16)
}

fn chips(data: PathBuf) {
    use soyuz_tui::hits::AiButton;
    let mut app = common::new_app(ai_config(&data), &data);
    let d = data.clone();
    let steps = vec![
        DynStep::when("the listing", move |a| at(a, &d), ctrl('i')),
        // The model chip opens the model menu; "Fast" there switches the route.
        DynStep::new("the model chip", |a| {
            if a.focus() != TuiFocus::Ai {
                return None;
            }
            click_on(a, |t| t == HitTarget::AiButton(AiButton::Profile))
        }),
        DynStep::new("Fast", |a| {
            let i = menu_entry(a, "Fast")?;
            click_on(a, |t| t == HitTarget::MenuItem { depth: 0, index: i as usize })
        }),
        // Alt+F: the folder access menu; the third entry is "contents".
        DynStep::when(
            "fast, menu closed",
            |a| {
                a.core.ai.panel.route == soyuz_appcore::ai::model::Route::Fast
                    && a.view().is_some_and(|v| v.ai.menu.is_none())
            },
            alt('f'),
        ),
        DynStep::when(
            "the access menu",
            |a| menu_entry(a, "Contents allowed (may be sent to the model)").is_some(),
            key(KeyCode::Down),
        ),
        DynStep::when("down", |_| true, key(KeyCode::Down)),
        DynStep::when("down", |_| true, key(KeyCode::Enter)),
        // Alt+O: the tools menu; Esc closes it.
        DynStep::when(
            "the folder's access is set",
            |a| a.core.config.ai.folders.iter().any(|r| r.access == AiAccess::Contents),
            alt('o'),
        ),
        DynStep::when("the tools menu", |a| menu_entry(a, "Manage connections…").is_some(), key(KeyCode::Esc)),
        DynStep::when("closed", |a| a.view().is_some_and(|v| v.ai.menu.is_none()), ctrl('q')),
    ];
    run_dyn(&mut app, steps, 120, 30);
    app.finish();
    common::ok();
}

// ---- Esc and Ctrl+I from the file list ------------------------------------------------

#[test]
fn esc_stops_and_ctrl_i_focuses_from_the_list() {
    match relaunch_fake(module_path!(), "esc_stops_and_ctrl_i_focuses_from_the_list", "move.toml", files) {
        Run::Child(data) => esc_from_list(data),
        Run::Parent(r) => assert!(r.data.join("a.txt").exists(), "stopped before anything ran"),
    }
}

fn esc_from_list(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    // A task started with the panel open but without the keyboard.
    app.core.ai.panel.open = true;
    app.core.ai.panel.composer = "move it".into();
    soyuz_tui::input::ai::apply(&mut app, vec![PanelAct::Send]);
    let steps = vec![
        Step::new(
            "the plan waits, the list has the keyboard",
            |a| card_waits(a) && a.focus() == TuiFocus::List,
            key(KeyCode::Esc),
        ),
        Step::new(
            "Esc in the list stopped the task; the panel stays",
            |a| ended(a) && task_end(a).is_some_and(|t| matches!(t.reason, Finish::Cancelled)) && a.core.ai.panel.open,
            ctrl('i'),
        ),
        Step::new(
            "Ctrl+I gave the open panel the keyboard",
            |a| a.core.ai.panel.open && a.focus() == TuiFocus::Ai,
            ctrl('i'),
        ),
        Step::new("Ctrl+I in the panel closes it", |a| !a.core.ai.panel.open && a.focus() == TuiFocus::List, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 120, 30);
    assert!(app.core.fileops.jobs.history().is_empty());
    app.finish();
    common::ok();
}

// ---- small screens --------------------------------------------------------------------

#[test]
fn tiny_sizes_with_the_composer_card_and_json_editor() {
    match relaunch_fake(module_path!(), "tiny_sizes_with_the_composer_card_and_json_editor", "move.toml", files) {
        Run::Child(data) => tiny(data),
        Run::Parent(_) => {}
    }
}

fn tiny(data: PathBuf) {
    let mut app = common::new_app(ai_config(&data), &data);
    let mut steps = Vec::new();
    ask(&mut steps, &data, "move it");
    steps.push(Step::new("a plan card waits", card_waits, ctrl('q')));
    common::run_script(&mut app, steps, 120, 30);
    // The JSON editor of the waiting card on top.
    let card = app
        .core
        .ai
        .chat
        .iter()
        .find_map(|c| match c {
            ChatItem::Plan(p) if p.state == CardState::Pending => Some(p.as_ref().clone()),
            _ => None,
        })
        .expect("a waiting card");
    let sel = soyuz_tui::input::ai::Sel::Plan { id: card.id, task: card.task.0 };
    soyuz_tui::input::dialogs::plan_json::open(&mut app, sel, &card);
    // A long multi-line composer, the smart search card with filters, the JSON editor on
    // top: frames at every small size.
    app.core.ai.panel.composer = "日本語 line\n\u{1b}[2J two\nthree\nfour\nfive\nsix\nseven".into();
    app.core.ai.smart.open = true;
    app.core.ai.smart.input = "big pdfs from last week".into();
    app.core.ai.smart.query_text = "ext:pdf size:>1MB".into();
    app.core.ai.smart.pills = soyuz_appcore::ai::smart_search::parse_text("ext:pdf size:>1MB");
    let mut io = NullIo::default();
    for w in (0..=50).step_by(3) {
        for h in 0..=16 {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            app.frame(&mut terminal, &[], &mut io).expect("a frame");
            let screen = common::screen(&terminal);
            assert!(!screen.contains('\u{1b}'), "escape sequences are made safe");
        }
    }
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    app.frame(&mut terminal, &[], &mut io).expect("a frame");
    assert!(json_open(&app), "the JSON editor stayed open");
    app.core.ai.stop();
    app.finish();
    common::ok();
}
