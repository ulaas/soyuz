//! Headless tests of Phase T7 (agent C): the disk usage treemap screen. Each test runs in
//! its own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`,
//! like the other files here.
//!
//! `disk_usage_treemap` is bound to F12 (it has no default chord). The pane's cursor is
//! put on a file first, so the treemap opens on the pane's folder. Nothing here opens a
//! file (Enter only on folders, no double click) and the trash is asked, then cancelled:
//! no `FileOp` ever runs.

use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use soyuz_appcore::models::treemap::layout::{Tile, squarify};
use soyuz_core::config::Config;
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::view::screens::treemap::{ASPECT, StatusVm, TreemapVm, geometry, snap};
use soyuz_tui::view::screens::{ScreenVm, ScreensVm};

use crate::common::{self, Run, Step, ctrl, key};

const MB: u64 = 1_000_000;

fn config() -> Config {
    let mut c = common::test_config();
    c.keys.bindings.insert("disk_usage_treemap".into(), vec!["F12".into()]);
    c
}

fn fixture(data: &Path) {
    let file = |p: &Path, len: u64| std::fs::File::create(p).unwrap().set_len(len).unwrap();
    std::fs::create_dir_all(data.join("b").join("c")).unwrap();
    file(&data.join("a.bin"), 3 * MB);
    file(&data.join("b").join("x.bin"), MB);
    file(&data.join("b").join("c").join("y.bin"), MB);
}

fn screens(a: &App) -> Option<&ScreensVm> {
    a.view().map(|v| &v.screens).filter(|s| s.visible)
}

fn vm(a: &App) -> Option<&TreemapVm> {
    match &screens(a)?.screen {
        Some(ScreenVm::Treemap(t)) => Some(t.as_ref()),
        _ => None,
    }
}

/// The scan finished and the map (or, on narrow screens, the list) shows `names`.
fn shows(a: &App, names: &[&str]) -> bool {
    vm(a).is_some_and(|t| {
        let shown = |n: &str| t.tiles.iter().any(|x| x.label == n) || t.rows.iter().any(|r| r.name == n);
        matches!(t.status, StatusVm::Done(_)) && names.iter().all(|n| shown(n))
    })
}

fn selected(a: &App) -> Option<String> {
    vm(a)?.tiles.iter().find(|t| t.selected).map(|t| t.label.clone())
}

fn crumbs(a: &App) -> Vec<String> {
    vm(a).map(|t| t.crumbs.iter().map(|(_, n)| n.clone()).collect()).unwrap_or_default()
}

fn click(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}

/// A cell on the top edge of the "b" frame on a 120 x 32 screen (a: 3 MB, b: 2 MB, laid
/// out like the view does). Its inside holds the tiles of x.bin and c (a click on c
/// would drill into c).
fn b_edge() -> (u16, u16) {
    let map = geometry(Rect::new(0, 0, 120, 32), false, false).map;
    let tiles = squarify(&[3.0, 2.0], Tile::new(0.0, 0.0, f64::from(map.width), f64::from(map.height) * ASPECT));
    let (x, y, w, h) = snap(&tiles, ASPECT)[1].expect("b has a tile");
    assert!(w > 4 && h > 2, "b is a frame");
    (map.x + x + 2, map.y + y)
}

/// The pane in `data` with its cursor on a.bin, then F12.
fn open_steps(data: &Path) -> Vec<Step> {
    let d = data.to_path_buf();
    vec![
        Step::new(
            "the listing of data (b/, a.bin)",
            move |a| common::at(a, &d) && common::labels(a) == ["b/", "a.bin"],
            key(KeyCode::End),
        ),
        Step::new("the cursor on a.bin", |a| common::cursor(a).as_deref() == Some("a.bin"), key(KeyCode::F(12))),
    ]
}

#[test]
fn browse_drill_trash_cancel_and_reveal() {
    match common::relaunch(module_path!(), "browse_drill_trash_cancel_and_reveal", fixture) {
        Run::Child(data) => browse(&data),
        Run::Parent(r) => {
            assert!(r.data.join("a.bin").is_file(), "the trash was cancelled");
            assert!(r.data.join("b").join("c").join("y.bin").is_file());
        }
    }
}

fn browse(data: &Path) {
    let mut app = common::new_app(config(), data);
    let mut steps = open_steps(data);
    let b_dir = data.join("b");
    let b_dir2 = b_dir.clone();
    let d = data.to_path_buf();
    let (bx, by) = b_edge();
    steps.extend([
        Step::new(
            "the screen shows a.bin and b with the totals",
            |a| {
                a.focus() == TuiFocus::Tool
                    && shows(a, &["a.bin", "b"])
                    && vm(a).is_some_and(|t| t.tiles.len() >= 2 && t.totals.contains('3'))
            },
            key(KeyCode::Right),
        ),
        Step::new(
            "the first arrow selects the largest tile",
            |a| selected(a).as_deref() == Some("a.bin"),
            key(KeyCode::Right),
        ),
        Step::new("Right moves to b", |a| selected(a).as_deref() == Some("b"), key(KeyCode::Enter)),
        Step::new(
            "Enter zoomed into b (its scan of c finished)",
            |a| crumbs(a).last().map(String::as_str) == Some("b") && shows(a, &["x.bin"]),
            key(KeyCode::Backspace),
        ),
        Step::new(
            "Backspace went up, b stays selected",
            |a| crumbs(a).len() == 1 && selected(a).as_deref() == Some("b"),
            key(KeyCode::Left),
        ),
        Step::new("Left selects a.bin", |a| selected(a).as_deref() == Some("a.bin"), key(KeyCode::Delete)),
        Step::new(
            "Del asks to move a.bin to the trash",
            |a| vm(a).and_then(|t| t.confirm.as_deref()).is_some_and(|q| q.contains("a.bin")),
            key(KeyCode::Esc),
        ),
        Step::new(
            "Esc cancelled; the list next (Tab)",
            |a| vm(a).is_some_and(|t| t.confirm.is_none() && !t.list_focused),
            key(KeyCode::Tab),
        ),
        Step::new(
            "the side list has the keyboard: Largest files (L)",
            |a| vm(a).is_some_and(|t| t.list_focused && t.rows.iter().any(|r| r.name == "a.bin")),
            key(KeyCode::Char('l')),
        ),
        Step::new(
            "the largest files below the root",
            |a| {
                vm(a).is_some_and(|t| {
                    let mut names: Vec<&str> = t.rows.iter().map(|r| r.name.as_str()).collect();
                    names.first() == Some(&"a.bin") && {
                        names.sort_unstable();
                        names == ["a.bin", "x.bin", "y.bin"]
                    }
                })
            },
            key(KeyCode::Tab),
        ),
        Step::new("back on the map: a click on b drills in", |a| vm(a).is_some_and(|t| !t.list_focused), click(bx, by)),
        Step::new(
            "the click zoomed into b; O opens it in the pane",
            |a| crumbs(a).last().map(String::as_str) == Some("b"),
            key(KeyCode::Char('o')),
        ),
        Step::new(
            "the screen stepped aside: the pane shows b",
            move |a| screens(a).is_none() && a.focus() == TuiFocus::List && common::at(a, &b_dir),
            key(KeyCode::Backspace),
        ),
        Step::new(
            "the pane went back to data",
            move |a| common::at(a, &d) && common::cursor(a).is_some(),
            key(KeyCode::End),
        ),
        Step::new("the cursor on a.bin again", |a| common::cursor(a).as_deref() == Some("a.bin"), key(KeyCode::F(12))),
        Step::new(
            "the same treemap shows again with its results (still in b)",
            |a| {
                a.focus() == TuiFocus::Tool && crumbs(a).last().map(String::as_str) == Some("b") && shows(a, &["x.bin"])
            },
            key(KeyCode::Esc),
        ),
        Step::new(
            "Esc closed it; the panes get keys again",
            move |a| screens(a).is_none() && a.focus() == TuiFocus::List && !common::at(a, &b_dir2),
            ctrl('q'),
        ),
    ]);
    common::run_script(&mut app, steps, 120, 32);
    common::ok();
}

#[test]
fn tiny_screen_shows_the_list() {
    match common::relaunch(module_path!(), "tiny_screen_shows_the_list", fixture) {
        Run::Child(data) => tiny(&data),
        Run::Parent(_) => {}
    }
}

fn tiny(data: &Path) {
    let mut app = common::new_app(config(), data);
    let mut steps = open_steps(data);
    steps.extend([
        Step::new(
            "the list alone on a narrow screen",
            |a| shows(a, &["a.bin"]) && vm(a).is_some_and(|t| t.list_focused && t.rows.len() == 2),
            key(KeyCode::Down),
        ),
        Step::new(
            "Down moves to b",
            |a| vm(a).is_some_and(|t| t.rows.iter().any(|r| r.cursor && r.name == "b")),
            key(KeyCode::Enter),
        ),
        Step::new("Enter on b zooms in", |a| crumbs(a).last().map(String::as_str) == Some("b"), key(KeyCode::Esc)),
        Step::new("Esc closed it", |a| screens(a).is_none() && a.focus() == TuiFocus::List, ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 36, 10);
    common::ok();
}
