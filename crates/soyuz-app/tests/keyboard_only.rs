//! Keyboard-only path (roadmap F10): drives the real binary with synthetic key events
//! only (`SOYUZ_SCRIPT` `keys:` steps; no pointer input at all) through open, copy
//! (duplicate), rename, permanent delete, settings and the command palette, in English
//! and in Turkish, and checks the results on disk and through `expect:` steps.
//!
//! Launches the GUI in portable mode on a temporary folder (no user settings or folders
//! are touched; permanent delete is used so the Recycle Bin is never involved). Needs a
//! desktop session:
//! `cargo test -p soyuz-app --test keyboard_only -- --ignored --nocapture`.

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
}

fn script(data: &Path, lang: &str, palette_query: &str) -> String {
    let d = data.display();
    let sub = data.join("sub");
    [
        "welcome:off".to_string(),
        format!("config:general.language=\"{lang}\""),
        "config:keys.preset=\"explorer\"".into(),
        format!("expect:lang={lang}"),
        format!("cd:{d}"),
        "wait:1500".into(),
        // Open a folder with Enter, go back up with Backspace.
        "select:sub".into(),
        "keys:{Enter}".into(),
        "wait:700".into(),
        "expect:cwd=sub".into(),
        "keys:{Backspace}".into(),
        "wait:700".into(),
        format!("expect:cwd={}", data.file_name().unwrap().to_string_lossy()),
        // Copy: duplicate a.txt (Ctrl+D) — no OS clipboard involved.
        "select:a.txt".into(),
        "keys:{Ctrl+D}".into(),
        "wait:1200".into(),
        // Rename b.txt inline (F2 selects the stem; typing replaces it).
        "select:b.txt".into(),
        "keys:{F2}".into(),
        "wait:400".into(),
        "keys:renamed{Enter}".into(),
        "wait:1000".into(),
        format!("expect:exists={}", data.join("renamed.txt").display()),
        // Permanent delete with keyboard confirmation (Enter = the dialog's default).
        "select:c.txt".into(),
        "keys:{Shift+Delete}".into(),
        "wait:500".into(),
        "expect:dialog=open".into(),
        "keys:{Enter}".into(),
        "wait:1200".into(),
        "expect:dialog=closed".into(),
        format!("expect:missing={}", data.join("c.txt").display()),
        // Settings open and close with the keyboard.
        "keys:{Ctrl+Comma}".into(),
        "wait:600".into(),
        "expect:settings=open".into(),
        "keys:{Escape}".into(),
        "wait:500".into(),
        "expect:settings=closed".into(),
        // Command palette: type a query in the UI language, run it with Enter.
        "keys:{Ctrl+Shift+P}".into(),
        "wait:500".into(),
        "expect:palette=open".into(),
        format!("keys:{palette_query}"),
        "wait:500".into(),
        "keys:{Enter}".into(),
        "wait:800".into(),
        "expect:palette=closed".into(),
        // The new folder starts in inline rename: accept the default name.
        "keys:{Enter}".into(),
        "wait:1000".into(),
        format!("expect:exists={}", sub.display()),
        "quit".into(),
    ]
    .join(";")
}

fn run(lang: &str, palette_query: &str) {
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
        .env("SOYUZ_SCRIPT", script(&data, lang, palette_query))
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
    run("en-US", "new folder");
}

#[test]
#[ignore = "launches the GUI; run manually or in a desktop CI job"]
fn keyboard_only_turkish() {
    run("tr-TR", "yeni klas");
}
