//! Headless tests of T5: editing and scheduling sync profiles (W3 compare profiles). Each test runs
//! in its own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! `sync_profiles` is bound to F12 (it has no default chord): it opens the compare screen on
//! its profiles page. Profiles a test starts with are saved into the sandbox's config
//! directory by the parent (`seed`), and the parent reads `sync_profiles.toml` back after
//! the child quit.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{Event, KeyCode};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::models::compare::{Method, Preset};
use soyuz_appcore::models::compare_profiles::{self, Schedule, SyncProfile};
use soyuz_appcore::tr;
use soyuz_core::config::{Config, PORTABLE_DATA_DIR};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::view::compare::{BodyVm, ProfilesVm};
use soyuz_tui::view::dialogs::DialogVm;

use crate::common::{self, Run, Step, ctrl, key, shift};

fn config() -> Config {
    let mut c = common::test_config();
    c.keys.bindings.insert("sync_profiles".into(), vec!["F12".into()]);
    c
}

fn page(a: &App) -> Option<&ProfilesVm> {
    match &a.view()?.compare.body {
        BodyVm::Profiles(p) if a.view()?.compare.visible => Some(p),
        _ => None,
    }
}

/// The names listed on the profiles page.
fn names(a: &App) -> Vec<String> {
    page(a).map(|p| p.rows.iter().map(|r| r.name.clone()).collect()).unwrap_or_default()
}

/// The name of the selected profile.
fn selected(a: &App) -> Option<String> {
    page(a)?.rows.iter().find(|r| r.cursor).map(|r| r.name.clone())
}

fn dialog(a: &App) -> Option<&DialogVm> {
    a.view()?.dialog.as_ref()
}

fn error(a: &App) -> Option<String> {
    dialog(a)?.error.clone()
}

fn paste(s: &Path) -> Event {
    Event::Paste(s.to_string_lossy().into_owned())
}

fn text(s: &str) -> Event {
    Event::Paste(s.to_string())
}

/// `n` presses of `code` (no condition).
fn keys(steps: &mut Vec<Step>, code: KeyCode, n: usize) {
    for _ in 0..n {
        steps.push(Step::new("a key", |_| true, key(code)));
    }
}

fn folders(data: &Path) {
    let (l, r) = (data.join("src"), data.join("dst"));
    std::fs::create_dir_all(&l).unwrap();
    std::fs::create_dir_all(&r).unwrap();
    std::fs::write(l.join("a.txt"), b"a").unwrap();
    std::fs::write(r.join("b.txt"), b"b").unwrap();
}

/// The sandbox's config directory (portable mode, next to `data`).
fn config_dir(data: &Path) -> PathBuf {
    data.parent().unwrap().join(PORTABLE_DATA_DIR)
}

/// Save `profiles` where the child's app finds them.
fn seed(data: &Path, profiles: &[SyncProfile]) {
    let dir = config_dir(data);
    std::fs::create_dir_all(&dir).unwrap();
    compare_profiles::save_profiles_in(&dir, profiles);
    assert!(soyuz_appcore::services::store_file::flush(Duration::from_secs(10)));
}

fn existing(data: &Path) -> SyncProfile {
    SyncProfile {
        name: "Existing".into(),
        left: data.join("src"),
        right: data.join("dst"),
        method: Method::Content,
        preset: Preset::MirrorRight,
        schedule: Schedule::Daily { hour: 7, minute: 15 },
    }
}

#[test]
fn create_edit_rename_run_delete() {
    match common::relaunch(module_path!(), "create_edit_rename_run_delete", folders) {
        Run::Child(data) => lifecycle(data),
        Run::Parent(r) => {
            assert!(compare_profiles::load_profiles_in(&r.config_dir()).is_empty(), "deleted:\n{}", r.log);
            // The run synced both ways (mode "Update") and its report stays on disk.
            assert!(r.data.join("dst").join("a.txt").is_file() && r.data.join("src").join("b.txt").is_file());
            let reports = compare_profiles::reports_dir_in(&r.config_dir(), "Papers");
            assert!(!compare_profiles::load_reports_in(&reports, 5).is_empty(), "a report of Papers");
        }
    }
}

fn lifecycle(data: PathBuf) {
    let (src, dst) = (data.join("src"), data.join("dst"));
    // No scheduled runs: the profile runs only when the test starts it.
    let mut c = config();
    c.sync.scheduler = false;
    let mut app = common::new_app(c, &data);
    let d = data.clone();
    let mut steps = vec![
        Step::new("the listing", move |a| common::at(a, &d), key(KeyCode::F(12))),
        Step::new("the empty profiles page", |a| page(a).is_some_and(|p| p.empty.is_some()), key(KeyCode::Char('n'))),
        Step::new(
            "the new profile dialog, on the name",
            |a| dialog(a).is_some_and(|d| d.title == tr!("tui-cmpprof-title-new") && d.focus == 0),
            text("Docs"),
        ),
        Step::new("a name", |_| true, key(KeyCode::Tab)),
        Step::new("on the left folder", |a| dialog(a).is_some_and(|d| d.focus == 1), paste(&src)),
        Step::new("a left folder", |_| true, key(KeyCode::Tab)),
        Step::new("on the right folder", |a| dialog(a).is_some_and(|d| d.focus == 2), paste(&dst)),
        Step::new("a right folder", |_| true, key(KeyCode::Enter)),
        Step::new(
            "saved, listed and selected",
            |a| {
                dialog(a).is_none()
                    && selected(a).as_deref() == Some("Docs")
                    && compare_profiles::profile("Docs").is_some_and(|p| p.schedule == Schedule::Manual)
                    && common::toasts(a).iter().any(|t| t.contains("Docs"))
            },
            key(KeyCode::Char('e')),
        ),
        Step::new("the edit dialog", |a| dialog(a).is_some_and(|d| d.title.contains("Docs")), key(KeyCode::Tab)),
    ];
    // Name → left → right → method → mode → schedule.
    keys(&mut steps, KeyCode::Tab, 4);
    steps.extend([
        Step::new(
            "on the schedule, the minutes field off",
            |a| dialog(a).is_some_and(|d| d.focus == 5 && !d.fields[6].enabled && !d.fields[7].enabled),
            key(KeyCode::Right),
        ),
        Step::new(
            "\"Every…\" picked: the minutes field on",
            |a| dialog(a).is_some_and(|d| d.fields[6].enabled && !d.fields[7].enabled),
            key(KeyCode::Tab),
        ),
        Step::new("on the minutes", |a| dialog(a).is_some_and(|d| d.focus == 6), key(KeyCode::Backspace)),
        Step::new("6", |_| true, key(KeyCode::Backspace)),
        Step::new("empty", |_| true, text("15")),
        Step::new("15 minutes", |_| true, key(KeyCode::Enter)),
        Step::new(
            "the schedule is saved; the page shows it",
            |a| {
                dialog(a).is_none()
                    && compare_profiles::profile("Docs")
                        .is_some_and(|p| p.schedule == Schedule::Interval { minutes: 15 })
                    && page(a).and_then(|p| p.detail.as_ref()).is_some_and(|d| d.schedule.contains("15"))
            },
            key(KeyCode::F(2)),
        ),
        Step::new(
            "the rename dialog",
            |a| dialog(a).is_some_and(|d| d.title == tr!("tui-cmpprof-title-rename", name = "Docs")),
            key(KeyCode::Home),
        ),
        Step::new("the start of the name", |_| true, text("Papers ")),
        Step::new("typed in front", |_| true, key(KeyCode::End)),
    ]);
    keys(&mut steps, KeyCode::Backspace, 5);
    steps.extend([
        Step::new("\"Papers\"", |_| true, key(KeyCode::Enter)),
        Step::new(
            "renamed and still selected",
            |a| {
                dialog(a).is_none()
                    && names(a) == ["Papers"]
                    && selected(a).as_deref() == Some("Papers")
                    && compare_profiles::profile("Docs").is_none()
            },
            key(KeyCode::Char('r')),
        ),
        Step::new(
            "the run ended with a report, listed under the profile",
            |a| {
                !compare_profiles::is_running("Papers")
                    && compare_profiles::last_report("Papers").is_some()
                    && common::wait_jobs_idle(a)
                    && page(a).and_then(|p| p.detail.as_ref()).is_some_and(|d| {
                        d.last.0 != tr!("tools-profiles-never-run")
                            && !d.reports.is_empty()
                            && d.reports[0].0 != tr!("common-loading")
                            && d.reports[0].0 != tr!("tools-profiles-no-runs")
                    })
            },
            key(KeyCode::Delete),
        ),
        Step::new(
            "the delete confirmation",
            |a| dialog(a).is_some_and(|d| d.alert && d.title.contains("Papers")),
            key(KeyCode::Char('d')),
        ),
        Step::new(
            "deleted",
            |a| {
                dialog(a).is_none()
                    && page(a).is_some_and(|p| p.empty.is_some())
                    && compare_profiles::profiles().is_empty()
            },
            ctrl('q'),
        ),
    ]);
    common::run_script(&mut app, steps, 120, 36);
    assert!(soyuz_appcore::services::store_file::flush(Duration::from_secs(10)));
    common::ok();
}

#[test]
fn validation_errors_and_file_round_trip() {
    match common::relaunch(module_path!(), "validation_errors_and_file_round_trip", |data| {
        folders(data);
        seed(data, &[existing(data)]);
    }) {
        Run::Child(data) => validation(data),
        Run::Parent(r) => {
            // Saving the profile unchanged writes it back as it was.
            assert_eq!(compare_profiles::load_profiles_in(&r.config_dir()), vec![existing(&r.data)], "{}", r.log);
        }
    }
}

fn validation(data: PathBuf) {
    let src = data.join("src");
    let mut app = common::new_app(config(), &data);
    let d = data.clone();
    let mut steps = vec![
        Step::new("the listing", move |a| common::at(a, &d), key(KeyCode::F(12))),
        Step::new(
            "the seeded profile with its schedule",
            |a| {
                names(a) == ["Existing"]
                    && page(a).and_then(|p| p.detail.as_ref()).is_some_and(|d| d.schedule.contains("07:15"))
            },
            key(KeyCode::Char('n')),
        ),
        Step::new("the new profile dialog", |a| dialog(a).is_some(), key(KeyCode::Enter)),
        Step::new(
            "an empty name is refused",
            |a| error(a).as_deref() == Some(tr!("tui-cmpprof-err-empty-name")) && dialog(a).unwrap().focus == 0,
            text("Existing"),
        ),
        Step::new("a taken name", |a| error(a).is_none(), key(KeyCode::Enter)),
        Step::new(
            "the name is taken",
            |a| error(a) == Some(tr!("tui-cmpprof-err-duplicate", name = "Existing")) && dialog(a).unwrap().focus == 0,
            text(" 2"),
        ),
        Step::new("another name", |_| true, key(KeyCode::Enter)),
        Step::new(
            "without folders",
            |a| error(a).as_deref() == Some(tr!("tui-cmpprof-err-missing-path")) && dialog(a).unwrap().focus == 1,
            paste(&src),
        ),
        Step::new("a left folder", |_| true, key(KeyCode::Tab)),
        Step::new("the right folder", |a| dialog(a).is_some_and(|d| d.focus == 2), paste(&src)),
        Step::new("the same folder on both sides", |_| true, key(KeyCode::Enter)),
    ];
    steps.extend([
        Step::new(
            "the same folders are refused",
            |a| error(a).as_deref() == Some(tr!("tui-cmpprof-err-same-paths")) && dialog(a).unwrap().focus == 2,
            key(KeyCode::Esc),
        ),
        Step::new(
            "cancelled: nothing saved",
            |a| dialog(a).is_none() && names(a) == ["Existing"],
            key(KeyCode::Char('e')),
        ),
        Step::new("the edit dialog", |a| dialog(a).is_some(), key(KeyCode::Tab)),
    ]);
    // To the time field: name → left → right → method → mode → schedule → time (the
    // minutes field is off for a daily schedule).
    keys(&mut steps, KeyCode::Tab, 5);
    steps.extend([
        Step::new("on the time", |a| dialog(a).is_some_and(|d| d.focus == 7), text("x")),
        Step::new("a bad time", |_| true, key(KeyCode::Enter)),
        Step::new(
            "a bad time is refused",
            |a| error(a).as_deref() == Some(tr!("tui-cmpprof-err-time")) && dialog(a).unwrap().focus == 7,
            key(KeyCode::Backspace),
        ),
        Step::new("07:15 again", |a| error(a).is_none(), shift(KeyCode::Tab)),
        Step::new("the schedule", |a| dialog(a).is_some_and(|d| d.focus == 5), key(KeyCode::Enter)),
        Step::new(
            "saved unchanged",
            |a| dialog(a).is_none() && common::toasts(a).iter().any(|t| t.contains("Existing")),
            ctrl('q'),
        ),
    ]);
    common::run_script(&mut app, steps, 120, 36);
    assert!(soyuz_appcore::services::store_file::flush(Duration::from_secs(10)));
    common::ok();
}

#[test]
fn delete_is_refused_while_running() {
    match common::relaunch(module_path!(), "delete_is_refused_while_running", |data| {
        let (l, r) = (data.join("src"), data.join("dst"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        // Same size, compared byte by byte: the planning lasts long enough.
        for d in [&l, &r] {
            let f = std::fs::File::create(d.join("big.bin")).unwrap();
            f.set_len(1 << 30).unwrap();
        }
        seed(data, &[SyncProfile { schedule: Schedule::Manual, ..existing(data) }]);
    }) {
        Run::Child(data) => running(data),
        Run::Parent(r) => assert!(compare_profiles::load_profiles_in(&r.config_dir()).is_empty(), "{}", r.log),
    }
}

fn running(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let d = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| common::at(a, &d), key(KeyCode::F(12))),
        Step::new("the profile", |a| names(a) == ["Existing"], key(KeyCode::Char('r'))),
        Step::new(
            "running",
            |a| compare_profiles::is_running("Existing") && page(a).is_some_and(|p| p.running && p.rows[0].running),
            key(KeyCode::Delete),
        ),
        Step::new("the delete confirmation", |a| dialog(a).is_some_and(|d| d.alert), key(KeyCode::Char('d'))),
        Step::new(
            "refused while it runs",
            |a| {
                error(a) == Some(tr!("tui-cmpprof-err-running", name = "Existing"))
                    && compare_profiles::profile("Existing").is_some()
            },
            key(KeyCode::Esc),
        ),
        Step::new("closed", |a| dialog(a).is_none(), key(KeyCode::Char('x'))),
        Step::new(
            "the run was cancelled",
            |a| !compare_profiles::is_running("Existing") && common::wait_jobs_idle(a),
            key(KeyCode::Delete),
        ),
        Step::new("the delete confirmation again", |a| dialog(a).is_some_and(|d| d.alert), key(KeyCode::Enter)),
        Step::new("deleted", |a| dialog(a).is_none() && names(a).is_empty(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 120, 36);
    assert!(soyuz_appcore::services::store_file::flush(Duration::from_secs(10)));
    common::ok();
}

#[test]
fn tiny_sizes_with_profiles_and_the_dialog() {
    match common::relaunch(module_path!(), "tiny_sizes_with_profiles_and_the_dialog", |data| {
        folders(data);
        seed(data, &[existing(data)]);
    }) {
        Run::Child(data) => tiny(data),
        Run::Parent(_) => {}
    }
}

fn tiny(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let d = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| common::at(a, &d), key(KeyCode::F(12))),
        Step::new("the profile", |a| names(a) == ["Existing"], ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    // Quit leaves the screen's state: frames at every small size, on the page, then with
    // the edit, rename and delete dialogs over it.
    let mut io = NullIo::default();
    let events = [
        None,
        Some(key(KeyCode::Char('e'))),
        Some(key(KeyCode::Esc)),
        Some(key(KeyCode::F(2))),
        Some(key(KeyCode::Esc)),
        Some(key(KeyCode::Delete)),
    ];
    for ev in events {
        if let Some(ev) = ev {
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            app.frame(&mut terminal, &[ev], &mut io).expect("a frame");
        }
        for w in (0..=40).step_by(3) {
            for h in 0..=16 {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                app.frame(&mut terminal, &[], &mut io).expect("a frame");
            }
        }
    }
    assert!(app.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.alert));
    app.finish();
    common::ok();
}
