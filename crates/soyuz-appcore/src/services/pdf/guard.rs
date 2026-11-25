//! In-process guards for engine calls: panics are caught, runaway work is abandoned.
//!
//! hayro has no cancellation hook, so a render past its budget can only be left to
//! finish on its own thread (it keeps burning CPU until then; out-of-process rendering
//! is deferred). Decoder out-of-memory still aborts the process.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use super::PdfError;

/// Stack of engine threads: deeply nested content (forms in forms, patterns) recurses.
pub const STACK: usize = 16 << 20;

/// How often a waiting caller checks `stale`.
const TICK: Duration = Duration::from_millis(25);

/// Run `f` on its own thread ([`STACK`]) and wait for it: [`PdfError::Panicked`] when it
/// panics, [`PdfError::TimedOut`] past `budget` and [`PdfError::Stale`] once `stale()`
/// says the caller no longer wants the result. In the last two cases the thread is
/// abandoned: it finishes in the background and its result is dropped.
pub fn run_guarded<T: Send + 'static>(
    budget: Duration,
    stale: &dyn Fn() -> bool,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, PdfError> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("soyuz-pdf-job".into())
        .stack_size(STACK)
        .spawn(move || {
            let _ = tx.send(catch(f));
        })
        .map_err(|e| PdfError::Other(e.to_string()))?;
    let start = Instant::now();
    loop {
        match rx.recv_timeout(TICK) {
            Ok(r) => return r,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return Err(PdfError::Panicked),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if stale() {
                    return Err(PdfError::Stale);
                }
                if start.elapsed() >= budget {
                    return Err(PdfError::TimedOut);
                }
            }
        }
    }
}

/// `f()` with a panic turned into [`PdfError::Panicked`] (logged).
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, PdfError> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        log::warn!("PDF engine panicked: {msg}");
        PdfError::Panicked
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_panics_timeouts_and_stale() {
        assert_eq!(run_guarded(Duration::from_secs(5), &|| false, || 7), Ok(7));
        assert_eq!(
            run_guarded(Duration::from_secs(5), &|| false, || -> u8 { panic!("boom (expected by a test)") }),
            Err(PdfError::Panicked)
        );
        let t = Instant::now();
        let r = run_guarded(Duration::from_millis(100), &|| false, || std::thread::sleep(Duration::from_secs(2)));
        assert_eq!(r, Err(PdfError::TimedOut));
        assert!(t.elapsed() < Duration::from_secs(1), "abandoned, not joined");
        let r = run_guarded(Duration::from_secs(30), &|| true, || std::thread::sleep(Duration::from_secs(2)));
        assert_eq!(r, Err(PdfError::Stale));
    }

    #[test]
    fn deep_recursion_fits_the_stack() {
        fn depth(n: u32) -> u32 {
            let pad = [n as u8; 512];
            if n == 0 { u32::from(pad[0]) } else { depth(n - 1) + u32::from(pad[n as usize % 512] > 0) }
        }
        assert!(run_guarded(Duration::from_secs(10), &|| false, || depth(8_000)).is_ok());
    }
}
