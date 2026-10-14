//! Phase T8 (P4): the shipped `soyuz-tui` binary in headless mode (`SOYUZ_TUI_HEADLESS=<w>x<h>`)
//! with a `SOYUZ_SCRIPT` smoke run in a portable sandbox; asserts on its shots and
//! `portable-data/soyuz-tui.log` (no `expect FAILED`). Nothing here needs a terminal: the
//! binary runs with its output piped, as on a CI runner.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use soyuz_core::config;

/// How long one headless run may take.
const RUN_CAP: Duration = Duration::from_secs(120);

/// A sandbox: a copy of the binary next to a `portable` marker, and a data folder.
struct Sandbox {
    tmp: tempfile::TempDir,
    exe: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = tempfile::tempdir().expect("temp dir");
        let bin = Path::new(env!("CARGO_BIN_EXE_soyuz-tui"));
        let exe = tmp.path().join(bin.file_name().expect("exe name"));
        std::fs::copy(bin, &exe).expect("copy soyuz-tui");
        std::fs::write(tmp.path().join(config::PORTABLE_MARKER), b"").expect("portable marker");
        // Nothing that reaches the network, the index, the CLI server or the MCP pipe.
        let cfg = tmp.path().join(config::PORTABLE_DATA_DIR);
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(
            cfg.join("config.toml"),
            "[updates]\nweekly_check = false\n[cli]\nserver_enabled = false\n[search.index]\nenabled = false\n[ai]\nenabled = false\n",
        )
        .unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("sub")).unwrap();
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.txt"), b"b").unwrap();
        Sandbox { tmp, exe, data }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.tmp.path().join(name)
    }

    /// Run the binary headless at `size` with `script` and `args`; returns its output and
    /// the log file.
    fn run(&self, size: &str, script: &str, args: &[&str]) -> (Output, String) {
        let mut child = Command::new(&self.exe)
            .args(args)
            .arg(&self.data)
            .env("SOYUZ_TUI_HEADLESS", size)
            .env("SOYUZ_SCRIPT", script)
            .env_remove("RUST_LOG")
            .env_remove("WT_SESSION")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start soyuz-tui");
        let start = Instant::now();
        while child.try_wait().expect("wait").is_none() {
            if start.elapsed() > RUN_CAP {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let out = child.wait_with_output().expect("output");
        let log =
            std::fs::read_to_string(self.path(config::PORTABLE_DATA_DIR).join("soyuz-tui.log")).unwrap_or_default();
        assert!(start.elapsed() <= RUN_CAP, "the headless run did not end within {RUN_CAP:?}\nlog:\n{log}");
        (out, log)
    }
}

fn failed(log: &str) -> Vec<&str> {
    log.lines().filter(|l| l.contains("SOYUZ_SCRIPT: expect FAILED")).collect()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The binary runs a script headless: text and SVG shots, a resize, checks that all
/// held, and a clean quit.
#[test]
fn headless_binary_takes_shots() {
    let sb = Sandbox::new();
    let (txt, svg, small) = (sb.path("shots/s.txt"), sb.path("shots/s.svg"), sb.path("shots/small.txt"));
    let script = format!(
        "wait_for:has=a.txt;expect:cwd=data;expect:focus=List;select:sub;keys:{{Down}};wait_for:cursor=a.txt;\
         shot:{};shot:{};size:60x20;expect:screen~b.txt;shot:{};hover:1,1;quit",
        txt.display(),
        svg.display(),
        small.display()
    );
    let (out, log) = sb.run("100x30", &script, &[]);
    let detail = format!("status {:?}\nstderr:\n{}\nlog:\n{log}", out.status, String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{detail}");
    assert!(failed(&log).is_empty(), "{detail}");
    assert!(log.contains("tui: headless 100x30, key map overlay Defaults"), "{detail}");
    assert!(log.contains("SOYUZ_SCRIPT: expect ok cwd=data"), "{detail}");
    // GUI-only steps are skipped with a warning.
    assert!(log.contains("SOYUZ_SCRIPT: hover is not a terminal UI step"), "{detail}");
    let text = read(&txt);
    assert_eq!(text.lines().count(), 30, "{text}");
    assert!(text.contains("a.txt") && text.contains("b.txt") && text.contains("sub"), "{text}");
    let svg = read(&svg);
    assert!(svg.starts_with("<svg") || svg.starts_with("<?xml"), "{svg}");
    assert!(svg.contains("a.txt"), "{svg}");
    let small = read(&small);
    assert_eq!(small.lines().count(), 20, "{small}");
    assert!(small.lines().all(|l| l.chars().count() <= 60), "{small}");
}

/// Without `quit` the run ends once the script is done; `--screen-reader` turns on
/// screen-reader mode for the run (no box drawing on screen) without saving it; a failed
/// check fails the run.
#[test]
fn headless_binary_ends_with_its_script() {
    let sb = Sandbox::new();
    let shot = sb.path("sr.txt");
    let script = format!("wait_for:has=a.txt;expect:screen~a.txt;shot:{}", shot.display());
    let (out, log) = sb.run("80x24", &script, &["--screen-reader"]);
    let detail = format!("status {:?}\nlog:\n{log}", out.status);
    assert!(out.status.success(), "{detail}");
    assert!(failed(&log).is_empty(), "{detail}");
    assert!(log.contains("SOYUZ_SCRIPT: done, quitting (headless)"), "{detail}");
    let text = read(&shot);
    assert!(!text.chars().any(soyuz_tui::a11y::is_box_char), "{text}");
    let saved = read(&sb.path(config::PORTABLE_DATA_DIR).join("config.toml"));
    assert!(!saved.contains("screen_reader = true"), "{saved}");

    // A check that does not hold: logged, and the exit code says so.
    let (out, log) = sb.run("80x24", "expect:cwd=nope;quit", &[]);
    assert!(!out.status.success(), "log:\n{log}");
    assert_eq!(failed(&log).len(), 1, "log:\n{log}");
}
