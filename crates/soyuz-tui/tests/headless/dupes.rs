//! Headless tests of Phase T7 (agent C): the duplicate finder screen. Each test runs in its
//! own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`, like
//! the other files here.
//!
//! `find_duplicates` is bound to F12 (it has no default chord). The trash is only asked
//! for, then cancelled: no trash job is ever submitted (the model's pure
//! `trash_request` is unit-tested in the core), so the real trash is never touched.

use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_core::config::Config;
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::view::screens::dupes::{DupesVm, LineKind, StatusVm};
use soyuz_tui::view::screens::{ScreenVm, ScreensVm};

use crate::common::{self, Run, Step, ctrl, key, key_mods};

const FILES: [&str; 4] = ["a.jpg", "b.jpg", "c.jpg", "other.jpg"];

fn config() -> Config {
    let mut c = common::test_config();
    c.keys.bindings.insert("find_duplicates".into(), vec!["F12".into()]);
    c
}

/// Three identical 200 KB files and one different one in `pics`.
fn fixture(data: &Path) {
    let pics = data.join("pics");
    std::fs::create_dir_all(&pics).unwrap();
    let same = vec![7u8; 200_000];
    for n in &FILES[..3] {
        std::fs::write(pics.join(n), &same).unwrap();
    }
    std::fs::write(pics.join("other.jpg"), vec![1u8; 200_000]).unwrap();
}

fn screens(a: &App) -> Option<&ScreensVm> {
    a.view().map(|v| &v.screens).filter(|s| s.visible)
}

fn vm(a: &App) -> Option<&DupesVm> {
    match &screens(a)?.screen {
        Some(ScreenVm::Dupes(d)) => Some(d.as_ref()),
        _ => None,
    }
}

/// The search ended with one group of three.
fn one_group(a: &App) -> bool {
    vm(a).is_some_and(|d| {
        matches!(d.status, StatusVm::Summary(_))
            && d.lines.iter().filter(|l| matches!(l.kind, LineKind::Header { .. })).count() == 1
            && d.lines.iter().filter(|l| matches!(l.kind, LineKind::File { .. })).count() == 3
    })
}

fn marked(a: &App) -> Option<usize> {
    vm(a).map(|d| d.marked)
}

fn all_there(data: &Path) {
    for n in FILES {
        assert!(data.join("pics").join(n).is_file(), "{n} is still there");
    }
}

#[test]
fn find_mark_refuse_and_cancel_the_trash() {
    match common::relaunch(module_path!(), "find_mark_refuse_and_cancel_the_trash", fixture) {
        Run::Child(data) => find(&data),
        Run::Parent(r) => all_there(&r.data),
    }
}

fn find(data: &Path) {
    let pics = data.join("pics");
    let mut app = common::new_app(config(), &pics);
    let p = pics.clone();
    let steps = vec![
        Step::new("the listing of pics", move |a| common::at(a, &p), key(KeyCode::F(12))),
        Step::new(
            "the screen found one group of three",
            |a| a.focus() == TuiFocus::Tool && one_group(a) && vm(a).is_some_and(|d| d.list_focused),
            key_mods(KeyCode::Char('N'), KeyModifiers::SHIFT),
        ),
        Step::new("Shift+N kept the newest in every group: two marked", |a| marked(a) == Some(2), key(KeyCode::Delete)),
        Step::new(
            "Del asks to move the two to the trash",
            |a| vm(a).and_then(|d| d.confirm.as_deref()).is_some_and(|q| q.contains('2')),
            key(KeyCode::Esc),
        ),
        Step::new(
            "Esc cancelled; C clears the marks",
            |a| vm(a).is_some_and(|d| d.confirm.is_none()) && marked(a) == Some(2),
            key(KeyCode::Char('c')),
        ),
        Step::new("no marks; down to the first file", |a| marked(a) == Some(0), key(KeyCode::Down)),
        Step::new(
            "the cursor on a file: Space marks it",
            |a| vm(a).is_some_and(|d| d.lines.iter().any(|l| l.cursor && matches!(l.kind, LineKind::File { .. }))),
            key(KeyCode::Char(' ')),
        ),
        Step::new("one marked", |a| marked(a) == Some(1), key(KeyCode::Down)),
        Step::new(
            "the second file",
            |a| vm(a).is_some_and(|d| d.lines.iter().any(|l| l.cursor && l.index == 2)),
            key(KeyCode::Char(' ')),
        ),
        Step::new("two marked", |a| marked(a) == Some(2), key(KeyCode::Down)),
        Step::new(
            "the last file",
            |a| vm(a).is_some_and(|d| d.lines.iter().any(|l| l.cursor && l.index == 3)),
            key(KeyCode::Char(' ')),
        ),
        Step::new(
            "the last copy is refused (keep one)",
            |a| {
                marked(a) == Some(2)
                    && vm(a)
                        .is_some_and(|d| d.lines.iter().any(|l| matches!(l.kind, LineKind::File { refused: true, .. })))
            },
            key(KeyCode::Up),
        ),
        Step::new("x asks for the trash too", |a| vm(a).is_some_and(|d| d.confirm.is_none()), key(KeyCode::Char('x'))),
        Step::new(
            "the confirm bar again; N cancels it",
            |a| vm(a).is_some_and(|d| d.confirm.is_some()),
            key(KeyCode::Char('n')),
        ),
        Step::new("cancelled; Esc closes", |a| vm(a).is_some_and(|d| d.confirm.is_none()), key(KeyCode::Esc)),
        Step::new(
            "closed; the pane has the keyboard",
            |a| screens(a).is_none() && a.focus() == TuiFocus::List,
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 110, 30);
    all_there(data);
    common::ok();
}

#[test]
fn short_screen_folds_the_setup() {
    match common::relaunch(module_path!(), "short_screen_folds_the_setup", fixture) {
        Run::Child(data) => short(&data),
        Run::Parent(r) => all_there(&r.data),
    }
}

fn short(data: &Path) {
    let pics = data.join("pics");
    let mut app = common::new_app(config(), &pics);
    let p = pics.clone();
    let steps = vec![
        Step::new("the listing of pics", move |a| common::at(a, &p), key(KeyCode::F(12))),
        Step::new(
            "one group; the setup is one summary row",
            |a| one_group(a) && vm(a).is_some_and(|d| !d.open && d.summary.contains("pics")),
            key(KeyCode::F(2)),
        ),
        Step::new(
            "F2 opens the setup on the roots",
            |a| vm(a).is_some_and(|d| d.open && d.root_cursor == Some(0)),
            key(KeyCode::Right),
        ),
        Step::new("Tab to the field", |a| vm(a).is_some_and(|d| d.open), key(KeyCode::Tab)),
        Step::new("the field has the keyboard", |a| vm(a).is_some_and(|d| d.add.cursor.is_some()), key(KeyCode::Tab)),
        Step::new("the sizes", |a| vm(a).is_some_and(|d| d.sizes_focused), key(KeyCode::Tab)),
        Step::new(
            "back on the list: the setup folds again",
            |a| vm(a).is_some_and(|d| !d.open && d.list_focused),
            key(KeyCode::Esc),
        ),
        Step::new("closed", |a| screens(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 44, 12);
    common::ok();
}

/// The n / o / s chips of a group header keep that rule in the group (mouse).
#[test]
fn header_chips_keep_in_the_group() {
    match common::relaunch(module_path!(), "header_chips_keep_in_the_group", fixture) {
        Run::Child(data) => header_chips(&data),
        Run::Parent(r) => all_there(&r.data),
    }
}

fn header_chips(data: &Path) {
    use soyuz_tui::layout::HitTarget;
    use soyuz_tui::view::screens::dupes::chip;

    use super::ai::{DynStep, click_on, run_dyn};

    let pics = data.join("pics");
    let mut app = common::new_app(config(), &pics);
    let p = pics.clone();
    let chip_at = |i: u8| move |a: &App| click_on(a, |t| t == HitTarget::ScreenChip(chip::KEEP_IN + i));
    let steps = vec![
        DynStep::when("the listing of pics", move |a| common::at(a, &p), key(KeyCode::F(12))),
        DynStep::new("one group: click its n chip", move |a| {
            if one_group(a) && marked(a) == Some(0) { chip_at(0)(a) } else { None }
        }),
        DynStep::new("n kept the newest: two marked; c clears", move |a| {
            (marked(a) == Some(2)).then(|| key(KeyCode::Char('c')))
        }),
        DynStep::new("cleared: click the s chip", move |a| if marked(a) == Some(0) { chip_at(2)(a) } else { None }),
        DynStep::when("s kept the shortest path: two marked", |a| marked(a) == Some(2), key(KeyCode::Esc)),
        DynStep::when("closed", |a| screens(a).is_none(), ctrl('q')),
    ];
    run_dyn(&mut app, steps, 110, 30);
    all_there(data);
    common::ok();
}
