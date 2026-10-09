//! Drag files out of the app into other applications (Windows: OLE `DoDragDrop`).
//!
//! The caller prepares the clipboard-format payloads (`CF_HDROP` `DROPFILES`, text, ...);
//! this module hands them to a hand-written `IDataObject` / `IDropSource` pair in
//! `sys/windows/dragdrop.rs` (ole32 + user32 + kernel32 only, no shell APIs) and reports
//! what the target did. Unsupported (always an error) on other platforms.

use std::io;

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
        Err(io::Error::new(io::ErrorKind::Unsupported, "drag-out is only implemented on Windows"))
    }
}

/// The primary mouse button is physically held right now (guards against starting a
/// drag after a release the app never saw). `false` off Windows.
pub fn primary_button_down() -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::dragdrop::primary_button_down()
    }
    #[cfg(not(windows))]
    {
        false
    }
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
        Err(io::Error::new(io::ErrorKind::Unsupported, "drag-out is only implemented on Windows"))
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
