//! The index worker thread: crawl, watch, journal catch-up, contents, tags.
//!
//! Runs in background mode (low CPU / I/O priority). Writes are batched into
//! transactions (committed every few thousand changes or 300 ms, and before waiting);
//! each commit bumps the shared generation and wakes the UI.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, select};
use rusqlite::Connection;
use soyuz_core::{EntryKind, FsErrorKind};
use soyuz_fs::ListOptions;

use crate::db::{self, RootState, Row};
use crate::exclude::Excluder;
use crate::sql::KIND_DIR;
use crate::usn::{self, Action};
use crate::{IndexConfig, Msg, Phase, Shared, THROTTLE_BATCH, UsnState};

const COMMIT_OPS: usize = 4000;
const COMMIT_AGE: Duration = Duration::from_millis(300);
const CHECKPOINT_EVERY: Duration = Duration::from_secs(60);
const STATUS_COUNTS_EVERY: Duration = Duration::from_secs(2);
/// More journal changes than this → crawl instead.
const MAX_JOURNAL_CHANGES: usize = 500_000;
const JOURNAL_BUF: usize = 256 * 1024;
/// Parallel file readers for content indexing.
const CONTENT_READERS: usize = 4;

struct Crawl {
    root: PathBuf,
    /// Full crawls sweep rows they didn't see and mark the root complete.
    full: bool,
    /// The root was already complete (verification crawl).
    verify: bool,
    stack: Vec<(PathBuf, bool)>,
    started: bool,
}

struct Volume {
    root: PathBuf,
    handle: soyuz_fs::usn::UsnVolume,
    journal_id: u64,
    /// Checkpoint to store next (one tick behind the journal, so changes still in the
    /// watcher's debounce window are replayed on the next start).
    pending_checkpoint: i64,
}

struct Worker {
    cfg: IndexConfig,
    shared: Arc<Shared>,
    conn: Connection,
    roots: Vec<PathBuf>,
    excl: Excluder,
    db_dir: PathBuf,
    generation: i64,
    crawls: VecDeque<Crawl>,
    in_tx: bool,
    ops: usize,
    tx_started: Instant,
    since_pause: u64,
    content_dirty: bool,
    volumes: Vec<Volume>,
    last_checkpoint: Instant,
    last_counts: Instant,
    _watcher: Option<notify::RecommendedWatcher>,
    watch_rx: Receiver<WatchMsg>,
    /// Changed paths waiting for the debounce to settle.
    pending: HashSet<PathBuf>,
    pending_since: Option<Instant>,
    last_event: Instant,
}

/// From the recursive watcher.
enum WatchMsg {
    Paths(Vec<PathBuf>),
    /// The OS dropped events (buffer overflow): re-check everything.
    Rescan,
}

/// Pending changes are applied after this long, even while events keep coming.
const PENDING_MAX_AGE: Duration = Duration::from_secs(2);
/// More pending paths than this → re-check the roots instead.
const PENDING_MAX: usize = 50_000;

/// A plain recursive `notify` watcher per root. (`soyuz_fs::Watcher` uses the
/// debouncer's file-id cache, which walks the whole tree up front — far too slow for a
/// home folder.)
fn start_watcher(
    roots: &[PathBuf],
    tx: crossbeam_channel::Sender<WatchMsg>,
) -> (Option<notify::RecommendedWatcher>, Vec<String>) {
    use notify::{EventKind, RecursiveMode, Watcher as _};
    let mut warnings = Vec::new();
    let handler = move |res: notify::Result<notify::Event>| match res {
        Ok(ev) if ev.need_rescan() => {
            let _ = tx.send(WatchMsg::Rescan);
        }
        Ok(ev) if matches!(ev.kind, EventKind::Access(_)) || ev.paths.is_empty() => {}
        Ok(ev) => {
            let _ = tx.send(WatchMsg::Paths(ev.paths));
        }
        Err(e) => log::debug!("index watcher: {e}"),
    };
    let mut w = match notify::recommended_watcher(handler) {
        Ok(w) => w,
        Err(e) => {
            warnings.push(format!("No change watcher: {e}"));
            return (None, warnings);
        }
    };
    for r in roots {
        if let Err(e) = w.watch(r, RecursiveMode::Recursive) {
            warnings.push(format!("Can't watch {}: {e}", r.display()));
        }
    }
    (Some(w), warnings)
}

pub(crate) fn run(cfg: IndexConfig, shared: Arc<Shared>, rx: Receiver<Msg>) {
    // Low CPU priority; I/O stays at normal priority (background mode starves the
    // crawl 5-10x behind any other disk activity) and is paced by `throttle_ms`.
    soyuz_fs::usn::low_priority_thread();
    let conn = match db::open(&cfg.db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("index: can't open {}: {e}", cfg.db_path.display());
            shared.status.lock().phase = Phase::Failed(format!("Can't open the index database: {e}"));
            (shared.wake)();
            return;
        }
    };
    let (excl, mut warnings) = Excluder::new(&cfg.exclude);
    let mut roots: Vec<PathBuf> = Vec::new();
    for r in &cfg.roots {
        let c = crate::canonical(r);
        if !c.is_dir() {
            warnings.push(format!("{} isn't a folder; skipped", r.display()));
            continue;
        }
        // Nested roots are covered by their parent.
        if !roots.iter().any(|o| o == &c || c.starts_with(o)) {
            roots.retain(|o| !o.starts_with(&c));
            roots.push(c);
        }
    }
    let db_dir = cfg.db_path.parent().map(crate::canonical).unwrap_or_default();
    let (wtx, watch_rx) = crossbeam_channel::unbounded();
    let mut w = Worker {
        shared,
        conn,
        excl,
        db_dir,
        generation: 0,
        crawls: VecDeque::new(),
        in_tx: false,
        ops: 0,
        tx_started: Instant::now(),
        since_pause: 0,
        content_dirty: cfg.content,
        volumes: Vec::new(),
        last_checkpoint: Instant::now(),
        last_counts: Instant::now() - STATUS_COUNTS_EVERY,
        _watcher: None,
        watch_rx,
        pending: HashSet::new(),
        pending_since: None,
        last_event: Instant::now(),
        roots,
        cfg,
    };
    {
        let mut st = w.shared.status.lock();
        st.roots = w.roots.clone();
        st.warnings = warnings;
        st.content_enabled = w.cfg.content;
    }
    let t_init = Instant::now();
    if let Err(e) = w.init(wtx) {
        log::error!("index init: {e}");
        w.shared.status.lock().phase = Phase::Failed(format!("Index error: {e}"));
        (w.shared.wake)();
        return;
    }
    log::debug!("index: ready to work after {:?}", t_init.elapsed());
    w.main_loop(&rx);
    w.commit();
    w.checkpoint(true);
    w.commit();
}

impl Worker {
    fn stopping(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }

    fn init(&mut self, wtx: crossbeam_channel::Sender<WatchMsg>) -> rusqlite::Result<()> {
        self.generation = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='gen'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let removed = db::prune_roots(&self.conn, &self.roots)?;
        if removed > 0 {
            log::info!("index: dropped {removed} rows of removed roots");
        }
        if !self.cfg.content && db::content_count(&self.conn)? > 0 {
            db::clear_contents(&self.conn)?;
        }
        // Watch first, so nothing that happens from here on is missed.
        let (watcher, warnings) = start_watcher(&self.roots, wtx);
        self._watcher = watcher;
        self.shared.status.lock().warnings.extend(warnings);
        let mut ready = Vec::new();
        let roots = self.roots.clone();
        let mut usn_state = if self.cfg.usn { None } else { Some(UsnState::Off) };
        if self.cfg.usn {
            // Probe every root's volume up front, so the status explains the journal.
            for root in &roots {
                self.volume_for(root, &mut usn_state);
            }
        }
        for root in roots {
            let state = db::root_state(&self.conn, &root)?;
            let complete = state.is_some_and(|s| s.complete);
            let caught_up = complete && self.cfg.usn && self.catch_up(&root, &mut usn_state);
            if complete {
                ready.push(root.clone());
            }
            if !caught_up {
                self.crawls.push_back(Crawl {
                    root: root.clone(),
                    full: true,
                    verify: complete,
                    stack: vec![(root.clone(), false)],
                    started: false,
                });
            }
        }
        self.commit();
        {
            let mut st = self.shared.status.lock();
            st.ready_roots = ready;
            st.usn = usn_state.unwrap_or_default();
        }
        self.refresh_counts(true);
        Ok(())
    }

    /// Open (or reuse) the journal of the volume holding `root`.
    fn volume_for(&mut self, root: &Path, state: &mut Option<UsnState>) -> Option<usize> {
        let vroot = soyuz_fs::volume_root(root).ok()?;
        if let Some(i) = self.volumes.iter().position(|v| v.root == vroot) {
            return Some(i);
        }
        let handle = match soyuz_fs::usn::UsnVolume::open(&vroot) {
            Ok(h) => h,
            Err(e) => {
                let s = match e.kind {
                    FsErrorKind::PermissionDenied => UsnState::NeedsAdmin,
                    _ => UsnState::Unsupported(e.to_string()),
                };
                state.get_or_insert(s);
                return None;
            }
        };
        let info = match handle.query() {
            Ok(i) => i,
            Err(e) => {
                state.get_or_insert(UsnState::Unsupported(format!("no change journal on {}: {e}", vroot.display())));
                return None;
            }
        };
        state.get_or_insert(UsnState::Active);
        self.volumes.push(Volume {
            root: vroot,
            handle,
            journal_id: info.journal_id,
            pending_checkpoint: info.next_usn,
        });
        Some(self.volumes.len() - 1)
    }

    /// Apply journal changes since the stored checkpoint. `false` → crawl instead.
    fn catch_up(&mut self, root: &Path, state: &mut Option<UsnState>) -> bool {
        let Some(vi) = self.volume_for(root, state) else { return false };
        let vroot = self.volumes[vi].root.clone();
        let Ok(Some((jid, from))) = db::volume_checkpoint(&self.conn, &vroot) else { return false };
        let Ok(info) = self.volumes[vi].handle.query() else { return false };
        if jid != info.journal_id || from < info.first_usn.max(info.lowest_valid_usn) || from > info.next_usn {
            log::info!("index: journal of {} was reset or wrapped; crawling", vroot.display());
            return false;
        }
        self.shared.status.lock().phase = Phase::CatchingUp;
        (self.shared.wake)();
        let mut records = Vec::new();
        let mut buf = vec![0u8; JOURNAL_BUF];
        let mut usn_at = from;
        while usn_at < info.next_usn {
            let n = match self.volumes[vi].handle.read_journal(jid, usn_at, &mut buf) {
                Ok(n) => n,
                Err(e) => {
                    log::warn!("index: journal read failed: {e}; crawling");
                    return false;
                }
            };
            let Ok((next, recs)) = usn::parse_buffer(&buf[..n]) else { return false };
            records.extend(recs);
            if next as i64 <= usn_at || n <= 8 {
                break;
            }
            usn_at = next as i64;
            if records.len() > MAX_JOURNAL_CHANGES * 4 {
                return false;
            }
        }
        let changes = usn::coalesce(&records);
        if changes.len() > MAX_JOURNAL_CHANGES {
            return false;
        }
        let vol = &self.volumes[vi].handle;
        let plan = usn::plan(&changes, |frn| vol.path_of(frn).ok());
        let mut applied = 0u64;
        for a in plan {
            match a {
                Action::Refresh { path, crawl, .. } => {
                    if path.starts_with(root) {
                        self.apply_path(&path, crawl);
                        applied += 1;
                    }
                }
                Action::Forget { frn } => {
                    let paths = db::paths_by_frn(&self.conn, frn, &vroot).unwrap_or_default();
                    for p in paths.into_iter().filter(|p| p.starts_with(root)) {
                        self.begin();
                        self.ops += db::delete_tree(&self.conn, &p).unwrap_or(0);
                        applied += 1;
                    }
                }
            }
            self.maybe_commit();
        }
        self.commit();
        let _ = db::set_volume_checkpoint(&self.conn, &vroot, info.journal_id, info.next_usn);
        self.content_dirty |= self.cfg.content;
        self.shared.status.lock().journal_changes += applied;
        log::info!("index: applied {applied} journal changes for {}", root.display());
        true
    }

    /// Store journal checkpoints (`force`: at shutdown).
    fn checkpoint(&mut self, force: bool) {
        if !force && self.last_checkpoint.elapsed() < CHECKPOINT_EVERY {
            return;
        }
        self.last_checkpoint = Instant::now();
        // Only meaningful once every root on the volume is complete.
        let ready = self.shared.status.lock().ready_roots.clone();
        for v in &mut self.volumes {
            let all_ready = self
                .roots
                .iter()
                .filter(|r| soyuz_fs::volume_root(r).ok().as_deref() == Some(v.root.as_path()))
                .all(|r| ready.contains(r));
            if !all_ready {
                continue;
            }
            let store = v.pending_checkpoint;
            let _ = db::set_volume_checkpoint(&self.conn, &v.root, v.journal_id, store);
            if let Ok(info) = v.handle.query()
                && info.journal_id == v.journal_id
            {
                v.pending_checkpoint = info.next_usn;
            }
        }
    }

    fn begin(&mut self) {
        if !self.in_tx && self.conn.execute_batch("BEGIN").is_ok() {
            self.in_tx = true;
            self.tx_started = Instant::now();
        }
    }

    fn commit(&mut self) {
        if !self.in_tx {
            return;
        }
        if let Err(e) = self.conn.execute_batch("COMMIT") {
            log::warn!("index commit: {e}");
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        self.in_tx = false;
        if self.ops > 0 {
            self.ops = 0;
            self.shared.generation.fetch_add(1, Ordering::AcqRel);
            self.shared.status.lock().updated = Some(SystemTime::now());
            self.refresh_counts(false);
            (self.shared.wake)();
        }
    }

    fn maybe_commit(&mut self) {
        if self.in_tx && (self.ops >= COMMIT_OPS || self.tx_started.elapsed() >= COMMIT_AGE) {
            self.commit();
        }
    }

    fn refresh_counts(&mut self, force: bool) {
        if !force && self.last_counts.elapsed() < STATUS_COUNTS_EVERY {
            return;
        }
        self.last_counts = Instant::now();
        let counts = db::counts(&self.conn).unwrap_or((0, 0));
        let content = if self.cfg.content { db::content_count(&self.conn).unwrap_or(0) } else { 0 };
        let mut st = self.shared.status.lock();
        st.files = counts.0;
        st.folders = counts.1;
        st.content_files = content;
    }

    fn root_of(&self, p: &Path) -> Option<&PathBuf> {
        self.roots.iter().filter(|r| p.starts_with(r)).max_by_key(|r| r.as_os_str().len())
    }

    fn skip(&self, root: &Path, p: &Path) -> bool {
        (!self.db_dir.as_os_str().is_empty() && p.starts_with(&self.db_dir)) || self.excl.excluded_below(root, p)
    }

    /// Re-read one path (watcher / journal / touch): upsert, delete or crawl it.
    fn apply_path(&mut self, p: &Path, crawl_dir: bool) {
        let Some(root) = self.root_of(p).cloned() else { return };
        if p == root || self.skip(&root, p) {
            return;
        }
        self.begin();
        match soyuz_fs::stat(p, false) {
            Ok(e) => {
                let parent = p.parent().unwrap_or(&root);
                let hidden_above = if parent == root {
                    false
                } else {
                    db::hidden_of(&self.conn, parent).ok().flatten().unwrap_or(false)
                };
                let was = db::kind_of(&self.conn, p).ok().flatten();
                let row = Row::from_entry(&e, hidden_above);
                let hidden = row.hidden;
                if db::upsert(&self.conn, &row, self.generation).is_ok() {
                    self.ops += 1;
                }
                if e.kind == EntryKind::Dir && (crawl_dir || was != Some(KIND_DIR)) {
                    self.crawls.push_back(Crawl {
                        root: root.clone(),
                        full: false,
                        verify: false,
                        stack: vec![(p.to_path_buf(), hidden)],
                        started: false,
                    });
                }
                self.content_dirty |= self.cfg.content && e.kind == EntryKind::File;
            }
            Err(e) if e.kind == FsErrorKind::NotFound => {
                self.ops += db::delete_tree(&self.conn, p).unwrap_or(0);
            }
            Err(e) => log::debug!("index: stat {}: {e}", p.display()),
        }
    }

    fn on_watch(&mut self, msg: WatchMsg) {
        match msg {
            WatchMsg::Paths(paths) => {
                self.pending_since.get_or_insert_with(Instant::now);
                self.last_event = Instant::now();
                self.pending.extend(paths);
                if self.pending.len() > PENDING_MAX {
                    self.pending.clear();
                    self.pending_since = None;
                    self.rescan();
                }
            }
            WatchMsg::Rescan => self.rescan(),
        }
    }

    /// Apply pending watcher changes once they settle. Returns whether it did.
    fn flush_pending(&mut self, force: bool) -> bool {
        let Some(since) = self.pending_since else { return false };
        let settled = self.last_event.elapsed() >= self.cfg.watch_debounce;
        if !force && !settled && since.elapsed() < PENDING_MAX_AGE {
            return false;
        }
        self.pending_since = None;
        let mut paths: Vec<PathBuf> = self.pending.drain().collect();
        // Parents first, so a new folder is crawled once rather than per child.
        paths.sort();
        for p in paths {
            if self.roots.contains(&p) {
                self.check_lost(&p);
            } else {
                self.apply_path(&p, false);
            }
            self.maybe_commit();
        }
        self.commit();
        true
    }

    /// The OS dropped events: re-check the roots (once each).
    fn rescan(&mut self) {
        for r in self.roots.clone() {
            if !self.crawls.iter().any(|c| c.full && c.root == r) {
                let ready = self.shared.status.lock().ready_roots.contains(&r);
                self.crawls.push_back(Crawl {
                    root: r.clone(),
                    full: true,
                    verify: ready,
                    stack: vec![(r.clone(), false)],
                    started: false,
                });
            }
        }
    }

    /// A root itself changed: drop it when it's gone.
    fn check_lost(&mut self, r: &Path) {
        if r.exists() {
            return;
        }
        self.begin();
        self.ops += db::delete_tree(&self.conn, r).unwrap_or(0);
        let _ = db::set_root_state(&self.conn, r, RootState::default());
        self.shared.status.lock().ready_roots.retain(|x| x != r);
        self.shared.status.lock().warnings.push(format!("{} was removed", r.display()));
    }

    fn on_msg(&mut self, m: Msg) -> bool {
        match m {
            Msg::Shutdown => return false,
            Msg::Tags(tags) => {
                self.begin();
                if db::set_tags(&self.conn, &tags).is_ok() {
                    self.ops += 1;
                }
                self.commit();
            }
            Msg::Touch(paths) => {
                for p in paths {
                    let p = soyuz_fs::strip_verbatim(&p);
                    self.apply_path(&p, true);
                }
                self.commit();
            }
            Msg::Rebuild => {
                self.commit();
                let _ = self.conn.execute_batch(
                    "DELETE FROM files; DELETE FROM roots; DELETE FROM volumes; DELETE FROM contents; DELETE FROM content_state;",
                );
                self.ops += 1;
                self.crawls.clear();
                for r in self.roots.clone() {
                    self.crawls.push_back(Crawl {
                        root: r.clone(),
                        full: true,
                        verify: false,
                        stack: vec![(r, false)],
                        started: false,
                    });
                }
                {
                    let mut st = self.shared.status.lock();
                    st.ready_roots.clear();
                    st.journal_changes = 0;
                }
                self.content_dirty = self.cfg.content;
                self.shared.generation.fetch_add(1, Ordering::AcqRel);
                self.refresh_counts(true);
                (self.shared.wake)();
            }
        }
        true
    }

    /// Crawl one folder of the front crawl. Returns whether there was crawl work.
    fn crawl_step(&mut self) -> bool {
        let Some(c) = self.crawls.front_mut() else { return false };
        if !c.started {
            c.started = true;
            if c.full {
                // A full crawl: new generation (rows it doesn't see are swept at the end).
                let (root, verify) = (c.root.clone(), c.verify);
                self.generation += 1;
                let _ = self.conn.execute(
                    "INSERT INTO meta(key, value) VALUES('gen', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    [self.generation.to_string()],
                );
                let estimate = if verify { None } else { self.estimate(&root) };
                log::info!(
                    "index: {} {} (estimate {estimate:?})",
                    if verify { "verifying" } else { "crawling" },
                    root.display()
                );
                let mut st = self.shared.status.lock();
                st.phase = if verify { Phase::Verifying } else { Phase::Crawling };
                st.scanned = 0;
                st.estimate = estimate;
            }
        }
        let Some(c) = self.crawls.front_mut() else { return false };
        let Some((dir, hidden_above)) = c.stack.pop() else {
            // Done with this crawl.
            let c = self.crawls.pop_front().expect("front exists");
            if c.full {
                self.begin();
                self.ops += db::sweep(&self.conn, &c.root, self.generation).unwrap_or(0);
                let _ = db::set_root_state(
                    &self.conn,
                    &c.root,
                    RootState {
                        complete: true,
                        generation: self.generation,
                        crawled_at: Some(crate::query::unix_secs(SystemTime::now())),
                    },
                );
                self.ops += 1;
                self.commit();
                self.refresh_counts(true);
                log::info!("index: {} ready", c.root.display());
                let mut st = self.shared.status.lock();
                if !st.ready_roots.contains(&c.root) {
                    st.ready_roots.push(c.root.clone());
                }
                st.current = None;
                st.estimate = None;
            }
            self.content_dirty |= self.cfg.content;
            return true;
        };
        let root = c.root.clone();
        let opts = ListOptions { include_hidden: true, stat_links: false, ..Default::default() };
        let entries = match soyuz_fs::read_dir(&dir, &opts) {
            Ok(v) => v,
            Err(e) => {
                log::debug!("index: list {}: {e}", dir.display());
                if dir == root && e.kind == FsErrorKind::NotFound {
                    self.shared.status.lock().warnings.push(format!("{} is gone", root.display()));
                }
                return true;
            }
        };
        self.begin();
        let mut push = Vec::new();
        for e in &entries {
            let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if self.excl.is_excluded(&e.path, &name)
                || (!self.db_dir.as_os_str().is_empty() && e.path.starts_with(&self.db_dir))
            {
                continue;
            }
            let row = Row::from_entry(e, hidden_above);
            if e.kind == EntryKind::Dir {
                push.push((e.path.clone(), row.hidden));
            }
            if db::upsert(&self.conn, &row, self.generation).is_ok() {
                self.ops += 1;
            }
        }
        let n = entries.len() as u64;
        if let Some(c) = self.crawls.front_mut() {
            c.stack.extend(push.into_iter().rev());
        }
        {
            let mut st = self.shared.status.lock();
            st.scanned += n;
            st.current = Some(dir);
        }
        self.since_pause += n;
        if self.since_pause >= THROTTLE_BATCH {
            self.since_pause = 0;
            self.commit();
            if self.cfg.throttle_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.cfg.throttle_ms));
            }
        }
        self.maybe_commit();
        true
    }

    /// Expected number of entries below `root` from a whole-volume MFT enumeration
    /// (`FSCTL_ENUM_USN_DATA`), when the journal is usable.
    fn estimate(&mut self, root: &Path) -> Option<u64> {
        if !self.cfg.usn {
            return None;
        }
        let mut state = None;
        let vi = self.volume_for(root, &mut state)?;
        let root_frn = soyuz_fs::stat(root, false).ok()?.inode?;
        let vol = &self.volumes[vi].handle;
        let mut buf = vec![0u8; JOURNAL_BUF];
        let mut next = 0u64;
        // frn → (parent, excluded-by-name) for folders; files only need their parent.
        let mut dirs: HashMap<u64, (u64, bool)> = HashMap::new();
        let mut files: Vec<u64> = Vec::new();
        let t0 = Instant::now();
        loop {
            if self.shared.stop.load(Ordering::SeqCst) || t0.elapsed() > Duration::from_secs(20) {
                return None;
            }
            let n = match vol.enum_mft(next, &mut buf) {
                Ok(Some(n)) => n,
                Ok(None) => break,
                Err(_) => return None,
            };
            let (cursor, recs) = usn::parse_buffer(&buf[..n]).ok()?;
            for r in recs {
                if r.is_dir() {
                    dirs.insert(r.frn, (r.parent_frn, self.excl.is_excluded(Path::new(&r.name), &r.name)));
                } else {
                    files.push(r.parent_frn);
                }
            }
            if cursor <= next || n <= 8 {
                break;
            }
            next = cursor;
        }
        // Memoized "is below root" per folder.
        let mut under: HashMap<u64, bool> = HashMap::new();
        under.insert(root_frn, true);
        let mut check = |start: u64, dirs: &HashMap<u64, (u64, bool)>| -> bool {
            let mut chain = Vec::new();
            let mut cur = start;
            let res = loop {
                if let Some(&u) = under.get(&cur) {
                    break u;
                }
                match dirs.get(&cur) {
                    Some(&(_, true)) => break false,
                    Some(&(parent, false)) if parent != cur && chain.len() < 4096 => {
                        chain.push(cur);
                        cur = parent;
                    }
                    _ => break false,
                }
            };
            for c in chain {
                under.insert(c, res);
            }
            res
        };
        let mut count = 0u64;
        let dir_keys: Vec<(u64, u64)> = dirs.iter().map(|(k, v)| (*k, v.0)).collect();
        for (frn, parent) in dir_keys {
            if frn != root_frn && check(parent, &dirs) && !dirs.get(&frn).is_some_and(|d| d.1) {
                count += 1;
            }
        }
        for parent in files {
            if check(parent, &dirs) {
                count += 1;
            }
        }
        Some(count)
    }

    /// Index the contents of a batch of text files. Returns whether there was work.
    fn content_step(&mut self) -> bool {
        if !self.cfg.content || !self.content_dirty {
            return false;
        }
        let todo = db::content_todo(&self.conn, &self.cfg.content_extensions, self.cfg.content_max_bytes, 256)
            .unwrap_or_default();
        if todo.is_empty() {
            self.content_dirty = false;
            self.refresh_counts(true);
            return false;
        }
        self.shared.status.lock().phase = Phase::Contents;
        // Reading is dominated by open() latency (antivirus scanning): a few readers in
        // parallel, then one write transaction.
        let max = self.cfg.content_max_bytes;
        let readers = CONTENT_READERS.min(todo.len()).max(1);
        let chunk = todo.len().div_ceil(readers);
        let bodies: Vec<Option<String>> = std::thread::scope(|s| {
            let handles: Vec<_> = todo
                .chunks(chunk)
                .map(|part| {
                    s.spawn(move || part.iter().map(|t| crate::content::read_text(&t.path, max)).collect::<Vec<_>>())
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        if self.stopping() || bodies.len() != todo.len() {
            return true;
        }
        self.begin();
        for (t, body) in todo.iter().zip(&bodies) {
            if db::set_content(&self.conn, t, body.as_deref()).is_ok() {
                self.ops += 1;
            }
        }
        self.commit();
        self.shared.status.lock().content_files += todo.len() as u64;
        if self.cfg.throttle_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.cfg.throttle_ms * (todo.len() as u64).div_ceil(64)));
        }
        true
    }

    fn main_loop(&mut self, rx: &Receiver<Msg>) {
        loop {
            if self.stopping() {
                return;
            }
            while let Ok(m) = rx.try_recv() {
                if !self.on_msg(m) {
                    return;
                }
            }
            while let Ok(ev) = self.watch_rx.try_recv() {
                self.on_watch(ev);
            }
            if self.flush_pending(false) || self.crawl_step() || self.content_step() {
                continue;
            }
            // Idle.
            self.commit();
            {
                let mut st = self.shared.status.lock();
                if !matches!(st.phase, Phase::Failed(_)) && st.phase != Phase::Idle {
                    st.phase = Phase::Idle;
                    st.current = None;
                    drop(st);
                    self.refresh_counts(true);
                    (self.shared.wake)();
                }
            }
            self.checkpoint(false);
            self.commit();
            let wait = if self.pending_since.is_some() { self.cfg.watch_debounce } else { CHECKPOINT_EVERY };
            select! {
                recv(rx) -> m => match m {
                    Ok(m) => {
                        if !self.on_msg(m) {
                            return;
                        }
                    }
                    Err(_) => return,
                },
                recv(self.watch_rx) -> ev => {
                    if let Ok(ev) = ev {
                        self.on_watch(ev);
                    }
                }
                default(wait) => {}
            }
        }
    }
}
