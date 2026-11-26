//! Who keeps an item in use: the programs that have a file, a folder, or anything inside
//! a folder open.
//!
//! On Windows a folder cannot be renamed, moved (to the recycle bin, too) or removed while
//! another program has something inside it open, and the system only answers "access is
//! denied". [`blockers`] finds those programs so the refusal can name them, and
//! [`close`] asks one to exit. Elsewhere open files never block these operations, so the
//! list is always empty.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::sysmon::{ControlError, ProcKey};

/// Paths kept per program (the first found).
pub const MAX_PATHS: usize = 3;

/// How far [`blockers`] looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Files and folders examined at most.
    pub entries: usize,
    /// Time spent at most.
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { entries: 20_000, time: Duration::from_millis(1500) }
    }
}

/// A program that has the item, or something inside it, open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocker {
    pub pid: u32,
    /// `None` for a process we may not look at (a system process, another user's): it
    /// cannot be closed from here.
    pub key: Option<ProcKey>,
    /// The program's own description (`Windows Command Processor`), else the name of its
    /// file without the extension, else `PID 1234`.
    pub name: String,
    pub exe: Option<PathBuf>,
    /// This very process.
    pub own: bool,
    /// The first [`MAX_PATHS`] paths it has open.
    pub paths: Vec<PathBuf>,
    /// How many paths it has open, of those examined.
    pub open: usize,
}

impl Blocker {
    /// Whether [`close`] can act on it.
    pub fn closable(&self) -> bool {
        self.key.is_some() && !self.own
    }
}

/// What [`blockers`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blockers {
    /// Most open paths first.
    pub list: Vec<Blocker>,
    /// `false` when a limit was reached before everything was examined.
    pub complete: bool,
}

/// The programs that have `path`, or (for a folder) anything inside it, open. Folder links
/// are not followed. A worker thread's call: it opens every file it examines.
pub fn blockers(path: &Path, limits: Limits) -> Blockers {
    imp::blockers(path, limits)
}

/// Ask a program to exit: by closing its windows (it may ask to save first, or refuse), or
/// with `force` at once, losing its unsaved work.
pub fn close(blocker: &Blocker, force: bool) -> Result<(), ControlError> {
    let Some(key) = blocker.key.filter(|_| !blocker.own) else { return Err(ControlError::Denied) };
    let sig = if force { crate::sysmon::ProcSignal::Kill } else { crate::sysmon::ProcSignal::Terminate };
    crate::sysmon::signal(key, sig)
}

/// Whether the program still runs.
pub fn is_running(blocker: &Blocker) -> bool {
    imp::is_running(blocker)
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use super::{Blocker, Blockers, Limits, MAX_PATHS};
    use crate::sys::windows::{fs as wfs, in_use as sys, sysmon};

    struct Scan {
        limits: Limits,
        started: Instant,
        examined: usize,
        complete: bool,
        own_pid: u32,
        /// In the order found.
        found: Vec<(u32, Vec<PathBuf>, usize)>,
        index: HashMap<u32, usize>,
    }

    impl Scan {
        /// `false`: a limit is reached.
        fn examine(&mut self, path: &Path, dir: bool) -> bool {
            if self.examined >= self.limits.entries || self.started.elapsed() >= self.limits.time {
                self.complete = false;
                return false;
            }
            self.examined += 1;
            for pid in sys::pids_using(path).unwrap_or_default() {
                // Our own handles on folders are watches, which the operations release
                // themselves (`watch::with_released`).
                if dir && pid == self.own_pid {
                    continue;
                }
                let i = *self.index.entry(pid).or_insert_with(|| {
                    self.found.push((pid, Vec::new(), 0));
                    self.found.len() - 1
                });
                let (_, paths, open) = &mut self.found[i];
                *open += 1;
                if paths.len() < MAX_PATHS {
                    paths.push(path.to_path_buf());
                }
            }
            true
        }

        fn walk(&mut self, dir: &Path) -> bool {
            let Ok(rd) = wfs::read_dir(dir) else { return true };
            let mut subdirs = Vec::new();
            for fe in rd.flatten() {
                let p = dir.join(&fe.name);
                let is_dir = fe.stat.is_dir();
                if !self.examine(&p, is_dir) {
                    return false;
                }
                if is_dir && !fe.stat.is_reparse() {
                    subdirs.push(p);
                }
            }
            subdirs.iter().all(|d| self.walk(d))
        }
    }

    pub(super) fn blockers(path: &Path, limits: Limits) -> Blockers {
        let path = crate::path::strip_verbatim(&crate::path::normalize(path));
        let Ok(stat) = wfs::stat_path(&path, false) else { return Blockers { list: Vec::new(), complete: true } };
        let mut scan = Scan {
            limits,
            started: Instant::now(),
            examined: 0,
            complete: true,
            own_pid: std::process::id(),
            found: Vec::new(),
            index: HashMap::new(),
        };
        let is_dir = stat.is_dir();
        if scan.examine(&path, is_dir) && is_dir && !stat.is_reparse() {
            scan.walk(&path);
        }
        let mut list: Vec<Blocker> = scan
            .found
            .into_iter()
            .map(|(pid, paths, open)| {
                let own = pid == scan.own_pid;
                let who = sysmon::identify(pid);
                let (key, exe, description) = match who {
                    Some((key, exe, description)) => (Some(key), exe, description),
                    None => (None, None, None),
                };
                let stem = exe.as_ref().and_then(|e| e.file_stem()).map(|s| s.to_string_lossy().into_owned());
                let name = description
                    .filter(|d| !d.trim().is_empty())
                    .or(stem)
                    .unwrap_or_else(|| if pid == 4 { "System".to_string() } else { format!("PID {pid}") });
                Blocker { pid, key, name, exe, own, paths, open }
            })
            .collect();
        list.sort_by(|a, b| b.open.cmp(&a.open).then_with(|| a.name.cmp(&b.name)));
        Blockers { list, complete: scan.complete }
    }

    pub(super) fn is_running(blocker: &Blocker) -> bool {
        blocker.key.is_some_and(sysmon::is_running)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    use super::{Blocker, Blockers, Limits};

    pub(super) fn blockers(_path: &Path, _limits: Limits) -> Blockers {
        Blockers { list: Vec::new(), complete: true }
    }

    pub(super) fn is_running(_blocker: &Blocker) -> bool {
        false
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::*;

    /// A command prompt that waits on its input, with `dir` as its working folder.
    fn prompt_in(dir: &Path) -> std::process::Child {
        Command::new("cmd.exe")
            .arg("/k")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn wait_for(mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn nothing_open_means_no_blockers() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/a.txt"), b"x").unwrap();
        let b = blockers(d.path(), Limits::default());
        assert_eq!(b, Blockers { list: Vec::new(), complete: true });
        assert!(blockers(&d.path().join("missing"), Limits::default()).list.is_empty());
    }

    #[test]
    fn our_own_open_file_is_named_and_not_closable() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let held = std::fs::File::open(&f).unwrap();
        let b = blockers(d.path(), Limits::default());
        assert_eq!(b.list.len(), 1, "{b:?}");
        let me = &b.list[0];
        assert!(me.own && !me.closable());
        assert_eq!(me.pid, std::process::id());
        assert_eq!((me.open, me.paths.len()), (1, 1));
        assert_eq!(me.paths[0].file_name().unwrap(), "a.txt");
        assert_eq!(close(me, true), Err(ControlError::Denied));
        drop(held);
        assert!(blockers(d.path(), Limits::default()).list.is_empty());
    }

    #[test]
    fn a_program_working_inside_a_folder_is_found_and_ended() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("outer").join("inner");
        std::fs::create_dir_all(&sub).unwrap();
        let mut child = prompt_in(&sub);
        let outer = d.path().join("outer");
        let mut found = Blockers::default();
        assert!(wait_for(|| {
            found = blockers(&outer, Limits::default());
            !found.list.is_empty()
        }));
        assert!(found.complete);
        let b = found.list.iter().find(|b| b.pid == child.id()).expect("the child is listed");
        assert!(b.closable() && !b.own);
        assert!(b.exe.as_ref().is_some_and(|e| e.file_name().is_some_and(|n| n.eq_ignore_ascii_case("cmd.exe"))));
        assert!(!b.name.is_empty() && !b.name.starts_with("PID "));
        assert_eq!(b.paths[0].file_name().unwrap(), "inner");
        assert!(is_running(b));
        // The folder cannot be renamed while the program sits inside it.
        let moved = d.path().join("moved");
        assert!(std::fs::rename(&outer, &moved).is_err());

        close(b, true).unwrap();
        assert!(wait_for(|| !is_running(b)));
        let _ = child.wait();
        assert!(wait_for(|| std::fs::rename(&outer, &moved).is_ok()));
        assert!(blockers(&moved, Limits::default()).list.is_empty());
    }

    #[test]
    fn limits_stop_the_scan() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(d.path().join(format!("{i}.txt")), b"x").unwrap();
        }
        let b = blockers(d.path(), Limits { entries: 5, time: Duration::from_secs(5) });
        assert!(!b.complete && b.list.is_empty());
        let b = blockers(d.path(), Limits { entries: 1000, time: Duration::ZERO });
        assert!(!b.complete);
    }
}
