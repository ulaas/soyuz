//! Console FFI for the GUI-subsystem `soyuz.exe` (kernel32 only): parent attachment for
//! the command-line modes, and an own console for the terminal UI.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, FALSE, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, CONSOLE_MODE, CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT,
    CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT, GetConsoleCP, GetConsoleMode, GetConsoleOutputCP, GetConsoleWindow,
    GetStdHandle, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCP, SetConsoleCtrlHandler, SetConsoleMode,
    SetConsoleOutputCP, SetConsoleTitleW, SetStdHandle,
};

use crate::console::ConsoleState;

const CP_UTF8: u32 = 65001;
/// `CONIN$` / `CONOUT$`, NUL-terminated UTF-16 (no allocation: used from panic hooks).
const CONIN: [u16; 7] = utf16z(*b"CONIN$\0");
const CONOUT: [u16; 8] = utf16z(*b"CONOUT$\0");

const fn utf16z<const N: usize>(ascii: [u8; N]) -> [u16; N] {
    let mut out = [0u16; N];
    let mut i = 0;
    while i < N {
        out[i] = ascii[i] as u16;
        i += 1;
    }
    out
}

/// Attach to the console of the parent process (the shell that started us).
pub(crate) fn attach_parent() -> bool {
    // SAFETY: plain kernel32 call without pointers; fails harmlessly when the parent
    // has no console or we already have one.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

/// A console handle opened by name (`CONIN$` / `CONOUT$`), closed on drop unless kept.
struct ConHandle(HANDLE);

impl ConHandle {
    fn open(name: &[u16]) -> Option<Self> {
        // SAFETY: `name` is NUL-terminated; null security attributes and template are
        // allowed. Sharing read+write is what the console devices require.
        let h = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        (h != INVALID_HANDLE_VALUE && !h.is_null()).then_some(Self(h))
    }

    /// Give up ownership (the handle becomes a process std handle for its lifetime).
    fn keep(self) -> HANDLE {
        let h = self.0;
        std::mem::forget(self);
        h
    }
}

impl Drop for ConHandle {
    fn drop(&mut self) {
        // SAFETY: we own the handle and close it once.
        unsafe { CloseHandle(self.0) };
    }
}

fn console_mode(h: HANDLE) -> Option<u32> {
    let mut mode: CONSOLE_MODE = 0;
    // SAFETY: `mode` is a valid out pointer; a non-console handle just fails.
    (unsafe { GetConsoleMode(h, &mut mode) } != 0).then_some(mode)
}

fn std_is_console(which: STD_HANDLE) -> bool {
    // SAFETY: plain call; the result may be null / invalid, which GetConsoleMode rejects.
    let h = unsafe { GetStdHandle(which) };
    !h.is_null() && h != INVALID_HANDLE_VALUE && console_mode(h).is_some()
}

/// Make sure the process has a console: `Ok(true)` when a new one was allocated.
pub(crate) fn ensure_console() -> io::Result<bool> {
    // SAFETY: plain kernel32 call.
    let has_window = !unsafe { GetConsoleWindow() }.is_null();
    if has_window {
        return Ok(false);
    }
    // SAFETY: plain kernel32 call.
    if unsafe { AllocConsole() } == 0 {
        let err = io::Error::last_os_error();
        // Already attached to a console without a window (CREATE_NO_WINDOW, some
        // pseudo consoles): use that one.
        if err.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
            return Ok(false);
        }
        return Err(err);
    }
    // AllocConsole fills the std handles only when they were not set at process start;
    // a launcher that passed its own (STARTF_USESTDHANDLES: pipes, NUL) keeps them.
    // Point stdin / stdout at the new console in that case. stderr stays as it is: a
    // redirected stderr (log file) must not be drawn over the TUI.
    for (which, name) in [(STD_INPUT_HANDLE, &CONIN[..]), (STD_OUTPUT_HANDLE, &CONOUT[..])] {
        if !std_is_console(which)
            && let Some(h) = ConHandle::open(name)
        {
            let h = h.keep();
            // SAFETY: `h` is a valid console handle owned by the process from now on.
            unsafe { SetStdHandle(which, h) };
        }
    }
    Ok(true)
}

pub(crate) fn set_title(title: &str) {
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    unsafe { SetConsoleTitleW(wide.as_ptr()) };
}

/// Read the current code pages and the `CONIN$` / `CONOUT$` modes.
pub(crate) fn capture() -> ConsoleState {
    // SAFETY: plain kernel32 calls.
    let (input_cp, output_cp) = unsafe { (GetConsoleCP(), GetConsoleOutputCP()) };
    ConsoleState {
        input_cp,
        output_cp,
        input_mode: ConHandle::open(&CONIN).and_then(|h| console_mode(h.0)),
        output_mode: ConHandle::open(&CONOUT).and_then(|h| console_mode(h.0)),
    }
}

pub(crate) fn set_utf8() {
    // SAFETY: plain kernel32 calls.
    unsafe {
        SetConsoleCP(CP_UTF8);
        SetConsoleOutputCP(CP_UTF8);
    }
}

/// Write back a captured state. Allocation-free (panic-hook safe). A code page of 0
/// (the getter failed: no console) is not written.
pub(crate) fn apply(state: &ConsoleState) {
    // SAFETY: plain kernel32 calls / handles opened just above.
    unsafe {
        if state.input_cp != 0 {
            SetConsoleCP(state.input_cp);
        }
        if state.output_cp != 0 {
            SetConsoleOutputCP(state.output_cp);
        }
        if let (Some(mode), Some(h)) = (state.input_mode, ConHandle::open(&CONIN)) {
            SetConsoleMode(h.0, mode);
        }
        if let (Some(mode), Some(h)) = (state.output_mode, ConHandle::open(&CONOUT)) {
            SetConsoleMode(h.0, mode);
        }
    }
}

static CTRL_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);
/// Ctrl+C / Ctrl+Break are handled (swallowed) by [`ctrl_handler`] while set.
static CTRL_C_HANDLED: AtomicBool = AtomicBool::new(false);

/// See `crate::console::set_ctrl_c_handled`. Uses the one registered [`ctrl_handler`],
/// never `SetConsoleCtrlHandler(NULL, TRUE)`: that "ignore Ctrl+C" attribute would be
/// inherited by the child processes, so the shell would ignore Ctrl+C too.
pub(crate) fn set_ctrl_c_handled(on: bool) -> io::Result<()> {
    CTRL_C_HANDLED.store(on, Ordering::Release);
    if on { install_ctrl_handler() } else { Ok(()) }
}

/// Register [`ctrl_handler`] once; the callback itself is looked up on each event.
pub(crate) fn install_ctrl_handler() -> io::Result<()> {
    if CTRL_HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    // SAFETY: `ctrl_handler` has the PHANDLER_ROUTINE signature and lives forever.
    if unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), TRUE) } == 0 {
        CTRL_HANDLER_INSTALLED.store(false, Ordering::Release);
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Runs on a system-created thread. See `crate::console::set_close_handler` for the
/// semantics: close / logoff / shutdown run the callback and restore the console, then
/// return TRUE (Windows terminates the process); Ctrl+C / Ctrl+Break return TRUE (handled:
/// the process keeps running) while [`set_ctrl_c_handled`] is on, else fall through.
unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> windows_sys::core::BOOL {
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT if CTRL_C_HANDLED.load(Ordering::Acquire) => TRUE,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            // Never unwind across the FFI boundary.
            let _ = std::panic::catch_unwind(|| {
                if let Some(f) = crate::console::close_handler() {
                    f();
                }
                crate::console::restore();
            });
            TRUE
        }
        _ => FALSE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_names_are_nul_terminated_utf16() {
        assert_eq!(String::from_utf16_lossy(&CONIN), "CONIN$\0");
        assert_eq!(String::from_utf16_lossy(&CONOUT), "CONOUT$\0");
    }

    #[test]
    fn capture_does_not_touch_the_console() {
        // Read-only: works with or without a console (the test runner may have none).
        let s = capture();
        if s.input_cp == 0 {
            assert_eq!(s.output_cp, 0);
        }
    }

    /// Opens a real console window (`AllocConsole`) when the test process has none, and
    /// changes code pages: run by hand only, never under a shared terminal.
    #[test]
    #[ignore = "allocates a console window / changes the console code pages"]
    fn tui_console_round_trip() {
        let before = capture();
        let session = crate::console::tui_console().unwrap();
        // SAFETY: plain kernel32 calls.
        unsafe {
            assert_eq!(GetConsoleOutputCP(), CP_UTF8);
            assert_eq!(GetConsoleCP(), CP_UTF8);
        }
        assert!(std_is_console(STD_OUTPUT_HANDLE));
        drop(session);
        if before.input_cp != 0 {
            assert_eq!(capture(), before);
        }
    }
}
