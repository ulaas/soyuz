//! The plumbing between the player's threads: a bounded queue ([`Bounded`]) that a
//! producer can be pulled out of (seek, stop) and a [`Signal`] the threads sleep on so a
//! command wakes them at once instead of at their next poll. Pure `std`, no libraries.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Why a `push` returned without queuing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PushError {
    /// [`Bounded::interrupt`] was called (a seek is pending): the producer should look
    /// at its commands; the item is dropped.
    Interrupted,
    /// [`Bounded::close`] was called: no consumer will come.
    Closed,
}

/// What a timed `pop` found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pop<T> {
    Item(T),
    /// Nothing arrived before the timeout.
    Empty,
    /// Closed and drained.
    Closed,
}

struct Inner<T> {
    items: VecDeque<T>,
    closed: bool,
    interrupted: bool,
}

/// A bounded FIFO between one producer and one consumer. `push` blocks while the queue
/// is full; [`interrupt`](Self::interrupt) makes it return early until the producer
/// acknowledges with [`clear_interrupt`](Self::clear_interrupt), [`flush`](Self::flush)
/// drops what is queued, [`close`](Self::close) ends both sides.
pub(crate) struct Bounded<T> {
    inner: Mutex<Inner<T>>,
    not_full: Condvar,
    not_empty: Condvar,
    cap: usize,
}

impl<T> Bounded<T> {
    pub(crate) fn new(cap: usize) -> Bounded<T> {
        Bounded {
            inner: Mutex::new(Inner { items: VecDeque::with_capacity(cap.max(1)), closed: false, interrupted: false }),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
            cap: cap.max(1),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner<T>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `item`, waiting while the queue is full.
    pub(crate) fn push(&self, item: T) -> Result<(), PushError> {
        let mut g = self.lock();
        loop {
            if g.closed {
                return Err(PushError::Closed);
            }
            if g.interrupted {
                return Err(PushError::Interrupted);
            }
            if g.items.len() < self.cap {
                g.items.push_back(item);
                drop(g);
                self.not_empty.notify_one();
                return Ok(());
            }
            g = self.not_full.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// The next item, waiting up to `timeout` for one.
    pub(crate) fn pop_timeout(&self, timeout: Duration) -> Pop<T> {
        let deadline = Instant::now() + timeout;
        let mut g = self.lock();
        loop {
            if let Some(item) = g.items.pop_front() {
                drop(g);
                self.not_full.notify_one();
                return Pop::Item(item);
            }
            if g.closed {
                return Pop::Closed;
            }
            let now = Instant::now();
            if now >= deadline {
                return Pop::Empty;
            }
            g = self.not_empty.wait_timeout(g, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Drop every queued item; returns how many there were.
    pub(crate) fn flush(&self) -> usize {
        let mut g = self.lock();
        let n = g.items.len();
        g.items.clear();
        drop(g);
        self.not_full.notify_all();
        n
    }

    /// Make a blocked (or the next) `push` return [`PushError::Interrupted`].
    pub(crate) fn interrupt(&self) {
        self.lock().interrupted = true;
        self.not_full.notify_all();
    }

    /// The producer saw the interruption; `push` works again.
    pub(crate) fn clear_interrupt(&self) {
        self.lock().interrupted = false;
    }

    /// End the queue: pushes fail, pops drain what is left and then report `Closed`.
    pub(crate) fn close(&self) {
        self.lock().closed = true;
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().items.len()
    }
}

/// A wake-up for sleeping threads: `wait_timeout` returns when `notify` was called
/// after the wait began, or when the time is up. Waiters re-check their own conditions
/// (a notification carries no payload), so a missed one costs at most a timeout.
pub(crate) struct Signal {
    epoch: Mutex<u64>,
    cv: Condvar,
}

impl Signal {
    pub(crate) fn new() -> Signal {
        Signal { epoch: Mutex::new(0), cv: Condvar::new() }
    }

    pub(crate) fn notify(&self) {
        *self.epoch.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.cv.notify_all();
    }

    /// `true` when woken by `notify`, `false` on timeout.
    pub(crate) fn wait_timeout(&self, timeout: Duration) -> bool {
        let g = self.epoch.lock().unwrap_or_else(|e| e.into_inner());
        let start = *g;
        let (g, _) = self.cv.wait_timeout_while(g, timeout, |e| *e == start).unwrap_or_else(|e| e.into_inner());
        *g != start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn fifo_bounded_and_flushable() {
        let q: Bounded<u32> = Bounded::new(2);
        q.push(1).unwrap();
        q.push(2).unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Pop::Item(1));
        q.push(3).unwrap();
        assert_eq!(q.flush(), 2);
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Pop::Empty);
        q.close();
        assert_eq!(q.push(4), Err(PushError::Closed));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Pop::Closed);
        assert!(Bounded::<u8>::new(0).push(1).is_ok(), "a zero capacity still holds one item");
    }

    #[test]
    fn full_push_blocks_until_pop_interrupt_or_close() {
        let q: Arc<Bounded<u32>> = Arc::new(Bounded::new(1));
        q.push(1).unwrap();
        let (q2, started) = (Arc::clone(&q), Arc::new(AtomicBool::new(false)));
        let s2 = Arc::clone(&started);
        let t = std::thread::spawn(move || {
            started.store(true, Ordering::SeqCst);
            q2.push(2)
        });
        while !s2.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(!t.is_finished(), "blocked while full");
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Pop::Item(1));
        assert_eq!(t.join().unwrap(), Ok(()));
        assert_eq!(q.len(), 1);
        // An interrupt frees a blocked producer and sticks until cleared.
        let q2 = Arc::clone(&q);
        let t = std::thread::spawn(move || q2.push(3));
        std::thread::sleep(Duration::from_millis(20));
        q.interrupt();
        assert_eq!(t.join().unwrap(), Err(PushError::Interrupted));
        assert_eq!(q.pop_timeout(Duration::ZERO), Pop::Item(2));
        assert_eq!(q.push(4), Err(PushError::Interrupted), "still interrupted");
        q.clear_interrupt();
        q.push(4).unwrap();
        // Close frees a blocked producer too, and the consumer drains before `Closed`.
        let q2 = Arc::clone(&q);
        let t = std::thread::spawn(move || q2.push(5));
        std::thread::sleep(Duration::from_millis(20));
        q.close();
        assert_eq!(t.join().unwrap(), Err(PushError::Closed));
        assert_eq!(q.pop_timeout(Duration::ZERO), Pop::Item(4));
        assert_eq!(q.pop_timeout(Duration::ZERO), Pop::Closed);
    }

    #[test]
    fn pop_waits_for_a_push() {
        let q: Arc<Bounded<u32>> = Arc::new(Bounded::new(4));
        let q2 = Arc::clone(&q);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            q2.push(7).unwrap();
        });
        let started = Instant::now();
        assert_eq!(q.pop_timeout(Duration::from_secs(5)), Pop::Item(7));
        assert!(started.elapsed() < Duration::from_secs(4), "woke on the push, not the timeout");
        t.join().unwrap();
    }

    #[test]
    fn signal_wakes_or_times_out() {
        let s = Arc::new(Signal::new());
        let started = Instant::now();
        assert!(!s.wait_timeout(Duration::from_millis(10)));
        assert!(started.elapsed() >= Duration::from_millis(10));
        let s2 = Arc::clone(&s);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            s2.notify();
        });
        assert!(s.wait_timeout(Duration::from_secs(5)));
        t.join().unwrap();
    }
}
