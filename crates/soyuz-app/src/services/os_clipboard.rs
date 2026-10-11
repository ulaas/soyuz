//! OS clipboard interop for files: copy / cut in Soyuz puts the files on the
//! system clipboard in the native format, and Paste reads files copied in Explorer /
//! Finder / Nautilus, honouring move-vs-copy.
//!
//! * Windows: `CF_HDROP` (a hand-built `DROPFILES` block) + the registered
//!   `"Preferred DropEffect"` format + `CF_UNICODETEXT`, through raw user32/kernel32
//!   calls (`os_clipboard_windows.rs`). No shell or COM APIs.
//! * Linux: `x-special/gnome-copied-files`, `text/uri-list`,
//!   `application/x-kde-cutselection` and the paths as text, served natively: the Wayland
//!   `ext-data-control-v1` protocol on our own connection (`os_clipboard_wayland.rs`), or
//!   the X11 `CLIPBOARD` selection (`os_clipboard_x11.rs`). The selection is owned by a
//!   background thread until another application replaces it. Only when neither works
//!   and `[linux] clipboard_fallback_tools` is on, `wl-copy` / `xclip` are run.
//! * Other unix: the same formats through `wl-copy` / `xclip` when installed; best effort.
//! * macOS: the general `NSPasteboard` (file URLs + text) through `soyuz_fs::pasteboard`;
//!   cut state is kept in-process.
//!
//! The codecs (DROPFILES, uri-list, gnome-copied-files) are pure and unit-tested on every
//! OS. [`paste_key_fallback`] covers egui-winit not emitting any event for Ctrl+V when
//! the clipboard holds files but no text.

use std::path::{Path, PathBuf};

use egui::{Event, Id, Key};
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Keymap, Mods};

use crate::keys::Dispatched;
use crate::ops::fileops::clipboard::ClipMode;

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "os_clipboard_windows.rs"]
mod imp;

#[cfg(not(windows))]
#[path = "os_clipboard_unix.rs"]
mod imp;

#[cfg(target_os = "linux")]
#[path = "os_clipboard_wayland.rs"]
mod wayland;

#[cfg(target_os = "linux")]
#[path = "os_clipboard_x11.rs"]
mod x11;

/// A selection this process serves from a background thread (Linux native backends).
#[cfg(target_os = "linux")]
#[derive(Clone, Default)]
struct SelectionOwner {
    /// True while the selection is ours (false once another client took it).
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set to make the owner thread give the selection up (only while still ours).
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// `[linux] clipboard_fallback_tools` (kept current by `services::filemanager1::pump`).
static FALLBACK_TOOLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Apply `[linux] clipboard_fallback_tools`; the first call also probes the native
/// clipboard in the background, so the first copy does not wait for it.
pub fn set_fallback_tools(on: bool) {
    FALLBACK_TOOLS.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(target_os = "linux")]
    imp::probe_in_background();
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn fallback_tools() -> bool {
    FALLBACK_TOOLS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Put `paths` on the system clipboard as files (plus their paths as text where the
/// format allows both). Returns `false` when no native mechanism is available, so the
/// caller can fall back to plain text.
pub fn set_files(mode: ClipMode, paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    let paths: Vec<PathBuf> = paths.iter().map(|p| soyuz_fs::strip_verbatim(p)).collect();
    #[cfg(windows)]
    match imp::set_files(mode, &paths) {
        Ok(()) => true,
        Err(e) => {
            log::debug!("os clipboard: cannot set files: {e}");
            false
        }
    }
    // Elsewhere the write spawns external tools: decide up front whether one exists
    // (for the caller's text fallback), then write off the UI thread.
    #[cfg(not(windows))]
    {
        if !imp::can_set(paths.len()) {
            return false;
        }
        imp::write_async(move || {
            if let Err(e) = imp::set_files(mode, &paths) {
                log::debug!("os clipboard: cannot set files: {e}");
            }
        });
        true
    }
}

/// Files on the clipboard, with copy-vs-move.
pub type Files = Option<(ClipMode, Vec<PathBuf>)>;

/// What to do with the clipboard's files once they are known (runs on the UI thread).
pub type Then = Box<dyn FnOnce(&mut crate::app::SoyuzApp, Files) + Send>;

/// Files currently on the system clipboard (put there by any application). Blocking
/// outside Windows (a round trip to the clipboard owner): never call it on the UI thread
/// there, use [`with_files`].
pub fn get_files() -> Files {
    imp::get_files().filter(|(_, p)| !p.is_empty())
}

/// Read the files on the system clipboard and run `then` with them on the UI thread.
/// Windows: right away (the clipboard read is a quick local call). Elsewhere the read
/// waits for the clipboard owner (another process), so it runs on a worker thread and
/// `then` runs from [`pump`] once it lands, at most a few frames later.
pub fn with_files(app: &mut crate::app::SoyuzApp, then: Then) {
    #[cfg(windows)]
    then(app, get_files());
    #[cfg(not(windows))]
    {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let wake = app.services.mailbox.postman().wake();
        let spawned = std::thread::Builder::new().name("soyuz-clip-read".into()).spawn(move || {
            let _ = tx.send(get_files());
            wake();
        });
        match spawned {
            Ok(_) => PENDING.lock().push(rx, then),
            Err(e) => {
                log::debug!("os clipboard: cannot start the reader: {e}");
                then(app, None);
            }
        }
    }
}

/// Run the continuations of clipboard reads that have landed (call once per frame).
pub fn pump(app: &mut crate::app::SoyuzApp) {
    #[cfg(not(windows))]
    {
        let ready = PENDING.lock().take_ready();
        for (then, files) in ready {
            then(app, files);
        }
    }
    #[cfg(windows)]
    let _ = app;
}

/// Clipboard reads in flight, in request order.
#[cfg_attr(windows, allow(dead_code))]
struct Pending<T> {
    reads: Vec<(crossbeam_channel::Receiver<Files>, T)>,
}

#[cfg_attr(windows, allow(dead_code))]
impl<T> Pending<T> {
    const fn new() -> Self {
        Pending { reads: Vec::new() }
    }

    fn push(&mut self, rx: crossbeam_channel::Receiver<Files>, then: T) {
        self.reads.push((rx, then));
    }

    /// The finished reads at the front of the queue (later ones wait for earlier ones, so
    /// two pastes run in the order they were asked for). A reader that died counts as
    /// "no files".
    fn take_ready(&mut self) -> Vec<(T, Files)> {
        let mut out = Vec::new();
        while let Some((rx, _)) = self.reads.first() {
            let files = match rx.try_recv() {
                Ok(f) => f,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => None,
            };
            let (_, then) = self.reads.remove(0);
            out.push((then, files));
        }
        out
    }
}

#[cfg(not(windows))]
static PENDING: parking_lot::Mutex<Pending<Then>> = parking_lot::Mutex::new(Pending::new());

/// Cheap check used to enable "Paste" in menus (Windows only; elsewhere `false`, the
/// internal clipboard decides).
pub fn has_files() -> bool {
    imp::has_files()
}

/// After a cut from the system clipboard was pasted (moved), empty the clipboard like
/// Explorer does, but only if it still holds a cut.
pub fn clear_cut() {
    #[cfg(windows)]
    if let Some((ClipMode::Cut, _)) = imp::get_files() {
        imp::clear();
    }
    // Reading spawns external tools elsewhere: do it off the UI thread.
    #[cfg(not(windows))]
    imp::write_async(imp::clear_if_cut);
}

// ---------------------------------------------------------------------------------------
// Ctrl+V without a paste event
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct PasteKeyState {
    /// A Paste event (or the fallback) handled the current V press.
    handled: bool,
    /// Ctrl+V was physically down last frame (Windows key-state polling).
    down: bool,
    /// When the fallback fired (to drop a late `Event::Paste` for the same press).
    fired_at: Option<f64>,
}

/// egui-winit turns Ctrl+V into `Event::Paste(text)` and drops the key event; when the
/// clipboard holds files but no text, nothing at all reaches the app. Detect the chord
/// from the native key state (Windows: fires on key-down) or from the V key-release
/// event (everywhere) and append the action bound to it in the file list. A Paste event
/// arriving right after the fallback fired for the same press is dropped, so a paste
/// never runs twice. Call once per frame while the file list owns the keyboard.
pub fn paste_key_fallback(ctx: &egui::Context, keymap: &Keymap, dispatched: &mut Vec<Dispatched>) {
    let id = Id::new("soyuz.os_clipboard.paste_key");
    let mut st: PasteKeyState = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let now = ctx.input(|i| i.time);
    let is_v = |c: &KeyChord| c.key == KeyName::V && c.mods.contains(Mods::CTRL) && !c.mods.contains(Mods::ALT);
    if dispatched.iter().any(|d| is_v(&d.chord)) {
        if st.fired_at.is_some_and(|t| now - t < 0.75) {
            dispatched.retain(|d| !is_v(&d.chord));
            st.fired_at = None;
        }
        st.handled = true;
    }
    let mut fire: Option<KeyChord> = None;
    if let Some((ctrl, shift, v)) = imp::paste_keys_down() {
        let down = ctrl && v && ctx.input(|i| i.focused);
        if down && !st.down && !st.handled {
            let mods = if shift { Mods::CTRL | Mods::SHIFT } else { Mods::CTRL };
            fire = Some(KeyChord::new(mods, KeyName::V));
        }
        st.down = down;
    }
    let released = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key { key: Key::V, pressed: false, modifiers, .. } => Some(*modifiers),
            _ => None,
        })
    });
    if let Some(m) = released {
        if (m.ctrl || m.command) && !m.alt && !st.handled && fire.is_none() {
            let mods = if m.shift { Mods::CTRL | Mods::SHIFT } else { Mods::CTRL };
            fire = Some(KeyChord::new(mods, KeyName::V));
        }
        st.handled = false;
        if fire.is_none() {
            st.fired_at = st.fired_at.filter(|t| now - t < 0.75);
        }
    }
    if let Some(chord) = fire
        && let Some(action) =
            keymap.lookup(KeyContext::FileList, chord).filter(|a| matches!(a, Action::Paste | Action::PasteAsLink))
    {
        dispatched.push(Dispatched { action: action.clone(), chord });
        st.handled = released.is_none();
        st.fired_at = Some(now);
    }
    ctx.data_mut(|d| d.insert_temp(id, st));
}

// ---------------------------------------------------------------------------------------
// Codecs
// ---------------------------------------------------------------------------------------

/// `DROPEFFECT_COPY` / `DROPEFFECT_MOVE` / `DROPEFFECT_LINK`.
pub const DROPEFFECT_COPY: u32 = 1;
pub const DROPEFFECT_MOVE: u32 = 2;
pub const DROPEFFECT_LINK: u32 = 4;

/// Size of the `DROPFILES` header (`pFiles: u32, pt: POINT, fNC: BOOL, fWide: BOOL`).
pub const DROPFILES_HEADER: usize = 20;

/// Value of the "Preferred DropEffect" format for a clipboard mode (Explorer writes
/// `COPY | LINK` for copy and `MOVE` for cut).
pub fn drop_effect(mode: ClipMode) -> u32 {
    match mode {
        ClipMode::Copy => DROPEFFECT_COPY | DROPEFFECT_LINK,
        ClipMode::Cut => DROPEFFECT_MOVE,
    }
}

/// Mode for a "Preferred DropEffect" value (missing → copy).
pub fn mode_from_effect(effect: Option<u32>) -> ClipMode {
    match effect {
        Some(e) if e & DROPEFFECT_MOVE != 0 && e & DROPEFFECT_COPY == 0 => ClipMode::Cut,
        _ => ClipMode::Copy,
    }
}

fn wide(p: &Path) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str().encode_wide().collect()
    }
    #[cfg(not(windows))]
    {
        p.to_string_lossy().encode_utf16().collect()
    }
}

fn from_wide(units: &[u16]) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_wide(units))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(String::from_utf16_lossy(units))
    }
}

/// Build a `CF_HDROP` payload: a `DROPFILES` header followed by NUL-terminated UTF-16
/// paths and a final NUL.
pub fn build_dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::with_capacity(DROPFILES_HEADER + paths.len() * 64);
    out.extend_from_slice(&(DROPFILES_HEADER as u32).to_le_bytes()); // pFiles
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    out.extend_from_slice(&0i32.to_le_bytes()); // fNC
    out.extend_from_slice(&1i32.to_le_bytes()); // fWide
    for p in paths {
        for u in wide(p) {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// Parse a `CF_HDROP` payload (wide or ANSI file list).
pub fn parse_dropfiles(data: &[u8]) -> Option<Vec<PathBuf>> {
    if data.len() < DROPFILES_HEADER {
        return None;
    }
    let off = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    let wide_chars = i32::from_le_bytes(data[16..20].try_into().ok()?) != 0;
    let body = data.get(off..)?;
    let mut out = Vec::new();
    if wide_chars {
        let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        for part in units.split(|u| *u == 0) {
            if part.is_empty() {
                break;
            }
            out.push(from_wide(part));
        }
    } else {
        for part in body.split(|b| *b == 0) {
            if part.is_empty() {
                break;
            }
            // ANSI code page: ASCII is exact, anything else is decoded as Latin-1.
            out.push(PathBuf::from(part.iter().map(|b| *b as char).collect::<String>()));
        }
    }
    Some(out)
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// `file://` URI of an absolute path (RFC 8089; percent-encoded UTF-8). Windows drive
/// paths become `file:///C:/…`, UNC paths `file://server/share/…`.
pub fn path_to_uri(p: &Path) -> String {
    let s = soyuz_fs::strip_verbatim(p).to_string_lossy().replace('\\', "/");
    let (prefix, rest) = if let Some(unc) = s.strip_prefix("//") {
        ("file://", unc.to_string())
    } else if s.as_bytes().get(1) == Some(&b':') {
        ("file:///", s.clone())
    } else {
        ("file://", s.clone())
    };
    let mut out = String::from(prefix);
    for (i, b) in rest.bytes().enumerate() {
        // Keep the drive colon readable (`C:`).
        if unreserved(b) || (b == b':' && i == 1 && prefix == "file:///") {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Path of a `file://` URI (`None` for other schemes or remote hosts on unix).
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let bytes = percent_decode(path);
    let decoded = String::from_utf8_lossy(&bytes).into_owned();
    let host = if host.eq_ignore_ascii_case("localhost") { "" } else { host };
    if cfg!(windows) {
        if !host.is_empty() {
            return Some(PathBuf::from(format!(r"\\{host}{}", decoded.replace('/', "\\"))));
        }
        let d = decoded.strip_prefix('/').filter(|d| d.as_bytes().get(1) == Some(&b':')).unwrap_or(&decoded);
        Some(PathBuf::from(d.replace('/', "\\")))
    } else {
        if !host.is_empty() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        }
        #[cfg(not(unix))]
        {
            Some(PathBuf::from(decoded))
        }
    }
}

/// `text/uri-list` body (CRLF separated, per RFC 2483).
pub fn encode_uri_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| path_to_uri(p) + "\r\n").collect()
}

/// Parse `text/uri-list` (comments and non-file URIs are skipped).
pub fn decode_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(uri_to_path).collect()
}

/// `x-special/gnome-copied-files` body: `copy` / `cut`, then one URI per line.
pub fn encode_gnome(mode: ClipMode, paths: &[PathBuf]) -> String {
    let mut s = String::from(if mode == ClipMode::Cut { "cut" } else { "copy" });
    for p in paths {
        s.push('\n');
        s.push_str(&path_to_uri(p));
    }
    s
}

pub fn decode_gnome(text: &str) -> Option<(ClipMode, Vec<PathBuf>)> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let mode = match lines.next()? {
        "cut" => ClipMode::Cut,
        "copy" => ClipMode::Copy,
        _ => return None,
    };
    let paths: Vec<PathBuf> = lines.filter_map(uri_to_path).collect();
    (!paths.is_empty()).then_some((mode, paths))
}

/// Plain-text form of the paths (one per line), written next to the file formats.
pub fn paths_text(paths: &[PathBuf]) -> String {
    crate::ops::fileops::clipboard::Clipboard::os_text(paths)
}

/// `application/x-kde-cutselection`: present with `1` on a cut (KDE / Dolphin).
pub const KDE_CUT: &str = "application/x-kde-cutselection";
pub const GNOME_FILES: &str = "x-special/gnome-copied-files";
pub const URI_LIST: &str = "text/uri-list";
/// Text MIME types (and the X11 text targets) the paths are offered as.
pub const TEXT_TYPES: &[&str] = &["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING", "TEXT"];

/// The MIME types (and bodies) a file selection is offered as, preferred first.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn selection_formats(mode: ClipMode, paths: &[PathBuf]) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![
        (GNOME_FILES.to_string(), encode_gnome(mode, paths).into_bytes()),
        (URI_LIST.to_string(), encode_uri_list(paths).into_bytes()),
    ];
    if mode == ClipMode::Cut {
        out.push((KDE_CUT.to_string(), b"1".to_vec()));
    }
    let text = paths_text(paths).into_bytes();
    out.extend(TEXT_TYPES.iter().map(|t| (t.to_string(), text.clone())));
    out
}

/// Files from a selection offering `types`, reading bodies with `read`: GNOME's format
/// first, then `text/uri-list` (a cut when KDE's cut marker says `1`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn files_from_selection(
    types: &[String],
    mut read: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Option<(ClipMode, Vec<PathBuf>)> {
    let has = |t: &str| types.iter().any(|x| x == t);
    if has(GNOME_FILES)
        && let Some(r) = read(GNOME_FILES).and_then(|b| decode_gnome(&String::from_utf8_lossy(&b)))
    {
        return Some(r);
    }
    if !has(URI_LIST) {
        return None;
    }
    let list = decode_uri_list(&String::from_utf8_lossy(&read(URI_LIST)?));
    if list.is_empty() {
        return None;
    }
    let cut = has(KDE_CUT) && read(KDE_CUT).is_some_and(|b| b.first() == Some(&b'1'));
    Some((if cut { ClipMode::Cut } else { ClipMode::Copy }, list))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn pending_reads_complete_in_request_order() {
        let mut p: Pending<u32> = Pending::new();
        let (tx1, rx1) = crossbeam_channel::bounded(1);
        let (tx2, rx2) = crossbeam_channel::bounded(1);
        let (tx3, rx3) = crossbeam_channel::bounded::<Files>(1);
        p.push(rx1, 1);
        p.push(rx2, 2);
        p.push(rx3, 3);
        assert!(p.take_ready().is_empty(), "nothing landed yet");
        tx2.send(Some((ClipMode::Cut, vec![pb("/b")]))).unwrap();
        assert!(p.take_ready().is_empty(), "the second read waits for the first");
        tx1.send(None).unwrap();
        let got = p.take_ready();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].0, got[0].1.clone()), (1, None));
        assert_eq!((got[1].0, got[1].1.clone()), (2, Some((ClipMode::Cut, vec![pb("/b")]))));
        drop(tx3); // a reader that died counts as "no files"
        let got = p.take_ready();
        assert_eq!((got[0].0, got[0].1.clone()), (3, None));
        assert!(p.reads.is_empty());
    }

    #[test]
    fn dropfiles_round_trip() {
        let paths = vec![pb(r"C:\Users\me\Fïle one.txt"), pb(r"D:\x"), pb(r"\\server\share\日本.doc")];
        let data = build_dropfiles(&paths);
        assert_eq!(&data[0..4], &20u32.to_le_bytes());
        assert_eq!(&data[16..20], &1i32.to_le_bytes(), "fWide");
        assert!(data.ends_with(&[0, 0, 0, 0]), "double NUL terminator");
        assert_eq!(parse_dropfiles(&data).unwrap(), paths);
        assert!(parse_dropfiles(&build_dropfiles(&[])).unwrap().is_empty());
        assert!(parse_dropfiles(&[1, 2, 3]).is_none());
    }

    #[test]
    fn dropfiles_ansi_and_offset() {
        // A producer with a larger header offset and ANSI names.
        let mut d = Vec::new();
        d.extend_from_slice(&24u32.to_le_bytes());
        d.extend_from_slice(&[0; 12]);
        d.extend_from_slice(&0i32.to_le_bytes());
        d.extend_from_slice(&[0xAA; 4]); // padding up to pFiles
        d.extend_from_slice(b"C:\\a.txt\0C:\\b\0\0");
        assert_eq!(parse_dropfiles(&d).unwrap(), vec![pb("C:\\a.txt"), pb("C:\\b")]);
    }

    #[test]
    fn drop_effect_modes() {
        assert_eq!(mode_from_effect(Some(drop_effect(ClipMode::Cut))), ClipMode::Cut);
        assert_eq!(mode_from_effect(Some(drop_effect(ClipMode::Copy))), ClipMode::Copy);
        assert_eq!(mode_from_effect(Some(DROPEFFECT_COPY | DROPEFFECT_MOVE)), ClipMode::Copy);
        assert_eq!(mode_from_effect(None), ClipMode::Copy);
    }

    #[test]
    fn uri_encoding() {
        if cfg!(windows) {
            assert_eq!(path_to_uri(&pb(r"C:\My Docs\a#1.txt")), "file:///C:/My%20Docs/a%231.txt");
            assert_eq!(path_to_uri(&pb(r"\\?\C:\x")), "file:///C:/x");
            assert_eq!(path_to_uri(&pb(r"\\srv\share\f")), "file://srv/share/f");
            assert_eq!(uri_to_path("file:///C:/My%20Docs/a%231.txt").unwrap(), pb(r"C:\My Docs\a#1.txt"));
            assert_eq!(uri_to_path("file://srv/share/f").unwrap(), pb(r"\\srv\share\f"));
        } else {
            assert_eq!(path_to_uri(&pb("/home/me/a b/ü.txt")), "file:///home/me/a%20b/%C3%BC.txt");
            assert_eq!(uri_to_path("file:///home/me/a%20b/%C3%BC.txt").unwrap(), pb("/home/me/a b/ü.txt"));
            assert_eq!(uri_to_path("file://localhost/etc/x").unwrap(), pb("/etc/x"));
            assert!(uri_to_path("file://remote/etc/x").is_none());
        }
        assert!(uri_to_path("https://example.com/x").is_none());
        assert_eq!(percent_decode("100%"), b"100%", "trailing percent kept");
    }

    #[test]
    fn uri_list_round_trip() {
        let paths = if cfg!(windows) {
            vec![pb(r"C:\a b\c.txt"), pb(r"D:\ünï\x%y")]
        } else {
            vec![pb("/a b/c.txt"), pb("/ünï/x%y")]
        };
        let text = encode_uri_list(&paths);
        assert!(text.ends_with("\r\n"));
        assert_eq!(decode_uri_list(&format!("# comment\r\n{text}\r\nhttp://x/y\r\n")), paths);
    }

    #[test]
    fn selection_formats_round_trip() {
        let paths = if cfg!(windows) { vec![pb(r"C:\x y")] } else { vec![pb("/x y"), pb("/z")] };
        for mode in [ClipMode::Copy, ClipMode::Cut] {
            let f = selection_formats(mode, &paths);
            let types: Vec<String> = f.iter().map(|(t, _)| t.clone()).collect();
            assert_eq!(types[0], GNOME_FILES);
            assert_eq!(types.contains(&KDE_CUT.to_string()), mode == ClipMode::Cut);
            let read = |t: &str| f.iter().find(|(x, _)| x == t).map(|(_, b)| b.clone());
            assert_eq!(files_from_selection(&types, read), Some((mode, paths.clone())));
            // Without GNOME's format: uri-list plus KDE's cut marker.
            let kde: Vec<String> = types.iter().filter(|t| *t != GNOME_FILES).cloned().collect();
            assert_eq!(files_from_selection(&kde, read), Some((mode, paths.clone())));
        }
        let text_only = vec!["text/plain".to_string()];
        assert_eq!(files_from_selection(&text_only, |_| Some(b"/etc".to_vec())), None);
    }

    #[test]
    fn gnome_copied_files() {
        let paths = if cfg!(windows) { vec![pb(r"C:\x y")] } else { vec![pb("/x y"), pb("/z")] };
        let s = encode_gnome(ClipMode::Cut, &paths);
        assert!(s.starts_with("cut\nfile://"));
        assert_eq!(decode_gnome(&s).unwrap(), (ClipMode::Cut, paths.clone()));
        assert_eq!(decode_gnome(&encode_gnome(ClipMode::Copy, &paths)).unwrap().0, ClipMode::Copy);
        assert!(decode_gnome("copy\n").is_none());
        assert!(decode_gnome("hello\nfile:///x").is_none());
    }
}
