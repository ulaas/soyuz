//! Drag files out of the app into other applications (Windows: OLE `DoDragDrop`; X11:
//! an XDND drag source), plus the Wayland drop target (files dragged *into* the window).
//!
//! The caller prepares the clipboard-format payloads (`CF_HDROP` `DROPFILES`, text, ...);
//! this module hands them to a hand-written `IDataObject` / `IDropSource` pair in
//! `sys/windows/dragdrop.rs` (ole32 + user32 + kernel32 only, no shell APIs) and reports
//! what the target did.
//!
//! Linux:
//! * **X11 drag-out** ([`start_x11_drag`]): a non-modal XDND source on its own X
//!   connection and background thread (`sys/linux/xdnd.rs`). It owns `XdndSelection`,
//!   offers `text/uri-list` + `x-special/gnome-copied-files` + plain text, follows the
//!   pointer with `QueryPointer` (the implicit button grab stays with the app window, so
//!   no grab is needed), talks `XdndEnter/Position/Leave/Drop` to `XdndAware` windows and
//!   maps `XdndFinished` (and a `DELETE` conversion) to a [`DragReport`] ([`xdnd_report`]).
//! * **Wayland drop target** ([`start_wayland_drop_target`]): winit 0.30 has no Wayland
//!   drag-and-drop, so `sys/linux/wayland_dnd.rs` attaches to the window's own
//!   `wl_display` with a separate event queue (dispatched on a background thread), binds
//!   `wl_data_device` for the seat and reports drags over the window's `wl_surface` as
//!   [`DropTargetEvent`]s. Only copy is accepted ([`drop_actions`]).
//! * **Wayland drag-out** ([`start_wayland_drag`]): a `wl_data_source` on the same
//!   thread, started with the press serial our own `wl_pointer` saw; see
//!   [`wayland_drag_report`] for how its result is read.
//!
//! macOS: not implemented (stub). A native drag-out would need an `NSDraggingSource` on
//! the window's `NSView` (`beginDraggingSessionWithItems:event:source:`), which winit does
//! not expose; [`do_drag`] reports `Unsupported` and [`primary_button_down`] is `false`.
//! Files dragged *into* the window work through winit.

use std::io;
use std::path::PathBuf;

/// `DROPEFFECT_*` values.
pub const DROPEFFECT_NONE: u32 = 0;
pub const DROPEFFECT_COPY: u32 = 1;
pub const DROPEFFECT_MOVE: u32 = 2;
pub const DROPEFFECT_LINK: u32 = 4;

/// A clipboard format offered by the drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Format {
    /// `CF_HDROP` (15): a `DROPFILES` block.
    Hdrop,
    /// `CF_UNICODETEXT` (13): NUL-terminated UTF-16 text.
    UnicodeText,
    /// A registered format by name ("Preferred DropEffect", "FileNameW", ...).
    Named(String),
}

/// Formats offered, in preference order.
#[derive(Clone, Debug, Default)]
pub struct DragData {
    pub formats: Vec<(Format, Vec<u8>)>,
}

/// How the drag ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Dropped on a target.
    Dropped,
    /// Esc, or released where nothing accepts the data.
    Cancelled,
    /// The pointer came back over the app's own window (the internal drag resumes).
    Returned,
}

/// What a drag-out did, including the formats the target wrote back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragReport {
    pub outcome: Outcome,
    /// The `DROPEFFECT_*` `DoDragDrop` returned (`DROPEFFECT_NONE` unless dropped).
    pub effect: u32,
    /// "Performed DropEffect" written by the target, if any.
    pub performed: Option<u32>,
    /// "Logical Performed DropEffect" (set by targets doing an optimized move).
    pub logical_performed: Option<u32>,
    /// "Paste Succeeded" written by the target, if any.
    pub paste_succeeded: Option<u32>,
}

impl DragReport {
    /// A report for an outcome without any target feedback.
    pub fn plain(outcome: Outcome, effect: u32) -> Self {
        DragReport { outcome, effect, performed: None, logical_performed: None, paste_succeeded: None }
    }
}

/// Run a modal drag of `data` on the calling UI thread (it must own the window the drag
/// started from and run a message loop). `allowed` is a `DROPEFFECT_*` mask; moving the
/// pointer back over the top-level window `return_hwnd` (0 = never) ends the OLE drag with
/// [`Outcome::Returned`]. Returns when the drag is over.
pub fn do_drag(data: &DragData, allowed: u32, return_hwnd: isize) -> io::Result<DragReport> {
    #[cfg(windows)]
    {
        use crate::sys::windows::dragdrop as d;
        let formats = resolve(data)?;
        let r = d::do_drag(formats, allowed, return_hwnd)?;
        let read = |name: &str| {
            let id = d::register_format(name);
            r.written
                .iter()
                .find(|(cf, _)| *cf == id && id != 0)
                .and_then(|(_, b)| Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?)))
        };
        let outcome = if r.dropped {
            Outcome::Dropped
        } else if r.returned {
            Outcome::Returned
        } else {
            Outcome::Cancelled
        };
        Ok(DragReport {
            outcome,
            effect: r.effect,
            performed: read("Performed DropEffect"),
            logical_performed: read("Logical Performed DropEffect"),
            paste_succeeded: read("Paste Succeeded"),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (data, allowed, return_hwnd);
        Err(io::Error::new(io::ErrorKind::Unsupported, "drag-out is not implemented on this platform"))
    }
}

/// The primary mouse button is physically held right now (guards against starting a
/// drag after a release the app never saw). Wayland (drop target running): the state
/// our own `wl_pointer` saw; X11: `QueryPointer` on `$DISPLAY`. `false` elsewhere.
pub fn primary_button_down() -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::dragdrop::primary_button_down()
    }
    #[cfg(target_os = "linux")]
    {
        use crate::sys::linux::{wayland_dnd, xdnd};
        if wayland_dnd::running() { wayland_dnd::button_down() } else { xdnd::primary_button_down() }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Windows: tell our own window `hwnd` the primary button went up (the real release went
/// to the OLE loop) for windows whose input the app cannot amend before a frame. A no-op
/// elsewhere.
pub fn release_button(hwnd: isize) {
    #[cfg(windows)]
    crate::sys::windows::dragdrop::post_button_up(hwnd);
    #[cfg(not(windows))]
    let _ = hwnd;
}

/// Diagnostics / tests: build the data object and read every offered format back through
/// its COM vtable (`EnumFormatEtc`, `QueryGetData`, `GetData`). Block sizes may be rounded
/// up by the allocator, so returned bytes can carry trailing padding.
pub fn read_back(data: &DragData) -> io::Result<Vec<(Format, Vec<u8>)>> {
    #[cfg(windows)]
    {
        let formats = resolve(data)?;
        let (got, _) = crate::sys::windows::dragdrop::read_back(formats.clone(), None)?;
        Ok(got
            .into_iter()
            .map(|(cf, bytes)| {
                let fmt = formats
                    .iter()
                    .position(|(id, _)| *id == cf)
                    .map(|i| data.formats[i].0.clone())
                    .unwrap_or(Format::Named(format!("#{cf}")));
                (fmt, bytes)
            })
            .collect())
    }
    #[cfg(not(windows))]
    {
        let _ = data;
        Err(io::Error::new(io::ErrorKind::Unsupported, "drag-out is not implemented on this platform"))
    }
}

#[cfg(windows)]
fn resolve(data: &DragData) -> io::Result<Vec<(u16, Vec<u8>)>> {
    data.formats
        .iter()
        .map(|(f, bytes)| {
            let id = match f {
                Format::Hdrop => 15,
                Format::UnicodeText => 13,
                Format::Named(n) => match crate::sys::windows::dragdrop::register_format(n) {
                    0 => return Err(io::Error::other(format!("RegisterClipboardFormat({n}) failed"))),
                    id => id,
                },
            };
            Ok((id, bytes.clone()))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// freedesktop drag and drop (X11 XDND source, Wayland drop target). The pure parts below
// compile everywhere so they are unit-tested on every OS.

/// The file-list MIME type of the freedesktop drag-and-drop / clipboard world (RFC 2483).
pub const MIME_URI_LIST: &str = "text/uri-list";
/// GNOME's file-list type: `copy` / `cut`, then one URI per line.
pub const MIME_GNOME_FILES: &str = "x-special/gnome-copied-files";

/// `wl_data_device_manager.dnd_action` bits.
pub const DND_ACTION_COPY: u32 = 1;
pub const DND_ACTION_MOVE: u32 = 2;
pub const DND_ACTION_ASK: u32 = 4;

/// The local paths of a `text/uri-list` (CRLF or LF lines, `#` comments) or
/// `x-special/gnome-copied-files` (leading `copy` / `cut` line) payload. Non-`file:`
/// URIs and remote hosts are skipped. The text is untrusted: nothing but `file:` URIs
/// with an absolute path come out.
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && *l != "copy" && *l != "cut")
        .filter_map(crate::launch::path_from_file_uri)
        .collect()
}

/// `text/uri-list` for `paths` (CRLF-terminated lines, as RFC 2483 asks).
pub fn uri_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| crate::launch::file_uri(p) + "\r\n").collect()
}

/// `x-special/gnome-copied-files` for `paths` (`cut` when the drag suggests a move).
pub fn gnome_copied_files(paths: &[PathBuf], cut: bool) -> String {
    let mut s = String::from(if cut { "cut" } else { "copy" });
    for p in paths {
        s.push('\n');
        s.push_str(&crate::launch::file_uri(p));
    }
    s
}

/// The MIME type to receive from a drag offer: `text/uri-list`, else GNOME's file list;
/// `None` when the offer carries no files.
pub fn pick_mime(offered: &[String]) -> Option<&'static str> {
    [MIME_URI_LIST, MIME_GNOME_FILES].into_iter().find(|m| offered.iter().any(|o| o == m))
}

/// `(actions, preferred)` to answer a Wayland drag offer with, given the source's
/// `dnd_action` mask (0 = a pre-v3 source without action negotiation).
///
/// External drops are always *copied* into the app (`transfer(.., Copy)` behind
/// [`DropTargetEvent::Files`]), so only copy is accepted: accepting "move" would let the
/// source delete its files as soon as the drop finishes, before the copy job has run.
/// `None` when the source cannot copy (the drag shows "not accepted").
pub fn drop_actions(source_actions: u32) -> Option<(u32, u32)> {
    (source_actions == 0 || source_actions & DND_ACTION_COPY != 0).then_some((DND_ACTION_COPY, DND_ACTION_COPY))
}

/// A drag of files over the app's window, as seen by the Wayland drop target. Positions
/// are surface-local logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum DropTargetEvent {
    /// A drag carrying files entered the window.
    Enter {
        x: f64,
        y: f64,
    },
    Motion {
        x: f64,
        y: f64,
    },
    /// The drag left (or was cancelled) without a drop.
    Leave,
    /// Dropped here; the paths follow as [`DropTargetEvent::Files`] once read.
    Drop {
        x: f64,
        y: f64,
    },
    /// The dropped paths (empty when the source sent nothing usable).
    Files(Vec<PathBuf>),
    /// The app's own Wayland drag-out came back over the window (the internal drag
    /// resumes while the compositor drag goes on).
    OwnEnter {
        x: f64,
        y: f64,
    },
    OwnMotion {
        x: f64,
        y: f64,
    },
    /// It left the window again.
    OwnLeave,
    /// It was dropped on the window: an internal drop at that position.
    OwnDrop {
        x: f64,
        y: f64,
    },
}

/// Receives [`DropTargetEvent`]s on the drop target's background threads.
pub type DropSink = Box<dyn Fn(DropTargetEvent) + Send + Sync>;

/// Wayland: become a drop target for files on the app window. `window` is the app's
/// window (its display and window handles must be Wayland ones, else `InvalidInput`).
///
/// # Lifetime contract
/// The drop target keeps using the window's `wl_display` and `wl_surface` after this
/// call returns: the window and its Wayland connection must stay alive until
/// [`stop_wayland_drop_target`] runs (call it before the window goes away, e.g. from the
/// app's exit hook). The typed handles (instead of raw pointers) make sure what is
/// passed really is a live Wayland display / surface at the time of the call.
///
/// A separate event queue on the same connection is dispatched on a background thread;
/// the window toolkit's own queue is never touched. Errors when the compositor has no
/// `wl_data_device_manager` / `wl_seat` (logged by the caller; drops then simply do
/// nothing). Unsupported off Linux.
pub fn start_wayland_drop_target<W>(window: &W, sink: DropSink) -> io::Result<()>
where
    W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + ?Sized,
{
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
    let display = window.display_handle().map_err(|e| io::Error::other(e.to_string()))?.as_raw();
    let surface = window.window_handle().map_err(|e| io::Error::other(e.to_string()))?.as_raw();
    let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) = (display, surface) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a Wayland window"));
    };
    #[cfg(target_os = "linux")]
    {
        crate::sys::linux::wayland_dnd::start(
            display.display.as_ptr() as usize,
            surface.surface.as_ptr() as usize,
            sink,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (display, surface, sink);
        Err(io::Error::new(io::ErrorKind::Unsupported, "Wayland drop target is Linux-only"))
    }
}

/// Stop the Wayland drop target (releases the data device, ends the thread). Idempotent.
pub fn stop_wayland_drop_target() {
    #[cfg(target_os = "linux")]
    crate::sys::linux::wayland_dnd::stop();
}

/// The `DragReport` of an XDND drag that ended with a drop: `finished` is
/// `Some((accepted, action))` from `XdndFinished` (`None` when the target never sent it
/// in time), `action` a `DROPEFFECT_*` value, `delete_requested` whether the target
/// converted `XdndSelection` to `DELETE` (the XDND way of asking the source to remove a
/// moved item).
///
/// A move without `DELETE` means the target moved the files itself (file managers do):
/// reported as an optimized move (`DROPEFFECT_NONE` + logical move), so the app only
/// refreshes and never deletes anything on its own.
pub fn xdnd_report(finished: Option<(bool, u32)>, delete_requested: bool) -> DragReport {
    let Some((true, action)) = finished else {
        return DragReport::plain(Outcome::Dropped, DROPEFFECT_NONE);
    };
    if action & DROPEFFECT_MOVE != 0 {
        if delete_requested {
            let mut r = DragReport::plain(Outcome::Dropped, DROPEFFECT_MOVE);
            r.performed = Some(DROPEFFECT_MOVE);
            return r;
        }
        let mut r = DragReport::plain(Outcome::Dropped, DROPEFFECT_NONE);
        r.logical_performed = Some(DROPEFFECT_MOVE);
        return r;
    }
    DragReport::plain(Outcome::Dropped, action & (DROPEFFECT_COPY | DROPEFFECT_LINK))
}

/// The `DragReport` of a Wayland drag-out that finished (`dnd_finished`) with the
/// `dnd_action` bits last announced. Wayland has no way for the target to say it moved
/// the files itself, and file managers do move them themselves (asynchronously, so the
/// sources may still exist when `dnd_finished` arrives): a move is therefore always
/// reported as an optimized move (the app refreshes, never deletes).
pub fn wayland_drag_report(finished: bool, action: u32) -> DragReport {
    if !finished {
        return DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE);
    }
    if action & DND_ACTION_MOVE != 0 {
        let mut r = DragReport::plain(Outcome::Dropped, DROPEFFECT_NONE);
        r.logical_performed = Some(DROPEFFECT_MOVE);
        return r;
    }
    let effect = if action & DND_ACTION_COPY != 0 { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
    DragReport::plain(Outcome::Dropped, effect)
}

/// A running non-modal drag-out (X11 XDND or Wayland). Poll it every frame until it
/// reports.
#[cfg(target_os = "linux")]
pub struct DragSession {
    rx: crossbeam_channel::Receiver<DragReport>,
    cancel: Box<dyn Fn() + Send + Sync>,
}

#[cfg(target_os = "linux")]
impl DragSession {
    /// The final report once the drag is over (`Some` exactly once).
    pub fn poll(&self) -> Option<DragReport> {
        self.rx.try_recv().ok()
    }

    /// End the drag as cancelled (Esc in the app window).
    pub fn cancel(&self) {
        (self.cancel)();
    }
}

/// X11: start a non-modal XDND drag of `paths` on a background thread. `preferred` is
/// the `DROPEFFECT_*` suggested when no modifier is held (Ctrl = copy, Shift = move,
/// Ctrl+Shift = link, read from the pointer's modifier mask); `own_window` is the app's
/// X11 window (the pointer coming back over it ends the drag with
/// [`Outcome::Returned`]). The primary button must be held: releasing it drops.
#[cfg(target_os = "linux")]
pub fn start_x11_drag(paths: &[PathBuf], preferred: u32, own_window: u32) -> io::Result<DragSession> {
    let paths = absolute(paths)?;
    let (rx, cancel) = crate::sys::linux::xdnd::start(paths, preferred, own_window)?;
    Ok(DragSession { rx, cancel: Box::new(move || cancel.store(true, std::sync::atomic::Ordering::Relaxed)) })
}

/// Wayland: start a drag of `paths` from the window the drop target runs on
/// ([`start_wayland_drop_target`]), using the serial of the primary-button press that
/// is still held. The target / compositor picks copy or move (modifiers).
#[cfg(target_os = "linux")]
pub fn start_wayland_drag(paths: &[PathBuf]) -> io::Result<DragSession> {
    let paths = absolute(paths)?;
    let rx = crate::sys::linux::wayland_dnd::start_drag(paths)?;
    Ok(DragSession { rx, cancel: Box::new(crate::sys::linux::wayland_dnd::cancel_drag) })
}

/// Whether a Wayland drag-out can start (the drop target thread is up with a pointer).
pub fn wayland_drag_ready() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::sys::linux::wayland_dnd::drag_ready()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
fn absolute(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = paths.iter().filter(|p| p.is_absolute()).cloned().collect();
    if paths.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nothing to drag"));
    }
    Ok(paths)
}

/// Plain-text form of a dragged file list (one path per line).
pub fn plain_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod fd_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn uri_lists_round_trip() {
        let paths = vec![PathBuf::from("/home/u/My Docs/a#b.txt"), PathBuf::from("/tmp/ü")];
        let list = uri_list(&paths);
        assert_eq!(list, "file:///home/u/My%20Docs/a%23b.txt\r\nfile:///tmp/%C3%BC\r\n");
        assert_eq!(parse_uri_list(&list), paths);
        let gnome = gnome_copied_files(&paths, true);
        assert!(gnome.starts_with("cut\nfile:///home/"));
        assert_eq!(parse_uri_list(&gnome), paths);
        assert_eq!(parse_uri_list(&gnome_copied_files(&paths, false)), paths);
        assert_eq!(plain_text(&paths), "/home/u/My Docs/a#b.txt\n/tmp/ü");
    }

    #[cfg(unix)]
    #[test]
    fn uri_list_parsing_is_strict() {
        let text = "# comment\r\nfile:///a/b\r\n\r\nhttps://example.com/x\nfile://otherhost/c\nfile://localhost/d%20e\n  file:///f  \nrelative/path\n";
        assert_eq!(parse_uri_list(text), vec![PathBuf::from("/a/b"), PathBuf::from("/d e"), PathBuf::from("/f")]);
        assert!(parse_uri_list("").is_empty());
    }

    #[test]
    fn mime_and_action_choice() {
        let s = |v: &[&str]| v.iter().map(|m| m.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_mime(&s(&["text/plain", MIME_URI_LIST])), Some(MIME_URI_LIST));
        assert_eq!(pick_mime(&s(&[MIME_GNOME_FILES, MIME_URI_LIST])), Some(MIME_URI_LIST));
        assert_eq!(pick_mime(&s(&[MIME_GNOME_FILES])), Some(MIME_GNOME_FILES));
        assert_eq!(pick_mime(&s(&["text/plain", "image/png"])), None);
        assert_eq!(drop_actions(DND_ACTION_COPY | DND_ACTION_MOVE), Some((DND_ACTION_COPY, DND_ACTION_COPY)));
        assert_eq!(drop_actions(DND_ACTION_COPY | DND_ACTION_ASK), Some((DND_ACTION_COPY, DND_ACTION_COPY)));
        assert_eq!(drop_actions(0), Some((DND_ACTION_COPY, DND_ACTION_COPY)), "pre-v3 source");
        assert_eq!(drop_actions(DND_ACTION_MOVE), None, "never accept a move-only drag");
        assert_eq!(drop_actions(DND_ACTION_ASK), None);
    }

    #[test]
    fn wayland_reports() {
        assert_eq!(wayland_drag_report(false, DND_ACTION_COPY).outcome, Outcome::Cancelled);
        assert_eq!(wayland_drag_report(true, DND_ACTION_COPY), DragReport::plain(Outcome::Dropped, DROPEFFECT_COPY));
        assert_eq!(wayland_drag_report(true, 0), DragReport::plain(Outcome::Dropped, DROPEFFECT_NONE));
        let r = wayland_drag_report(true, DND_ACTION_MOVE);
        assert_eq!((r.effect, r.logical_performed), (DROPEFFECT_NONE, Some(DROPEFFECT_MOVE)), "never delete");
    }

    #[test]
    fn xdnd_reports() {
        assert_eq!(xdnd_report(None, false), DragReport::plain(Outcome::Dropped, DROPEFFECT_NONE));
        assert_eq!(xdnd_report(Some((false, DROPEFFECT_COPY)), false).effect, DROPEFFECT_NONE);
        assert_eq!(xdnd_report(Some((true, DROPEFFECT_COPY)), false).effect, DROPEFFECT_COPY);
        assert_eq!(xdnd_report(Some((true, DROPEFFECT_LINK)), false).effect, DROPEFFECT_LINK);
        // Target moved the files itself: optimized move.
        let r = xdnd_report(Some((true, DROPEFFECT_MOVE)), false);
        assert_eq!((r.effect, r.logical_performed), (DROPEFFECT_NONE, Some(DROPEFFECT_MOVE)));
        // Target asked the source to delete: unoptimized move.
        let r = xdnd_report(Some((true, DROPEFFECT_MOVE)), true);
        assert_eq!((r.effect, r.performed), (DROPEFFECT_MOVE, Some(DROPEFFECT_MOVE)));
    }
}

#[cfg(test)]
mod drop_target_tests {
    use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

    /// A window without Wayland handles (here: a web display, no window handle).
    struct NotWayland;
    impl HasDisplayHandle for NotWayland {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::web())
        }
    }
    impl HasWindowHandle for NotWayland {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::Unavailable)
        }
    }

    #[test]
    fn drop_target_needs_live_wayland_handles() {
        let r = super::start_wayland_drop_target(&NotWayland, Box::new(|_| {}));
        assert!(r.is_err(), "no raw pointer is ever made up from a non-Wayland window");
    }
}
