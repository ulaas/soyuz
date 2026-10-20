//! Headless tests of step T2.8: the status line and the filter field. Each test runs in
//! its own sandbox (`crate::common::relaunch`).

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use soyuz_core::config::TypeAhead;
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;

use crate::common::{self, Run, Step, at, ctrl, cursor, key, labels};

fn files(data: &Path, names: &[&str]) {
    for n in names {
        std::fs::write(data.join(n), n.as_bytes()).unwrap();
    }
}

/// The focused pane's filter text.
fn filter(a: &App) -> String {
    a.core.ws.pane().filter.clone()
}

/// The status line's filter field: its text and cursor column, as last drawn.
fn field(a: &App) -> Option<(String, Option<u16>)> {
    a.view()?.status.filter.as_ref().map(|f| (f.text.clone(), f.cursor))
}

#[test]
fn filter_action_narrows_and_esc_clears() {
    match common::relaunch(module_path!(), "filter_action_narrows_and_esc_clears", |data| {
        files(data, &["a.txt", "b.txt", "bb.txt", "c.txt"]);
    }) {
        Run::Child(data) => filter_action(data),
        Run::Parent(_) => {}
    }
}

fn filter_action(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let steps = vec![
        Step::new(
            "the listing",
            move |a| at(a, &d) && labels(a) == ["a.txt", "b.txt", "bb.txt", "c.txt"],
            // The Filter action (Ctrl+F) opens the field.
            ctrl('f'),
        ),
        Step::new(
            "the open filter field",
            |a| a.focus() == TuiFocus::Filter && field(a) == Some((String::new(), Some(0))),
            key(KeyCode::Char('b')),
        ),
        Step::new(
            "two matches",
            |a| filter(a) == "b" && labels(a) == ["b.txt", "bb.txt"] && field(a) == Some(("b".into(), Some(1))),
            key(KeyCode::Char('b')),
        ),
        Step::new("one match", |a| labels(a) == ["bb.txt"], key(KeyCode::Esc)),
        Step::new(
            "the cleared filter, the list focused",
            |a| a.focus() == TuiFocus::List && filter(a).is_empty() && labels(a).len() == 4 && field(a).is_none(),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn type_ahead_filter_and_enter_jumps_to_the_first_match() {
    match common::relaunch(module_path!(), "type_ahead_filter_and_enter_jumps_to_the_first_match", |data| {
        files(data, &["a.txt", "b1.txt", "b2.txt"]);
    }) {
        Run::Child(data) => type_ahead_filter(data),
        Run::Parent(_) => {}
    }
}

fn type_ahead_filter(data: PathBuf) {
    let mut c = common::test_config();
    c.behaviour.type_ahead = TypeAhead::Filter;
    let mut app = common::new_app(c, &data);
    let d = data.clone();
    let steps = vec![
        Step::new(
            "the listing without a cursor",
            move |a| at(a, &d) && labels(a) == ["a.txt", "b1.txt", "b2.txt"] && cursor(a).is_none(),
            // Type-ahead in filter mode: the letter goes into the filter, the field opens.
            key(KeyCode::Char('b')),
        ),
        Step::new(
            "the field with the cursor after the typed letter",
            |a| {
                a.focus() == TuiFocus::Filter
                    && filter(a) == "b"
                    && labels(a) == ["b1.txt", "b2.txt"]
                    && field(a) == Some(("b".into(), Some(1)))
            },
            key(KeyCode::Char('x')),
        ),
        Step::new("typed at the end: no match", |a| filter(a) == "bx" && labels(a).is_empty(), key(KeyCode::Backspace)),
        Step::new("two matches again", |a| filter(a) == "b" && labels(a).len() == 2, key(KeyCode::Enter)),
        Step::new(
            "the list focused on the first match, the filter kept",
            |a| {
                a.focus() == TuiFocus::List
                    && cursor(a).as_deref() == Some("b1.txt")
                    && filter(a) == "b"
                    && field(a) == Some(("b".into(), None))
            },
            ctrl('q'),
        ),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 80, 24);
    // The status line: the filter, the filtered count and the selection.
    let screen = common::screen(&terminal);
    let status = screen.lines().nth(23).unwrap_or_default();
    assert!(status.starts_with(" Filter b "), "{screen}");
    assert!(status.contains("2 of 3 items · 1 selected"), "{status:?}");
    assert!(status.contains("Name ↑"), "{status:?}");
    assert!(app.hits().iter().any(|(_, t)| *t == HitTarget::Filter));
    assert!(app.hits().iter().any(|(_, t)| *t == HitTarget::Status));
    common::ok();
}

#[test]
fn focus_mode_hides_the_chrome_and_brings_it_back() {
    match common::relaunch(module_path!(), "focus_mode_hides_the_chrome_and_brings_it_back", |data| {
        files(data, &["a.txt"]);
    }) {
        Run::Child(data) => focus_mode(data),
        Run::Parent(_) => {}
    }
}

fn focus_mode(data: PathBuf) {
    use crossterm::event::KeyModifiers;
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let shift_ctrl_b = || common::key_mods(KeyCode::Char('B'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    let steps = vec![
        Step::new("the listing with the status line", move |a| at(a, &d) && a.core.ui.status_visible, shift_ctrl_b()),
        Step::new(
            "focus mode: no sidebar, no status line",
            |a| a.core.ui.focus_mode.is_some() && !a.core.ui.sidebar_visible && !a.core.ui.status_visible,
            shift_ctrl_b(),
        ),
        Step::new("the status line back", |a| a.core.ui.focus_mode.is_none() && a.core.ui.status_visible, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}
