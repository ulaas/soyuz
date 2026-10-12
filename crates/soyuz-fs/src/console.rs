//! Console helpers for command-line modes of GUI binaries.
//!
//! Two different needs live here:
//! - [`attach_parent`]: `--help` / `--version` / `soyuz cp …` print into the shell that
//!   started a GUI-subsystem `soyuz.exe`.
//! - [`tui_console`]: `soyuz --tui` needs a console of its *own*. A GUI-subsystem process
//!   never attaches to the parent shell's console for the TUI: the shell has already
//!   printed its prompt again (it does not wait for GUI-subsystem children), so the TUI
//!   and the shell would both read the same keyboard input. Instead a release build
//!   without a console opens a new console window (`AllocConsole`); a debug build
//!   (console subsystem) or a process started inside a terminal already has one and
//!   uses it as is. That includes `soyuz-tui.exe`, a console-subsystem binary: Windows
//!   gives it the console of the terminal that started it (which waits for it), or a
//!   new one when it is started from Explorer, so it always runs in place.
//!
//! On unix every terminal concern (raw mode, signals) belongs to the TUI itself, so the
//! session type is a no-op there.

use std::io;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Windows: attach to the parent process's console so a GUI-subsystem binary run from a
/// terminal (`soyuz cp …`) can print there. `false` when there is no console to attach
/// to (or on other platforms, where stdout already works).
pub fn attach_parent() -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::console::attach_parent()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The console the TUI runs in, from [`tui_console`].
///
/// While it lives, the console input and output code pages are UTF-8 (65001). Dropping
/// it (or calling [`restore`], e.g. from a panic hook) puts back the code pages and the
/// input / output console modes saved on entry, so a shared console (debug build, a
/// terminal that started a console-subsystem build) is left the way it was found.
///
/// A console allocated by [`tui_console`] is deliberately **not** freed on drop
/// (`FreeConsole`): it closes by itself when the process exits, and keeping it until
/// then means a late error message (printed after the TUI left the alternate screen)
/// still has somewhere to go.
#[derive(Debug)]
#[must_use = "dropping the session restores the console immediately"]
pub struct ConsoleSession {
    allocated: bool,
}

impl ConsoleSession {
    /// `true` when [`tui_console`] opened a new console window for this process
    /// (release build started without a console); `false` when an existing console is
    /// used (debug build, started from a terminal) and on unix.
    pub fn allocated(&self) -> bool {
        self.allocated
    }
}

impl Drop for ConsoleSession {
    fn drop(&mut self) {
        restore();
    }
}

/// Prepare the console for the terminal UI.
///
/// Windows:
/// - The process already has a console (`GetConsoleWindow` is non-null: always the
///   case for the console-subsystem `soyuz-tui.exe` and debug builds): use it as is (the
///   standard handles are not touched; check [`is_terminal_session`] to refuse
///   redirected stdio).
/// - No console (GUI-subsystem release build, started from Explorer or from a shell):
///   `AllocConsole` opens a new console window titled "Soyuz", and standard input /
///   output are pointed at it (`CONIN$` / `CONOUT$`) unless they already are console
///   handles. Rust's `std::io::stdin/stdout` look the handle up with `GetStdHandle` on
///   every call and crossterm opens `CONIN$` / `CONOUT$` itself for modes, events and
///   the window size, so both talk to the new console afterwards.
/// - Saves the console code pages and modes, then switches both code pages to UTF-8.
///
/// Unix: no-op (`allocated() == false`).
pub fn tui_console() -> io::Result<ConsoleSession> {
    #[cfg(windows)]
    {
        let allocated = crate::sys::windows::console::ensure_console()?;
        if allocated {
            crate::sys::windows::console::set_title("Soyuz");
        }
        SAVED.store(crate::sys::windows::console::capture());
        crate::sys::windows::console::set_utf8();
        Ok(ConsoleSession { allocated })
    }
    #[cfg(not(windows))]
    {
        Ok(ConsoleSession { allocated: false })
    }
}

/// Put back the console code pages and modes saved by [`tui_console`].
///
/// Safe to call from a panic hook or a console control handler and idempotent: only the
/// first call after [`tui_console`] does anything (the saved state is taken atomically),
/// later calls and the [`ConsoleSession`] drop are no-ops. No-op on unix.
pub fn restore() {
    if let Some(saved) = SAVED.take() {
        #[cfg(windows)]
        crate::sys::windows::console::apply(&saved);
        #[cfg(not(windows))]
        let _ = saved;
    }
}

/// Run `f` when the console window is closed or the session ends.
///
/// Windows (`SetConsoleCtrlHandler`):
/// - `CTRL_CLOSE_EVENT` (the console window's close button, or the terminal tab
///   closing), `CTRL_LOGOFF_EVENT`, `CTRL_SHUTDOWN_EVENT`: `f` runs on a thread the
///   system creates for the handler (the main thread keeps running, so `f` must
///   synchronise with it: e.g. set a quit flag and wait for the main loop to save).
///   Afterwards the saved code pages / modes are restored and the handler returns
///   `TRUE`; Windows then terminates the process. The budget is about **5 seconds**
///   for close (`SPI_GETHUNGAPPTIMEOUT`) and logoff / shutdown before the process is
///   killed, so `f` must only do quick work (flush session / config state), never wait
///   on the user. Note: a process that loads user32 / gdi32 (the GUI build does) does
///   not get logoff / shutdown events through this handler; for it they arrive as
///   window messages, and only the close event is reliable.
/// - `CTRL_C_EVENT` / `CTRL_BREAK_EVENT`: not handled (the handler returns `FALSE`), so
///   the next handler in the chain or the default (`ExitProcess`) applies. While the
///   TUI is in raw mode `ENABLE_PROCESSED_INPUT` is off and Ctrl+C arrives as a normal
///   key event instead of a signal, so this only matters before raw mode is entered or
///   after it is left, where ending the process is the expected behaviour. The handler
///   does not restore the console for these either: another handler further down the
///   chain could swallow the event and the process would keep running with its
///   settings undone.
///
/// Calling it again replaces `f`. Unix: stores `f` and does nothing else (the TUI
/// handles `SIGHUP` / `SIGTERM` itself).
pub fn set_close_handler(f: fn()) -> io::Result<()> {
    if let Ok(mut slot) = CLOSE_HANDLER.write() {
        *slot = Some(f);
    }
    #[cfg(windows)]
    {
        crate::sys::windows::console::install_ctrl_handler()
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// The function registered with [`set_close_handler`], if any.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn close_handler() -> Option<fn()> {
    CLOSE_HANDLER.read().ok().and_then(|slot| *slot)
}

static CLOSE_HANDLER: RwLock<Option<fn()>> = RwLock::new(None);

/// `true` when stdin and stdout are both terminals (not pipes / files) and, on unix,
/// `TERM` is not `dumb`: the conditions for drawing a full-screen TUI.
pub fn is_terminal_session() -> bool {
    use std::io::IsTerminal;
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return false;
    }
    #[cfg(unix)]
    {
        !term_is_dumb(std::env::var_os("TERM").as_deref())
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `TERM=dumb` (Emacs `M-x shell`, some CI runners) cannot draw a TUI. An unset `TERM`
/// is not treated as dumb: a real tty without it (e.g. some service shells) still works.
#[cfg_attr(not(unix), allow(dead_code))]
fn term_is_dumb(term: Option<&std::ffi::OsStr>) -> bool {
    term.is_some_and(|t| t.to_str().is_some_and(|t| t.trim() == "dumb"))
}

/// Console code pages and modes saved on entry. A mode is `None` when it could not be
/// read (e.g. no console input buffer); it is then not written back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConsoleState {
    pub(crate) input_cp: u32,
    pub(crate) output_cp: u32,
    pub(crate) input_mode: Option<u32>,
    pub(crate) output_mode: Option<u32>,
}

/// Lock-free slot for the saved [`ConsoleState`], so [`restore`] can run from a panic
/// hook or a control-handler thread without locks that might be poisoned or held.
pub(crate) struct SavedSlot {
    armed: AtomicBool,
    input_cp: AtomicU32,
    output_cp: AtomicU32,
    input_mode: AtomicU32,
    output_mode: AtomicU32,
    /// Bit 0: `input_mode` is valid, bit 1: `output_mode` is valid.
    mode_valid: AtomicU32,
}

impl SavedSlot {
    pub(crate) const fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            input_cp: AtomicU32::new(0),
            output_cp: AtomicU32::new(0),
            input_mode: AtomicU32::new(0),
            output_mode: AtomicU32::new(0),
            mode_valid: AtomicU32::new(0),
        }
    }

    /// Save `state` unless a state is already saved: nested sessions keep the outermost
    /// (original) state. Returns whether `state` was stored.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn store(&self, state: ConsoleState) -> bool {
        if self.armed.load(Ordering::Acquire) {
            return false;
        }
        self.input_cp.store(state.input_cp, Ordering::Relaxed);
        self.output_cp.store(state.output_cp, Ordering::Relaxed);
        self.input_mode.store(state.input_mode.unwrap_or(0), Ordering::Relaxed);
        self.output_mode.store(state.output_mode.unwrap_or(0), Ordering::Relaxed);
        let valid = u32::from(state.input_mode.is_some()) | (u32::from(state.output_mode.is_some()) << 1);
        self.mode_valid.store(valid, Ordering::Relaxed);
        self.armed.store(true, Ordering::Release);
        true
    }

    /// Take the saved state; `None` when nothing is saved or it was already taken.
    pub(crate) fn take(&self) -> Option<ConsoleState> {
        if !self.armed.swap(false, Ordering::AcqRel) {
            return None;
        }
        let valid = self.mode_valid.load(Ordering::Relaxed);
        Some(ConsoleState {
            input_cp: self.input_cp.load(Ordering::Relaxed),
            output_cp: self.output_cp.load(Ordering::Relaxed),
            input_mode: (valid & 1 != 0).then(|| self.input_mode.load(Ordering::Relaxed)),
            output_mode: (valid & 2 != 0).then(|| self.output_mode.load(Ordering::Relaxed)),
        })
    }
}

static SAVED: SavedSlot = SavedSlot::new();

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn state(mode: Option<u32>) -> ConsoleState {
        ConsoleState { input_cp: 437, output_cp: 850, input_mode: mode, output_mode: Some(0x7) }
    }

    #[test]
    fn saved_slot_round_trips_once() {
        let slot = SavedSlot::new();
        assert_eq!(slot.take(), None);
        assert!(slot.store(state(Some(0x1f7))));
        assert_eq!(slot.take(), Some(state(Some(0x1f7))));
        // Idempotent: a second restore (panic hook, then drop) finds nothing.
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn saved_slot_keeps_the_outermost_state() {
        let slot = SavedSlot::new();
        assert!(slot.store(state(Some(1))));
        assert!(!slot.store(state(Some(2))));
        assert_eq!(slot.take(), Some(state(Some(1))));
        // After a restore the next session saves again.
        assert!(slot.store(state(None)));
        assert_eq!(slot.take(), Some(state(None)));
    }

    #[test]
    fn missing_modes_stay_missing() {
        let slot = SavedSlot::new();
        let s = ConsoleState { input_cp: 65001, output_cp: 65001, input_mode: None, output_mode: None };
        assert!(slot.store(s));
        assert_eq!(slot.take(), Some(s));
    }

    #[test]
    fn dumb_term_detection() {
        assert!(term_is_dumb(Some(OsStr::new("dumb"))));
        assert!(term_is_dumb(Some(OsStr::new(" dumb "))));
        assert!(!term_is_dumb(Some(OsStr::new("xterm-256color"))));
        assert!(!term_is_dumb(Some(OsStr::new(""))));
        assert!(!term_is_dumb(None));
    }

    #[test]
    fn restore_without_session_is_a_no_op() {
        // Nothing saved by this test binary: must not touch the runner's console.
        restore();
        restore();
    }

    #[test]
    fn close_handler_is_stored() {
        fn on_close() {}
        set_close_handler(on_close).unwrap();
        assert!(close_handler().is_some());
    }
}
