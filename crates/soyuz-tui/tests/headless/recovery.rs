//! Headless tests of T3: interrupted jobs and crash reports. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! The child plants what a crashed run leaves in its portable config dir (a job journal
//! without a live lock, a crash report), then runs the startup recovery like
//! `soyuz --tui` does (`services::crash::startup`) before building the app.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crossterm::event::KeyCode;
use soyuz_appcore::services::crash;
use soyuz_core::config;
use soyuz_jobs::{JobKind, JobOptions, JournalJob};
use soyuz_tui::input::TuiFocus;

use crate::common::{self, Run, Step, ctrl, dialog_title, key, wait_jobs_idle};

/// `data/src/a.txt`, to be copied to `data/dst` by the interrupted job.
fn setup(data: &Path) {
    std::fs::create_dir_all(data.join("src")).unwrap();
    std::fs::create_dir_all(data.join("dst")).unwrap();
    std::fs::write(data.join("src").join("a.txt"), b"payload").unwrap();
}

/// A crashed instance's journal: an interrupted copy of `src/a.txt` into `dst`.
fn plant_journal(data: &Path) {
    let job = JournalJob {
        id: 7,
        label: "Copy a.txt".into(),
        kind: JobKind::Copy {
            sources: vec![data.join("src").join("a.txt")],
            dest_dir: data.join("dst"),
            rename_to: None,
        },
        options: JobOptions { journal: true, ..JobOptions::default() },
        started: SystemTime::now(),
        records: Vec::new(),
        stamps: Vec::new(),
        items_done: 0,
        items_total: 1,
        bytes_done: 0,
        bytes_total: 7,
    };
    let body = serde_json::json!({ "version": 1, "pid": 1, "jobs": [job] });
    let dir = config::config_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(soyuz_jobs::journal::JOURNAL_FILE), body.to_string()).unwrap();
}

/// Adopted journals still waiting for a decision.
fn recovered_journals(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("jobs-recovered-")).count())
        .unwrap_or(0)
}

const TITLE: &str = "Soyuz closed during an operation";

fn no_dialog(a: &soyuz_tui::app::App) -> bool {
    dialog_title(a).is_none() && a.focus() == TuiFocus::List
}

#[test]
fn recovery_resume_finishes_the_copy() {
    match common::relaunch(module_path!(), "recovery_resume_finishes_the_copy", setup) {
        Run::Child(data) => recovery(data, 'r'),
        Run::Parent(p) => {
            assert_eq!(std::fs::read(p.data.join("dst").join("a.txt")).unwrap(), b"payload");
            assert_eq!(recovered_journals(&p.config_dir()), 0, "the journal was dismissed");
        }
    }
}

#[test]
fn recovery_discard_leaves_files_alone() {
    match common::relaunch(module_path!(), "recovery_discard_leaves_files_alone", setup) {
        Run::Child(data) => recovery(data, 'd'),
        Run::Parent(p) => {
            assert!(!p.data.join("dst").join("a.txt").exists());
            assert!(p.data.join("src").join("a.txt").is_file());
            assert_eq!(recovered_journals(&p.config_dir()), 0, "the journal was dismissed");
        }
    }
}

fn recovery(data: PathBuf, choice: char) {
    plant_journal(&data);
    crash::startup(false);
    let dir = config::config_dir();
    assert_eq!(recovered_journals(&dir), 1, "the dead journal was adopted");
    let mut app = common::new_app(common::test_config(), &data);
    let resumed = choice == 'r';
    let dst = data.join("dst").join("a.txt");
    let steps = vec![
        Step::new(
            "the recovery dialog with the job",
            |a| {
                a.view()
                    .and_then(|v| v.dialog.as_ref())
                    .is_some_and(|d| d.title == TITLE && d.list.iter().any(|r| r.text == "Copy a.txt") && d.alert)
            },
            key(KeyCode::Char(choice)),
        ),
        Step::new(
            "decided",
            move |a| {
                no_dialog(a)
                    && recovered_journals(&config::config_dir()) == 0
                    && wait_jobs_idle(a)
                    && dst.is_file() == resumed
            },
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[test]
fn recovery_decide_later_keeps_the_journal() {
    match common::relaunch(module_path!(), "recovery_decide_later_keeps_the_journal", setup) {
        Run::Child(data) => {
            plant_journal(&data);
            crash::startup(false);
            let mut app = common::new_app(common::test_config(), &data);
            let steps = vec![
                Step::new("the dialog", |a| dialog_title(a).as_deref() == Some(TITLE), key(KeyCode::Esc)),
                Step::new("closed", no_dialog, ctrl('q')),
            ];
            common::run_script(&mut app, steps, 100, 30);
            common::ok();
        }
        Run::Parent(p) => assert_eq!(recovered_journals(&p.config_dir()), 1, "offered again next start"),
    }
}

#[test]
fn crash_reports_view_and_delete() {
    match common::relaunch(module_path!(), "crash_reports_view_and_delete", |_| {}) {
        Run::Child(data) => crash_reports(data),
        Run::Parent(p) => {
            let left = std::fs::read_dir(p.config_dir().join("crashes"))
                .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".txt")).count())
                .unwrap_or(0);
            assert_eq!(left, 0, "the reports were deleted");
        }
    }
}

fn crash_reports(data: PathBuf) {
    let dir = crash::crashes_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let report = dir.join("crash-20261012-101010.txt");
    std::fs::write(&report, "Soyuz crash report\nPanic: boom in the test\n\tat somewhere\n").unwrap();
    crash::startup(false);
    let mut app = common::new_app(common::test_config(), &data);
    let dialog = |a: &soyuz_tui::app::App| a.view().and_then(|v| v.dialog.clone());
    let steps = vec![
        Step::new(
            "the report listed",
            move |a| {
                dialog(a).is_some_and(|d| {
                    d.title == "Soyuz closed unexpectedly" && d.list.iter().any(|r| r.text == "boom in the test")
                })
            },
            key(KeyCode::Char('v')),
        ),
        Step::new(
            "its text shown",
            move |a| dialog(a).is_some_and(|d| d.list.iter().any(|r| r.text == "Soyuz crash report")),
            key(KeyCode::Esc),
        ),
        Step::new(
            "back to the list",
            move |a| dialog(a).is_some_and(|d| d.list.iter().any(|r| r.text == "boom in the test")),
            key(KeyCode::Char('d')),
        ),
        Step::new("deleted", move |a| no_dialog(a) && !report.exists(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}
