//! Headless core (Phase T0): builds an `AppCore` with no window and no egui context
//! (`NullHost`, no texture uploader), lists a temporary folder, runs actions through the
//! same dispatcher the GUI uses (select all, show hidden files, new folder through the
//! jobs engine) and ticks the core frame phases until the results settle.
//!
//! The core keeps its settings, session, palette memory and job history in the config
//! directory, which follows the executable in portable mode. So the test re-runs itself
//! from a copy of this test binary placed in a temporary directory next to a `portable`
//! marker: nothing outside that directory is read or written.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use soyuz_appcore::actions;
use soyuz_appcore::core::{AppCore, Startup};
use soyuz_appcore::host::{Host, NullHost, WindowCmd};
use soyuz_core::config::{self, Action, Config};

/// Set in the re-run child: the folder to list.
const CHILD: &str = "SOYUZ_HEADLESS_CORE_DATA";
const TEST: &str = "core_runs_without_a_window";

#[test]
fn core_runs_without_a_window() {
    if let Some(data) = std::env::var_os(CHILD) {
        run_core(PathBuf::from(data));
        return;
    }
    let tmp = tempfile::tempdir().expect("temp dir");
    let exe = std::env::current_exe().expect("test binary");
    let copy = tmp.path().join(exe.file_name().expect("exe name"));
    std::fs::copy(&exe, &copy).expect("copy the test binary");
    std::fs::write(tmp.path().join(config::PORTABLE_MARKER), b"").expect("portable marker");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(data.join("sub")).unwrap();
    std::fs::write(data.join("a.txt"), b"a").unwrap();
    std::fs::write(data.join("b.txt"), b"bb").unwrap();
    std::fs::write(data.join(".dot"), b"hidden").unwrap();

    let out = Command::new(&copy)
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, &data)
        .output()
        .expect("run the copied test binary");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "headless run failed:\n{log}");
    assert!(log.contains("headless core: ok"), "the child did not run the test:\n{log}");
    let dirs = std::fs::read_dir(&data).unwrap().filter_map(Result::ok).filter(|e| e.path().is_dir()).count();
    assert_eq!(dirs, 2, "sub + the new folder");
    assert!(tmp.path().join(config::PORTABLE_DATA_DIR).is_dir(), "config went to the portable data dir");
}

/// One frame of a frontend without a window (the GUI runs its own pumps in between).
fn tick(app: &mut AppCore, host: &dyn Host) {
    app.begin_frame();
    app.pump_services(host);
    app.pump_jobs(host);
    app.pump_background(host);
    app.apply_cmds(host);
    app.prepare_visible_panes();
    app.apply_cmds(host);
    app.update_title(host);
    app.end_frame();
}

/// Tick until `done` holds (or fail after a generous timeout).
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

fn names(app: &AppCore) -> Vec<String> {
    let pane = app.ws.pane();
    let mut v: Vec<String> = pane.visible.iter().map(|&i| pane.entries[i].name.clone()).collect();
    v.sort();
    v
}

fn run_core(data: PathBuf) {
    // Never fall back to the real config directory.
    let dir = config::config_dir();
    let root = data.parent().expect("temp root");
    assert!(dir.ends_with(config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());
    assert_eq!(
        dir.parent().and_then(|d| d.canonicalize().ok()),
        root.canonicalize().ok(),
        "config dir {} is outside the temp root",
        dir.display()
    );

    let mut c = Config::default();
    c.general.show_hidden = false;
    c.general.honour_dot_hidden = true;
    c.updates.weekly_check = false;
    c.cli.server_enabled = false;
    c.search.index.enabled = false;
    let startup = Startup { open: Some(data.clone()), secondary: true, safe_mode: false, ..Default::default() };
    let host = NullHost::default();
    let mut app = AppCore::new(c, startup, host.wake(), soyuz_appcore::services::preview_gfx::no_upload());
    assert_eq!(app.ws.pane().location, soyuz_core::Location::Dir(data.clone()));

    // The first listing.
    settle(&mut app, &host, "the listing", |a| names(a) == ["a.txt", "b.txt", "sub"]);
    let title =
        host.windows.lock().unwrap().iter().any(|(_, c)| matches!(c, WindowCmd::Title(t) if t.contains("data")));
    assert!(title, "the title follows the pane");

    // Select all.
    actions::execute(&mut app, Action::SelectAll, &host);
    assert_eq!(app.ws.pane().selection.len(), 3);

    // Show hidden files: the dot file appears once the pane re-filters.
    actions::execute(&mut app, Action::ToggleHidden, &host);
    assert!(app.ui.show_hidden);
    settle(&mut app, &host, "hidden files", |a| names(a).iter().any(|n| n == ".dot"));

    // New folder: a job on the jobs engine, then the watcher / refresh shows it.
    let before = names(&app).len();
    actions::execute(&mut app, Action::NewFolder, &host);
    settle(&mut app, &host, "the new folder", |a| {
        a.fileops.jobs.pending().count() == 0
            && names(a).len() == before + 1
            && a.ws.pane().entries.iter().filter(|e| e.is_dir_like()).count() == 2
    });
    let new_dir =
        app.ws.pane().entries.iter().find(|e| e.is_dir_like() && e.name != "sub").expect("new folder").path.clone();
    assert!(Path::new(&new_dir).is_dir(), "{} exists on disk", new_dir.display());

    // Core prompts and frontend requests stay queued for a frontend; none is needed here.
    assert!(!app.prompt_open());
    app.save_all();
    println!("headless core: ok");
}
