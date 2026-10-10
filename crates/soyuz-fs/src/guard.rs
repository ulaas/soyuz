//! Debug-build guard against filesystem calls on the UI thread.
//!
//! The app calls [`register_ui_thread`] once from its UI thread. In debug builds, every
//! guarded public entry point of this crate then logs a warning, naming the caller's
//! source location, whenever it runs on that thread. Only the outermost guarded call is
//! reported (nested calls inside this crate stay quiet), and each call site only once.
//! Release builds compile the check away.

#[cfg(debug_assertions)]
mod imp {
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::panic::Location;
    use std::sync::OnceLock;
    use std::thread::ThreadId;

    use parking_lot::Mutex;

    static UI_THREAD: OnceLock<ThreadId> = OnceLock::new();
    /// `(file, line, column)` of a reported call site.
    type CallSite = (&'static str, u32, u32);

    static SEEN: Mutex<Option<HashSet<CallSite>>> = Mutex::new(None);

    thread_local! {
        static DEPTH: Cell<u32> = const { Cell::new(0) };
    }

    pub(crate) fn register() {
        let _ = UI_THREAD.set(std::thread::current().id());
    }

    pub(crate) fn on_ui_thread() -> bool {
        UI_THREAD.get().is_some_and(|id| *id == std::thread::current().id())
    }

    pub(crate) struct Scope;

    impl Drop for Scope {
        fn drop(&mut self) {
            DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        }
    }

    #[track_caller]
    pub(crate) fn check(op: &'static str) -> Scope {
        let outer = DEPTH.with(|d| {
            let v = d.get();
            d.set(v + 1);
            v == 0
        });
        if outer && on_ui_thread() {
            let loc = Location::caller();
            let key = (loc.file(), loc.line(), loc.column());
            if SEEN.lock().get_or_insert_with(HashSet::new).insert(key) {
                log::warn!("soyuz-fs: `{op}` called on the UI thread from {}:{}", loc.file(), loc.line());
            }
        }
        Scope
    }
}

/// Mark the calling thread as the UI thread (call once, from the UI thread).
/// A no-op in release builds.
pub fn register_ui_thread() {
    #[cfg(debug_assertions)]
    imp::register();
}

/// True when the calling thread is the registered UI thread (always `false` in release).
pub fn on_ui_thread() -> bool {
    #[cfg(debug_assertions)]
    {
        imp::on_ui_thread()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Held for the duration of a guarded call (marks nested calls as inner).
#[must_use]
pub(crate) struct GuardScope {
    #[cfg(debug_assertions)]
    _inner: imp::Scope,
}

/// Log (once per call site, debug builds only) when `op` runs on the UI thread and is
/// not nested inside another guarded call.
#[track_caller]
#[inline]
pub(crate) fn check(op: &'static str) -> GuardScope {
    #[cfg(debug_assertions)]
    {
        GuardScope { _inner: imp::check(op) }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = op;
        GuardScope {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unregistered_threads_are_not_ui() {
        let t = std::thread::spawn(super::on_ui_thread).join().unwrap();
        assert!(!t);
        let _g = super::check("test");
        let _inner = super::check("nested");
    }
}
