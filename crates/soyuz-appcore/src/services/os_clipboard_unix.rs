//! Linux / macOS clipboard file interop.
//!
//! * Linux: native first ([`native`]: Wayland `ext-data-control-v1`, else the X11
//!   `CLIPBOARD` selection); the tools below only when `[linux] clipboard_fallback_tools`
//!   is on and no native mechanism works.
//!
//! The command-line tools (best effort, nothing is linked):
//!
//! * Wayland: `wl-copy --type x-special/gnome-copied-files` / `wl-paste --type …`;
//! * X11: `xclip -selection clipboard -t x-special/gnome-copied-files` (reads also try
//!   `text/uri-list`, which KDE and most toolkits offer);
//!
//! macOS: the general `NSPasteboard` through `soyuz_fs::pasteboard` (file URLs, one item
//! per file, plus the paths as text); no tool is spawned. macOS has no standard cut
//! flavour: a cut is remembered here with the pasteboard's change count and only
//! reported while nobody else has written to the pasteboard.
//!
//! Every child process (Linux / BSD) runs with a short timeout so a hung tool cannot stall the UI.
//! stdin is written and stdout drained on helper threads, so payloads above the pipe
//! buffer (64 KB) cannot deadlock against the child. Writes (`set_files`, `clear`) run
//! on a background thread; reads wait for pending writes so a paste right after a copy
//! sees the new contents.

#[cfg(not(target_os = "macos"))]
use std::io::{Read, Write};
use std::path::PathBuf;
#[cfg(not(target_os = "macos"))]
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
#[cfg(not(target_os = "macos"))]
fn in_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .any(|d| std::fs::metadata(d.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
    })
}

/// Run `cmd args`, feeding `input` on stdin; returns stdout on success.
#[cfg(not(target_os = "macos"))]
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
fn tools_can_set() -> bool {
    in_path(if wayland() { "wl-copy" } else { "xclip" })
}

/// Whether the external tools may be used.
#[cfg(not(target_os = "macos"))]
fn tools_allowed() -> bool {
    // Linux has a native clipboard: the tools are an opt-in fallback there.
    !cfg!(target_os = "linux") || super::fallback_tools()
}

/// Never blocks (called on the UI thread on Copy / Cut): before the native backend has
/// been probed, the answer is "yes" and the write (on its own thread) finds out.
#[cfg(not(target_os = "macos"))]
pub(super) fn can_set(_n: usize) -> bool {
    #[cfg(target_os = "linux")]
    match native::probed() {
        Some(Some(_)) => return true,
        Some(None) => {}
        None => {
            probe_in_background();
            return true;
        }
    }
    tools_allowed() && tools_can_set()
}

#[cfg(target_os = "macos")]
pub(super) fn can_set(n: usize) -> bool {
    n > 0
}

#[cfg(not(target_os = "macos"))]
pub(super) fn set_files(mode: ClipMode, paths: &[PathBuf]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    match native::set_files(mode, paths) {
        Ok(()) => return Ok(()),
        Err(e) if !tools_allowed() => return Err(e),
        Err(e) => log::info!("clipboard: {e}; trying the fallback tools"),
    }
    if !tools_allowed() {
        return Err(tr!("services-clipboard-unavailable").into());
    }
    let body = encode_gnome(mode, paths);
    let ok = if wayland() {
        run("wl-copy", &["--type", "x-special/gnome-copied-files"], Some(body.as_bytes())).is_some()
    } else {
        run("xclip", &["-selection", "clipboard", "-t", "x-special/gnome-copied-files", "-i"], Some(body.as_bytes()))
            .is_some()
    };
    if ok { Ok(()) } else { Err(tr!("services-clipboard-no-tool").into()) }
}

/// Files on the clipboard, after pending background writes have landed.
pub(super) fn get_files() -> Option<(ClipMode, Vec<PathBuf>)> {
    wait_writes();
    read_files()
}

/// Empty the clipboard if it still holds a cut (runs on a background writer thread).
/// Linux: a cut we own is dropped by its owner thread (only while it is still ours); a
/// cut from another application is cleared only if its owner did not change since it
/// was read.
pub(super) fn clear_if_cut() {
    #[cfg(target_os = "linux")]
    match native::clear_if_cut() {
        Ok(()) => return,
        Err(e) if !tools_allowed() => {
            log::debug!("clipboard: {e}");
            return;
        }
        Err(e) => log::debug!("clipboard: {e}; trying the fallback tools"),
    }
    if let Some((ClipMode::Cut, _)) = read_files() {
        clear();
    }
}

#[cfg(not(target_os = "macos"))]
fn read_files() -> Option<(ClipMode, Vec<PathBuf>)> {
    #[cfg(target_os = "linux")]
    match native::read_files() {
        Ok(r) => return r,
        Err(e) if !tools_allowed() => {
            log::debug!("clipboard: {e}");
            return None;
        }
        Err(e) => log::debug!("clipboard: {e}; trying the fallback tools"),
    }
    if !tools_allowed() {
        return None;
    }
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
    #[cfg(target_os = "linux")]
    match native::clear() {
        Ok(()) => return,
        Err(e) => log::debug!("clipboard: {e}"),
    }
    if !tools_allowed() {
        return;
    }
    if wayland() {
        let _ = run("wl-copy", &["--clear"], None);
    } else {
        let _ = run("xclip", &["-selection", "clipboard", "-i"], Some(b""));
    }
}

/// The cut this process put on the macOS pasteboard: `(change count after the write,
/// mode)`. Any later write by another app bumps the change count and ends the cut.
#[cfg(target_os = "macos")]
static MAC_OWNED: parking_lot::Mutex<Option<(isize, ClipMode)>> = parking_lot::Mutex::new(None);

#[cfg(target_os = "macos")]
pub(super) fn set_files(mode: ClipMode, paths: &[PathBuf]) -> Result<(), String> {
    let text = super::paths_text(paths);
    let count = soyuz_fs::pasteboard::write_files(paths, &text)
        .ok_or_else(|| tr!("services-clipboard-mac-failed").to_string())?;
    *MAC_OWNED.lock() = Some((count, mode));
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_files() -> Option<(ClipMode, Vec<PathBuf>)> {
    let paths = soyuz_fs::pasteboard::read_files()?;
    let owned = *MAC_OWNED.lock();
    let mode = match owned {
        Some((count, mode)) if count == soyuz_fs::pasteboard::change_count() => mode,
        _ => ClipMode::Copy,
    };
    Some((mode, paths))
}

#[cfg(target_os = "macos")]
pub(super) fn clear() {
    *MAC_OWNED.lock() = None;
    soyuz_fs::pasteboard::clear();
}

pub(super) fn has_files() -> bool {
    false
}

/// No native key-state query here: the V key-release event is used instead.
pub(super) fn paste_keys_down() -> Option<(bool, bool, bool)> {
    None
}

/// The native backend has been probed and works (never blocks; starts the probe when it
/// has not run yet).
#[cfg(target_os = "linux")]
pub(super) fn native_ready() -> bool {
    match native::probed() {
        Some(b) => b.is_some(),
        None => {
            probe_in_background();
            false
        }
    }
}

/// The clipboard's text, after pending background writes have landed.
#[cfg(target_os = "linux")]
pub(super) fn get_text() -> Option<String> {
    wait_writes();
    native::get_text().unwrap_or_else(|e| {
        log::debug!("clipboard: {e}");
        None
    })
}

/// Probe the native clipboard on a helper thread (once).
#[cfg(target_os = "linux")]
pub(super) fn probe_in_background() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("soyuz-clip-probe".into()).spawn(|| {
            native::backend();
        });
    });
}

/// The built-in Linux clipboard: Wayland `ext-data-control-v1`, else X11.
#[cfg(target_os = "linux")]
pub(super) mod native {
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;

    use parking_lot::Mutex;

    use super::super::{SelectionOwner, TEXT_TYPES, files_from_selection, selection_formats, wayland, x11};
    use crate::ops::fileops::clipboard::ClipMode;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Backend {
        Wayland,
        X11,
    }

    static BACKEND: OnceLock<Option<Backend>> = OnceLock::new();

    /// The backend if the probe already ran (`None` = not probed yet). Never blocks.
    pub fn probed() -> Option<Option<Backend>> {
        BACKEND.get().copied()
    }

    /// The usable backend (probed once: one Wayland roundtrip / one X connection, each
    /// bounded by a timeout). Blocks on the first call: call off the UI thread.
    pub fn backend() -> Option<Backend> {
        *BACKEND.get_or_init(|| {
            let set = |v: &str| std::env::var_os(v).is_some_and(|x| !x.is_empty());
            let b = if set("WAYLAND_DISPLAY") && wayland::available() {
                Some(Backend::Wayland)
            } else if set("DISPLAY") && x11::available() {
                Some(Backend::X11)
            } else {
                None
            };
            log::info!("clipboard: native backend {b:?}");
            b
        })
    }

    /// What this process put on the clipboard.
    enum Content {
        Files(ClipMode, Vec<PathBuf>),
        Text(String),
    }

    /// What this process currently holds on the clipboard (until another app takes it).
    struct Owned {
        content: Content,
        owner: SelectionOwner,
    }

    static OWNED: Mutex<Option<Owned>> = Mutex::new(None);

    /// Our selection's content, while it is still ours.
    fn owned<R>(f: impl FnOnce(&Content) -> R) -> Option<R> {
        let g = OWNED.lock();
        g.as_ref().filter(|o| o.owner.alive.load(Ordering::SeqCst)).map(|o| f(&o.content))
    }

    fn files_of(c: &Content) -> Option<(ClipMode, Vec<PathBuf>)> {
        match c {
            Content::Files(mode, paths) => Some((*mode, paths.clone())),
            Content::Text(_) => None,
        }
    }

    #[cfg(test)]
    fn owned_files() -> Option<(ClipMode, Vec<PathBuf>)> {
        owned(files_of).flatten()
    }

    fn set(content: Content, formats: Vec<(String, Vec<u8>)>) -> Result<(), String> {
        let owner = match backend() {
            Some(Backend::Wayland) => wayland::set(formats)?,
            Some(Backend::X11) => x11::set(formats)?,
            None => return Err(tr!("services-clipboard-unavailable").into()),
        };
        // The new selection replaces the previous one; make sure its thread ends.
        if let Some(old) = OWNED.lock().replace(Owned { content, owner }) {
            old.owner.release.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    pub fn set_files(mode: ClipMode, paths: &[PathBuf]) -> Result<(), String> {
        set(Content::Files(mode, paths.to_vec()), selection_formats(mode, paths))
    }

    pub fn set_text(text: &str) -> Result<(), String> {
        let formats = TEXT_TYPES.iter().map(|t| (t.to_string(), text.as_bytes().to_vec())).collect();
        set(Content::Text(text.to_owned()), formats)
    }

    /// `Ok(None)`: the clipboard holds no text.
    pub fn get_text() -> Result<Option<String>, String> {
        // Our own selection: no round trip through the display server.
        if let Some(r) = owned(|c| match c {
            Content::Text(t) => Some(t.clone()),
            Content::Files(_, paths) => Some(super::super::paths_text(paths)),
        }) {
            return Ok(r);
        }
        read(|types, rd| {
            let t = TEXT_TYPES.iter().find(|t| types.iter().any(|x| x == *t))?;
            Some(String::from_utf8_lossy(&rd(t)?).into_owned())
        })
    }

    /// Run `f` with the selection's types and a reader (see the backends' `read`).
    pub fn read<R>(f: impl FnOnce(&[String], &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> R) -> Result<R, String> {
        match backend() {
            Some(Backend::Wayland) => wayland::read(f),
            Some(Backend::X11) => x11::read(f),
            None => Err(tr!("services-clipboard-unavailable").into()),
        }
    }

    /// `Ok(None)`: the clipboard holds no files.
    pub fn read_files() -> Result<Option<(ClipMode, Vec<PathBuf>)>, String> {
        // Our own selection: no round trip through the display server.
        if let Some(r) = owned(files_of) {
            return Ok(r);
        }
        read(|types, rd| files_from_selection(types, rd))
    }

    /// Empty the clipboard if it holds a cut: ours is released by its owner thread
    /// (which gives it up only while still the owner); another application's is cleared
    /// only if the selection did not change since it was read.
    pub fn clear_if_cut() -> Result<(), String> {
        {
            let mut g = OWNED.lock();
            if let Some(o) = g.as_ref().filter(|o| o.owner.alive.load(Ordering::SeqCst)) {
                if matches!(o.content, Content::Files(ClipMode::Cut, _)) {
                    o.owner.release.store(true, Ordering::SeqCst);
                    *g = None;
                }
                return Ok(());
            }
        }
        let is_cut = |r: &Option<(ClipMode, Vec<PathBuf>)>| matches!(r, Some((ClipMode::Cut, _)));
        match backend() {
            Some(Backend::Wayland) => wayland::read_and_clear_if(|t, rd| files_from_selection(t, rd), is_cut).map(drop),
            Some(Backend::X11) => x11::read_and_clear_if(|t, rd| files_from_selection(t, rd), is_cut).map(drop),
            None => Err(tr!("services-clipboard-unavailable").into()),
        }
    }

    pub fn clear() -> Result<(), String> {
        if let Some(old) = OWNED.lock().take() {
            old.owner.release.store(true, Ordering::SeqCst);
        }
        match backend() {
            Some(Backend::Wayland) => wayland::clear(),
            Some(Backend::X11) => x11::clear(),
            None => Err(tr!("services-clipboard-unavailable").into()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn wl_paste(args: &[&str]) -> Option<String> {
            let out = std::process::Command::new("wl-paste").args(args).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        }

        /// A cut is cleared after a paste, a copy is left alone; our own selection is
        /// released by its owner thread.
        #[test]
        fn clear_if_cut_only_clears_cuts() {
            if std::env::var_os("SOYUZ_CLIPBOARD_TESTS").is_none() {
                return;
            }
            assert!(backend().is_some(), "no native clipboard backend");
            let through_server = || read(|types, rd| files_from_selection(types, rd)).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let a = dir.path().join("a.txt");
            std::fs::write(&a, b"a").unwrap();
            set_files(ClipMode::Copy, std::slice::from_ref(&a)).unwrap();
            clear_if_cut().unwrap();
            assert_eq!(through_server(), Some((ClipMode::Copy, vec![a.clone()])), "a copy stays");
            set_files(ClipMode::Cut, std::slice::from_ref(&a)).unwrap();
            clear_if_cut().unwrap();
            assert_eq!(owned_files(), None);
            let start = std::time::Instant::now();
            while through_server().is_some() && start.elapsed() < std::time::Duration::from_secs(2) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert_eq!(through_server(), None, "the released cut is gone");
        }

        /// Copies files to the real session clipboard, reads them back through our own
        /// paste path and through `wl-paste`, then restores the previous text.
        #[test]
        fn native_round_trip() {
            if std::env::var_os("SOYUZ_CLIPBOARD_TESTS").is_none() {
                return;
            }
            let Some(b) = backend() else { panic!("no native clipboard backend") };
            // Save the user's text (restored with wl-copy, which keeps serving it after
            // this test process exits).
            let saved = read(|types, rd| {
                types.iter().any(|t| t.starts_with("text/plain")).then(|| rd("text/plain;charset=utf-8")).flatten()
            })
            .ok()
            .flatten();
            let dir = tempfile::tempdir().unwrap();
            let a = dir.path().join("a b.txt");
            let c = dir.path().join("ç.txt");
            std::fs::write(&a, b"a").unwrap();
            std::fs::write(&c, b"c").unwrap();
            let paths = vec![a.clone(), c.clone()];

            set_files(ClipMode::Cut, &paths).unwrap();
            assert_eq!(owned_files(), Some((ClipMode::Cut, paths.clone())));
            // Through the display server (not the in-process shortcut).
            let got = read(|types, rd| files_from_selection(types, rd)).unwrap();
            assert_eq!(got, Some((ClipMode::Cut, paths.clone())));
            if b == Backend::Wayland && std::process::Command::new("wl-paste").arg("--version").output().is_ok() {
                let types = wl_paste(&["--list-types"]).unwrap_or_default();
                for t in ["x-special/gnome-copied-files", "text/uri-list", "application/x-kde-cutselection"] {
                    assert!(types.lines().any(|l| l == t), "{t} missing from {types:?}");
                }
                let list = wl_paste(&["--no-newline", "-t", "text/uri-list"]).unwrap();
                assert!(list.contains("a%20b.txt") && list.contains("%C3%A7.txt"), "{list}");
                let gnome = wl_paste(&["--no-newline", "-t", "x-special/gnome-copied-files"]).unwrap();
                assert!(gnome.starts_with("cut\nfile://"), "{gnome}");
            }
            set_files(ClipMode::Copy, &paths[..1]).unwrap();
            let got = read(|types, rd| files_from_selection(types, rd)).unwrap();
            assert_eq!(got, Some((ClipMode::Copy, paths[..1].to_vec())));
            clear().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            assert_eq!(read_files().unwrap(), None);

            if let Some(text) = saved {
                let child = std::process::Command::new("wl-copy")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .spawn();
                if let Ok(mut child) = child {
                    use std::io::Write;
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(&text);
                    }
                    let _ = child.wait();
                }
            }
        }
    }
}
