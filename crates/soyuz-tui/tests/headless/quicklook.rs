//! Headless tests of T4: Quick Look. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! `QuickLook` is Space in the file list (the Explorer preset).

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::view::quicklook::QuickLookVm;

use crate::common::{self, Run, Step, ctrl, key};

fn vm(app: &App) -> Option<&QuickLookVm> {
    app.view().map(|v| &v.quicklook).filter(|q| q.visible)
}

fn shows(app: &App, title: &str, needle: &str) -> bool {
    vm(app).is_some_and(|q| q.title == title && q.body_text().iter().any(|l| l.contains(needle)))
}

fn has_file(app: &App, name: &str) -> bool {
    common::labels(app).iter().any(|l| l == name)
}

fn files(data: &std::path::Path) {
    std::fs::write(data.join("a.txt"), "alpha content\n").unwrap();
    std::fs::write(data.join("b.txt"), "bravo content\n").unwrap();
    std::fs::write(data.join("c.txt"), "charlie content\n").unwrap();
}

#[test]
fn space_opens_right_steps_esc_closes() {
    let run = common::relaunch(module_path!(), "space_opens_right_steps_esc_closes", files);
    if let Run::Child(data) = run {
        keys(data);
    }
}

fn keys(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let steps = vec![
        Step::new("the listing", |a| has_file(a, "a.txt"), key(KeyCode::Home)),
        Step::new("the cursor on a.txt", |a| common::cursor(a).as_deref() == Some("a.txt"), key(KeyCode::Char(' '))),
        Step::new(
            "Quick Look on a.txt with its content",
            |a| {
                shows(a, "a.txt", "alpha content")
                    && a.focus() == TuiFocus::QuickLook
                    && vm(a).is_some_and(|q| q.position == "1 / 3" && !q.size.is_empty())
                    && common::toasts(a).is_empty()
            },
            key(KeyCode::Right),
        ),
        Step::new(
            "the next file, the pane's cursor follows",
            |a| shows(a, "b.txt", "bravo content") && common::cursor(a).as_deref() == Some("b.txt"),
            key(KeyCode::End),
        ),
        Step::new(
            "End: the last file",
            |a| shows(a, "c.txt", "charlie content") && vm(a).is_some_and(|q| q.position == "3 / 3"),
            key(KeyCode::Up),
        ),
        Step::new("Up: back one", |a| shows(a, "b.txt", "bravo content"), key(KeyCode::Esc)),
        Step::new(
            "closed, the list has the keyboard",
            |a| {
                vm(a).is_none()
                    && a.focus() == TuiFocus::List
                    && !a.core.ui.quicklook.open
                    && common::cursor(a).as_deref() == Some("b.txt")
            },
            key(KeyCode::Down),
        ),
        Step::new("the list's keys work again", |a| common::cursor(a).as_deref() == Some("c.txt"), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE })
}

#[test]
fn mouse_steps_and_closes() {
    let run = common::relaunch(module_path!(), "mouse_steps_and_closes", files);
    if let Run::Child(data) = run {
        clicks(data);
    }
}

fn clicks(data: PathBuf) {
    let mut c = common::test_config();
    c.tui.mouse = true;
    let mut app = common::new_app(c, &data);
    let left = MouseEventKind::Down(MouseButton::Left);
    // 80 × 24: the card is at (2, 1)..(78, 23), its title row is row 2 with `‹` at
    // column 3, `›` at 73 and `×` at 76.
    let steps = vec![
        Step::new("the listing", |a| has_file(a, "a.txt"), key(KeyCode::Home)),
        Step::new("the cursor on a.txt", |a| common::cursor(a).as_deref() == Some("a.txt"), key(KeyCode::Char(' '))),
        Step::new("open", |a| shows(a, "a.txt", "alpha"), mouse(left, 73, 2)),
        Step::new("› steps", |a| shows(a, "b.txt", "bravo"), mouse(left, 3, 2)),
        Step::new("‹ steps back", |a| shows(a, "a.txt", "alpha"), mouse(left, 40, 10)),
        Step::new("a click inside keeps it open", |a| shows(a, "a.txt", "alpha"), mouse(left, 0, 0)),
        Step::new(
            "a click outside closes",
            |a| vm(a).is_none() && a.focus() == TuiFocus::List,
            key(KeyCode::Char(' ')),
        ),
        Step::new("open again", |a| shows(a, "a.txt", "alpha"), mouse(left, 76, 2)),
        Step::new("× closes", |a| vm(a).is_none() && !a.core.ui.quicklook.open, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn enter_opens_a_folder() {
    let run = common::relaunch(module_path!(), "enter_opens_a_folder", |data| {
        std::fs::create_dir(data.join("sub")).unwrap();
        std::fs::write(
            data.join("sub").join("inner.txt"),
            "inside
",
        )
        .unwrap();
        std::fs::write(
            data.join("z.txt"),
            "z
",
        )
        .unwrap();
    });
    if let Run::Child(data) = run {
        enter(data);
    }
}

fn enter(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let sub = data.join("sub");
    let steps = vec![
        Step::new("the listing", |a| has_file(a, "z.txt"), key(KeyCode::Home)),
        Step::new("the cursor on sub", |a| common::cursor(a).as_deref() == Some("sub/"), key(KeyCode::Char(' '))),
        Step::new("the folder's summary", |a| vm(a).is_some_and(|q| q.title == "sub"), key(KeyCode::Enter)),
        Step::new(
            "Enter closes and opens the folder",
            move |a| vm(a).is_none() && common::at(a, &sub) && has_file(a, "inner.txt"),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}
