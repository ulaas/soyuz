//! Folder compare on a headless core: `CompareModel` scans two temp trees, the two-way
//! conflict is answered, the plan is submitted as one `JobKind::Sync` job through the jobs
//! engine, the new two-way state is recorded after the job and the folders compare again
//! as in sync.
//!
//! The core keeps its job history and the two-way states in the config directory, so the
//! test re-runs itself from a copy of this binary next to a `portable` marker (like
//! `headless_core.rs`): nothing outside the temporary directory is read or written.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use soyuz_appcore::core::{AppCore, Startup};
use soyuz_appcore::host::{Host, NullHost};
use soyuz_appcore::models::compare::{CompareModel, Preset, ReviewStep, RowAction, Stage};
use soyuz_appcore::models::compare_state::{self, Resolution};
use soyuz_core::config::{self, Config};

/// Set in the re-run child: the temp root holding `L` and `R`.
const CHILD: &str = "SOYUZ_COMPARE_MODEL_DATA";
const TEST: &str = "compare_model_scans_resolves_and_syncs";

#[test]
fn compare_model_scans_resolves_and_syncs() {
    if let Some(data) = std::env::var_os(CHILD) {
        run(PathBuf::from(data));
        return;
    }
    let tmp = tempfile::tempdir().expect("temp dir");
    let exe = std::env::current_exe().expect("test binary");
    let copy = tmp.path().join(exe.file_name().expect("exe name"));
    std::fs::copy(&exe, &copy).expect("copy the test binary");
    std::fs::write(tmp.path().join(config::PORTABLE_MARKER), b"").expect("portable marker");
    let data = tmp.path().join("data");
    let (l, r) = (data.join("L"), data.join("R"));
    std::fs::create_dir_all(l.join("sub")).unwrap();
    std::fs::create_dir_all(&r).unwrap();
    std::fs::write(l.join("only-left.txt"), b"left").unwrap();
    std::fs::write(l.join("sub/deep.txt"), b"deep").unwrap();
    std::fs::write(r.join("only-right.txt"), b"right").unwrap();
    std::fs::write(l.join("both.txt"), b"left version").unwrap();
    std::fs::write(r.join("both.txt"), b"right!").unwrap();

    let out = Command::new(&copy)
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, &data)
        .output()
        .expect("run the copied test binary");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "compare run failed:\n{log}");
    assert!(log.contains("compare model: ok"), "the child did not run the test:\n{log}");
    // Both sides now hold everything, the conflict kept both versions.
    for side in [&l, &r] {
        for f in ["only-left.txt", "only-right.txt", "sub/deep.txt", "both.txt"] {
            assert!(side.join(f).is_file(), "{} has {f}", side.display());
        }
        let copies = std::fs::read_dir(side)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("both (conflict right "))
            .count();
        assert_eq!(copies, 1, "{} has the conflict copy", side.display());
    }
    assert_eq!(std::fs::read(l.join("both.txt")).unwrap(), b"left version", "keep both keeps the left version");
}

/// One frame of a frontend without a window.
fn tick(app: &mut AppCore, host: &dyn Host) {
    app.begin_frame();
    app.pump_services(host);
    app.pump_jobs(host);
    app.pump_background(host);
    app.apply_cmds(host);
    app.end_frame();
}

/// Tick (and poll the model) until `done` holds.
fn settle(app: &mut AppCore, host: &dyn Host, m: &mut CompareModel, what: &str, done: impl Fn(&CompareModel) -> bool) {
    let start = Instant::now();
    loop {
        tick(app, host);
        m.poll(app);
        if done(m) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run(data: PathBuf) {
    let dir = config::config_dir();
    assert!(dir.ends_with(config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());
    assert_eq!(
        dir.parent().and_then(|d| d.canonicalize().ok()),
        data.parent().and_then(|d| d.canonicalize().ok()),
        "config dir {} is outside the temp root",
        dir.display()
    );
    let (l, r) = (data.join("L"), data.join("R"));

    let mut c = Config::default();
    c.updates.weekly_check = false;
    c.cli.server_enabled = false;
    c.search.index.enabled = false;
    c.sync.scheduler = false;
    let startup = Startup { open: Some(data.clone()), secondary: true, safe_mode: false, ..Default::default() };
    let host = NullHost::default();
    let mut app = AppCore::new(c, startup, host.wake(), soyuz_appcore::services::preview_gfx::no_upload());

    // Two-way, first run: the file edited on both sides with close times is a conflict.
    let mut m = CompareModel::new(l.clone(), None, Vec::new(), host.wake());
    m.right = r.display().to_string();
    m.preset = Preset::TwoWay;
    m.start();
    settle(&mut app, &host, &mut m, "the scan", |m| !m.is_scanning());
    assert!(matches!(m.stage(), Stage::Review), "{:?}", m.error());
    assert!(m.first_run());
    let (total, open) = m.conflict_counts();
    assert_eq!((total, open), (1, 1), "{:?}", m.rows());
    m.step = ReviewStep::Conflicts;
    let ci = m.conflict_rows()[0];
    assert!(m.resolve(ci, Resolution::KeepBoth));
    assert_eq!(m.rows()[ci].action, RowAction::KeepBoth);

    // Submit: one sync job through the jobs engine; then the state is recorded and the
    // folders are compared again.
    m.step = ReviewStep::Plan;
    let job = m.submit(&mut app).expect("a plan with actions");
    assert_eq!(m.job(), Some(job));
    assert!(app.fileops.jobs.snapshot(job).is_some(), "the job is on the engine");
    settle(&mut app, &host, &mut m, "the sync, the state and the rescan", |m| matches!(m.stage(), Stage::Review));
    let snap = app.fileops.jobs.snapshot(job).expect("job snapshot");
    assert!(matches!(snap.state, soyuz_jobs::JobState::Done(_)), "{:?}", snap.state);
    assert!(m.rows().is_empty(), "in sync after the job: {:?}", m.rows());
    assert!(!m.first_run(), "the two-way state was recorded");
    assert!(compare_state::load_state(&compare_state::state_path(&l, &r)).is_some(), "state file saved");
    assert!(m.plan().is_some_and(|p| p.actions.is_empty()));
    assert!(m.submit(&mut app).is_none(), "nothing to submit");

    app.save_all();
    println!("compare model: ok");
}
