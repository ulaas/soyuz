//! Headless tests of step T2.9: mouse input. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! On the 80 × 24 test terminal with one pane the location line is row 0 and list item
//! `i` is row `1 + i` (each step checks the click target before it clicks).

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_core::config::Config;
use soyuz_tui::app::App;
use soyuz_tui::layout::HitTarget;

use crate::common::{self, Run, Step, at, ctrl, cursor, key, labels};

fn mouse(kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) -> Event {
    Event::Mouse(MouseEvent { kind, column, row, modifiers })
}

fn click(column: u16, row: u16) -> Event {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row, KeyModifiers::NONE)
}

fn ctrl_click(column: u16, row: u16) -> Event {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row, KeyModifiers::CONTROL)
}

/// Item `index` of the focused pane is under `(x, y)` in the last frame.
fn row_at(a: &App, x: u16, y: u16, index: usize) -> bool {
    matches!(a.hits().at(x, y), Some(HitTarget::PaneRow { index: i, .. }) if i == index)
}

/// Visible positions of the selected items of the focused pane.
fn selected(a: &App) -> Vec<usize> {
    let p = a.core.ws.pane();
    (0..p.visible.len()).filter(|&i| p.key_at(i).is_some_and(|k| p.selection.contains(k))).collect()
}

fn three_items(data: &Path) {
    std::fs::create_dir_all(data.join("sub")).unwrap();
    std::fs::write(data.join("sub").join("inner.txt"), b"i").unwrap();
    std::fs::write(data.join("a.txt"), b"a").unwrap();
    std::fs::write(data.join("b.txt"), b"bb").unwrap();
}

fn listed(data: &Path) -> impl Fn(&App) -> bool + 'static {
    let d = data.to_path_buf();
    move |a| at(a, &d) && labels(a) == ["sub/", "a.txt", "b.txt"]
}

#[test]
fn click_moves_the_cursor_and_ctrl_click_toggles() {
    match common::relaunch(module_path!(), "click_moves_the_cursor_and_ctrl_click_toggles", three_items) {
        Run::Child(data) => clicks(data),
        Run::Parent(_) => {}
    }
}

fn clicks(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let ready = listed(&data);
    let steps = vec![
        Step::new("the listing", move |a| ready(a) && row_at(a, 5, 2, 1), click(5, 2)),
        Step::new(
            "the cursor on the clicked a.txt",
            |a| cursor(a).as_deref() == Some("a.txt") && selected(a) == [1] && row_at(a, 5, 3, 2),
            ctrl_click(5, 3),
        ),
        Step::new("a.txt and b.txt selected", |a| selected(a) == [1, 2] && row_at(a, 5, 2, 1), ctrl_click(5, 2)),
        Step::new("a.txt toggled off", |a| selected(a) == [2] && cursor(a).as_deref() == Some("a.txt"), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    assert!(at(&app, &data), "single clicks never navigate");
    app.finish();
    common::ok();
}

#[test]
fn double_click_enters_a_folder() {
    match common::relaunch(module_path!(), "double_click_enters_a_folder", three_items) {
        Run::Child(data) => double_click(data),
        Run::Parent(_) => {}
    }
}

fn double_click(data: PathBuf) {
    let mut c: Config = common::test_config();
    // The widest interval: a slow test machine still makes it a double-click.
    c.general.double_click_speed_ms = 2000;
    let mut app = common::new_app(c, &data);
    let ready = listed(&data);
    let sub = data.join("sub");
    let steps = vec![
        Step::new("the listing", move |a| ready(a) && row_at(a, 5, 1, 0), click(5, 1)),
        Step::new("the cursor on sub/", |a| cursor(a).as_deref() == Some("sub/"), click(5, 1)),
        Step::new("the subfolder", move |a| at(a, &sub) && labels(a) == ["inner.txt"], ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    app.finish();
    common::ok();
}

#[test]
fn wheel_scrolls_without_moving_the_cursor() {
    let setup = |data: &Path| {
        for i in 0..60 {
            std::fs::write(data.join(format!("f{i:02}.txt")), b"x").unwrap();
        }
    };
    match common::relaunch(module_path!(), "wheel_scrolls_without_moving_the_cursor", setup) {
        Run::Child(data) => wheel(data),
        Run::Parent(_) => {}
    }
}

fn wheel(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let down = || mouse(MouseEventKind::ScrollDown, 5, 5, KeyModifiers::NONE);
    let up = || mouse(MouseEventKind::ScrollUp, 5, 5, KeyModifiers::NONE);
    let pane = |a: &App| (a.core.ws.pane().scroll_y, a.core.ws.pane().cursor_pos());
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d) && labels(a).len() == 22, key(KeyCode::Down)),
        Step::new("the cursor on f00.txt", |a| cursor(a).as_deref() == Some("f00.txt"), down()),
        Step::new("scrolled by three rows", move |a| pane(a) == (3.0, Some(0)), down()),
        Step::new("scrolled by six rows", move |a| pane(a) == (6.0, Some(0)), up()),
        Step::new(
            "back up by three rows, the cursor still on f00.txt",
            move |a| {
                pane(a) == (3.0, Some(0))
                    && labels(a).first().map(String::as_str) == Some("f03.txt")
                    && cursor(a).is_none()
            },
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    assert_eq!(app.core.ws.pane().scroll_y, 3.0, "the wheel offset survives the following frames");
    app.finish();
    common::ok();
}

#[test]
fn mouse_off_ignores_clicks() {
    match common::relaunch(module_path!(), "mouse_off_ignores_clicks", three_items) {
        Run::Child(data) => mouse_off(data),
        Run::Parent(_) => {}
    }
}

fn mouse_off(data: PathBuf) {
    let mut c = common::test_config();
    c.tui.mouse = false;
    let mut app = common::new_app(c, &data);
    let ready = listed(&data);
    let steps = vec![
        Step::new("the listing", move |a| ready(a) && row_at(a, 5, 2, 1), click(5, 2)),
        Step::new("a frame after the click", |_| true, mouse(MouseEventKind::ScrollDown, 5, 2, KeyModifiers::NONE)),
        Step::new("a frame after the wheel", |_| true, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    let p = app.core.ws.pane();
    assert!(p.cursor.is_none() && p.selection.is_empty(), "the click was ignored");
    assert_eq!(p.scroll_y, 0.0, "the wheel was ignored");
    app.finish();
    common::ok();
}
