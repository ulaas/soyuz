//! Process signals and console events for the terminal UI.
//!
//! * unix: `SIGTERM`, `SIGHUP` (the terminal went away) and `SIGINT` (sent by `kill`;
//!   in raw mode Ctrl+C is a key, Copy) become [`TuiEvent::Terminate`]: the loop saves
//!   everything and quits. `SIGCONT` becomes [`TuiEvent::Resume`]: the loop re-enters raw
//!   mode and the alternate screen and repaints (the shell resets the terminal while the
//!   process is stopped). In raw mode Ctrl+Z is a key too (Undo), so the TUI is only ever
//!   stopped from outside (`kill -STOP`). `SIGQUIT` (only from `kill` in raw mode) ends
//!   the TUI like `SIGTERM`.
//! * While a child program owns the terminal ([`set_suspended`], `crate::suspend`) the
//!   terminal is in cooked mode, so Ctrl+C / Ctrl+\ send `SIGINT` / `SIGQUIT` to the whole
//!   foreground process group, Soyuz included: both are dropped then (the child gets its
//!   own copy), and so is `SIGCONT` (a child stopped with Ctrl+Z stops the group; the
//!   suspend repaints on its own when the child returns). `SIGTERM` / `SIGHUP` still end
//!   the TUI (the frame loop handles them once the child is back).
//! * Windows: closing the console window (or logoff / shutdown) runs a handler on a
//!   system thread. It sends [`TuiEvent::Terminate`] and waits (up to [`CLOSE_BUDGET`])
//!   until the frame loop has saved ([`notify_saved`]); Windows then ends the process.
//!   While suspended, Ctrl+C / Ctrl+Break are handled by that handler
//!   (`soyuz_fs::console::set_ctrl_c_handled`), so they only interrupt the child.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::Sender;
use parking_lot::{Condvar, Mutex};

use crate::app::TuiEvent;

/// How long the Windows close handler waits for the save (the system allows ~5 s).
pub const CLOSE_BUDGET: Duration = Duration::from_secs(4);

/// Where the console close handler sends its event.
static CLOSE_TX: Mutex<Option<Sender<TuiEvent>>> = Mutex::new(None);
/// Set by the frame loop once a terminate request was handled (everything saved).
static SAVED: Mutex<bool> = Mutex::new(false);
static SAVED_CV: Condvar = Condvar::new();
/// A child program owns the terminal ([`set_suspended`]).
static SUSPENDED: AtomicBool = AtomicBool::new(false);

/// A child program owns the terminal (`true`) or the TUI has it back (`false`): Ctrl+C
/// (and on unix Ctrl+\ and `SIGCONT`) must not end or disturb Soyuz meanwhile.
pub fn set_suspended(on: bool) {
    SUSPENDED.store(on, Ordering::Release);
    if let Err(e) = soyuz_fs::console::set_ctrl_c_handled(on) {
        log::warn!("tui: cannot handle Ctrl+C while suspended: {e}");
    }
}

/// [`set_suspended`] is on.
pub fn suspended() -> bool {
    SUSPENDED.load(Ordering::Acquire)
}

/// What a signal becomes: `None` drops it (pure; unix numbers, the caller passes
/// [`suspended`]).
#[cfg_attr(not(unix), allow(dead_code))]
fn event_for(sig: i32, suspended: bool) -> Option<TuiEvent> {
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGCONT, SIGINT, SIGQUIT};
        if suspended && (sig == SIGINT || sig == SIGQUIT || sig == SIGCONT) {
            return None;
        }
        Some(if sig == SIGCONT { TuiEvent::Resume } else { TuiEvent::Terminate })
    }
    #[cfg(not(unix))]
    {
        let _ = (sig, suspended);
        Some(TuiEvent::Terminate)
    }
}

/// Start forwarding signals / console events to `tx`.
pub fn install(tx: Sender<TuiEvent>) {
    *CLOSE_TX.lock() = Some(tx.clone());
    if let Err(e) = soyuz_fs::console::set_close_handler(on_console_close) {
        log::warn!("tui: no console close handler: {e}");
    }
    #[cfg(unix)]
    unix::spawn(tx);
    #[cfg(not(unix))]
    drop(tx);
}

/// The frame loop finished handling [`TuiEvent::Terminate`]: release a waiting console
/// close handler.
pub fn notify_saved() {
    *SAVED.lock() = true;
    SAVED_CV.notify_all();
}

/// Console close / logoff / shutdown (Windows; runs on a system thread).
fn on_console_close() {
    let tx = CLOSE_TX.lock().clone();
    let Some(tx) = tx else { return };
    if tx.send(TuiEvent::Terminate).is_err() {
        return; // the loop is gone
    }
    let mut saved = SAVED.lock();
    if !*saved {
        let _ = SAVED_CV.wait_for(&mut saved, CLOSE_BUDGET);
    }
}

#[cfg(unix)]
mod unix {
    use crossbeam_channel::Sender;
    use signal_hook::consts::{SIGCONT, SIGHUP, SIGINT, SIGQUIT, SIGTERM};
    use signal_hook::iterator::Signals;

    use crate::app::TuiEvent;

    pub fn spawn(tx: Sender<TuiEvent>) {
        let mut signals = match Signals::new([SIGTERM, SIGHUP, SIGINT, SIGQUIT, SIGCONT]) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("tui: cannot watch signals: {e}");
                return;
            }
        };
        let spawned = std::thread::Builder::new().name("soyuz-tui-signals".into()).spawn(move || {
            for sig in signals.forever() {
                let Some(ev) = super::event_for(sig, super::suspended()) else {
                    log::debug!("tui: signal {sig} dropped while a child owns the terminal");
                    continue;
                };
                if tx.send(ev).is_err() {
                    break;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("tui: cannot watch signals: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_handler_waits_for_the_save() {
        let (tx, rx) = crossbeam_channel::unbounded();
        *CLOSE_TX.lock() = Some(tx);
        let loop_thread = std::thread::spawn(move || {
            assert!(matches!(rx.recv().unwrap(), TuiEvent::Terminate));
            notify_saved();
        });
        let start = std::time::Instant::now();
        on_console_close();
        assert!(start.elapsed() < CLOSE_BUDGET, "returned once saved");
        loop_thread.join().unwrap();
        *CLOSE_TX.lock() = None;
    }

    #[test]
    fn suspended_flag_round_trips() {
        set_suspended(true);
        assert!(suspended());
        set_suspended(false);
        assert!(!suspended());
    }

    #[cfg(unix)]
    #[test]
    fn signals_while_suspended() {
        use signal_hook::consts::{SIGCONT, SIGHUP, SIGINT, SIGQUIT, SIGTERM};
        for sig in [SIGINT, SIGQUIT, SIGCONT] {
            assert!(event_for(sig, true).is_none(), "{sig} dropped while suspended");
        }
        assert!(matches!(event_for(SIGINT, false), Some(TuiEvent::Terminate)));
        assert!(matches!(event_for(SIGQUIT, false), Some(TuiEvent::Terminate)));
        assert!(matches!(event_for(SIGCONT, false), Some(TuiEvent::Resume)));
        for sig in [SIGTERM, SIGHUP] {
            assert!(matches!(event_for(sig, true), Some(TuiEvent::Terminate)), "{sig} still ends the TUI");
        }
    }
}
