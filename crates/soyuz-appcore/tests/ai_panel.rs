//! The AI panel's toolkit-free logic on a headless core: `@` mention candidates from the
//! active pane (selection, this folder, open tabs, matching entries) and the "Esc stops
//! the running task" rule. Like `headless_core.rs`, the test re-runs itself from a copy
//! of this binary next to a `portable` marker, so the core's config directory is inside
//! a temporary folder and nothing outside it is read or written.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use soyuz_appcore::actions;
use soyuz_appcore::ai::panel::{self, Mention};
use soyuz_appcore::core::{AppCore, Startup};
use soyuz_appcore::host::{Host, NullHost};
use soyuz_appcore::icon::IconId;
use soyuz_core::config::{self, Action, Config};

/// Set in the re-run child: the folder to list.
const CHILD: &str = "SOYUZ_AI_PANEL_DATA";
const TEST: &str = "mention_candidates_on_a_headless_core";

#[test]
fn mention_candidates_on_a_headless_core() {
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
    std::fs::write(data.join("alpha.txt"), b"a").unwrap();
    std::fs::write(data.join("beta.txt"), b"bb").unwrap();

    let out = Command::new(&copy)
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, &data)
        .output()
        .expect("run the copied test binary");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "headless run failed:\n{log}");
    assert!(log.contains("ai panel: ok"), "the child did not run the test:\n{log}");
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

fn icons(m: &[Mention]) -> Vec<IconId> {
    m.iter().map(|m| m.icon).collect()
}

fn run_core(data: PathBuf) {
    let dir = config::config_dir();
    assert!(dir.ends_with(config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());

    let mut c = Config::default();
    c.updates.weekly_check = false;
    c.cli.server_enabled = false;
    c.search.index.enabled = false;
    let startup = Startup { open: Some(data.clone()), secondary: true, safe_mode: false, ..Default::default() };
    let host = NullHost::default();
    let mut app = AppCore::new(c, startup, host.wake(), soyuz_appcore::services::preview_gfx::no_upload());
    settle(&mut app, &host, "the listing", |a| a.ws.pane().entries.len() == 3);

    // Empty query, nothing selected: only "this folder" (the tab shows the same folder).
    let m = panel::mention_candidates(&app, "");
    assert_eq!(icons(&m), vec![IconId::FolderOpen], "{m:?}");
    assert_eq!(m[0].paths, vec![data.clone()]);

    // A query matches entries of the pane by name (case-insensitive).
    let m = panel::mention_candidates(&app, "ALPHA");
    assert_eq!(icons(&m), vec![IconId::File], "{m:?}");
    assert_eq!(m[0].label, "alpha.txt");
    assert_eq!(m[0].paths, vec![data.join("alpha.txt")]);
    let m = panel::mention_candidates(&app, "sub");
    assert_eq!(icons(&m), vec![IconId::FolderSimple], "{m:?}");
    let m = panel::mention_candidates(&app, "here");
    assert_eq!(icons(&m), vec![IconId::FolderOpen], "{m:?}");
    assert!(panel::mention_candidates(&app, "zzz-nothing").is_empty());

    // With a selection, "selection" comes first and carries every selected path.
    actions::execute(&mut app, Action::SelectAll, &host);
    let m = panel::mention_candidates(&app, "");
    assert_eq!(icons(&m), vec![IconId::SelectionAll, IconId::FolderOpen], "{m:?}");
    assert_eq!(m[0].paths.len(), 3);
    let m = panel::mention_candidates(&app, "sel");
    assert_eq!(icons(&m)[0], IconId::SelectionAll, "{m:?}");

    // Esc stops nothing while no task runs, panel open or not.
    app.ai.panel.open = true;
    assert!(!panel::escape_stops_task(&app));
    app.ai.panel.open = false;
    assert!(!panel::escape_stops_task(&app));

    app.save_all();
    println!("ai panel: ok");
}
