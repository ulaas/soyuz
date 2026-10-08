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

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(not(target_os = "macos"))]
use super::{decode_gnome, decode_uri_list, encode_gnome};
use crate::ops::fileops::clipboard::ClipMode;

const TIMEOUT: Duration = Duration::from_millis(400);

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
    if let Some(data) = input {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(data).ok()?;
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = Vec::new();
                if let Some(mut so) = child.stdout.take() {
                    use std::io::Read;
                    so.read_to_end(&mut out).ok()?;
                }
                return Some(out);
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

#[cfg(not(target_os = "macos"))]
pub(super) fn get_files() -> Option<(ClipMode, Vec<PathBuf>)> {
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
pub(super) fn get_files() -> Option<(ClipMode, Vec<PathBuf>)> {
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
