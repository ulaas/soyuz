//! Headless tests of T5: the folder compare screen (W2-compare-screen). Each test runs in its own
//! sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! `compare_folders` is bound to F12 (it has no default chord), so the keyboard path that
//! queues `ToolCmd::Compare` is the one tested; the panes are set up with the keyboard too
//! (F9 dual panes, the second one in `right`).

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::models::compare::RowAction;
use soyuz_core::config::Config;
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::view::compare::{BodyVm, CompareVm, TableVm};

use crate::common::{self, Run, Step, ctrl, key};

fn config() -> Config {
    let mut c = common::test_config();
    c.keys.bindings.insert("compare_folders".into(), vec!["F12".into()]);
    c
}

fn vm(a: &App) -> Option<&CompareVm> {
    a.view().map(|v| &v.compare).filter(|c| c.visible)
}

fn table(a: &App) -> Option<&TableVm> {
    match &vm(a)?.body {
        BodyVm::Table(t) => Some(t),
        _ => None,
    }
}

fn rels(a: &App) -> Vec<String> {
    table(a).map(|t| t.rows.iter().map(|r| r.rel.clone()).collect()).unwrap_or_default()
}

fn pane_at(a: &App, i: usize, dir: &Path) -> bool {
    a.core.ws.tab().panes.get(i).is_some_and(|p| p.location == soyuz_core::Location::Dir(dir.to_path_buf()))
}

fn click(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}

/// The first cell of click target `t` in the last frame.
fn target_at(a: &App, t: HitTarget) -> Option<(u16, u16)> {
    a.hits().iter().filter(|(_, h)| *h == t).last().map(|(r, _)| (r.x, r.y))
}

/// Steps from the app opened on `left` to the screen opened by F12 on a dual-pane tab
/// with `right` in the second pane.
fn open_steps(data: &Path) -> Vec<Step> {
    let (d, l) = (data.to_path_buf(), data.join("left"));
    let r = data.join("right");
    let l2 = l.clone();
    vec![
        Step::new("the listing of left", move |a| common::at(a, &l), key(KeyCode::F(9))),
        Step::new("dual panes", |a| a.core.ws.tab().shown() == 2, key(KeyCode::Tab)),
        Step::new("the second pane focused", |a| a.core.ws.tab().focused == 1, key(KeyCode::Backspace)),
        Step::new(
            "the second pane in the data folder on left/",
            move |a| pane_at(a, 1, &d) && common::cursor(a).as_deref() == Some("left/"),
            key(KeyCode::Down),
        ),
        Step::new("the cursor on right/", |a| common::cursor(a).as_deref() == Some("right/"), key(KeyCode::Enter)),
        Step::new("the second pane in right", move |a| pane_at(a, 1, &r), key(KeyCode::Tab)),
        Step::new(
            "the first pane focused, in left",
            move |a| a.core.ws.tab().focused == 0 && pane_at(a, 0, &l2),
            key(KeyCode::F(12)),
        ),
    ]
}

fn fixture(data: &Path) {
    let (l, r) = (data.join("left"), data.join("right"));
    std::fs::create_dir_all(l.join("sub")).unwrap();
    std::fs::create_dir_all(&r).unwrap();
    std::fs::write(r.join("a.txt"), b"old").unwrap();
    std::fs::write(l.join("a.txt"), b"the newer, longer contents").unwrap();
    std::fs::write(l.join("sub").join("b.txt"), b"b").unwrap();
    std::fs::write(l.join("c.txt"), b"c").unwrap();
}

/// Every file under `dir`, relative, with its contents.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.strip_prefix(dir).unwrap().to_path_buf(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn mirror_from_a_dual_pane_tab() {
    match common::relaunch(module_path!(), "mirror_from_a_dual_pane_tab", fixture) {
        Run::Child(data) => mirror(data),
        Run::Parent(r) => {
            let (l, rt) = (tree(&r.data.join("left")), tree(&r.data.join("right")));
            assert_eq!(l, rt, "the right side equals the left one");
            assert_eq!(l.len(), 3);
        }
    }
}

fn mirror(data: PathBuf) {
    let mut app = common::new_app(config(), &data.join("left"));
    let mut steps = open_steps(&data);
    steps.extend([
        Step::new(
            "the keyboard's Compare folders opened the screen and the rows came in",
            |a| {
                a.focus() == TuiFocus::Tool
                    && table(a).is_some_and(|t| t.focused && t.total == 3)
                    && ["a.txt", "c.txt", "sub"].iter().all(|n| rels(a).iter().any(|r| r == n))
            },
            key(KeyCode::Char('1')),
        ),
        Step::new(
            "chip 1 (key 1) hides the only-left rows",
            |a| {
                table(a).is_some_and(|t| !t.chips[0].on && t.total == 1)
                    && rels(a) == ["a.txt"]
                    && target_at(a, HitTarget::CompareChip(0)) == Some(chip_cell())
            },
            click(chip_cell().0, chip_cell().1),
        ),
        Step::new(
            "a click on chip 1 shows them again; the mirror mode next (P)",
            |a| table(a).is_some_and(|t| t.chips.iter().all(|c| c.on) && t.total == 3),
            key(KeyCode::Char('p')),
        ),
        Step::new(
            "every row copies to the right",
            |a| table(a).is_some_and(|t| t.total == 3 && t.rows.iter().all(|r| r.action == RowAction::CopyRight)),
            key(KeyCode::Enter),
        ),
        Step::new(
            "the plan copies, deletes nothing",
            |a| matches!(vm(a).map(|v| &v.body), Some(BodyVm::Plan(p)) if p.total > 0 && !p.danger),
            key(KeyCode::Enter),
        ),
        Step::new(
            "the sync job ran and the folders were compared again: in sync",
            |a| {
                common::wait_jobs_idle(a)
                    && a.core.fileops.jobs.snapshots().iter().any(|s| s.state.is_terminal())
                    && matches!(vm(a).map(|v| &v.body), Some(BodyVm::Note(..)))
                    && vm(a).is_some_and(|v| !v.scanning)
            },
            key(KeyCode::Esc),
        ),
        Step::new("Esc closed the screen", |a| vm(a).is_none() && a.focus() == TuiFocus::List, ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 120, 32);
    common::ok();
}

/// Chip 1's first cell on a 120 x 32 screen.
fn chip_cell() -> (u16, u16) {
    let g = soyuz_tui::view::compare::geometry(ratatui::layout::Rect::new(0, 0, 120, 32));
    (g.chips.x, g.chips.y)
}

#[test]
fn esc_cancels_a_scan() {
    match common::relaunch(module_path!(), "esc_cancels_a_scan", |data| {
        let (l, r) = (data.join("left"), data.join("right"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(l.join("only.txt"), b"x").unwrap();
        // Same size, compared byte by byte: long enough to cancel.
        for d in [&l, &r] {
            let f = std::fs::File::create(d.join("big.bin")).unwrap();
            f.set_len(256 << 20).unwrap();
        }
    }) {
        Run::Child(data) => cancel(data),
        Run::Parent(_) => {}
    }
}

fn cancel(data: PathBuf) {
    let mut app = common::new_app(config(), &data.join("left"));
    let mut steps = open_steps(&data);
    steps.extend([
        Step::new("the first result (date and size)", |a| table(a).is_some_and(|t| t.focused), key(KeyCode::Char('m'))),
        Step::new(
            "the second result (size)",
            |a| a.core.ws.tab().shown() == 2 && table(a).is_some_and(|t| t.focused) && !vm(a).unwrap().scanning,
            key(KeyCode::Char('m')),
        ),
        Step::new("comparing the contents", |a| vm(a).is_some_and(|v| v.scanning), key(KeyCode::Esc)),
        Step::new(
            "Esc cancelled the scan; the screen stays open",
            |a| {
                vm(a).is_some_and(|v| !v.scanning && !matches!(v.body, BodyVm::Busy(..))) && a.focus() == TuiFocus::Tool
            },
            key(KeyCode::Esc),
        ),
        Step::new("the second Esc closes it", |a| vm(a).is_none(), ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[test]
fn tiny_sizes_with_the_screen_open() {
    match common::relaunch(module_path!(), "tiny_sizes_with_the_screen_open", fixture) {
        Run::Child(data) => tiny(data),
        Run::Parent(_) => {}
    }
}

fn tiny(data: PathBuf) {
    let mut app = common::new_app(config(), &data.join("left"));
    let mut steps = open_steps(&data);
    steps.push(Step::new("the rows", |a| table(a).is_some_and(|t| t.total == 3), ctrl('q')));
    common::run_script(&mut app, steps, 100, 30);
    // Quit leaves the screen's state: frames at every small size, each page.
    let mut io = NullIo::default();
    let events = [Some(key(KeyCode::Enter)), Some(key(KeyCode::F(8))), None];
    for ev in events {
        for w in (0..=40).step_by(3) {
            for h in 0..=16 {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                app.frame(&mut terminal, &[], &mut io).expect("a frame");
            }
        }
        // Then the plan (Enter), then the profiles page (F8).
        let Some(ev) = ev else { continue };
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        app.frame(&mut terminal, &[ev], &mut io).expect("a frame");
        app.frame(&mut terminal, &[], &mut io).expect("a frame");
    }
    assert!(matches!(app.view().map(|v| &v.compare.body), Some(BodyVm::Profiles(_))));
    app.finish();
    common::ok();
}
