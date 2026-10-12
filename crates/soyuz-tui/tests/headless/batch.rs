//! Headless tests of T3: batch rename. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_tui::app::App;

use crate::common::{self, Run, Step, at, ctrl, dialog_title, key, key_mods, labels, wait_jobs_idle};

/// The preview rows of the dialog drawn in the last frame.
fn preview(app: &App) -> Vec<String> {
    app.view()
        .and_then(|v| v.dialog.as_ref())
        .map(|d| d.list.iter().map(|r| r.text.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn batch_rename_numbers_the_items_in_one_undoable_job() {
    let run = common::relaunch(module_path!(), "batch_rename_numbers_the_items_in_one_undoable_job", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.txt"), b"b").unwrap();
    });
    match run {
        Run::Child(data) => numbering(data),
        Run::Parent(p) => {
            assert!(p.data.join("a.txt").is_file() && p.data.join("b.txt").is_file(), "undo restored both");
            assert!(!p.data.join("a 01.txt").exists());
        }
    }
}

fn numbering(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let (a1, b2) = (data.join("a 01.txt"), data.join("b 02.txt"));
    let (a, b) = (data.join("a.txt"), data.join("b.txt"));
    let mut steps = vec![Step::new(
        "the listing",
        move |a| at(a, &d) && labels(a) == ["a.txt", "b.txt"],
        key_mods(KeyCode::Char('R'), KeyModifiers::CONTROL | KeyModifiers::SHIFT),
    )];
    // Nothing selected: every item. Tab to "Numbering" (the 7th field), then End.
    steps.push(Step::new(
        "the batch rename dialog",
        |a| dialog_title(a).is_some_and(|t| t.contains('2')) && preview(a).len() == 2,
        key(KeyCode::Tab),
    ));
    for _ in 0..5 {
        steps.push(Step::new("tabbing", |_| true, key(KeyCode::Tab)));
    }
    steps.extend([
        Step::new("on Numbering", |_| true, key(KeyCode::Right)),
        Step::new("numbering at the start", |a| preview(a).iter().any(|r| r.contains("01 a.txt")), key(KeyCode::Right)),
        Step::new(
            "the preview",
            |a| preview(a) == ["a.txt \u{2192} a 01.txt", "b.txt \u{2192} b 02.txt"],
            key(KeyCode::Enter),
        ),
        Step::new(
            "both renamed",
            move |a| a1.is_file() && b2.is_file() && dialog_title(a).is_none() && wait_jobs_idle(a),
            ctrl('z'),
        ),
        Step::new("one undo restored both", move |app| a.is_file() && b.is_file() && wait_jobs_idle(app), ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[test]
fn batch_rename_highlights_conflicts() {
    let run = common::relaunch(module_path!(), "batch_rename_highlights_conflicts", |data| {
        std::fs::write(data.join("x1.txt"), b"a").unwrap();
        std::fs::write(data.join("x2.txt"), b"b").unwrap();
    });
    match run {
        Run::Child(data) => conflicts(data),
        Run::Parent(p) => {
            assert!(p.data.join("x1.txt").is_file() && p.data.join("x2.txt").is_file(), "nothing renamed");
        }
    }
}

fn conflicts(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let mut steps = vec![
        Step::new(
            "the listing",
            move |a| at(a, &d) && labels(a).len() == 2,
            key_mods(KeyCode::Char('R'), KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        ),
        Step::new("the dialog", |a| preview(a).len() == 2, key(KeyCode::Tab)),
    ];
    // Find "1" → "2": x1.txt would become x2.txt, which x2.txt keeps.
    common::type_text(&mut steps, "1");
    steps.push(Step::new("Find typed", |_| true, key(KeyCode::Tab)));
    common::type_text(&mut steps, "2");
    steps.extend([
        Step::new(
            "the conflict flagged and the button off",
            |a| {
                let Some(d) = a.view().and_then(|v| v.dialog.as_ref()) else { return false };
                d.list.first().is_some_and(|r| r.role == Some(soyuz_tui::theme::Role::Danger) && !r.right.is_empty())
                    && d.buttons.first().is_some_and(|b| !b.enabled)
            },
            key(KeyCode::Enter),
        ),
        Step::new("Enter did nothing", |a| dialog_title(a).is_some(), key(KeyCode::Esc)),
        Step::new("closed", |a| dialog_title(a).is_none(), ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}
