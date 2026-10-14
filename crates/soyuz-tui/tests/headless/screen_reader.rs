//! Phase T8 (P3): screen-reader mode end to end: the palette toggle flips
//! `[tui] screen_reader` (saved), the look changes (no box drawing, one pane, the caret).

use std::path::PathBuf;

use crossterm::event::KeyCode;
use soyuz_appcore::palette::Exec;
use soyuz_core::config::{self, Action};
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;

use crate::common::{self, Run, Step, at, ctrl, key};

fn palette_open(a: &App) -> bool {
    a.core.palette.open && a.focus() == TuiFocus::Palette
}

fn drawn_with(a: &App, input: &str) -> bool {
    a.view().is_some_and(|v| v.palette.open && v.palette.input == input)
}

fn selected(a: &App) -> Option<&Exec> {
    a.core.palette.results.get(a.core.palette.selected).map(|i| &i.exec)
}

/// The palette's "Toggle screen-reader mode" turns the mode on and saves it; the screen
/// then has no box drawing, the status line reads one sentence (here the toast), the
/// terminal's cursor shows on the list's cursor row and the status line was rewritten in
/// full through the terminal's raw output.
#[test]
fn the_palette_toggle_turns_screen_reader_mode_on_and_saves_it() {
    match common::relaunch(module_path!(), "the_palette_toggle_turns_screen_reader_mode_on_and_saves_it", |data| {
        std::fs::create_dir(data.join("sub")).unwrap();
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => toggle_child(data),
        Run::Parent(_) => {}
    }
}

fn toggle_child(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    assert!(!app.screen_reader());
    let d = data.clone();
    let mut steps = vec![Step::new("the listing", move |a| at(a, &d) && a.view().is_some(), ctrl('k'))];
    let word = "screen-reader";
    for (i, c) in word.char_indices() {
        let typed = &word[..i];
        steps.push(Step::new(
            "the palette with the text so far",
            move |a| palette_open(a) && drawn_with(a, typed),
            key(KeyCode::Char(c)),
        ));
    }
    steps.push(Step::new(
        "Toggle screen-reader mode selected",
        |a| {
            palette_open(a)
                && drawn_with(a, "screen-reader")
                && selected(a) == Some(&Exec::Action(Action::ToggleScreenReader))
        },
        key(KeyCode::Enter),
    ));
    steps.push(Step::new(
        "screen-reader mode drawn",
        |a| {
            a.screen_reader()
                && !a.core.palette.open
                && a.regions().other_pane.is_empty()
                && a.view().is_some_and(|v| !v.palette.open && !v.toasts.lines.is_empty())
        },
        ctrl('q'),
    ));
    let (terminal, io) = common::run_script(&mut app, steps, 80, 16);
    assert!(app.core.config.tui.screen_reader, "the setting is on");
    let screen = common::screen(&terminal);
    assert!(!screen.chars().any(soyuz_tui::a11y::is_box_char), "{screen}");
    let lines: Vec<&str> = screen.lines().collect();
    // The toast reads in the status row, in full.
    assert!(lines[15].starts_with("Info: Screen-reader mode on"), "{screen}");
    // Folders end in `/`, no icons, the location on top.
    assert!(lines.iter().any(|l| l.starts_with("  sub/")), "{screen}");
    // The terminal's cursor is shown, on the list's cursor row.
    let b = terminal.backend();
    assert!(b.cursor_visible(), "{screen}");
    let row = b.cursor_position().y as usize;
    assert!(lines[row].contains("a.txt") || lines[row].contains("sub/"), "cursor on {row}: {screen}");
    // The status line was rewritten in full (save cursor, move, text, erase, restore).
    let written = String::from_utf8_lossy(&io.written);
    assert!(written.contains("\x1b7\x1b[16;1H\x1b[0mInfo: Screen-reader mode on\x1b[K\x1b8"), "{written:?}");
    app.finish();
    // Saved to config.toml.
    let saved = config::load(&config::default_path()).expect("config.toml").config;
    assert!(saved.tui.screen_reader, "saved");
    common::ok();
}

/// The run-only override (`--screen-reader`) draws the same look without touching the
/// setting: a dual layout shows only the focused pane plus the "Other pane" line, and
/// nothing is saved.
#[test]
fn the_run_override_draws_one_pane_and_is_never_saved() {
    match common::relaunch(module_path!(), "the_run_override_draws_one_pane_and_is_never_saved", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => override_child(data),
        Run::Parent(_) => {}
    }
}

fn override_child(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    app.sr_override = true;
    assert!(app.screen_reader() && !app.core.config.tui.screen_reader);
    let d = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d) && a.view().is_some(), key(KeyCode::F(9))),
        Step::new(
            "dual panes drawn as one",
            |a| a.core.ws.tab().shown() == 2 && a.view().is_some_and(|v| v.panes.len() == 1 && v.other_pane.is_some()),
            ctrl('q'),
        ),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 80, 16);
    let screen = common::screen(&terminal);
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[14].starts_with("Other pane: "), "{screen}");
    assert_eq!(app.regions().panes.len(), 1, "{screen}");
    assert!(app.regions().splitters.is_empty() && app.regions().titles.is_empty());
    // The status sentence (the new pane has no cursor row yet: no item leads it).
    assert!(lines[15].starts_with("1 item; ") && lines[15].contains("sorted by Name, ascending"), "{screen}");
    assert!(!screen.chars().any(soyuz_tui::a11y::is_box_char), "{screen}");
    app.finish();
    let saved = config::load(&config::default_path()).map(|l| l.config.tui.screen_reader).unwrap_or(false);
    assert!(!saved, "the override is never written");
    common::ok();
}

/// The keyboard tour (`headless::tour`) with `[tui] screen_reader = true`: every step still
/// works on the one-column, box-free screen.
#[test]
fn keyboard_tour_in_screen_reader_mode() {
    crate::headless::tour::run_tour(
        module_path!(),
        "keyboard_tour_in_screen_reader_mode",
        &crate::headless::tour::EN,
        soyuz_tui::overlay::Overlay::Defaults,
        |c| c.tui.screen_reader = true,
    );
}
