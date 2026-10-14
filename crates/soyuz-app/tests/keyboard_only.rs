//! Keyboard-only path: drives the real binary with synthetic key events
//! only (`SOYUZ_SCRIPT` `keys:` steps; no pointer input at all) through open, copy
//! (duplicate), rename, permanent delete, settings and the command palette, in English
//! and in Turkish, and checks the results on disk and through `expect:` / `wait_for:` steps
//! (each step waits for its check instead of a fixed time; a timeout logs `expect FAILED`).
//!
//! Launches the GUI in portable mode on a temporary folder (no user settings or folders
//! are touched; permanent delete is used so the Recycle Bin is never involved). Needs a
//! desktop session and a GPU adapter (the software WARP adapter is fine; CI forces it with
//! `WGPU_ADAPTER_NAME`):
//! `cargo test -p soyuz-app --test keyboard_only -- --ignored --nocapture`.
//!
//! When the window never comes up (no desktop session, no adapter) the test is skipped
//! with a note, unless `SOYUZ_GUI_TESTS=require` (then it fails).

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `expect FAILED` lines of a run log.
fn failures(log: &str) -> Vec<&str> {
    log.lines().filter(|l| l.contains("SOYUZ_SCRIPT: expect FAILED")).collect()
}

#[test]
fn finds_failed_expectations() {
    let log = "x SOYUZ_SCRIPT: expect ok cwd=a\ny SOYUZ_SCRIPT: expect FAILED cwd=b (actual a)\n";
    assert_eq!(failures(log).len(), 1);
    assert!(gui_came_up("[INFO soyuz_app::devscript] SOYUZ_SCRIPT: welcome:off"));
    assert!(!gui_came_up("[ERROR soyuz] soyuz: wgpu error: no suitable adapter found"));
}

fn script(data: &Path, lang: &str, palette_query: &str, new_folder: &str) -> String {
    let d = data.display();
    [
        "welcome:off".to_string(),
        format!("config:general.language=\"{lang}\""),
        "config:keys.preset=\"explorer\"".into(),
        "config:appearance.native_dialogs=false".into(),
        format!("expect:lang={lang}"),
        format!("cd:{d}"),
        format!("wait_for:cwd={}", data.file_name().unwrap().to_string_lossy()),
        // The listing is in once its files show (cwd changes first).
        "wait_for:has=c.txt".into(),
        // Open a folder with Enter, go back up with Backspace.
        "select:sub".into(),
        "keys:{Enter}".into(),
        "wait_for:cwd=sub".into(),
        "keys:{Backspace}".into(),
        format!("wait_for:cwd={}", data.file_name().unwrap().to_string_lossy()),
        "wait_for:has=a.txt".into(),
        // Copy: duplicate a.txt (Ctrl+D) — no OS clipboard involved.
        "select:a.txt".into(),
        "keys:{Ctrl+D}".into(),
        "wait:1200".into(),
        // Rename b.txt inline (F2 selects the stem; typing replaces it).
        "select:b.txt".into(),
        "keys:{F2}".into(),
        "wait:400".into(),
        "keys:renamed{Enter}".into(),
        format!("wait_for:exists={}", data.join("renamed.txt").display()),
        // The list catches up with the rename after the disk does; select only then.
        "wait_for:has=renamed.txt".into(),
        // Permanent delete with keyboard confirmation (Enter = the dialog's default).
        "select:c.txt".into(),
        "keys:{Shift+Delete}".into(),
        "wait_for:dialog=open".into(),
        "keys:{Enter}".into(),
        "wait_for:dialog=closed".into(),
        format!("wait_for:missing={}", data.join("c.txt").display()),
        // Settings open and close with the keyboard.
        "keys:{Ctrl+Comma}".into(),
        "wait_for:settings=open".into(),
        "keys:{Escape}".into(),
        "wait_for:settings=closed".into(),
        // Command palette: type a query in the UI language, run it with Enter.
        "keys:{Ctrl+Shift+P}".into(),
        "wait_for:palette=open".into(),
        format!("keys:{palette_query}"),
        "wait:500".into(),
        "keys:{Enter}".into(),
        "wait_for:palette=closed".into(),
        // The new folder starts in inline rename: accept the default name.
        "keys:{Enter}".into(),
        format!("wait_for:exists={}", data.join(new_folder).display()),
        format!("wait_for:has={new_folder}"),
        "quit".into(),
    ]
    .join(";")
}

/// The GUI rendered at least one frame (the first script step runs inside a frame).
fn gui_came_up(log: &str) -> bool {
    log.contains("SOYUZ_SCRIPT: welcome:off")
}

/// Skip (with a note, and a GitHub Actions warning) when the GUI cannot start here.
fn skip_or_fail(lang: &str, why: &str, log: &str) {
    let required = std::env::var("SOYUZ_GUI_TESTS").is_ok_and(|v| v.eq_ignore_ascii_case("require"));
    assert!(
        !required,
        "{lang}: the GUI did not start ({why}) and SOYUZ_GUI_TESTS=require

log:
{log}"
    );
    eprintln!("keyboard_only ({lang}): skipped, the GUI did not start ({why}). Last log lines:");
    for l in log.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev() {
        eprintln!("  {l}");
    }
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::warning::keyboard-only GUI test ({lang}) skipped: {why}");
    }
}

fn run(lang: &str, palette_query: &str, new_folder: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(data.join("sub")).unwrap();
    for f in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(data.join(f), f.as_bytes()).unwrap();
    }
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join(if cfg!(windows) { "soyuz.exe" } else { "soyuz" });
    std::fs::copy(env!("CARGO_BIN_EXE_soyuz"), &exe).unwrap();
    std::fs::write(bin.join("portable"), b"").unwrap();

    let mut child = Command::new(&exe)
        .env("SOYUZ_SCRIPT", script(&data, lang, palette_query, new_folder))
        // Linux: keep freedesktop thumbnail writes out of the real ~/.cache.
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
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
    // Software rendering on a CI runner is slow: allow more time there.
    let limit = if std::env::var_os("CI").is_some() { 240 } else { 90 };
    let deadline = Instant::now() + Duration::from_secs(limit);
    let mut timed_out = false;
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let log = reader.join().unwrap();
    if !gui_came_up(&log) {
        let why = match status {
            Some(s) => format!("exited with {s} before the first frame (no desktop session or GPU adapter?)"),
            None => format!("no frame within {limit} s"),
        };
        return skip_or_fail(lang, &why, &log);
    }
    assert!(
        !timed_out,
        "{lang}: soyuz did not quit within {limit} s

log:
{log}"
    );
    let failed = failures(&log);
    assert!(failed.is_empty(), "{lang}: failed expectations:\n{}\n\nlog:\n{log}", failed.join("\n"));
    let names: Vec<String> =
        std::fs::read_dir(&data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    // a.txt was duplicated, b.txt renamed, c.txt deleted, a folder created.
    assert!(names.iter().filter(|n| n.starts_with('a') && n.ends_with(".txt")).count() >= 2, "{lang}: {names:?}");
    assert!(names.contains(&"renamed.txt".to_string()) && !names.contains(&"b.txt".to_string()), "{lang}: {names:?}");
    assert!(!names.contains(&"c.txt".to_string()), "{lang}: {names:?}");
    let dirs = std::fs::read_dir(&data).unwrap().flatten().filter(|e| e.path().is_dir()).count();
    assert!(dirs >= 2, "{lang}: no new folder: {names:?}");
}

#[test]
#[ignore = "launches the GUI; run manually or in a desktop CI job"]
fn keyboard_only_english() {
    run("en-US", "new folder", "New folder");
}

#[test]
#[ignore = "launches the GUI; run manually or in a desktop CI job"]
fn keyboard_only_turkish() {
    run("tr-TR", "yeni klas", "Yeni klasör");
}
