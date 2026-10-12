//! Headless tests of T3: inline rename. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::view::rename::EditVm;

use crate::common::{self, Run, Step, at, ctrl, cursor, key, key_mods, labels, type_text, wait_jobs_idle};

/// The rename field drawn in the last frame.
fn field(app: &App) -> Option<EditVm> {
    app.view()?.focused_pane()?.rows().find_map(|r| r.edit.clone())
}

fn renaming(app: &App) -> bool {
    app.focus() == TuiFocus::Rename && field(app).is_some()
}

#[test]
fn inline_rename_renames_on_disk_and_undo_restores() {
    let run = common::relaunch(module_path!(), "inline_rename_renames_on_disk_and_undo_restores", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.txt"), b"b").unwrap();
    });
    match run {
        Run::Child(data) => rename_and_undo(data),
        Run::Parent(p) => {
            assert!(p.data.join("a.txt").is_file(), "undo restored a.txt");
            assert!(!p.data.join("x.txt").exists());
            assert!(p.data.join("b.txt").is_file(), "the cancelled rename kept b.txt");
        }
    }
}

fn rename_and_undo(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let (x, a, a2) = (data.join("x.txt"), data.join("a.txt"), data.join("a.txt"));
    let mut steps = vec![
        Step::new("the listing", move |a| at(a, &d) && labels(a) == ["a.txt", "b.txt"], key(KeyCode::Down)),
        Step::new("the cursor on a.txt", |a| cursor(a).as_deref() == Some("a.txt"), key(KeyCode::F(2))),
        // The stem is selected: typing replaces it and keeps the extension.
        Step::new(
            "the field with the stem selected",
            |a| renaming(a) && field(a).is_some_and(|f| f.text == "a.txt" && f.sel == Some((0, 1))),
            key(KeyCode::Char('x')),
        ),
        Step::new("the new name", |a| field(a).is_some_and(|f| f.text == "x.txt"), key(KeyCode::Enter)),
        Step::new("the rename on disk", move |a| x.is_file() && !a.fileops_renaming() && wait_jobs_idle(a), ctrl('z')),
        Step::new("undo restored a.txt", move |a| a2.is_file() && wait_jobs_idle(a), key(KeyCode::End)),
        Step::new("the cursor on b.txt", |a| cursor(a).as_deref() == Some("b.txt"), key(KeyCode::F(2))),
        Step::new("the field on b.txt", renaming, ctrl('a')),
    ];
    type_text(&mut steps, "a.txt");
    steps.extend([
        Step::new(
            "the error inline",
            |a| field(a).is_some_and(|f| f.text == "a.txt" && f.error.is_some()),
            key(KeyCode::Enter),
        ),
        Step::new(
            "Enter kept the invalid name in the field",
            |a| renaming(a) && field(a).is_some_and(|f| f.error.is_some()),
            key(KeyCode::Esc),
        ),
        Step::new("Esc cancelled", move |a| !a.fileops_renaming() && a.focus() == TuiFocus::List, ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 80, 24);
    assert!(a.is_file() && data.join("b.txt").is_file());
    common::ok();
}

#[test]
fn new_folder_ends_in_inline_rename() {
    let run = common::relaunch(module_path!(), "new_folder_ends_in_inline_rename", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    });
    match run {
        Run::Child(data) => new_folder(data),
        Run::Parent(p) => {
            assert!(p.data.join("photos").is_dir(), "the new folder got its name");
        }
    }
}

fn new_folder(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let photos = data.join("photos");
    let mut steps = vec![
        Step::new(
            "the listing",
            move |a| at(a, &d) && labels(a) == ["a.txt"],
            key_mods(KeyCode::Char('N'), KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        ),
        // A folder has no extension: the whole name is selected, typing replaces it.
        Step::new(
            "the new folder in the rename field",
            |a| renaming(a) && field(a).is_some_and(|f| f.sel == Some((0, f.text.len()))),
            key(KeyCode::Char('p')),
        ),
    ];
    type_text(&mut steps, "hotos");
    steps.extend([
        Step::new("the typed name", |a| field(a).is_some_and(|f| f.text == "photos"), key(KeyCode::Enter)),
        Step::new("the folder renamed", move |a| photos.is_dir() && wait_jobs_idle(a), ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

/// `App` helpers for these tests.
trait Renaming {
    fn fileops_renaming(&self) -> bool;
}

impl Renaming for App {
    fn fileops_renaming(&self) -> bool {
        self.core.fileops.rename.is_some()
    }
}
