//! Cooperative pause / resume / cancel for long-running operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use parking_lot::{Condvar, Mutex};

use crate::error::{FsError, FsResult};

const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const CANCELLED: u8 = 2;

/// Shared control block for any long operation (`Send + Sync`).
///
/// Workers call [`OpControl::checkpoint`] between items / chunks; it blocks while paused
/// and returns `Err(Interrupted)` once cancelled. Cancel always wins over pause.
#[derive(Debug, Default)]
pub struct OpControl {
    state: AtomicU8,
    lock: Mutex<()>,
    cv: Condvar,
}

impl OpControl {
    pub fn new() -> Arc<Self> {
        Arc::new(OpControl::default())
    }

    pub fn pause(&self) {
        // Never resurrect a cancelled op.
        let _ = self.state.compare_exchange(RUNNING, PAUSED, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        if self.state.compare_exchange(PAUSED, RUNNING, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            self.wake_all();
        }
    }

    /// Cancel; also wakes a paused waiter so it observes the cancellation.
    pub fn cancel(&self) {
        self.state.store(CANCELLED, Ordering::SeqCst);
        self.wake_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::SeqCst) == CANCELLED
    }

    pub fn is_paused(&self) -> bool {
        self.state.load(Ordering::SeqCst) == PAUSED
    }

    /// Blocks while paused; `Err(FsError{Interrupted})` when cancelled (cancel wins over
    /// pause, even while blocked).
    pub fn checkpoint(&self) -> FsResult<()> {
        match self.state.load(Ordering::SeqCst) {
            RUNNING => Ok(()),
            CANCELLED => Err(FsError::interrupted("checkpoint")),
            _ => {
                let mut guard = self.lock.lock();
                loop {
                    match self.state.load(Ordering::SeqCst) {
                        RUNNING => return Ok(()),
                        CANCELLED => return Err(FsError::interrupted("checkpoint")),
                        _ => self.cv.wait(&mut guard),
                    }
                }
            }
        }
    }

    fn wake_all(&self) {
        // Taking the lock orders the state change before a waiter's re-check.
        drop(self.lock.lock());
        self.cv.notify_all();
    }
}

/// Returned by streaming callbacks to continue or stop early.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    Continue,
    Stop,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn running_and_cancel() {
        let c = OpControl::new();
        assert!(c.checkpoint().is_ok());
        c.cancel();
        assert!(c.is_cancelled());
        assert!(c.checkpoint().unwrap_err().is_cancel());
        c.resume();
        c.pause();
        assert!(c.is_cancelled(), "cancel is terminal");
    }

    #[test]
    fn pause_blocks_until_resume() {
        // Behavioural, not timing-based: the waiter must not get past the checkpoint
        // while paused, however late its thread is scheduled.
        use std::sync::atomic::{AtomicBool, Ordering};
        let c = OpControl::new();
        c.pause();
        let passed = std::sync::Arc::new(AtomicBool::new(false));
        let (c2, p2) = (c.clone(), passed.clone());
        let t = std::thread::spawn(move || {
            c2.checkpoint().unwrap();
            p2.store(true, Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(80));
        assert!(!passed.load(Ordering::SeqCst), "checkpoint returned while paused");
        c.resume();
        t.join().unwrap();
        assert!(passed.load(Ordering::SeqCst));
    }

    #[test]
    fn cancel_wakes_paused_waiter() {
        let c = OpControl::new();
        c.pause();
        let c2 = c.clone();
        let t = std::thread::spawn(move || c2.checkpoint());
        std::thread::sleep(Duration::from_millis(30));
        c.cancel();
        assert!(t.join().unwrap().unwrap_err().is_cancel());
    }
}
