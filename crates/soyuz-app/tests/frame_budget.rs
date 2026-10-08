//! Scripted UI tour with `SOYUZ_FRAME_LOG`: after the initial load no frame may spend
//! more than the budget (16 ms by default, `SOYUZ_FRAME_BUDGET_MS` to change) in `ui()`.
//!
//! Launches the real binary on a temporary folder, in portable mode (a copy of the exe
//! next to a `portable` marker, so no user settings are read or written). Needs a
//! desktop session; meant for release builds:
//! `cargo test -p soyuz-app --release --test frame_budget -- --ignored --nocapture`.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `frame <n> ui <ms> ms` lines after the `mark` step → frame times in ms.
fn frame_times(log: &str, mark: &str) -> Vec<f64> {
    let mut counting = false;
    let mut out = Vec::new();
    for line in log.lines() {
        if line.contains(&format!("SOYUZ_SCRIPT: mark {mark}")) {
            counting = true;
            continue;
        }
        if !counting {
            continue;
        }
        if let Some(rest) = line.split("frame ").nth(1)
            && let Some(ms) = rest.split(" ui ").nth(1).and_then(|s| s.trim().strip_suffix("ms"))
            && let Ok(v) = ms.trim().parse::<f64>()
        {
            out.push(v);
        }
    }
    out
}

#[test]
fn parses_frame_log() {
    let log = "frame 1 ui 40.00 ms\nINFO SOYUZ_SCRIPT: mark tour\nINFO frame 2 ui 3.25 ms\nx frame 3 ui 12.00 ms\n";
    assert_eq!(frame_times(log, "tour"), vec![3.25, 12.0]);
}

#[test]
#[ignore = "launches the GUI; run manually or in a desktop CI job"]
fn scripted_tour_stays_within_frame_budget() {
    let budget: f64 = std::env::var("SOYUZ_FRAME_BUDGET_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(16.0);
    let tmp = tempfile::tempdir().unwrap();
    // A folder worth browsing: 5 000 files and a few subfolders.
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    for d in ["alpha", "beta", "gamma"] {
        std::fs::create_dir(data.join(d)).unwrap();
    }
    for i in 0..5_000 {
        std::fs::write(data.join(format!("file_{i:05}.txt")), b"x").unwrap();
    }
    // Portable copy of the binary.
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join(if cfg!(windows) { "soyuz.exe" } else { "soyuz" });
    std::fs::copy(env!("CARGO_BIN_EXE_soyuz"), &exe).unwrap();
    std::fs::write(bin.join("portable"), b"").unwrap();

    let d = data.display();
    let script = [
        "welcome:off".to_string(),
        format!("cd:{d}"),
        "wait:2500".into(),
        "mark:tour".into(),
        "select:file_00010.txt".into(),
        "wait:200".into(),
        "view_grid".into(),
        "wait:400".into(),
        "scroll:20000".into(),
        "wait:300".into(),
        "view_details".into(),
        "wait:300".into(),
        "toggle_dual_pane".into(),
        "wait:400".into(),
        format!("cd:{}", data.join("alpha").display()),
        "wait:300".into(),
        "toggle_dual_pane".into(),
        "wait:300".into(),
        "command_palette".into(),
        "wait:300".into(),
        "command_palette".into(),
        "wait:300".into(),
        "quit".into(),
    ]
    .join(";");
    let mut child = Command::new(&exe)
        .env("SOYUZ_SCRIPT", script)
        .env("SOYUZ_FRAME_LOG", "1")
        .env("RUST_LOG", "info,wgpu=warn,naga=warn,wgpu_hal=error")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch soyuz");
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut log = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            log.push_str(&line);
            log.push('\n');
        }
        log
    });
    let deadline = Instant::now() + Duration::from_secs(90);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("soyuz did not quit within 90 s");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let log = reader.join().unwrap();
    let times = frame_times(&log, "tour");
    assert!(times.len() > 10, "too few frames logged:\n{log}");
    let worst = times.iter().copied().fold(0.0, f64::max);
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
    println!("{} frames after load: worst {worst:.2} ms, p95 {p95:.2} ms (budget {budget} ms)", times.len());
    assert!(worst <= budget, "a frame took {worst:.2} ms (> {budget} ms)");
}
