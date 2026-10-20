//! Headless tests of step T2.7: the command palette. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode};
use soyuz_appcore::palette::Exec;
use soyuz_core::config::Action;
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;

use crate::common::{self, Run, Step, at, ctrl, key};

fn open(a: &App) -> bool {
    a.core.palette.open && a.focus() == TuiFocus::Palette
}

fn selected(a: &App) -> Option<&Exec> {
    a.core.palette.results.get(a.core.palette.selected).map(|i| &i.exec)
}

/// The last frame drew the palette with `input`.
fn drawn_with(a: &App, input: &str) -> bool {
    a.view().is_some_and(|v| v.palette.open && v.palette.input == input)
}

#[test]
fn palette_runs_a_command() {
    match common::relaunch(module_path!(), "palette_runs_a_command", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => run_command(data),
        Run::Parent(_) => {}
    }
}

fn run_command(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    assert!(!app.core.ui.show_hidden);
    let d = data.clone();
    let mut steps = vec![Step::new("the listing", move |a| at(a, &d) && a.view().is_some(), ctrl('k'))];
    // Type "hidden" one key per frame, each after the previous one reached the field.
    let word = "hidden";
    for (i, c) in word.char_indices() {
        let typed = &word[..i];
        steps.push(Step::new(
            "the palette with the text so far",
            move |a| open(a) && drawn_with(a, typed),
            key(KeyCode::Char(c)),
        ));
    }
    steps.push(Step::new(
        "“hidden” ranked Toggle hidden first",
        |a| open(a) && drawn_with(a, "hidden") && selected(a) == Some(&Exec::Action(Action::ToggleHidden)),
        key(KeyCode::Enter),
    ));
    steps.push(Step::new(
        "hidden files shown and the palette closed",
        |a| a.core.ui.show_hidden && !a.core.palette.open && a.view().is_some_and(|v| !v.palette.open),
        ctrl('q'),
    ));
    let (terminal, _) = common::run_script(&mut app, steps, 100, 30);
    assert!(app.core.ui.show_hidden);
    // No "not available" toast: the palette has a view now.
    let screen = common::screen(&terminal);
    assert!(!screen.contains("Not available"), "{screen}");
    app.finish();
    common::ok();
}

#[test]
fn palette_goes_to_a_folder_and_esc_closes() {
    match common::relaunch(module_path!(), "palette_goes_to_a_folder_and_esc_closes", |data| {
        std::fs::create_dir_all(data.join("target dir").join("inner")).unwrap();
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => go_to_folder(data),
        Run::Parent(_) => {}
    }
}

fn go_to_folder(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let target = data.join("target dir");
    let typed = target.display().to_string();
    let (d, t1, t2, typed2) = (data.clone(), target.clone(), target.clone(), typed.clone());
    let steps = vec![
        // Ctrl+P: go to folder.
        Step::new("the listing", move |a| at(a, &d) && a.view().is_some(), ctrl('p')),
        Step::new(
            "the go-to-folder palette",
            |a| open(a) && a.core.palette.base == soyuz_appcore::palette::query::Mode::Folders && drawn_with(a, ""),
            Event::Paste(typed.clone()),
        ),
        Step::new(
            "the folder offered first",
            move |a| {
                open(a)
                    && drawn_with(a, &typed2)
                    && match selected(a) {
                        Some(Exec::Go(soyuz_core::Location::Dir(p))) => p == &t1,
                        Some(Exec::Complete { go: Some(soyuz_core::Location::Dir(p)), .. }) => p == &t1,
                        _ => false,
                    }
            },
            key(KeyCode::Enter),
        ),
        Step::new("the folder", move |a| at(a, &t2) && !a.core.palette.open, ctrl('k')),
        // Esc with text clears it, Esc on an empty input closes.
        Step::new("the command palette", |a| open(a) && drawn_with(a, ""), key(KeyCode::Char('x'))),
        Step::new("the typed x", |a| open(a) && drawn_with(a, "x"), key(KeyCode::Esc)),
        Step::new("the cleared input", |a| open(a) && drawn_with(a, ""), key(KeyCode::Esc)),
        Step::new(
            "the palette closed",
            |a| !a.core.palette.open && a.view().is_some_and(|v| !v.palette.open) && a.focus() == TuiFocus::List,
            ctrl('q'),
        ),
    ];
    let _ = common::run_script(&mut app, steps, 100, 30);
    assert!(at(&app, &target));
    app.finish();
    common::ok();
}

/// Welcome is a GUI card (`actions::gui_only`): the terminal UI's palette leaves it out,
/// while About (a TUI dialog since T7) stays.
#[test]
fn palette_hides_gui_only_actions() {
    match common::relaunch(module_path!(), "palette_hides_gui_only_actions", |_| {}) {
        Run::Child(data) => hides_gui_only(data),
        Run::Parent(_) => {}
    }
}

fn hides_gui_only(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    assert!(soyuz_appcore::actions::gui_only(&Action::Welcome));
    let offered = |q: &str| -> Vec<Exec> {
        soyuz_appcore::palette::rank_commands(&app.core, q).into_iter().map(|i| i.exec).collect()
    };
    assert!(!offered("").contains(&Exec::Action(Action::Welcome)));
    assert!(!offered("welcome").contains(&Exec::Action(Action::Welcome)));
    assert!(offered("about").contains(&Exec::Action(Action::About)));
    app.finish();
    common::ok();
}

#[test]
fn clipboard_history_copies_an_earlier_set_again() {
    match common::relaunch(module_path!(), "clipboard_history_copies_an_earlier_set_again", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.txt"), b"b").unwrap();
    }) {
        Run::Child(data) => clipboard_history(data),
        Run::Parent(_) => {}
    }
}

fn clipboard_history(data: PathBuf) {
    // Copies stay inside the app: the system clipboard is never touched.
    soyuz_appcore::services::os_clipboard::set_system_clipboard(false);
    let mut app = common::new_app(common::test_config(), &data);
    app.host.set_local_clipboard(false);
    let (a, b) = (data.join("a.txt"), data.join("b.txt"));
    let clip = |app: &App| app.core.fileops.clipboard.for_paste().map(|(_, p)| p);
    let (a1, b1, a2) = (a.clone(), b.clone(), a.clone());
    let d = data.clone();
    let alt_ctrl_v = common::key_mods(
        KeyCode::Char('v'),
        crossterm::event::KeyModifiers::CONTROL | crossterm::event::KeyModifiers::ALT,
    );
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d) && common::labels(a).len() == 2, key(KeyCode::Home)),
        Step::new("a.txt under the cursor", |a| common::cursor(a).as_deref() == Some("a.txt"), ctrl('c')),
        Step::new("a.txt copied", move |x| clip(x) == Some(vec![a1.clone()]), key(KeyCode::Down)),
        Step::new("b.txt under the cursor", |a| common::cursor(a).as_deref() == Some("b.txt"), ctrl('c')),
        Step::new("b.txt copied", move |x| clip(x) == Some(vec![b1.clone()]), alt_ctrl_v),
        Step::new(
            "the history, newest first",
            |a| open(a) && a.core.palette.results.len() == 2 && matches!(selected(a), Some(Exec::ClipSet(0))),
            key(KeyCode::Down),
        ),
        Step::new("the older set picked", |a| matches!(selected(a), Some(Exec::ClipSet(1))), key(KeyCode::Enter)),
        Step::new("a.txt on the clipboard again", move |x| !open(x) && clip(x) == Some(vec![a2.clone()]), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}
