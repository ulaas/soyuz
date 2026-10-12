//! Windows clipboard shim: raw user32 / kernel32 calls only (`OpenClipboard`,
//! `GetClipboardData` / `SetClipboardData`, `GlobalAlloc` / `GlobalLock`,
//! `RegisterClipboardFormatW`). The `DROPFILES` block is built and parsed by hand in
//! the parent module. This is the only `unsafe` code in `soyuz-app`.

use std::path::PathBuf;
use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, GetAsyncKeyState};

use super::{build_dropfiles, drop_effect, mode_from_effect, parse_dropfiles, paths_text};
use crate::ops::fileops::clipboard::ClipMode;

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_V: i32 = 0x56;

/// Open clipboard session; closed on drop.
struct Session;

impl Session {
    /// `OpenClipboard` fails while another process holds it: retry briefly.
    ///
    /// The owner is our active window when there is one (calls come from the UI thread,
    /// whose window pumps messages): with a NULL owner, `EmptyClipboard` makes the
    /// clipboard ownerless and a following `SetClipboardData` may fail.
    fn open() -> Result<Session, String> {
        // SAFETY: plain query; returns our thread's active window or NULL.
        let owner: HWND = unsafe { GetActiveWindow() };
        for attempt in 0..10 {
            // SAFETY: plain Win32 call; `owner` is our window or NULL (allowed).
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Session);
            }
            std::thread::sleep(Duration::from_millis(5 + attempt * 5));
        }
        Err(format!("OpenClipboard failed: {}", std::io::Error::last_os_error()))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: only constructed after a successful OpenClipboard.
        unsafe {
            CloseClipboard();
        }
    }
}

fn register(name: &str) -> u32 {
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call.
    unsafe { RegisterClipboardFormatW(w.as_ptr()) }
}

fn preferred_drop_effect() -> u32 {
    register("Preferred DropEffect")
}

/// A filled movable global memory block we own until the clipboard takes it.
struct Block(HGLOBAL);

impl Block {
    /// Copy `bytes` into a new movable block.
    fn new(bytes: &[u8]) -> Result<Block, String> {
        // SAFETY: the block is allocated with at least `bytes.len()` bytes, locked,
        // filled with exactly `bytes.len()` bytes and unlocked; on failure it is freed.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
            if h.is_null() {
                return Err("GlobalAlloc failed".into());
            }
            let p = GlobalLock(h) as *mut u8;
            if p.is_null() {
                GlobalFree(h);
                return Err("GlobalLock failed".into());
            }
            ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            GlobalUnlock(h);
            Ok(Block(h))
        }
    }

    /// Hand the block to the clipboard (session must be open). On success the clipboard
    /// owns it; on failure it is freed by `Drop`.
    fn set(self, _s: &Session, format: u32) -> Result<(), String> {
        // SAFETY: the clipboard is open; `self.0` is a valid unlocked movable block.
        if unsafe { SetClipboardData(format, self.0 as HANDLE) }.is_null() {
            return Err(format!("SetClipboardData({format}) failed: {}", std::io::Error::last_os_error()));
        }
        std::mem::forget(self);
        Ok(())
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        // SAFETY: we still own the block (it was never accepted by the clipboard).
        unsafe {
            GlobalFree(self.0);
        }
    }
}

/// Read the bytes of a clipboard format (session must be open).
fn get_bytes(_s: &Session, format: u32) -> Option<Vec<u8>> {
    // SAFETY: the clipboard owns the handle; we lock it, copy `GlobalSize` bytes out
    // and unlock it again without keeping any pointer.
    unsafe {
        let h = GetClipboardData(format);
        if h.is_null() {
            return None;
        }
        let size = GlobalSize(h);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, size).to_vec();
        GlobalUnlock(h);
        Some(v)
    }
}

pub(super) fn set_files(mode: ClipMode, paths: &[PathBuf]) -> Result<(), String> {
    let hdrop = build_dropfiles(paths);
    let effect = drop_effect(mode).to_le_bytes();
    let text: Vec<u8> = paths_text(paths).encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
    let fmt_effect = preferred_drop_effect();
    // Allocate everything before touching the clipboard, so a failure leaves the
    // user's current clipboard untouched.
    let mut blocks = vec![(CF_HDROP, Block::new(&hdrop)?)];
    if fmt_effect != 0 {
        blocks.push((fmt_effect, Block::new(&effect)?));
    }
    blocks.push((CF_UNICODETEXT, Block::new(&text)?));
    let s = Session::open()?;
    // SAFETY: clipboard is open (held by `s`).
    if unsafe { EmptyClipboard() } == 0 {
        return Err(format!("EmptyClipboard failed: {}", std::io::Error::last_os_error()));
    }
    for (format, block) in blocks {
        if let Err(e) = block.set(&s, format) {
            // Never leave a partial set (e.g. files without the cut/copy effect, which
            // would turn a cut into a copy): clear what was written. Remaining blocks
            // are freed when the iterator drops.
            // SAFETY: clipboard is still open (held by `s`).
            unsafe {
                EmptyClipboard();
            }
            return Err(e);
        }
    }
    drop(s);
    Ok(())
}

/// Replace the clipboard with plain text (`CF_UNICODETEXT`).
pub(super) fn set_text(text: &str) -> Result<(), String> {
    let bytes: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
    // Allocate before touching the clipboard, so a failure leaves it untouched.
    let block = Block::new(&bytes)?;
    let s = Session::open()?;
    // SAFETY: clipboard is open (held by `s`).
    if unsafe { EmptyClipboard() } == 0 {
        return Err(format!("EmptyClipboard failed: {}", std::io::Error::last_os_error()));
    }
    block.set(&s, CF_UNICODETEXT)
}

pub(super) fn has_files() -> bool {
    // SAFETY: plain query, no clipboard session needed.
    unsafe { IsClipboardFormatAvailable(CF_HDROP) != 0 }
}

pub(super) fn get_files() -> Option<(ClipMode, Vec<PathBuf>)> {
    if !has_files() {
        return None;
    }
    let fmt_effect = preferred_drop_effect();
    let s = Session::open().ok()?;
    let paths = parse_dropfiles(&get_bytes(&s, CF_HDROP)?)?;
    let effect = (fmt_effect != 0)
        .then(|| get_bytes(&s, fmt_effect))
        .flatten()
        .and_then(|b| Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?)));
    Some((mode_from_effect(effect), paths))
}

pub(super) fn clear() {
    if let Ok(_s) = Session::open() {
        // SAFETY: clipboard is open (held by `_s`).
        unsafe {
            EmptyClipboard();
        }
    }
}

/// `(ctrl, shift, v)` physically down right now.
pub(super) fn paste_keys_down() -> Option<(bool, bool, bool)> {
    // SAFETY: plain key-state queries.
    let down = |vk: i32| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0;
    Some((down(VK_CONTROL), down(VK_SHIFT), down(VK_V)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round trip through the real clipboard (opt-in: it replaces the user's clipboard).
    #[test]
    #[ignore]
    fn real_clipboard_round_trip() {
        let paths = vec![PathBuf::from(r"C:\Windows\notepad.exe"), PathBuf::from(r"C:\Windows\win.ini")];
        set_files(ClipMode::Cut, &paths).unwrap();
        assert!(has_files());
        assert_eq!(get_files().unwrap(), (ClipMode::Cut, paths));
        clear();
        assert!(!has_files());
    }
}
