//! Directory watching: a thin `notify-debouncer-full` wrapper (one notify backend per
//! [`Watcher`], N watched directories). notify itself uses ReadDirectoryChangesW /
//! inotify / FSEvents.
//!
//! The debouncer runs without a file-id cache ([`NoCache`]). Its default cache
//! (`FileIdMap`) opens every created file and scans its whole map on every removal, on
//! notify's single event thread, and walks each watched directory on `watch` (seconds for
//! big folders). Under a burst of changes that thread fell behind, the kernel's
//! ReadDirectoryChangesW buffer overflowed and notify (8.x) dropped the overflowed batch
//! without any rescan signal (or, for `ERROR_NOTIFY_ENUM_DIR`, dropped the watch
//! entirely): new files silently never showed up. Rename pairs are still matched (the
//! debouncer pairs a rename-from with the following rename-to).
//!
//! A liveness thread also guards against watches that stop reporting for any reason
//! (see [`spawn_liveness`]): a directory whose modification time changes while its
//! watch stays silent is re-armed and reported as [`WatchKind::Rescan`].
//!
//! When the kernel watch limit is reached (inotify `ENOSPC`, i.e.
//! `fs.inotify.max_user_watches`, or `EMFILE` for `max_user_instances`), watches fall back
//! to a polling backend (notify's `PollWatcher`, every [`Watcher::set_poll_interval`])
//! instead of failing: changes still arrive, only later. [`Watcher::degraded`] and
//! [`Watcher::is_polled`] expose that state so the UI can say why updates are slow.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use notify::RecursiveMode;
use notify::event::{EventKind, ModifyKind, RemoveKind};
use notify_debouncer_full::{DebounceEventResult, Debouncer, NoCache, new_debouncer_opt};
use parking_lot::Mutex;
use soyuz_core::{FsError, FsErrorKind, FsResult};

/// Maximum number of paths in one [`WatchKind::Changed`]; more → [`WatchKind::Rescan`].
pub const MAX_CHANGED_PATHS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WatchId(pub u64);

#[derive(Clone, Debug)]
pub struct WatchEvent {
    pub id: WatchId,
    pub kind: WatchKind,
}

#[derive(Clone, Debug)]
pub enum WatchKind {
    /// Changed paths (≤ 64), deduplicated, expressed under the watched directory as given.
    Changed(Vec<PathBuf>),
    /// Too many events, backend overflow, or a watch that went silent: relist.
    Rescan,
    /// The watched directory was removed / renamed away / unmounted.
    Lost,
}

#[derive(Debug)]
struct Watched {
    dir: PathBuf,
    /// Canonical form of `dir` when it differs (FSEvents reports `/private/var/...` for
    /// `/var/...`); reported paths under it are mapped back onto `dir`.
    canon: Option<PathBuf>,
    recursive: bool,
    /// Served by the polling backend (the liveness check only tracks existence).
    polled: bool,
    /// `Lost` was already reported (by an event or the liveness check).
    lost: bool,
    /// Raw notifications seen for this watch (any path inside it, or the directory
    /// itself); the liveness check compares it across directory changes.
    activity: u64,
}

impl Watched {
    fn new(dir: PathBuf, canon: Option<PathBuf>, recursive: bool) -> Self {
        Watched { dir, canon, recursive, polled: false, lost: false, activity: 0 }
    }

    /// `p` expressed under `dir` (mapping a canonical prefix back), if it is inside.
    fn local(&self, p: &Path) -> Option<PathBuf> {
        if p.starts_with(&self.dir) {
            return Some(p.to_path_buf());
        }
        let c = self.canon.as_ref()?;
        p.strip_prefix(c)
            .ok()
            .map(|rest| if rest.as_os_str().is_empty() { self.dir.clone() } else { self.dir.join(rest) })
    }

    fn gone(&self) -> bool {
        !crate::list::exists_nofollow(&self.dir)
    }
}

type Registry = Arc<Mutex<HashMap<WatchId, Watched>>>;
type Backend = Debouncer<notify::RecommendedWatcher, NoCache>;
type PollBackend = Debouncer<notify::PollWatcher, NoCache>;

/// Default scan interval of polled watches (after the kernel watch limit was reached).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Owns one notify backend (plus a polling backend created on demand); `Send`.
pub struct Watcher {
    /// Native backend (`None` when it could not even start: inotify instance limit).
    /// Shared with the liveness thread, which re-arms silent watches.
    debouncer: Option<Arc<Mutex<Backend>>>,
    /// Polling backend for watches the native one refused for lack of resources.
    poller: Option<Arc<Mutex<PollBackend>>>,
    registry: Registry,
    next_id: u64,
    tx: crossbeam_channel::Sender<WatchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    debounce: Duration,
    poll_interval: Duration,
    /// Why some watches are polled (first resource error), if any are.
    degraded: Option<String>,
    /// Test hook: every new watch is polled.
    #[cfg(test)]
    force_poll: bool,
    /// Test hook: the native watch succeeds, then reports ENOSPC (a partial recursive add).
    #[cfg(test)]
    fake_limit: bool,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").field("watched", &self.registry.lock().len()).finish()
    }
}

impl Watcher {
    /// Events are sent on `tx`, followed by a call to `wake` (e.g. `ctx.request_repaint`).
    /// `wake` is called without any internal lock held.
    pub fn new(
        tx: crossbeam_channel::Sender<WatchEvent>,
        debounce: Duration,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> FsResult<Self> {
        let registry: Registry = Arc::default();
        let handler = event_handler(registry.clone(), tx.clone(), wake.clone());
        let mut degraded = None;
        let debouncer = match new_debouncer_opt(debounce, None, handler, NoCache::new(), notify::Config::default()) {
            Ok(d) => Some(Arc::new(Mutex::new(d))),
            Err(e) if is_resource_limit(&e) => {
                log::warn!("native file watching unavailable ({e}); polling instead");
                degraded = Some(limit_message(&e));
                None
            }
            Err(e) => return Err(notify_err("watch", None, e)),
        };
        spawn_liveness(Liveness {
            reg: Arc::downgrade(&registry),
            backend: debouncer.as_ref().map(Arc::downgrade).unwrap_or_default(),
            tx: tx.clone(),
            wake: wake.clone(),
            grace: stale_grace(debounce),
        });
        Ok(Watcher {
            debouncer,
            poller: None,
            registry,
            next_id: 1,
            tx,
            wake,
            debounce,
            poll_interval: DEFAULT_POLL_INTERVAL,
            degraded,
            #[cfg(test)]
            force_poll: false,
            #[cfg(test)]
            fake_limit: false,
        })
    }

    /// Scan interval of polled watches; applies to the polling backend created after this
    /// call (it is created when the first watch has to be polled).
    pub fn set_poll_interval(&mut self, interval: Duration) {
        self.poll_interval = interval.max(Duration::from_millis(250));
    }

    /// Why some watches are polled instead of pushed by the OS (`None` = none are), e.g.
    /// "the system limit of watched folders is reached (fs.inotify.max_user_watches)".
    pub fn degraded(&self) -> Option<&str> {
        self.degraded.as_deref()
    }

    /// Is the watch `id` served by polling (changes show up after the poll interval)?
    pub fn is_polled(&self, id: WatchId) -> bool {
        self.registry.lock().get(&id).is_some_and(|w| w.polled)
    }

    pub fn watch(&mut self, dir: &Path, recursive: bool) -> FsResult<WatchId> {
        if !crate::list::exists_nofollow(dir) {
            return Err(FsError::with_message(
                FsErrorKind::NotFound,
                "watch",
                Some(dir),
                "the directory does not exist",
            ));
        }
        let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        #[cfg(test)]
        let force_poll = self.force_poll;
        #[cfg(not(test))]
        let force_poll = false;
        #[cfg(test)]
        let fake_limit = self.fake_limit;
        #[cfg(not(test))]
        let fake_limit = false;
        let native = match &self.debouncer {
            Some(d) if !force_poll => {
                // Bind first: a guard in the `match` scrutinee would live through the arms.
                let res = d.lock().watch(dir, mode);
                let res = res.and_then(|()| {
                    if fake_limit { Err(notify::Error::new(notify::ErrorKind::MaxFilesWatch)) } else { Ok(()) }
                });
                match res {
                    Ok(()) => true,
                    Err(e) if is_resource_limit(&e) => {
                        log::warn!("cannot watch {} natively ({e}); polling it", dir.display());
                        // A recursive add that hit the limit half-way keeps the native watches it
                        // did add; drop them (unless another native watch of `dir` uses them).
                        let shared = self.registry.lock().values().any(|w| w.dir == dir && !w.polled);
                        if !shared {
                            let _ = d.lock().unwatch(dir);
                        }
                        if self.degraded.is_none() {
                            self.degraded = Some(limit_message(&e));
                        }
                        false
                    }
                    Err(e) => return Err(notify_err("watch", Some(dir), e)),
                }
            }
            _ => false,
        };
        if !native {
            self.poll_backend()?.lock().watch(dir, mode).map_err(|e| notify_err("watch", Some(dir), e))?;
        }
        let id = WatchId(self.next_id);
        self.next_id += 1;
        let canon =
            std::fs::canonicalize(dir).ok().map(|c| crate::path::strip_verbatim(&c)).filter(|c| c.as_path() != dir);
        let mut w = Watched::new(dir.to_path_buf(), canon, recursive);
        w.polled = !native;
        self.registry.lock().insert(id, w);
        Ok(id)
    }

    /// The polling backend, created on first use.
    fn poll_backend(&mut self) -> FsResult<&Arc<Mutex<PollBackend>>> {
        if self.poller.is_none() {
            let handler = event_handler(self.registry.clone(), self.tx.clone(), self.wake.clone());
            let config = notify::Config::default().with_poll_interval(self.poll_interval);
            let d = new_debouncer_opt(self.debounce, None, handler, NoCache::new(), config)
                .map_err(|e| notify_err("watch", None, e))?;
            self.poller = Some(Arc::new(Mutex::new(d)));
        }
        Ok(self.poller.as_ref().expect("just created"))
    }

    pub fn unwatch(&mut self, id: WatchId) {
        let removed = self.registry.lock().remove(&id);
        if let Some(w) = removed {
            let still_used = self.registry.lock().values().any(|o| o.dir == w.dir && o.polled == w.polled);
            if !still_used {
                if w.polled {
                    if let Some(p) = &self.poller {
                        let _ = p.lock().unwatch(&w.dir);
                    }
                } else if let Some(d) = &self.debouncer {
                    let _ = d.lock().unwatch(&w.dir);
                }
            }
        }
    }
}

/// The debouncer callback shared by the native and the polling backend.
fn event_handler(
    reg: Registry,
    tx: crossbeam_channel::Sender<WatchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> impl FnMut(DebounceEventResult) + Send + 'static {
    move |res: DebounceEventResult| {
        let events = dispatch(&reg, res);
        if events.is_empty() {
            return;
        }
        for ev in events {
            if tx.send(ev).is_err() {
                return;
            }
        }
        wake();
    }
}

/// The OS ran out of watch resources (inotify watch / instance limits).
fn is_resource_limit(e: &notify::Error) -> bool {
    match &e.kind {
        notify::ErrorKind::MaxFilesWatch => true,
        #[cfg(unix)]
        notify::ErrorKind::Io(io) => matches!(io.raw_os_error(), Some(libc::ENOSPC | libc::EMFILE)),
        _ => false,
    }
}

fn limit_message(e: &notify::Error) -> String {
    match &e.kind {
        #[cfg(unix)]
        notify::ErrorKind::Io(io) if io.raw_os_error() == Some(libc::EMFILE) => {
            "the system limit of file watchers is reached (fs.inotify.max_user_instances); folders are polled".into()
        }
        _ => "the system limit of watched folders is reached (fs.inotify.max_user_watches); folders are polled".into(),
    }
}

/// How long a watched directory may show a new modification time without any
/// notification before its watch counts as silent (debounce plus slack).
fn stale_grace(debounce: Duration) -> Duration {
    (debounce * 2 + Duration::from_millis(500)).max(Duration::from_millis(1500))
}

/// What the liveness thread needs (weak handles: it exits when the [`Watcher`] drops).
struct Liveness {
    reg: Weak<Mutex<HashMap<WatchId, Watched>>>,
    backend: Weak<Mutex<Backend>>,
    tx: crossbeam_channel::Sender<WatchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    grace: Duration,
}

/// Per-watch state of the liveness check.
#[derive(Debug)]
struct Health {
    /// Next check due and the current interval.
    due: Instant,
    interval: Duration,
    /// Directory modification time at the previous check (`None` before the first one or
    /// when it cannot be read).
    mtime: Option<SystemTime>,
    /// [`Watched::activity`] at the previous check.
    activity: u64,
    /// The directory changed without notifications: `(activity at that point, detected
    /// at)`. Resolved by any notification, or a silent watch after the grace period.
    suspect: Option<(u64, Instant)>,
}

/// Outcome of one liveness check of a present directory.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Fine,
    /// The directory changed but its watch stayed silent past the grace period.
    Silent,
}

impl Health {
    fn new(now: Instant, interval: Duration) -> Self {
        Health { due: now, interval, mtime: None, activity: 0, suspect: None }
    }

    /// Fold one observation in: the directory's modification time and its watch's
    /// activity counter.
    fn observe(&mut self, mtime: Option<SystemTime>, activity: u64, now: Instant, grace: Duration) -> Verdict {
        let changed = mtime.is_some() && self.mtime.is_some() && mtime != self.mtime;
        let mut verdict = Verdict::Fine;
        if let Some((before, since)) = self.suspect {
            if activity != before {
                self.suspect = None;
            } else if now.duration_since(since) >= grace {
                self.suspect = None;
                verdict = Verdict::Silent;
            }
        }
        // Notifications since the previous check account for a change seen now; without
        // any, they may still sit in the debouncer, so only suspect the watch for now.
        if verdict == Verdict::Fine && changed && activity == self.activity && self.suspect.is_none() {
            self.suspect = Some((activity, now));
        }
        if mtime.is_some() {
            self.mtime = mtime;
        }
        self.activity = activity;
        verdict
    }
}

/// Liveness thread, checking every watched directory about once a second:
/// * Some backends (ReadDirectoryChangesW keeps a removed directory "delete pending"
///   while its handle is open) never report the removal of the watched directory itself;
///   a light existence check reports `Lost` for them.
/// * A directory whose modification time changed while its watch reported nothing for a
///   grace period has a silent watch (dropped by the backend after an error, an overflow
///   that lost the batch, a redirector that stopped notifying): it is re-armed and
///   reported as `Rescan`, so its owner relists.
///
/// The registry is only snapshotted under its lock; the stats run outside it, so a dead
/// network share (a stat can block for tens of seconds) never stalls event dispatch or
/// `watch`/`unwatch` on the UI thread. Network paths and directories whose stat was slow
/// are checked less often (exponential back-off up to [`LIVENESS_MAX_INTERVAL`]).
fn spawn_liveness(l: Liveness) {
    let spawned = std::thread::Builder::new().name("soyuz-watch-alive".into()).spawn(move || {
        let mut health: HashMap<WatchId, Health> = HashMap::new();
        loop {
            std::thread::sleep(LIVENESS_INTERVAL);
            let snapshot: Vec<(WatchId, PathBuf, bool, u64, bool)> = {
                let Some(reg) = l.reg.upgrade() else { return };
                let reg = reg.lock();
                health.retain(|id, _| reg.contains_key(id));
                reg.iter()
                    .filter(|(_, w)| !w.lost)
                    .map(|(id, w)| (*id, w.dir.clone(), w.recursive, w.activity, w.polled))
                    .collect()
            };
            let mut gone = Vec::new();
            let mut silent: Vec<(PathBuf, bool)> = Vec::new();
            for (id, dir, recursive, activity, polled) in snapshot {
                let now = Instant::now();
                let h = health.entry(id).or_insert_with(|| {
                    Health::new(now, if is_network_path(&dir) { LIVENESS_REMOTE_INTERVAL } else { LIVENESS_INTERVAL })
                });
                if now < h.due {
                    continue;
                }
                let t = Instant::now();
                let probe = probe_dir(&dir);
                if t.elapsed() > LIVENESS_SLOW_STAT {
                    h.interval = (h.interval * 2).min(LIVENESS_MAX_INTERVAL);
                }
                h.due = Instant::now() + h.interval;
                match probe {
                    None => gone.push(id),
                    // A polled watch reports changes only once per poll interval: never
                    // "silent" by the native backend's grace period.
                    Some(_) if polled => {}
                    Some(mtime) => {
                        if h.observe(mtime, activity, Instant::now(), l.grace) == Verdict::Silent
                            && !silent.iter().any(|(d, _)| *d == dir)
                        {
                            silent.push((dir, recursive));
                        }
                    }
                }
            }
            if gone.is_empty() && silent.is_empty() {
                continue;
            }
            // Re-arm silent watches, outside the registry lock (unwatch / watch wait for
            // notify's thread, whose event callback takes the registry lock).
            if let Some(backend) = l.backend.upgrade() {
                for (dir, recursive) in &silent {
                    log::warn!("watch on {} went silent; re-arming it", dir.display());
                    let mode = if *recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
                    let mut b = backend.lock();
                    let _ = b.unwatch(dir);
                    if let Err(e) = b.watch(dir, mode) {
                        log::warn!("cannot re-arm the watch on {}: {e}", dir.display());
                    }
                }
            }
            let Some(reg) = l.reg.upgrade() else { return };
            let mut out = Vec::new();
            {
                let mut reg = reg.lock();
                for id in gone {
                    // Re-check under the lock: the watch may have been removed meanwhile.
                    if let Some(w) = reg.get_mut(&id)
                        && !w.lost
                    {
                        w.lost = true;
                        out.push(WatchEvent { id, kind: WatchKind::Lost });
                    }
                }
                for (id, w) in reg.iter() {
                    if !w.lost && silent.iter().any(|(d, _)| *d == w.dir) {
                        out.push(WatchEvent { id: *id, kind: WatchKind::Rescan });
                    }
                }
            }
            drop(reg);
            if out.is_empty() {
                continue;
            }
            for ev in out {
                if l.tx.send(ev).is_err() {
                    return;
                }
            }
            (l.wake)();
        }
    });
    if spawned.is_err() {
        log::warn!("could not start the watcher liveness thread");
    }
}

/// `None` when nothing exists at `dir` any more, else its modification time (`None`
/// inside when it cannot be read, e.g. access denied: then only existence is tracked).
fn probe_dir(dir: &Path) -> Option<Option<SystemTime>> {
    match crate::list::stat(dir, false) {
        Ok(e) => Some(e.modified),
        Err(_) if crate::list::exists_nofollow(dir) => Some(None),
        Err(_) => None,
    }
}

const LIVENESS_INTERVAL: Duration = Duration::from_millis(1000);
/// Initial liveness interval for network paths.
const LIVENESS_REMOTE_INTERVAL: Duration = Duration::from_secs(5);
/// Back-off ceiling for slow / unreachable volumes.
const LIVENESS_MAX_INTERVAL: Duration = Duration::from_secs(60);
/// A stat slower than this doubles that watch's interval.
const LIVENESS_SLOW_STAT: Duration = Duration::from_millis(200);

/// UNC / network-looking path (`\\server\share`, `//server/share`); cheap, lexical.
fn is_network_path(p: &Path) -> bool {
    let s = crate::path::strip_verbatim(p);
    let s = s.to_string_lossy();
    s.starts_with("\\\\") || s.starts_with("//")
}

fn notify_err(op: &'static str, path: Option<&Path>, e: notify::Error) -> FsError {
    let kind = match &e.kind {
        notify::ErrorKind::PathNotFound => FsErrorKind::NotFound,
        notify::ErrorKind::Io(io) => return FsError::from_io(op, path, io),
        notify::ErrorKind::MaxFilesWatch => FsErrorKind::NoSpace,
        _ => FsErrorKind::Io,
    };
    let mut fe = FsError::with_message(kind, op, path, e.to_string());
    if kind == FsErrorKind::NoSpace {
        fe.message = "the system limit of watched directories is reached (raise fs.inotify.max_user_watches)".into();
    }
    fe
}

/// Map debounced notify events onto per-watch events.
fn dispatch(reg: &Registry, res: DebounceEventResult) -> Vec<WatchEvent> {
    let mut reg = reg.lock();
    let events = match res {
        Ok(ev) => ev,
        Err(errors) => {
            log::warn!("watcher errors: {errors:?}");
            let mut out = Vec::new();
            for (id, w) in reg.iter_mut() {
                w.activity += 1;
                if w.gone() {
                    if !w.lost {
                        w.lost = true;
                        out.push(WatchEvent { id: *id, kind: WatchKind::Lost });
                    }
                } else {
                    out.push(WatchEvent { id: *id, kind: WatchKind::Rescan });
                }
            }
            return out;
        }
    };
    let mut changed: HashMap<WatchId, Vec<PathBuf>> = HashMap::new();
    let mut rescan: Vec<WatchId> = Vec::new();
    let mut lost: Vec<WatchId> = Vec::new();
    for ev in &events {
        if ev.need_rescan() {
            for (id, w) in reg.iter_mut() {
                w.activity += 1;
                rescan.push(*id);
            }
            continue;
        }
        for p in &ev.paths {
            for (id, w) in reg.iter_mut() {
                let Some(local) = w.local(p) else {
                    // An ancestor of the watched directory was removed or renamed away.
                    if w.dir.starts_with(p) && w.gone() && !lost.contains(id) {
                        lost.push(*id);
                    }
                    continue;
                };
                w.activity += 1;
                if local == w.dir {
                    let removed = matches!(
                        ev.kind,
                        EventKind::Remove(RemoveKind::Folder | RemoveKind::Any)
                            | EventKind::Modify(ModifyKind::Name(_))
                    );
                    if removed && w.gone() && !lost.contains(id) {
                        lost.push(*id);
                    }
                    continue;
                }
                let inside = w.recursive || local.parent() == Some(w.dir.as_path());
                if inside {
                    let v = changed.entry(*id).or_default();
                    if !v.contains(&local) {
                        v.push(local);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for id in &lost {
        if let Some(w) = reg.get_mut(id)
            && !w.lost
        {
            w.lost = true;
            out.push(WatchEvent { id: *id, kind: WatchKind::Lost });
        }
    }
    rescan.sort_by_key(|id| id.0);
    rescan.dedup();
    for id in &rescan {
        if !lost.contains(id) {
            out.push(WatchEvent { id: *id, kind: WatchKind::Rescan });
        }
    }
    for (id, paths) in changed {
        if lost.contains(&id) || rescan.contains(&id) {
            continue;
        }
        let kind = if paths.len() > MAX_CHANGED_PATHS { WatchKind::Rescan } else { WatchKind::Changed(paths) };
        out.push(WatchEvent { id, kind });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_paths_are_detected_lexically() {
        assert!(is_network_path(Path::new(r"\\server\share\dir")));
        assert!(is_network_path(Path::new(r"\\?\UNC\server\share")));
        assert!(is_network_path(Path::new("//server/share")));
        assert!(!is_network_path(Path::new(r"C:\data")));
        assert!(!is_network_path(Path::new("/home/u")));
    }

    fn watcher() -> (Watcher, crossbeam_channel::Receiver<WatchEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (Watcher::new(tx, Duration::from_millis(50), Arc::new(|| {})).unwrap(), rx)
    }

    fn collect(rx: &crossbeam_channel::Receiver<WatchEvent>, until: impl Fn(&[WatchEvent]) -> bool) -> Vec<WatchEvent> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while std::time::Instant::now() < deadline && !until(&got) {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
                got.push(ev);
            }
        }
        got
    }

    fn mentions(evs: &[WatchEvent], p: &Path) -> bool {
        evs.iter().any(|e| match &e.kind {
            WatchKind::Changed(v) => v.iter().any(|x| x.file_name() == p.file_name()),
            WatchKind::Rescan => true,
            WatchKind::Lost => false,
        })
    }

    #[test]
    fn create_delete_and_lost() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("watched");
        std::fs::create_dir(&dir).unwrap();
        let (mut w, rx) = watcher();
        let id = w.watch(&dir, false).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let f = dir.join("a.txt");
        std::fs::write(&f, b"1").unwrap();
        let evs = collect(&rx, |e| mentions(e, &f));
        assert!(mentions(&evs, &f), "create not reported: {evs:?}");
        assert!(evs.iter().all(|e| e.id == id));
        if let Some(WatchKind::Changed(v)) = evs.iter().map(|e| &e.kind).find(|k| matches!(k, WatchKind::Changed(_))) {
            assert!(v.iter().all(|p| p.starts_with(&dir)), "paths are under the watched dir as given: {v:?}");
        }
        std::fs::remove_file(&f).unwrap();
        let evs = collect(&rx, |e| mentions(e, &f));
        assert!(mentions(&evs, &f), "delete not reported: {evs:?}");
        // Removing the watched directory itself → Lost (backends that only report an
        // error get a Lost/Rescan decided by an existence check).
        std::fs::remove_dir(&dir).unwrap();
        let evs = collect(&rx, |e| e.iter().any(|x| matches!(x.kind, WatchKind::Lost)));
        assert!(
            evs.iter().any(|x| matches!(x.kind, WatchKind::Lost | WatchKind::Rescan)),
            "removal of the watched dir not reported: {evs:?}"
        );
        w.unwatch(id);
    }

    #[test]
    fn canonical_paths_map_back() {
        let w = Watched::new(PathBuf::from("/var/x"), Some(PathBuf::from("/private/var/x")), false);
        assert_eq!(w.local(Path::new("/private/var/x/f")), Some(PathBuf::from("/var/x/f")));
        assert_eq!(w.local(Path::new("/private/var/x")), Some(PathBuf::from("/var/x")));
        assert_eq!(w.local(Path::new("/var/x/g")), Some(PathBuf::from("/var/x/g")));
        assert_eq!(w.local(Path::new("/elsewhere")), None);
    }

    #[test]
    fn health_flags_a_change_without_notifications_after_the_grace_period() {
        let grace = Duration::from_millis(1500);
        let t0 = Instant::now();
        let m0 = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let m1 = m0 + Duration::from_secs(1);
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut h = Health::new(t0, LIVENESS_INTERVAL);
        assert_eq!(h.observe(Some(m0), 0, at(0), grace), Verdict::Fine);
        // Changed, no notification yet: only suspected (it may still be debounced).
        assert_eq!(h.observe(Some(m1), 0, at(1000), grace), Verdict::Fine);
        assert!(h.suspect.is_some());
        assert_eq!(h.observe(Some(m1), 0, at(2000), grace), Verdict::Fine);
        // Still silent after the grace period → silent watch, reported once.
        assert_eq!(h.observe(Some(m1), 0, at(2600), grace), Verdict::Silent);
        assert_eq!(h.observe(Some(m1), 0, at(3600), grace), Verdict::Fine);
    }

    #[test]
    fn health_accepts_changes_that_were_notified() {
        let grace = Duration::from_millis(1500);
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let m = |s: u64| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(s));
        let mut h = Health::new(t0, LIVENESS_INTERVAL);
        h.observe(m(1), 0, at(0), grace);
        // Notifications arrived before this check: fine at once.
        assert_eq!(h.observe(m(2), 3, at(1000), grace), Verdict::Fine);
        assert!(h.suspect.is_none());
        // Notification arrives after the change was seen: suspicion cleared.
        assert_eq!(h.observe(m(3), 3, at(2000), grace), Verdict::Fine);
        assert!(h.suspect.is_some());
        assert_eq!(h.observe(m(3), 4, at(3000), grace), Verdict::Fine);
        assert!(h.suspect.is_none());
        assert_eq!(h.observe(m(3), 4, at(9000), grace), Verdict::Fine);
        // An unreadable modification time never flags anything.
        assert_eq!(h.observe(None, 4, at(10000), grace), Verdict::Fine);
        assert_eq!(h.observe(None, 4, at(20000), grace), Verdict::Fine);
    }

    /// A watch the backend dropped silently (here: removed behind the registry's back,
    /// like notify does after `ERROR_NOTIFY_ENUM_DIR`) is noticed through the directory's
    /// modification time, re-armed and reported as `Rescan`; later changes arrive again.
    #[test]
    fn silent_watch_is_rearmed_and_rescanned() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("watched");
        std::fs::create_dir(&dir).unwrap();
        let (mut w, rx) = watcher();
        let id = w.watch(&dir, false).unwrap();
        std::thread::sleep(Duration::from_millis(1200)); // first liveness sample
        w.debouncer.as_ref().unwrap().lock().unwatch(&dir).unwrap();
        std::fs::write(dir.join("unseen.txt"), b"1").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut rescanned = false;
        while Instant::now() < deadline && !rescanned {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
                assert_eq!(ev.id, id);
                rescanned = matches!(ev.kind, WatchKind::Rescan);
            }
        }
        assert!(rescanned, "a silent watch must be reported as Rescan");
        let f = dir.join("after.txt");
        std::fs::write(&f, b"2").unwrap();
        let evs = collect(&rx, |e| mentions(e, &f));
        assert!(mentions(&evs, &f), "the re-armed watch reports again: {evs:?}");
        w.unwatch(id);
    }

    /// Bursts of short-lived files must not lose the files that stay (the debouncer's
    /// file-id cache made notify's thread fall behind and overflow, dropping batches).
    #[test]
    fn bursts_do_not_lose_surviving_files() {
        let d = tempfile::tempdir().unwrap();
        let big = d.path().join("big");
        let busy = d.path().join("busy");
        std::fs::create_dir(&big).unwrap();
        std::fs::create_dir(&busy).unwrap();
        for i in 0..5000 {
            std::fs::write(big.join(format!("f{i}")), b"").unwrap();
        }
        let (mut w, rx) = watcher();
        let t = Instant::now();
        w.watch(&big, false).unwrap();
        assert!(t.elapsed() < Duration::from_millis(500), "watch must not walk the directory: {:?}", t.elapsed());
        let busy_id = w.watch(&busy, false).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let mut survivors = Vec::new();
        for round in 0..10 {
            for i in 0..200 {
                let p = busy.join(format!("transient_file_with_a_long_name_{round}_{i}.part"));
                std::fs::write(&p, b"x").unwrap();
                std::fs::rename(&p, p.with_extension("o")).unwrap();
                std::fs::remove_file(p.with_extension("o")).unwrap();
            }
            let s = busy.join(format!("survivor_{round}.txt"));
            std::fs::write(&s, b"1").unwrap();
            survivors.push(s);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = std::collections::HashSet::new();
        let mut rescan = false;
        while Instant::now() < deadline && !rescan && !survivors.iter().all(|s| seen.contains(s)) {
            if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
                if ev.id != busy_id {
                    continue;
                }
                match ev.kind {
                    WatchKind::Changed(v) => seen.extend(v),
                    WatchKind::Rescan => rescan = true,
                    WatchKind::Lost => {}
                }
            }
        }
        let missing: Vec<_> = survivors.iter().filter(|s| !seen.contains(*s)).collect();
        assert!(rescan || missing.is_empty(), "lost without a rescan: {missing:?}");
    }

    /// Watches the native backend cannot take (forced here, like inotify's ENOSPC) are
    /// polled: creations, modifications and the removal still arrive.
    #[test]
    fn polled_watches_report_changes() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("polled");
        std::fs::create_dir(&dir).unwrap();
        let (mut w, rx) = watcher();
        w.set_poll_interval(Duration::from_millis(250));
        w.force_poll = true;
        let id = w.watch(&dir, false).unwrap();
        assert!(w.is_polled(id));
        let f = dir.join("p.txt");
        std::fs::write(&f, b"1").unwrap();
        let evs = collect(&rx, |e| mentions(e, &f));
        assert!(mentions(&evs, &f), "create not reported by polling: {evs:?}");
        w.unwatch(id);
        assert!(!w.is_polled(id));
    }

    /// A native watch that fails with a resource limit after a partial add is removed from
    /// the native backend before falling back to polling.
    #[test]
    fn limit_fallback_drops_partial_native_watches() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("deep");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        let (mut w, _rx) = watcher();
        if w.debouncer.is_none() {
            return;
        }
        w.fake_limit = true;
        let id = w.watch(&dir, true).unwrap();
        assert!(w.is_polled(id));
        assert!(w.degraded().is_some());
        // notify's Windows backend reports Ok for any unwatch, so only unix can tell.
        #[cfg(not(windows))]
        {
            let native = w.debouncer.clone().unwrap();
            assert!(native.lock().unwatch(&dir).is_err(), "the native watch was already removed");
        }
        w.unwatch(id);
    }

    #[test]
    fn resource_limits_are_recognised() {
        assert!(is_resource_limit(&notify::Error::new(notify::ErrorKind::MaxFilesWatch)));
        #[cfg(unix)]
        {
            let e = notify::Error::io(std::io::Error::from_raw_os_error(libc::ENOSPC));
            assert!(is_resource_limit(&e));
            assert!(limit_message(&e).contains("max_user_watches"));
            let e = notify::Error::io(std::io::Error::from_raw_os_error(libc::EMFILE));
            assert!(limit_message(&e).contains("max_user_instances"));
        }
        assert!(!is_resource_limit(&notify::Error::path_not_found()));
    }

    #[test]
    fn watching_a_missing_dir_fails() {
        let d = tempfile::tempdir().unwrap();
        let (mut w, _rx) = watcher();
        assert_eq!(w.watch(&d.path().join("missing"), false).unwrap_err().kind, FsErrorKind::NotFound);
    }
}
