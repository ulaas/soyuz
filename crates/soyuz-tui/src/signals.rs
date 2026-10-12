//! Process signals and console events for the terminal UI.
//!
//! * unix: `SIGTERM`, `SIGHUP` (the terminal went away) and `SIGINT` (sent by `kill`;
//!   in raw mode Ctrl+C is a key, Copy) become [`TuiEvent::Terminate`]: the loop saves
//!   everything and quits. `SIGCONT` becomes [`TuiEvent::Resume`]: the loop re-enters raw
//!   mode and the alternate screen and repaints (the shell resets the terminal while the
//!   process is stopped). In raw mode Ctrl+Z is a key too (Undo), so the TUI is only ever
//!   stopped from outside (`kill -STOP`).
//! * Windows: closing the console window (or logoff / shutdown) runs a handler on a
//!   system thread. It sends [`TuiEvent::Terminate`] and waits (up to [`CLOSE_BUDGET`])
//!   until the frame loop has saved ([`notify_saved`]); Windows then ends the process.

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
    use signal_hook::consts::{SIGCONT, SIGHUP, SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;

    use crate::app::TuiEvent;

    pub fn spawn(tx: Sender<TuiEvent>) {
        let mut signals = match Signals::new([SIGTERM, SIGHUP, SIGINT, SIGCONT]) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("tui: cannot watch signals: {e}");
                return;
            }
        };
        let spawned = std::thread::Builder::new().name("soyuz-tui-signals".into()).spawn(move || {
            for sig in signals.forever() {
                let ev = if sig == SIGCONT { TuiEvent::Resume } else { TuiEvent::Terminate };
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
}
