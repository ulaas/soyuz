//! Background loaders for what pages read on demand: the list reads block for milliseconds to
//! seconds, so they run on a short-lived worker and hand the result back via `poll`. Refresh
//! rates are the caller's: connections every 3-5 s, process details on selection change and
//! about every 2 s, never per painted frame.

use std::sync::Arc;

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use soyuz_fs::sysmon::{Connection, ControlError, DetailsWant, ProcDetails, ProcKey, ServiceInfo, StartupItem};

use crate::sanitize;

/// The wake callback: called on a worker thread. See `hub` for the rules.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A result on its way from a short-lived worker.
pub struct Loader<T> {
    rx: Receiver<T>,
    finished: bool,
}

impl<T: Send + 'static> Loader<T> {
    /// Runs `work` on its own thread; `wake` is called when the result is ready. Dropping the
    /// loader abandons the result, not the work.
    pub fn spawn(name: &str, wake: Wake, work: impl FnOnce() -> T + Send + 'static) -> Loader<T> {
        let (tx, rx) = bounded(1);
        let spawned = std::thread::Builder::new().name(format!("soyuz-sysmon-{name}")).spawn(move || {
            let value = work();
            if tx.send(value).is_ok() {
                wake();
            }
        });
        if let Err(e) = spawned {
            log::warn!("could not start a monitor loader: {e}");
        }
        Loader { rx, finished: false }
    }

    /// The result, once. `None`: still running (or already taken).
    pub fn poll(&mut self) -> Option<T> {
        if self.finished {
            return None;
        }
        match self.rx.try_recv() {
            Ok(v) => {
                self.finished = true;
                Some(v)
            }
            Err(TryRecvError::Empty) => None,
            // The worker panicked or never started.
            Err(TryRecvError::Disconnected) => {
                self.finished = true;
                None
            }
        }
    }

    /// False once the result was taken (or can never come).
    pub fn is_pending(&self) -> bool {
        !self.finished
    }
}

/// Open sockets, on a worker.
pub fn connections(wake: Wake) -> Loader<Vec<Connection>> {
    Loader::spawn("connections", wake, || {
        let mut list = soyuz_fs::sysmon::connections();
        sanitize::sanitize_connections(&mut list);
        list
    })
}

/// Services, on a worker.
pub fn services(wake: Wake) -> Loader<Vec<ServiceInfo>> {
    Loader::spawn("services", wake, || {
        let mut list = soyuz_fs::sysmon::services();
        sanitize::sanitize_services(&mut list);
        list
    })
}

/// Startup items, on a worker.
pub fn startup_items(wake: Wake) -> Loader<Vec<StartupItem>> {
    Loader::spawn("startup", wake, || {
        let mut list = soyuz_fs::sysmon::startup_items();
        sanitize::sanitize_startup(&mut list);
        list
    })
}

/// Details of one process (only the lists `want` asks for), on a worker.
pub fn process_details(key: ProcKey, want: DetailsWant, wake: Wake) -> Loader<Result<ProcDetails, ControlError>> {
    Loader::spawn("details", wake, move || {
        let mut d = soyuz_fs::sysmon::process_details(key, want)?;
        sanitize::sanitize_details(&mut d);
        Ok(d)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    fn counter() -> (Arc<AtomicU32>, Wake) {
        let n = Arc::new(AtomicU32::new(0));
        let m = n.clone();
        (
            n,
            Arc::new(move || {
                m.fetch_add(1, Ordering::SeqCst);
            }),
        )
    }

    fn wait<T: Send + 'static>(l: &mut Loader<T>) -> Option<T> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) && l.is_pending() {
            if let Some(v) = l.poll() {
                return Some(v);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    #[test]
    fn hands_the_result_back_once_and_wakes() {
        let (n, wake) = counter();
        let mut l = Loader::spawn("test", wake, || 42);
        assert_eq!(wait(&mut l), Some(42));
        assert_eq!(l.poll(), None);
        assert!(!l.is_pending());
        let start = Instant::now();
        while n.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panicking_worker_is_not_pending_forever() {
        let (_, wake) = counter();
        let mut l: Loader<u32> = Loader::spawn("test", wake, || panic!("expected in this test"));
        assert_eq!(wait(&mut l), None);
        assert!(!l.is_pending());
    }

    #[test]
    fn dropping_the_loader_is_harmless() {
        let (n, wake) = counter();
        let gate = Arc::new(parking_lot::Mutex::new(()));
        let held = gate.lock();
        let g = gate.clone();
        let l = Loader::spawn("test", wake, move || {
            drop(g.lock());
            1
        });
        drop(l);
        drop(held);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(n.load(Ordering::SeqCst), 0, "nobody is listening");
    }

    #[test]
    fn real_lists_do_not_crash() {
        let (_, wake) = counter();
        let mut c = connections(wake.clone());
        wait(&mut c);
        let mut s = startup_items(wake);
        wait(&mut s);
    }
}
