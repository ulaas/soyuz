//! Linux / macOS clipboard file interop through the platform's command-line tools
//! (best effort, nothing is linked):
//!
//! * Wayland: `wl-copy --type x-special/gnome-copied-files` / `wl-paste --type …`;
//! * X11: `xclip -selection clipboard -t x-special/gnome-copied-files` (reads also try
//!   `text/uri-list`, which KDE and most toolkits offer);
//! * macOS: `osascript` sets / reads a `public.file-url` (single file), otherwise the
//!   caller falls back to text.
//!
//! Every child process runs with a short timeout so a hung tool cannot stall the UI.
//! stdin is written and stdout drained on helper threads, so payloads above the pipe
//! buffer (64 KB) cannot deadlock against the child. Writes (`set_files`, `clear`) run
//! on a background thread; reads wait for pending writes so a paste right after a copy
//! sees the new contents.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(not(target_os = "macos"))]
use super::{decode_gnome, decode_uri_list, encode_gnome};
use crate::ops::fileops::clipboard::ClipMode;

const TIMEOUT: Duration = Duration::from_millis(400);

/// Background clipboard writes still running (reads wait for them, bounded).
static PENDING_WRITES: AtomicUsize = AtomicUsize::new(0);

/// Run `f` (a clipboard write) on a background thread.
pub(super) fn write_async(f: impl FnOnce() + Send + 'static) {
    PENDING_WRITES.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new().name("soyuz-clipboard".into()).spawn(move || {
        f();
        PENDING_WRITES.fetch_sub(1, Ordering::SeqCst);
    });
    if spawned.is_err() {
        PENDING_WRITES.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Wait (bounded) until background writes have finished.
fn wait_writes() {
    let start = Instant::now();
    while PENDING_WRITES.load(Ordering::SeqCst) > 0 && start.elapsed() < TIMEOUT * 2 {
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `name` is an executable somewhere on `PATH` (no process is spawned).
fn in_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .any(|d| std::fs::metadata(d.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
    })
}

/// Run `cmd args`, feeding `input` on stdin; returns stdout on success.
fn run(cmd: &str, args: &[&str], input: Option<&[u8]>) -> Option<Vec<u8>> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        // Writers (wl-copy / xclip) fork a background owner that inherits stdout: never
        // capture it there, or reading would block until the selection is replaced.
        .stdout(if input.is_some() { Stdio::null() } else { Stdio::piped() })
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Feed stdin from a thread: a child that doesn't read (or reads slowly) must not
    // block us past the timeout once the pipe buffer is full.
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        let data = data.to_vec();
        let _ = std::thread::Builder::new().name("soyuz-clip-in".into()).spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    // Drain stdout concurrently: a child writing more than the pipe buffer would
    // otherwise never exit, and `try_wait` would time out on every large clipboard.
    let reader = child.stdout.take().map(|mut so| {
        let (tx, rx) = crossbeam_channel::bounded::<Option<Vec<u8>>>(1);
        let _ = std::thread::Builder::new().name("soyuz-clip-out".into()).spawn(move || {
            let mut out = Vec::new();
            let _ = tx.send(so.read_to_end(&mut out).ok().map(|_| out));
        });
        rx
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let Some(rx) = reader else { return Some(Vec::new()) };
                let left = TIMEOUT.saturating_sub(start.elapsed()).max(Duration::from_millis(50));
                return rx.recv_timeout(left).ok().flatten();
            }
            Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// A tool for putting `n` files on the clipboard is available (cheap, no spawn), so the
/// caller can decide on the text fallback before the write runs in the background.
#[cfg(not(target_os = "macos"))]
pub(super) fn can_set(_n: usize) -> bool {
    in_path(if wayland() { "wl-copy" } else { "xclip" })
}

#[cfg(target_os = "macos")]
pub(super) fn can_set(n: usize) -> bool {
    n == 1 && in_path("osascript")
}

#[cfg(not(target_os = "macos"))]
pub(super) fn set_files(mode: ClipMode, paths: &[PathBuf]) -> Result<(), String> {
    let body = encode_gnome(mode, paths);
    let ok = if wayland() {
        run("wl-copy", &["--type", "x-special/gnome-copied-files"], Some(body.as_bytes())).is_some()
    } else {
        run("xclip", &["-selection", "clipboard", "-t", "x-special/gnome-copied-files", "-i"], Some(body.as_bytes()))
            .is_some()
    };
    if ok { Ok(()) } else { Err("neither wl-copy nor xclip is available".into()) }
}

/// Files on the clipboard, after pending background writes have landed.
pub(super) fn get_files() -> Option<(ClipMode, Vec<PathBuf>)> {
    wait_writes();
    read_files()
}

/// Empty the clipboard if it still holds a cut (runs on a background writer thread).
pub(super) fn clear_if_cut() {
    if let Some((ClipMode::Cut, _)) = read_files() {
        clear();
    }
}

#[cfg(not(target_os = "macos"))]
fn read_files() -> Option<(ClipMode, Vec<PathBuf>)> {
    let read = |target: &str| -> Option<String> {
        let out = if wayland() {
            run("wl-paste", &["--no-newline", "--type", target], None)?
        } else {
            run("xclip", &["-selection", "clipboard", "-o", "-t", target], None)?
        };
        Some(String::from_utf8_lossy(&out).into_owned())
    };
    if let Some(r) = read("x-special/gnome-copied-files").as_deref().and_then(decode_gnome) {
        return Some(r);
    }
    let list = decode_uri_list(&read("text/uri-list")?);
    (!list.is_empty()).then_some((ClipMode::Copy, list))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn clear() {
    if wayland() {
        let _ = run("wl-copy", &["--clear"], None);
    } else {
        let _ = run("xclip", &["-selection", "clipboard", "-i"], Some(b""));
    }
}

#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
pub(super) fn set_files(_mode: ClipMode, paths: &[PathBuf]) -> Result<(), String> {
    let [one] = paths else { return Err("only a single file can be put on the macOS clipboard".into()) };
    let script = format!("set the clipboard to (POSIX file {})", applescript_string(&one.to_string_lossy()));
    run("osascript", &["-e", &script], None).map(|_| ()).ok_or_else(|| "osascript failed".into())
}

#[cfg(target_os = "macos")]
fn read_files() -> Option<(ClipMode, Vec<PathBuf>)> {
    let out = run("osascript", &["-e", "POSIX path of (the clipboard as «class furl»)"], None)?;
    let p = String::from_utf8_lossy(&out).trim().to_string();
    (!p.is_empty()).then(|| (ClipMode::Copy, vec![PathBuf::from(p)]))
}

#[cfg(target_os = "macos")]
pub(super) fn clear() {}

pub(super) fn has_files() -> bool {
    false
}

/// No native key-state query here: the V key-release event is used instead.
pub(super) fn paste_keys_down() -> Option<(bool, bool, bool)> {
    None
}
