//! Headless tests of T3: the jobs panel. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! Jobs are submitted straight to the core's job service (a throttled copy runs long
//! enough to pause and cancel it); nothing goes to the real trash (the undo test moves).

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::services::jobs::{JobMeta, OpKind};
use soyuz_jobs::{JobKind, JobState};
use soyuz_tui::app::App;
use soyuz_tui::hits::JobButton;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;

use crate::common::{self, Run, Step, alt, ctrl, key};

/// The single job's state.
fn state(a: &App) -> Option<JobState> {
    a.core.fileops.jobs.snapshots().first().map(|s| s.state.clone())
}

/// Title and buttons of the panel's rows, as last drawn.
fn rows(a: &App) -> Vec<(String, Vec<JobButton>)> {
    a.view()
        .map(|v| v.jobs.items.iter().map(|i| (i.title.clone(), i.buttons.iter().map(|b| b.kind).collect())).collect())
        .unwrap_or_default()
}

fn has_button(a: &App, b: JobButton) -> bool {
    rows(a).first().is_some_and(|(_, bs)| bs.contains(&b))
}

fn click(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}

fn submit(app: &mut App, kind: JobKind, op: OpKind, sources: &[PathBuf], dest: &Path, throttle: Option<u64>) {
    let mut spec = app.core.fileops.jobs.spec(kind);
    if throttle.is_some() {
        spec.options.copy.throttle_bps = throttle;
    }
    let meta = JobMeta::new(op, sources, Some(dest.to_path_buf()));
    app.core.fileops.jobs.submit(spec, meta);
}

#[test]
fn copy_job_row_pause_resume_cancel() {
    match common::relaunch(module_path!(), "copy_job_row_pause_resume_cancel", |data| {
        std::fs::create_dir_all(data.join("src")).unwrap();
        std::fs::create_dir_all(data.join("dst")).unwrap();
        std::fs::write(data.join("src").join("big.bin"), vec![7u8; 8 << 20]).unwrap();
    }) {
        Run::Child(data) => copy_job(data),
        Run::Parent(r) => assert!(!r.data.join("dst").join("big.bin").exists(), "the cancelled copy is gone"),
    }
}

fn copy_job(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data.join("src"));
    let src = vec![data.join("src").join("big.bin")];
    let dst = data.join("dst");
    let kind = JobKind::Copy { sources: src.clone(), dest_dir: dst.clone(), rename_to: None };
    // 8 MiB at 512 KiB/s: 16 s, long enough to pause, resume and cancel.
    submit(&mut app, kind, OpKind::Copy, &src, &dst, Some(512 << 10));
    let steps = vec![
        Step::new(
            "the panel opens by itself, without the keyboard",
            |a| {
                a.view().is_some_and(|v| v.jobs.visible && !v.jobs.focused)
                    && rows(a).first().is_some_and(|(t, _)| t.contains("big.bin"))
                    && has_button(a, JobButton::Pause)
                    && a.focus() == TuiFocus::List
            },
            alt('j'),
        ),
        Step::new("the panel has the keyboard", |a| a.focus() == TuiFocus::Jobs, key(KeyCode::Char('p'))),
        Step::new(
            "paused",
            |a| state(a) == Some(JobState::Paused) && has_button(a, JobButton::Resume),
            key(KeyCode::Char(' ')),
        ),
        Step::new(
            "running again",
            |a| state(a) == Some(JobState::Running) && has_button(a, JobButton::Pause),
            key(KeyCode::Char('c')),
        ),
        Step::new(
            "cancelled, the row stays with Remove",
            |a| state(a) == Some(JobState::Cancelled) && has_button(a, JobButton::Dismiss),
            key(KeyCode::Delete),
        ),
        Step::new(
            "removed: the empty list",
            |a| a.core.fileops.jobs.snapshots().is_empty() && a.view().is_some_and(|v| v.jobs.empty.is_some()),
            key(KeyCode::Esc),
        ),
        Step::new(
            "the keyboard is back in the list, the panel stays",
            |a| a.focus() == TuiFocus::List && a.view().is_some_and(|v| v.jobs.visible),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

#[test]
fn history_shows_the_finished_job_and_undoes_it() {
    match common::relaunch(module_path!(), "history_shows_the_finished_job_and_undoes_it", |data| {
        std::fs::create_dir_all(data.join("src")).unwrap();
        std::fs::create_dir_all(data.join("dst")).unwrap();
        std::fs::write(data.join("src").join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => history(data),
        Run::Parent(r) => {
            assert!(r.data.join("src").join("a.txt").exists(), "the move was undone");
            assert!(!r.data.join("dst").join("a.txt").exists());
        }
    }
}

fn history(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data.join("src"));
    let src = vec![data.join("src").join("a.txt")];
    let dst = data.join("dst");
    let moved = dst.join("a.txt");
    let kind = JobKind::Move { sources: src.clone(), dest_dir: dst.clone(), rename_to: None };
    submit(&mut app, kind, OpKind::Move, &src, &dst, None);
    // 80 × 24, `[tui] jobs_rows` 8: the panel is rows 15..23 (header on 15), the first
    // entry on rows 16 and 17; its last button ends at the right edge.
    let steps = vec![
        Step::new("the move finished", move |a| common::wait_jobs_idle(a) && moved.exists(), alt('j')),
        Step::new(
            "the panel with the jobs tab",
            |a| a.focus() == TuiFocus::Jobs && a.hits().at(10, 15) == Some(HitTarget::BottomTab(1)),
            // A click on the History tab.
            click(10, 15),
        ),
        Step::new(
            "the history tab lists the move with Undo",
            |a| {
                a.view().is_some_and(|v| v.jobs.tab == 1)
                    && rows(a).first().is_some_and(|(t, _)| t.starts_with("Moved"))
                    && has_button(a, JobButton::Undo)
            },
            key(KeyCode::Char('u')),
        ),
        Step::new(
            "undone: Redo instead of Undo",
            |a| common::wait_jobs_idle(a) && has_button(a, JobButton::Redo),
            key(KeyCode::Tab),
        ),
        Step::new(
            "back on the jobs tab, the undo job listed",
            |a| a.view().is_some_and(|v| v.jobs.tab == 0 && v.jobs.len >= 1),
            key(KeyCode::Esc),
        ),
        Step::new("the list has the keyboard", |a| a.focus() == TuiFocus::List, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn mouse_buttons_and_focus() {
    match common::relaunch(module_path!(), "mouse_buttons_and_focus", |data| {
        std::fs::create_dir_all(data.join("src")).unwrap();
        std::fs::create_dir_all(data.join("dst")).unwrap();
        std::fs::write(data.join("src").join("big.bin"), vec![1u8; 8 << 20]).unwrap();
    }) {
        Run::Child(data) => mouse(data),
        Run::Parent(_) => {}
    }
}

/// The rect of the first row's `button`, as last drawn.
fn button_at(a: &App, button: JobButton) -> Option<(u16, u16)> {
    a.hits().iter().find_map(|(r, t)| (*t == HitTarget::JobButton { row: 0, button }).then_some((r.x, r.y)))
}

fn mouse(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data.join("src"));
    let src = vec![data.join("src").join("big.bin")];
    let dst = data.join("dst");
    let kind = JobKind::Copy { sources: src.clone(), dest_dir: dst.clone(), rename_to: None };
    submit(&mut app, kind, OpKind::Copy, &src, &dst, Some(512 << 10));
    // 80 × 24: the first row's buttons are "[Pause] [Cancel]" at the right edge of row 16.
    let steps = vec![
        Step::new(
            "the pause button at the expected place",
            |a| button_at(a, JobButton::Pause) == Some((64, 16)) && state(a) == Some(JobState::Running),
            click(65, 16),
        ),
        Step::new(
            "paused by the click; the panel took the keyboard",
            |a| state(a) == Some(JobState::Paused) && a.focus() == TuiFocus::Jobs,
            // A click in the file list gives the keyboard back.
            click(5, 3),
        ),
        Step::new(
            "the list has the keyboard again",
            |a| a.focus() == TuiFocus::List && button_at(a, JobButton::Cancel) == Some((72, 16)),
            click(73, 16),
        ),
        Step::new(
            "cancelled by the click",
            |a| state(a) == Some(JobState::Cancelled) && button_at(a, JobButton::Close) == Some((77, 15)),
            click(78, 15),
        ),
        Step::new("closed", |a| a.view().is_some_and(|v| !v.jobs.visible), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

fn right_click(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}

/// A job menu is open (its "Start now" row).
fn job_menu_open(a: &App) -> bool {
    a.view().and_then(|v| v.menu.menu.as_ref()).is_some_and(|m| {
        m.items.iter().any(|i| matches!(i, soyuz_tui::widgets::MenuItem::Entry(e) if e.label == soyuz_appcore::tr!("panels-start-now")))
    }) && a.focus() == TuiFocus::Menu
}

#[test]
fn toolbar_menu_and_log() {
    match common::relaunch(module_path!(), "toolbar_menu_and_log", |data| {
        std::fs::create_dir_all(data.join("src")).unwrap();
        std::fs::create_dir_all(data.join("dst")).unwrap();
        std::fs::write(data.join("src").join("big.bin"), vec![3u8; 8 << 20]).unwrap();
    }) {
        Run::Child(data) => toolbar(data),
        Run::Parent(r) => assert!(!r.data.join("dst").join("big.bin").exists(), "cancelled"),
    }
}

fn toolbar(data: PathBuf) {
    use crate::common::key_mods;
    let mut app = common::new_app(common::test_config(), &data.join("src"));
    let src = vec![data.join("src").join("big.bin")];
    let dst = data.join("dst");
    let kind = JobKind::Copy { sources: src.clone(), dest_dir: dst.clone(), rename_to: None };
    submit(&mut app, kind, OpKind::Copy, &src, &dst, Some(512 << 10));
    let log_shown = |a: &App| a.view().is_some_and(|v| v.jobs.items.first().is_some_and(|i| !i.log.is_empty()));
    // 80 × 24: the panel's header is row 15; "[Clear]" is the toolbar's last button, right
    // before the close mark; the first job is on rows 16 and 17.
    let steps = vec![
        Step::new(
            "the toolbar on the jobs tab",
            |a| {
                a.view().is_some_and(|v| v.jobs.toolbar.len() == 4)
                    && button_at(a, JobButton::PauseAll).is_some()
                    && state(a) == Some(JobState::Running)
            },
            alt('j'),
        ),
        Step::new(
            "the panel has the keyboard",
            |a| a.focus() == TuiFocus::Jobs,
            key_mods(KeyCode::Char('P'), KeyModifiers::SHIFT),
        ),
        Step::new(
            "all paused",
            |a| state(a) == Some(JobState::Paused),
            key_mods(KeyCode::Char('S'), KeyModifiers::SHIFT),
        ),
        Step::new("all running again", |a| state(a) == Some(JobState::Running), key(KeyCode::Char('l'))),
        Step::new(
            "the details under the row",
            move |a| log_shown(a) && rows(a).first().is_some_and(|(_, b)| b.contains(&JobButton::Details)),
            key(KeyCode::Char('l')),
        ),
        Step::new("the details hidden", move |a| !log_shown(a), key(KeyCode::Char('m'))),
        Step::new("the job menu from M", job_menu_open, key(KeyCode::Esc)),
        Step::new(
            "the menu closed",
            |a| !job_menu_open(a) && a.hits().at(5, 16) == Some(HitTarget::JobRow(0)),
            right_click(5, 16),
        ),
        Step::new("the job menu from a right click", job_menu_open, key(KeyCode::Esc)),
        Step::new("closed again", |a| a.focus() == TuiFocus::Jobs, key_mods(KeyCode::Char('C'), KeyModifiers::SHIFT)),
        Step::new(
            "all cancelled; Clear applies",
            |a| {
                state(a) == Some(JobState::Cancelled)
                    && a.view().is_some_and(|v| v.jobs.toolbar.last().is_some_and(|b| b.enabled))
                    && a.hits().at(71, 15) == Some(HitTarget::JobButton { row: 0, button: JobButton::ClearFinished })
            },
            click(71, 15),
        ),
        Step::new("cleared", |a| a.core.fileops.jobs.snapshots().is_empty(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}
