//! Headless tests of Phase T7 (agent A): the drop shelf dialog. Each test runs in its own
//! sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`, like the
//! other files here.
//!
//! Items reach the shelf the way "Add to shelf" puts them there (`ToolCmd::ShelfAdd`, which
//! also turns the shelf on); the dialog opens by itself and copies / moves into the
//! focused pane's folder through the jobs engine.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::models::tools::{ToolCmd, queue, shelf_open, shelf_paths};
use soyuz_appcore::tr;
use soyuz_tui::app::App;
use soyuz_tui::layout::HitTarget;

use super::workspaces::{no_dialog, row_cursor, rows};
use crate::common::{self, Run, Step, ctrl, dialog_title, key, wait_jobs_idle};

fn items(data: &Path) {
    std::fs::write(data.join("a.txt"), b"a").unwrap();
    std::fs::write(data.join("b.txt"), b"b").unwrap();
    std::fs::create_dir_all(data.join("dest")).unwrap();
}

fn button_at(a: &App, i: usize) -> Option<(u16, u16)> {
    a.hits().iter().find(|(_, t)| *t == HitTarget::DialogButton(i)).map(|(r, _)| (r.x + 2, r.y))
}

fn row_at(a: &App, i: usize) -> Option<(u16, u16)> {
    a.hits().iter().find(|(_, t)| *t == HitTarget::DialogRow(i)).map(|(r, _)| (r.x + 2, r.y))
}

fn press(x: u16, y: u16, modifiers: KeyModifiers) -> Event {
    Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers })
}

fn shelf_shown(a: &App) -> bool {
    dialog_title(a).as_deref() == Some(tr!("tools-shelf-title"))
}

#[test]
fn shelf_copies_moves_removes_and_clears() {
    match common::relaunch(module_path!(), "shelf_copies_moves_removes_and_clears", items) {
        Run::Child(data) => shelf(data),
        Run::Parent(r) => {
            let dest = r.data.join("dest");
            assert!(dest.join("a.txt").is_file() && r.data.join("a.txt").is_file(), "a copied");
            assert!(dest.join("b.txt").is_file() && !r.data.join("b.txt").exists(), "b moved");
        }
    }
}

fn shelf(data: PathBuf) {
    let mut c = common::test_config();
    c.tui.mouse = true;
    let dest = data.join("dest");
    let mut app = common::new_app(c, &dest);
    let (a_txt, b_txt) = (data.join("a.txt"), data.join("b.txt"));
    queue(ToolCmd::ShelfAdd(vec![a_txt.clone(), b_txt.clone(), data.join("gone.txt")]));
    let missing = tr!("tools-shelf-missing").to_string();
    let mut typing: VecDeque<Event> = VecDeque::new();
    let mut phase = 0;
    super::menu::drive(&mut app, |a| {
        if let Some(e) = typing.pop_front() {
            return Some(e);
        }
        let view = a.view().and_then(|v| v.dialog.as_ref());
        match phase {
            // Three items, the missing one marked; Space marks a.txt.
            0 if shelf_shown(a)
                && rows(a).len() == 3
                && view.is_some_and(|d| d.list[2].right == missing && d.list[0].right.contains("data")) =>
            {
                phase = 1;
                Some(key(KeyCode::Char(' ')))
            }
            // A click on Copy here copies the marked item.
            1 if rows(a).first().is_some_and(|r| r.starts_with("[x] a.txt")) => {
                let (x, y) = button_at(a, 0)?;
                phase = 2;
                Some(press(x, y, KeyModifiers::NONE))
            }
            // Ctrl+click marks b.txt instead, Move here moves it (and takes it off).
            2 if dest.join("a.txt").is_file() && wait_jobs_idle(a) => {
                let (x, y) = row_at(a, 0)?;
                let (bx, by) = row_at(a, 1)?;
                phase = 3;
                typing.push_back(press(bx, by, KeyModifiers::CONTROL));
                Some(press(x, y, KeyModifiers::CONTROL))
            }
            3 if rows(a).get(1).is_some_and(|r| r.starts_with("[x] b.txt")) && rows(a)[0].starts_with("[ ] a.txt") => {
                let (x, y) = button_at(a, 1)?;
                phase = 4;
                Some(press(x, y, KeyModifiers::NONE))
            }
            // Del removes the cursor row (the missing item).
            4 if dest.join("b.txt").is_file() && wait_jobs_idle(a) && rows(a).len() == 2 => {
                phase = 5;
                typing.push_back(key(KeyCode::Delete));
                Some(key(KeyCode::End))
            }
            5 if rows(a).len() == 1 && shelf_paths() == [a_txt.clone()] => {
                let (x, y) = button_at(a, 3)?;
                phase = 6;
                Some(press(x, y, KeyModifiers::NONE))
            }
            // Cleared: the empty text; Esc closes and turns the shelf off.
            6 if rows(a).is_empty() && shelf_paths().is_empty() && view.is_some_and(|d| !d.body.is_empty()) => {
                phase = 7;
                Some(key(KeyCode::Esc))
            }
            7 if no_dialog(a) && !shelf_open() => {
                phase = 8;
                Some(ctrl('q'))
            }
            _ => None,
        }
    });
    common::ok();
}

#[test]
fn toggle_shelf_opens_it_and_enter_goes_to_an_item() {
    match common::relaunch(module_path!(), "toggle_shelf_opens_it_and_enter_goes_to_an_item", items) {
        Run::Child(data) => toggle(data),
        Run::Parent(_) => {}
    }
}

fn toggle(data: PathBuf) {
    let dest = data.join("dest");
    let mut app = common::new_app(common::test_config(), &dest);
    soyuz_appcore::models::tools::restore_shelf(vec![data.join("b.txt")]);
    assert!(!shelf_open(), "restored closed");
    let mut steps = Vec::new();
    super::workspaces::palette(
        &mut steps,
        |a| a.view().is_some() && no_dialog(a),
        "toggle shelf",
        soyuz_core::config::Action::ToggleShelf,
    );
    steps.push(Step::new(
        "the shelf with b.txt",
        |a| shelf_shown(a) && rows(a).len() == 1 && row_cursor(a) == Some(0),
        key(KeyCode::Enter),
    ));
    let d = data.clone();
    steps.push(Step::new(
        "the pane shows b.txt's folder with b.txt on the cursor; the shelf is off",
        move |a| {
            no_dialog(a) && common::at(a, &d) && common::cursor(a).is_some_and(|c| c.contains("b.txt")) && !shelf_open()
        },
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn the_shelf_works_at_tiny_sizes() {
    match common::relaunch(module_path!(), "the_shelf_works_at_tiny_sizes", items) {
        Run::Child(data) => tiny(data),
        Run::Parent(_) => {}
    }
}

fn tiny(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    queue(ToolCmd::ShelfAdd(vec![data.join("a.txt"), data.join("b.txt")]));
    let steps = vec![
        Step::new("the shelf on a tiny screen", |a| shelf_shown(a) && rows(a).len() == 2, key(KeyCode::Down)),
        Step::new("moved", |a| row_cursor(a) == Some(1), key(KeyCode::Esc)),
        Step::new("closed", |a| no_dialog(a) && !shelf_open(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 22, 7);
    common::ok();
}
