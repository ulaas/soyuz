//! Core behaviour that protects the user's data and choices, on a real `AppCore` with no
//! window: `config.toml` is never replaced while it does not load, unsaved settings survive
//! a reload, a toast's Undo reverses the operation it names, a folder already gone falls
//! back to its nearest ancestor, and a late sync-navigation answer is ignored.
//!
//! Like `headless_core`, the test re-runs itself from a copy of this binary next to a
//! `portable` marker: nothing outside a temporary directory is read or written.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::{AppCore, Startup};
use soyuz_appcore::host::{Host, NullHost};
use soyuz_appcore::models::settings::{self, flush, update_config};
use soyuz_appcore::ops::fileops::FileOpCmd;
use soyuz_appcore::ops::fileops::drop::DropEffect;
use soyuz_appcore::services::config_watch::{self, ConfigWatch};
use soyuz_appcore::services::mailbox::UiMsg;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::{self, Config};

const CHILD: &str = "SOYUZ_DATA_SAFETY_DATA";
const TEST: &str = "data_safety";

#[test]
fn data_safety() {
    if let Some(data) = std::env::var_os(CHILD) {
        run_all(&PathBuf::from(data));
        return;
    }
    let tmp = tempfile::tempdir().expect("temp dir");
    let exe = std::env::current_exe().expect("test binary");
    let copy = tmp.path().join(exe.file_name().expect("exe name"));
    std::fs::copy(&exe, &copy).expect("copy the test binary");
    std::fs::write(tmp.path().join(config::PORTABLE_MARKER), b"").expect("portable marker");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let out = Command::new(&copy)
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, &data)
        .output()
        .expect("run the copied test binary");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "child failed:\n{log}");
    assert!(log.contains("data safety: ok"), "the child did not run the test:\n{log}");
}

fn run_all(data: &Path) {
    let dir = config::config_dir();
    assert!(dir.ends_with(config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());
    let path = dir.join("config.toml");
    std::fs::create_dir_all(&dir).unwrap();

    broken_file_at_startup_is_told_and_kept(&path, data);
    std::fs::remove_file(&path).unwrap();
    bad_value_found_at_save_time_is_kept(&path, data);
    std::fs::remove_file(&path).unwrap();
    held_change_is_saved_after_the_fix(&path, data);
    std::fs::remove_file(&path).unwrap();
    external_edit_keeps_a_pending_setting(&path, data);
    std::fs::remove_file(&path).unwrap();
    reload_without_pending_changes_replaces(&path, data);
    background_save_writes_and_keeps_the_refusal(&path, data);
    toast_undo_targets_the_operation_it_names(data);
    folder_gone_at_navigation_falls_back(data);
    late_probe_answer_is_ignored(data);
    println!("data safety: ok");
}

fn core(data: &Path) -> AppCore {
    let mut c = Config::default();
    c.updates.weekly_check = false;
    c.cli.server_enabled = false;
    c.search.index.enabled = false;
    c.layout.preview_pane = false;
    let startup = Startup { open: Some(data.to_path_buf()), secondary: true, ..Default::default() };
    AppCore::new(c, startup, NullHost::default().wake(), soyuz_appcore::services::preview_gfx::no_upload())
}

/// A core whose config file is `path`, with no watcher: the test delivers reloads itself.
fn core_on(path: &Path, data: &Path) -> AppCore {
    let mut app = core(data);
    app.config_watch = ConfigWatch::disabled(path.to_path_buf());
    app
}

fn tick(app: &mut AppCore, host: &dyn Host) {
    app.begin_frame();
    app.pump_services(host);
    app.pump_jobs(host);
    app.pump_background(host);
    app.apply_cmds(host);
    app.prepare_visible_panes();
    app.apply_cmds(host);
    app.end_frame();
}

fn settle(app: &mut AppCore, host: &dyn Host, what: &str, done: impl Fn(&AppCore) -> bool) {
    let start = Instant::now();
    loop {
        tick(app, host);
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn has_toast(app: &AppCore, title: &str) -> bool {
    app.ui.toasts.live.iter().any(|l| l.toast.title == title)
}

fn broken_file_at_startup_is_told_and_kept(path: &Path, data: &Path) {
    let host = NullHost::default();
    let broken = "[general\nshow_hidden = maybe\n";
    std::fs::write(path, broken).unwrap();
    // The real startup path: the watcher finds out, and a toast says so.
    let mut app = core(data);
    assert_eq!(app.config_watch.path(), path);
    assert!(app.config_watch.load_error.is_some(), "the file does not load");
    assert!(!app.ui.toasts.live.is_empty(), "the user is told at startup");
    update_config(&mut app, &host, |c| c.general.show_hidden = !c.general.show_hidden);
    flush(&mut app);
    assert_eq!(std::fs::read_to_string(path).unwrap(), broken, "the file is untouched");
    assert!(app.config_save.held);
    assert!(has_toast(&app, tr!("settings-save-refused")));
}

fn bad_value_found_at_save_time_is_kept(path: &Path, data: &Path) {
    let host = NullHost::default();
    // Valid TOML that does not fit the schema, and text that broke after the last load.
    for broken in ["[appearance]\nsidebar_width = \"wide\"\n", "[[[ nope"] {
        std::fs::write(path, broken).unwrap();
        let mut app = core_on(path, data);
        update_config(&mut app, &host, |c| c.general.show_hidden = !c.general.show_hidden);
        flush(&mut app);
        assert_eq!(std::fs::read_to_string(path).unwrap(), broken, "the file is untouched");
        assert!(has_toast(&app, tr!("settings-save-refused")));
    }
}

fn held_change_is_saved_after_the_fix(path: &Path, data: &Path) {
    let host = NullHost::default();
    let broken = "[[[ nope";
    std::fs::write(path, broken).unwrap();
    let mut app = core_on(path, data);
    app.config_watch.load_error = config::parse_str(broken).err();
    let wanted = !app.config.general.natural_sort;
    update_config(&mut app, &host, |c| c.general.natural_sort = wanted);
    flush(&mut app);
    assert!(app.config_save.held && app.config_save.due.is_none());

    // The user fixes the file, with a setting and a comment of their own.
    let fixed = "# mine\n[appearance]\nsidebar_width = 333\n";
    std::fs::write(path, fixed).unwrap();
    config_watch::handle(&mut app, &host, config::parse_str(fixed), false);
    assert!(app.config_watch.load_error.is_none());
    assert!(app.config_save.due.is_some(), "saving is enabled again");
    assert_eq!(app.config.appearance.sidebar_width, 333.0, "the file's setting");
    assert_eq!(app.config.general.natural_sort, wanted, "the held change");
    flush(&mut app);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.starts_with("# mine"), "{text}");
    let back = config::parse_str(&text).unwrap().config;
    assert_eq!((back.appearance.sidebar_width, back.general.natural_sort), (333.0, wanted));
}

fn external_edit_keeps_a_pending_setting(path: &Path, data: &Path) {
    let host = NullHost::default();
    let mut app = core_on(path, data);
    let wanted = !app.config.general.natural_sort;
    update_config(&mut app, &host, |c| c.general.natural_sort = wanted);
    // Before the save delay is over, another program edits the file.
    let text = "[appearance]\nsidebar_width = 333\n";
    std::fs::write(path, text).unwrap();
    config_watch::handle(&mut app, &host, config::parse_str(text), false);
    assert_eq!(app.config.general.natural_sort, wanted, "the toggle survives");
    assert_eq!(app.config.appearance.sidebar_width, 333.0, "the edit applies");
    flush(&mut app);
    let back = config::parse_str(&std::fs::read_to_string(path).unwrap()).unwrap().config;
    assert_eq!((back.general.natural_sort, back.appearance.sidebar_width), (wanted, 333.0));
}

fn reload_without_pending_changes_replaces(path: &Path, data: &Path) {
    let host = NullHost::default();
    let mut app = core_on(path, data);
    let text = "[appearance]\nsidebar_width = 222\n";
    config_watch::handle(&mut app, &host, config::parse_str(text), false);
    assert_eq!(app.config.appearance.sidebar_width, 222.0);
    assert!(!app.config_save.pending());
}

/// The debounced save runs on a worker: it reports back like a direct write, and the
/// refusal for a file that does not load still holds.
fn background_save_writes_and_keeps_the_refusal(path: &Path, data: &Path) {
    let host = NullHost::default();
    let _ = std::fs::remove_file(path);
    std::fs::write(path, "# mine\n").unwrap();
    let mut app = core_on(path, data);
    let wanted = !app.config.general.natural_sort;
    update_config(&mut app, &host, |c| c.general.natural_sort = wanted);
    settings::save_in_background(&mut app);
    // A change made while the write runs is not lost: it stays due, and goes out next.
    let later = app.config.appearance.sidebar_width + 7.0;
    update_config(&mut app, &host, |c| c.appearance.sidebar_width = later);
    let start = Instant::now();
    while settings::settle(&mut app) {
        assert!(start.elapsed() < Duration::from_secs(30), "the worker answers");
        std::thread::sleep(Duration::from_millis(5));
    }
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("# mine"), "{text}");
    assert_eq!(app.config_watch.last_text.as_deref(), Some(text.as_str()), "our own write is known");
    assert!(app.config_save.flashed.is_some());
    assert!(app.config_save.due.is_some(), "the later change is still pending");
    let baseline = app.config_save.baseline.as_ref().expect("baseline");
    assert_eq!(baseline.general.natural_sort, wanted);
    assert_ne!(baseline.appearance.sidebar_width, later, "the baseline is what was written");
    flush(&mut app);
    let back = config::parse_str(&std::fs::read_to_string(path).unwrap()).unwrap().config;
    assert_eq!((back.general.natural_sort, back.appearance.sidebar_width), (wanted, later));

    // A file that stopped loading is refused before any worker starts.
    let broken = "[[[ nope";
    std::fs::write(path, broken).unwrap();
    app.config_watch.load_error = config::parse_str(broken).err();
    update_config(&mut app, &host, |c| c.general.natural_sort = !wanted);
    settings::save_in_background(&mut app);
    assert!(!settings::settle(&mut app), "no write started");
    assert_eq!(std::fs::read_to_string(path).unwrap(), broken);
    assert!(app.config_save.held);
    assert!(has_toast(&app, tr!("settings-save-refused")));

    // A file that broke after the last load: the worker reads it and leaves it alone.
    app.config_watch.load_error = None;
    update_config(&mut app, &host, |c| c.general.natural_sort = wanted);
    settings::save_in_background(&mut app);
    settings::wait_for_save(&mut app);
    assert_eq!(std::fs::read_to_string(path).unwrap(), broken);
    assert!(app.config_save.held);
    std::fs::remove_file(path).unwrap();
}

/// Moves and renames only: the undo here never reaches the real trash.
fn toast_undo_targets_the_operation_it_names(data: &Path) {
    let dir = data.join("undo");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(dir.join("moved.txt"), b"m").unwrap();
    std::fs::write(dir.join("before.txt"), b"r").unwrap();

    let host = NullHost::default();
    let mut app = core(&dir);
    app.cmds.push(Cmd::FileOp(FileOpCmd::Transfer {
        paths: vec![dir.join("moved.txt")],
        dest: sub.clone(),
        effect: DropEffect::Move,
    }));
    settle(&mut app, &host, "the move's toast", |a| a.ui.toasts.live.iter().any(|l| l.toast.undoes.is_some()));
    assert!(sub.join("moved.txt").exists());
    let toast = app.ui.toasts.live.iter().find(|l| l.toast.undoes.is_some()).unwrap().toast.clone();

    // A quiet rename finishes within the toast's seven seconds: it is the newest entry now.
    let pane = app.ws.pane().id;
    app.cmds.push(Cmd::FileOp(FileOpCmd::Rename { pane, from: dir.join("before.txt"), to: "after.txt".into() }));
    settle(&mut app, &host, "the rename", |a| a.fileops.jobs.history().len() == 2);
    assert!(dir.join("after.txt").exists());

    app.cmds.push(toast.command().expect("the Undo button"));
    settle(&mut app, &host, "the undo", |a| a.fileops.jobs.history().iter().any(|h| h.undone));
    settle(&mut app, &host, "the file back", |_| dir.join("moved.txt").exists());
    assert!(!sub.join("moved.txt").exists(), "the operation the toast named was undone");
    assert!(dir.join("after.txt").exists() && !dir.join("before.txt").exists(), "the rename stays");
}

fn folder_gone_at_navigation_falls_back(data: &Path) {
    let parent = data.join("gone");
    std::fs::create_dir_all(&parent).unwrap();
    let missing = parent.join("never").join("there");
    let host = NullHost::default();
    let mut app = core(&parent);
    let pane = app.ws.pane().id;
    soyuz_appcore::ops::navigate(&mut app, pane, Location::Dir(missing), soyuz_appcore::ops::NavOpts::user());
    let want = Location::Dir(parent.clone());
    settle(&mut app, &host, "the ancestor", |a| a.ws.pane().location == want);
    assert!(has_toast(&app, tr!("ops-folder-gone")));
}

fn late_probe_answer_is_ignored(data: &Path) {
    let (a, b, c) = (data.join("probe-a"), data.join("probe-b"), data.join("probe-c"));
    for d in [&a, &b, &c] {
        std::fs::create_dir_all(d).unwrap();
    }
    let host = NullHost::default();
    let mut app = core(&a);
    let pane = app.ws.pane().id;
    let asked = app.ws.pane().generation;
    // The user moves on while a slow drive is being asked about `c`.
    soyuz_appcore::ops::navigate(&mut app, pane, Location::Dir(b.clone()), soyuz_appcore::ops::NavOpts::user());
    let probed = |generation| UiMsg::Probed { pane, generation, location: Location::Dir(c.clone()), exists: true };
    soyuz_appcore::ops::handle_msg(&mut app, probed(asked));
    assert_eq!(app.ws.pane().location, Location::Dir(b.clone()), "the stale answer changes nothing");
    let current = app.ws.pane().generation;
    soyuz_appcore::ops::handle_msg(&mut app, probed(current));
    assert_eq!(app.ws.pane().location, Location::Dir(c.clone()));
    tick(&mut app, &host);
}
