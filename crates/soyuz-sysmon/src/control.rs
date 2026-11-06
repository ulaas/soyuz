//! Process control safety on top of `soyuz_fs::sysmon::signal`: which processes need a stronger
//! warning, which signals are refused, and signalling a whole tree.

use std::collections::HashMap;

use soyuz_fs::sysmon::{ControlError, ProcKey, ProcRaw, ProcSignal};

use crate::frame::{Frame, ProcRow};

/// Names that freeze or crash the session when suspended or ended (matched without `.exe`,
/// ignoring case).
const CRITICAL_NAMES: [&str; 8] = ["csrss", "wininit", "winlogon", "smss", "lsass", "services", "dwm", "fontdrvhost"];

/// A well-known process the session depends on. The confirm dialog warns; Suspend is refused.
pub fn is_critical(raw: &ProcRaw) -> bool {
    if raw.key.pid == 1 {
        return true; // init, systemd
    }
    if raw.kernel {
        return true;
    }
    let name = raw.name.strip_suffix(".exe").or_else(|| raw.name.strip_suffix(".EXE")).unwrap_or(&raw.name);
    CRITICAL_NAMES.iter().any(|c| c.eq_ignore_ascii_case(name))
}

/// How careful a confirm dialog should be, most serious last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProcRisk {
    Normal,
    /// Another user's process (usually `Denied` anyway).
    OtherUser,
    /// A kernel thread, OS-internal pseudo process or well-known critical process.
    Critical,
}

/// The risk of acting on a process; `me` is the current user.
pub fn risk(raw: &ProcRaw, me: Option<&str>) -> ProcRisk {
    if is_critical(raw) {
        ProcRisk::Critical
    } else if let (Some(me), Some(user)) = (me, raw.user.as_deref())
        && me != user
    {
        ProcRisk::OtherUser
    } else {
        ProcRisk::Normal
    }
}

/// Signals refused outright, whatever the user confirms: suspending a critical process freezes
/// the session, and the only way out is a reset.
pub fn refuses(raw: &ProcRaw, sig: ProcSignal) -> bool {
    sig == ProcSignal::Suspend && is_critical(raw)
}

/// Sends a signal to a process, unless [`refuses`] says no (`ControlError::Denied`). The start
/// time check happens in the reader.
pub fn signal_row(row: &ProcRow, sig: ProcSignal) -> Result<(), ControlError> {
    if refuses(&row.raw, sig) {
        return Err(ControlError::Denied);
    }
    soyuz_fs::sysmon::signal(row.raw.key, sig)
}

/// The processes below `key` in the frame and `key` itself, children before parents (the order
/// to end them in). A parent link counts only when the parent is older than the child, since
/// PIDs get reused. Empty when `key` is not in the frame.
pub fn descendants(frame: &Frame, key: ProcKey) -> Vec<ProcKey> {
    let procs: &[ProcRow] = &frame.procs;
    let Some(root) = procs.iter().position(|p| p.raw.key == key) else { return Vec::new() };
    let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, p) in procs.iter().enumerate() {
        if let Some(parent) = p.raw.parent
            && parent != p.raw.key.pid
        {
            children.entry(parent).or_default().push(i);
        }
    }
    let mut seen = vec![false; procs.len()];
    let mut order = Vec::new();
    // Depth first; reversed at the end, every parent lands behind its children.
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        order.push(procs[i].raw.key);
        let me = &procs[i].raw;
        if let Some(kids) = children.get(&me.key.pid) {
            for &c in kids {
                let child = &procs[c].raw;
                let reused = child.key.start_ms > 0 && me.key.start_ms > child.key.start_ms;
                if !reused {
                    stack.push(c);
                }
            }
        }
    }
    order.reverse();
    order
}

/// What happened to each process of a tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeSignalReport {
    pub done: Vec<ProcKey>,
    /// Refused or failed, with the reason. A process that is already gone is not listed.
    pub failed: Vec<(ProcKey, ControlError)>,
}

/// Signals `key` and everything below it, children first, each with its own start time check.
/// Critical processes below `key` are skipped: the user confirmed the root, not them. The root
/// itself follows [`refuses`].
pub fn signal_tree(frame: &Frame, key: ProcKey, sig: ProcSignal) -> TreeSignalReport {
    signal_tree_with(frame, key, sig, &mut soyuz_fs::sysmon::signal)
}

/// [`signal_tree`] with the signal call injected.
pub(crate) fn signal_tree_with(
    frame: &Frame,
    key: ProcKey,
    sig: ProcSignal,
    send: &mut dyn FnMut(ProcKey, ProcSignal) -> Result<(), ControlError>,
) -> TreeSignalReport {
    let mut report = TreeSignalReport::default();
    for k in descendants(frame, key) {
        let Some(row) = frame.proc(k) else { continue };
        if refuses(&row.raw, sig) || (k != key && is_critical(&row.raw)) {
            report.failed.push((k, ControlError::Denied));
            continue;
        }
        match send(k, sig) {
            Ok(()) => report.done.push(k),
            Err(ControlError::Gone) => {}
            Err(e) => report.failed.push((k, e)),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{frame_of, key, row};
    use std::sync::Arc;

    fn named(name: &str) -> ProcRaw {
        ProcRaw { key: key(100, 1), name: Arc::from(name), ..Default::default() }
    }

    #[test]
    fn critical_processes() {
        for n in ["csrss.exe", "CSRSS.EXE", "wininit.exe", "smss.exe", "lsass.exe", "dwm.exe", "dwm"] {
            assert!(is_critical(&named(n)), "{n}");
        }
        for n in ["firefox.exe", "bash", "csrss2.exe", "systemd"] {
            assert!(!is_critical(&named(n)), "{n}");
        }
        let init = ProcRaw { key: key(1, 1), name: Arc::from("systemd"), ..Default::default() };
        assert!(is_critical(&init));
        let kernel = ProcRaw { kernel: true, ..named("kworker/0:1") };
        assert!(is_critical(&kernel));
    }

    #[test]
    fn suspend_is_refused_for_critical_ones_only() {
        let c = named("csrss.exe");
        assert!(refuses(&c, ProcSignal::Suspend));
        assert!(!refuses(&c, ProcSignal::Kill), "the dialog warns, the user decides");
        assert!(!refuses(&named("x"), ProcSignal::Suspend));
    }

    #[test]
    fn risk_levels() {
        let mut p = named("x");
        p.user = Some(Arc::from("bob"));
        assert_eq!(risk(&p, Some("bob")), ProcRisk::Normal);
        assert_eq!(risk(&p, Some("amy")), ProcRisk::OtherUser);
        assert_eq!(risk(&p, None), ProcRisk::Normal);
        p.name = Arc::from("lsass.exe");
        assert_eq!(risk(&p, Some("bob")), ProcRisk::Critical);
        assert!(ProcRisk::Critical > ProcRisk::OtherUser);
    }

    fn tree() -> Frame {
        frame_of(vec![
            row(10, Some(1), "root", 0.0, 0),
            row(11, Some(10), "kid", 0.0, 0),
            row(12, Some(11), "grandkid", 0.0, 0),
            row(13, Some(10), "kid2", 0.0, 0),
            row(20, Some(1), "stranger", 0.0, 0),
        ])
    }

    fn k(f: &Frame, i: usize) -> ProcKey {
        f.procs[i].raw.key
    }

    #[test]
    fn descendants_are_children_first() {
        let f = tree();
        let order = descendants(&f, k(&f, 0));
        assert_eq!(order.len(), 4);
        assert_eq!(*order.last().unwrap(), k(&f, 0));
        let pos = |x| order.iter().position(|o| *o == x).unwrap();
        assert!(pos(k(&f, 2)) < pos(k(&f, 1)));
        assert!(!order.contains(&k(&f, 4)));
        assert!(descendants(&f, key(99, 1)).is_empty());
    }

    #[test]
    fn descendants_ignore_reused_pids_and_survive_cycles() {
        let mut older_child = row(11, Some(10), "older", 0.0, 0);
        older_child.raw.key.start_ms = 5;
        let mut parent = row(10, None, "parent", 0.0, 0);
        parent.raw.key.start_ms = 50;
        let f = frame_of(vec![parent, older_child]);
        assert_eq!(descendants(&f, k(&f, 0)), [k(&f, 0)]);

        let f =
            frame_of(vec![row(1, Some(2), "a", 0.0, 0), row(2, Some(1), "b", 0.0, 0), row(3, Some(3), "c", 0.0, 0)]);
        assert_eq!(descendants(&f, k(&f, 0)).len(), 2);
        assert_eq!(descendants(&f, k(&f, 2)), [k(&f, 2)]);
    }

    #[test]
    fn tree_signal_goes_children_first_with_the_identity_of_each() {
        let f = tree();
        let mut sent = Vec::new();
        let mut send = |key: ProcKey, _| {
            sent.push(key);
            Ok(())
        };
        let report = signal_tree_with(&f, k(&f, 0), ProcSignal::Kill, &mut send);
        assert_eq!(report.done.len(), 4);
        assert!(report.failed.is_empty());
        assert_eq!(*sent.last().unwrap(), k(&f, 0));
        assert!(!sent.contains(&k(&f, 4)));
    }

    #[test]
    fn tree_signal_skips_critical_children_and_reports_failures() {
        let mut rows = vec![row(10, Some(5), "root", 0.0, 0), row(11, Some(10), "lsass.exe", 0.0, 0)];
        rows.push(row(12, Some(10), "locked", 0.0, 0));
        rows.push(row(13, Some(10), "gone", 0.0, 0));
        let f = frame_of(rows);
        let (locked, gone) = (k(&f, 2), k(&f, 3));
        let mut send = |key: ProcKey, _| {
            if key == locked {
                Err(ControlError::Denied)
            } else if key == gone {
                Err(ControlError::Gone)
            } else {
                Ok(())
            }
        };
        let r = signal_tree_with(&f, k(&f, 0), ProcSignal::Kill, &mut send);
        assert_eq!(r.done, [k(&f, 0)]);
        assert_eq!(r.failed.len(), 2, "the critical child and the denied one; a gone one is no failure");
        assert!(r.failed.contains(&(k(&f, 1), ControlError::Denied)));
    }

    #[test]
    fn suspending_a_critical_root_is_refused() {
        let f = frame_of(vec![row(10, Some(1), "csrss.exe", 0.0, 0)]);
        let mut send = |_: ProcKey, _| -> Result<(), ControlError> { panic!("must not be sent") };
        let r = signal_tree_with(&f, k(&f, 0), ProcSignal::Suspend, &mut send);
        assert_eq!(r.failed.len(), 1);
        assert!(signal_row(&f.procs[0], ProcSignal::Suspend).is_err());
    }
}
