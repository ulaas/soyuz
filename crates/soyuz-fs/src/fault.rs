//! Fault injection for torture tests (cargo feature `fault-inject`, ROADMAP R2).
//!
//! The copy / move / rename / delete / mkdir internals call [`hit`] at their I/O points:
//! opening a source, creating a destination, every data chunk written, every rename and
//! every delete. With the feature off `hit` is an empty `#[inline(always)]` function, so
//! release builds pay nothing. With the feature on, a test installs a [`FaultPlan`]
//! scoped to its temp directory (parallel tests do not see each other's faults) and the
//! matching operation fails with a real OS error code, so the normal error mapping,
//! cleanup and retry paths run exactly as they would for a real failure.
//!
//! ```ignore
//! let _g = soyuz_fs::fault::install(
//!     FaultPlan::new(tmp.path()).fail(FaultOp::Write, 3, FaultKind::no_space()),
//! );
//! ```

/// An I/O point the fault layer can intercept.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum FaultOp {
    /// Opening an existing file for reading (copy source).
    Open,
    /// Creating a file or directory (copy destination, temp file, `create_dir`).
    Create,
    /// One chunk of file data written by the copy engine.
    Write,
    /// A rename / same-device move (also the final rename of a copy's temp file).
    Rename,
    /// Removing a file, link or (empty) directory.
    Delete,
}

#[cfg(feature = "fault-inject")]
pub use imp::*;

/// Fault hook: `Err` = the injected failure for `op` on `path` (no-op without the feature).
#[cfg(not(feature = "fault-inject"))]
#[inline(always)]
pub(crate) fn hit(_op: FaultOp, _path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(feature = "fault-inject")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    use parking_lot::Mutex;

    use super::FaultOp;

    /// What happens when a rule fires.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum FaultKind {
        /// Fail with this raw OS error code (mapped like a real failure).
        Os(i32),
        /// The path disappears right before the operation (file or whole tree), which then
        /// fails naturally: "vanished between planning and running".
        Vanish,
        /// Sleep before the operation (slow disks, network shares).
        Slow(Duration),
    }

    impl FaultKind {
        /// Disk full (`ERROR_DISK_FULL` / `ENOSPC`).
        pub fn no_space() -> Self {
            FaultKind::Os(if cfg!(windows) { 112 } else { 28 })
        }
        /// Cross-device rename (`ERROR_NOT_SAME_DEVICE` / `EXDEV`).
        pub fn cross_device() -> Self {
            FaultKind::Os(if cfg!(windows) { 17 } else { 18 })
        }
        /// Access denied (`ERROR_ACCESS_DENIED` / `EACCES`).
        pub fn access_denied() -> Self {
            FaultKind::Os(if cfg!(windows) { 5 } else { 13 })
        }
        /// A generic I/O error (`ERROR_CRC` / `EIO`).
        pub fn io_error() -> Self {
            FaultKind::Os(if cfg!(windows) { 23 } else { 5 })
        }
        /// File in use (`ERROR_SHARING_VIOLATION` / `EBUSY`).
        pub fn in_use() -> Self {
            FaultKind::Os(if cfg!(windows) { 32 } else { 16 })
        }
    }

    /// One rule: the `nth` matching operation (1-based) and the `times - 1` after it fire.
    #[derive(Clone, Debug)]
    pub struct FaultRule {
        pub op: FaultOp,
        pub nth: u64,
        pub times: u64,
        /// Only paths whose file name contains this text match.
        pub name_contains: Option<String>,
        pub kind: FaultKind,
    }

    /// A set of rules that only applies to paths under `scope`.
    #[derive(Clone, Debug)]
    pub struct FaultPlan {
        pub scope: PathBuf,
        pub rules: Vec<FaultRule>,
    }

    impl FaultPlan {
        pub fn new(scope: impl Into<PathBuf>) -> Self {
            FaultPlan { scope: scope.into(), rules: Vec::new() }
        }

        /// The `nth` matching `op` fails once with `kind`.
        pub fn fail(self, op: FaultOp, nth: u64, kind: FaultKind) -> Self {
            self.rule(FaultRule { op, nth: nth.max(1), times: 1, name_contains: None, kind })
        }

        /// Every matching `op` from the `nth` on fails with `kind`.
        pub fn fail_from(self, op: FaultOp, nth: u64, kind: FaultKind) -> Self {
            self.rule(FaultRule { op, nth: nth.max(1), times: u64::MAX, name_contains: None, kind })
        }

        /// The first `op` on a path whose name contains `name` gets `kind`.
        pub fn on_name(self, op: FaultOp, name: &str, kind: FaultKind) -> Self {
            self.rule(FaultRule { op, nth: 1, times: 1, name_contains: Some(name.to_string()), kind })
        }

        /// Every `op` sleeps `d` (slow I/O).
        pub fn slow(self, op: FaultOp, d: Duration) -> Self {
            self.rule(FaultRule { op, nth: 1, times: u64::MAX, name_contains: None, kind: FaultKind::Slow(d) })
        }

        pub fn rule(mut self, r: FaultRule) -> Self {
            self.rules.push(r);
            self
        }
    }

    struct Installed {
        id: u64,
        scope: PathBuf,
        rules: Vec<(FaultRule, u64)>,
        fired: Vec<(FaultOp, PathBuf, FaultKind)>,
        seen: u64,
    }

    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    static NEXT: AtomicU64 = AtomicU64::new(1);
    static PLANS: Mutex<Vec<Installed>> = Mutex::new(Vec::new());

    /// Removes its plan when dropped.
    #[must_use = "the plan is removed when the guard is dropped"]
    pub struct FaultGuard {
        id: u64,
    }

    impl FaultGuard {
        /// Faults that fired so far: `(op, path, kind)` in order.
        pub fn fired(&self) -> Vec<(FaultOp, PathBuf, FaultKind)> {
            PLANS.lock().iter().find(|p| p.id == self.id).map(|p| p.fired.clone()).unwrap_or_default()
        }

        /// Number of intercepted operations inside the scope (fired or not).
        pub fn seen(&self) -> u64 {
            PLANS.lock().iter().find(|p| p.id == self.id).map(|p| p.seen).unwrap_or(0)
        }

        /// Stop injecting (the rules stay installed but never fire again).
        pub fn disarm(&self) {
            if let Some(p) = PLANS.lock().iter_mut().find(|p| p.id == self.id) {
                p.rules.clear();
            }
        }
    }

    impl Drop for FaultGuard {
        fn drop(&mut self) {
            let mut plans = PLANS.lock();
            plans.retain(|p| p.id != self.id);
            ACTIVE.store(plans.len(), Ordering::SeqCst);
        }
    }

    /// Install `plan` until the guard is dropped.
    pub fn install(plan: FaultPlan) -> FaultGuard {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut plans = PLANS.lock();
        plans.push(Installed {
            id,
            scope: plan.scope,
            rules: plan.rules.into_iter().map(|r| (r, 0)).collect(),
            fired: Vec::new(),
            seen: 0,
        });
        ACTIVE.store(plans.len(), Ordering::SeqCst);
        FaultGuard { id }
    }

    /// Fault hook used by the fs internals.
    pub(crate) fn hit(op: FaultOp, path: &Path) -> std::io::Result<()> {
        if ACTIVE.load(Ordering::Relaxed) == 0 {
            return Ok(());
        }
        let action = {
            let mut plans = PLANS.lock();
            let mut action = None;
            for p in plans.iter_mut().filter(|p| path.starts_with(&p.scope)) {
                p.seen += 1;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                for (rule, count) in p.rules.iter_mut() {
                    if rule.op != op || rule.name_contains.as_ref().is_some_and(|n| !name.contains(n.as_str())) {
                        continue;
                    }
                    *count += 1;
                    let fires = *count >= rule.nth && (*count - rule.nth) < rule.times;
                    if fires && action.is_none() {
                        action = Some(rule.kind.clone());
                        p.fired.push((op, path.to_path_buf(), rule.kind.clone()));
                    }
                }
            }
            action
        };
        match action {
            None => Ok(()),
            Some(FaultKind::Os(code)) => Err(std::io::Error::from_raw_os_error(code)),
            Some(FaultKind::Slow(d)) => {
                std::thread::sleep(d);
                Ok(())
            }
            Some(FaultKind::Vanish) => {
                let _ = match std::fs::symlink_metadata(path) {
                    Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
                    Ok(_) => std::fs::remove_file(path),
                    Err(e) => Err(e),
                };
                Ok(())
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rules_count_per_scope_and_fire_once() {
            let a = tempfile::tempdir().unwrap();
            let b = tempfile::tempdir().unwrap();
            let g = install(FaultPlan::new(a.path()).fail(FaultOp::Write, 2, FaultKind::no_space()));
            let f = a.path().join("f");
            assert!(hit(FaultOp::Write, &f).is_ok());
            assert!(hit(FaultOp::Write, &b.path().join("x")).is_ok(), "other scope");
            assert!(hit(FaultOp::Rename, &f).is_ok(), "other op");
            let e = hit(FaultOp::Write, &f).unwrap_err();
            assert_eq!(crate::err::io("copy", &f, e).kind, soyuz_core::FsErrorKind::NoSpace);
            assert!(hit(FaultOp::Write, &f).is_ok(), "fires once");
            assert_eq!(g.fired().len(), 1);
            assert_eq!(g.seen(), 4);
            drop(g);
            assert!(hit(FaultOp::Write, &f).is_ok());
        }

        #[test]
        fn vanish_and_name_filter() {
            let d = tempfile::tempdir().unwrap();
            let keep = d.path().join("keep.txt");
            let gone = d.path().join("gone.txt");
            std::fs::write(&keep, b"1").unwrap();
            std::fs::write(&gone, b"1").unwrap();
            let _g = install(FaultPlan::new(d.path()).on_name(FaultOp::Open, "gone", FaultKind::Vanish));
            hit(FaultOp::Open, &keep).unwrap();
            hit(FaultOp::Open, &gone).unwrap();
            assert!(keep.exists() && !gone.exists());
        }
    }
}
