//! `soyuz-index`: the optional background file index.
//!
//! * [`Index::start`] opens (or creates) an SQLite database with FTS5 tables and runs a
//!   low-priority worker thread that crawls the configured roots (throttled, skipping
//!   exclusions), keeps the index fresh from a recursive watcher, and on NTFS catches up
//!   with changes made while Soyuz was closed by reading the USN change journal
//!   (`soyuz_fs::usn`; needs administrator rights, otherwise it falls back to a
//!   verification crawl). Roots that can't be watched (Linux: the inotify watch limit is
//!   used up) are re-checked periodically instead ([`IndexStatus::polled_roots`],
//!   [`Index::set_rescan_interval`]). Optional content indexing stores the words of small
//!   text files.
//! * [`query::parse`] reads the query language (`kind:image size:>5MB modified:<7d
//!   tag:red ext:pdf path:docs "exact phrase" -exclude a OR b`); [`sql::select`] maps it
//!   to SQL + FTS for the index, and [`eval`] evaluates it against live search hits when
//!   the index is off ([`eval::to_search_query`] narrows the `soyuz-search` walk first).
//! * [`Index::search`] / [`Index::search_async`] run queries on read-only connections
//!   (WAL), so they never wait for the crawler.
#![forbid(unsafe_code)]

pub mod content;
pub mod db;
pub mod eval;
pub mod exclude;
pub mod query;
pub mod sql;
pub mod usn;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use soyuz_core::{EntryInfo, FsError, FsErrorKind, OpControl};
use soyuz_search::{SearchEvent, SearchHandle, SearchStats};

pub use eval::{Matcher, to_search_query};
pub use query::{Expr, KindClass, ParseError, Term, parse};

/// Everything the worker needs; built by the app from `[search.index]`.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexConfig {
    pub db_path: PathBuf,
    /// Folders to index (made absolute / canonical by the worker).
    pub roots: Vec<PathBuf>,
    /// Exclusion patterns (see [`exclude`]).
    pub exclude: Vec<String>,
    /// Index the words of small text files.
    pub content: bool,
    pub content_max_bytes: u64,
    /// Extensions whose contents are indexed (lower-case, no dot).
    pub content_extensions: Vec<String>,
    /// Use the NTFS change journal when possible.
    pub usn: bool,
    /// Pause (ms) after every [`THROTTLE_BATCH`] crawled entries.
    pub throttle_ms: u64,
    pub watch_debounce: Duration,
}

/// Entries crawled between throttle pauses.
pub const THROTTLE_BATCH: u64 = 1000;

impl Default for IndexConfig {
    fn default() -> Self {
        IndexConfig {
            db_path: PathBuf::from("soyuz-index.db"),
            roots: Vec::new(),
            exclude: Vec::new(),
            content: false,
            content_max_bytes: 1 << 20,
            content_extensions: content::DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
            usn: true,
            throttle_ms: 2,
            watch_debounce: Duration::from_millis(500),
        }
    }
}

/// What the worker is doing.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Starting,
    /// First crawl of a root (the root isn't searchable through the index yet).
    Crawling,
    /// Re-crawling already indexed roots to find changes made while Soyuz was closed
    /// (no change journal); the index stays searchable meanwhile.
    Verifying,
    /// Applying changes from the NTFS change journal.
    CatchingUp,
    /// Reading text file contents.
    Contents,
    /// Up to date, following changes.
    Idle,
    /// The database couldn't be opened; the index is off.
    Failed(String),
}

/// Whether the NTFS change journal is used.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum UsnState {
    /// `[search.index] usn_journal = false`, or not checked yet.
    #[default]
    Off,
    /// Reading the journal (changes since the last run are applied without a crawl).
    Active,
    /// Reading the journal needs administrator rights (Windows before 11).
    NeedsAdmin,
    /// Not an NTFS volume, no journal, or not Windows (reason [`USN_NOT_WINDOWS`]).
    Unsupported(String),
}

/// The [`UsnState::Unsupported`] reason off Windows (the journal is NTFS-only; the UI
/// can hide the journal row when it sees this).
pub const USN_NOT_WINDOWS: &str = "Windows NTFS only";

/// Default for [`Index::set_rescan_interval`].
pub const DEFAULT_RESCAN_INTERVAL: Duration = Duration::from_secs(600);

/// A snapshot of the index for the UI.
#[derive(Clone, Debug, Default)]
pub struct IndexStatus {
    pub phase: Phase,
    pub files: u64,
    pub folders: u64,
    /// Entries seen by the running crawl.
    pub scanned: u64,
    /// Expected entries for the running crawl (from the MFT, when available).
    pub estimate: Option<u64>,
    /// Folder being crawled.
    pub current: Option<PathBuf>,
    /// Canonical configured roots and the ones that are fully indexed.
    pub roots: Vec<PathBuf>,
    pub ready_roots: Vec<PathBuf>,
    pub usn: UsnState,
    /// Changes applied from the journal at startup.
    pub journal_changes: u64,
    /// Files whose contents are indexed.
    pub content_files: u64,
    pub content_enabled: bool,
    /// Non-fatal problems (bad exclusion patterns, unreadable roots …).
    pub warnings: Vec<String>,
    /// Roots without a change watcher because the system's watch limit is used up
    /// (Linux `fs.inotify.max_user_watches`): they're re-checked every rescan interval
    /// (folders whose modification time changed are re-read), so changes show up late.
    pub polled_roots: Vec<PathBuf>,
    /// Roots that can't be read right now (permission denied, I/O error, drive or mount
    /// away). Their index is kept (searches still find the old rows); they're re-checked
    /// every rescan interval and fully rescanned once they're back.
    pub unavailable_roots: Vec<PathBuf>,
    /// Last time anything was written.
    pub updated: Option<SystemTime>,
}

impl IndexStatus {
    /// Crawl progress 0..1 when an estimate exists.
    pub fn progress(&self) -> Option<f32> {
        let est = self.estimate.filter(|e| *e > 0)?;
        Some((self.scanned as f32 / est as f32).clamp(0.0, 0.99))
    }
}

/// A query for [`Index::search`].
#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub expr: Expr,
    pub include_hidden: bool,
    /// Restrict to these folders (descendants); empty = everything indexed.
    pub scope: Vec<PathBuf>,
    /// 0 = unlimited.
    pub limit: usize,
    pub now: SystemTime,
}

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("index database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

pub(crate) enum Msg {
    Rebuild,
    Tags(Vec<(PathBuf, Vec<String>)>),
    Touch(Vec<PathBuf>),
    Shutdown,
}

pub(crate) struct Shared {
    pub status: Mutex<IndexStatus>,
    pub generation: AtomicU64,
    pub stop: AtomicBool,
    pub wake: Arc<dyn Fn() + Send + Sync>,
    /// Polled roots are re-checked this often (ms).
    pub rescan_ms: AtomicU64,
}

struct Inner {
    shared: Arc<Shared>,
    tx: Sender<Msg>,
    db_path: PathBuf,
    readers: Mutex<Vec<rusqlite::Connection>>,
}

/// A running index. Dropping it stops the worker (waiting at most about a second).
pub struct Index {
    inner: Arc<Inner>,
    done: Receiver<()>,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index").field("db", &self.inner.db_path).finish()
    }
}

impl Index {
    /// Start the worker. `wake` is called after every batch of changes (repaint hook).
    pub fn start(cfg: IndexConfig, wake: Arc<dyn Fn() + Send + Sync>) -> Index {
        let shared = Arc::new(Shared {
            status: Mutex::new(IndexStatus { content_enabled: cfg.content, ..Default::default() }),
            generation: AtomicU64::new(1),
            stop: AtomicBool::new(false),
            wake,
            rescan_ms: AtomicU64::new(DEFAULT_RESCAN_INTERVAL.as_millis() as u64),
        });
        let (tx, rx) = crossbeam_channel::unbounded();
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let db_path = cfg.db_path.clone();
        let sh = shared.clone();
        let spawned = std::thread::Builder::new().name("soyuz-index".into()).spawn(move || {
            worker::run(cfg, sh, rx);
            let _ = done_tx.send(());
        });
        if let Err(e) = spawned {
            shared.status.lock().phase = Phase::Failed(format!("couldn't start the index thread: {e}"));
        }
        Index { inner: Arc::new(Inner { shared, tx, db_path, readers: Mutex::new(Vec::new()) }), done }
    }

    pub fn status(&self) -> IndexStatus {
        self.inner.shared.status.lock().clone()
    }

    /// How often roots without a change watcher ([`IndexStatus::polled_roots`]) are
    /// re-checked (`[search.index] rescan_interval_secs`; at least 0.1 s). Takes
    /// effect immediately.
    pub fn set_rescan_interval(&self, every: Duration) {
        let ms = every.as_millis().clamp(100, u64::MAX as u128) as u64;
        self.inner.shared.rescan_ms.store(ms, Ordering::Relaxed);
    }

    /// Bumped after every committed batch of changes (smart folders refresh on it).
    pub fn generation(&self) -> u64 {
        self.inner.shared.generation.load(Ordering::Acquire)
    }

    /// Drop everything and crawl again.
    pub fn rebuild(&self) {
        let _ = self.inner.tx.send(Msg::Rebuild);
    }

    /// Replace the color tags (`path`, lower-case slugs) used by `tag:`.
    pub fn set_tags(&self, tags: Vec<(PathBuf, Vec<String>)>) {
        let _ = self.inner.tx.send(Msg::Tags(tags));
    }

    /// Re-read these paths now (e.g. after an operation Soyuz itself performed).
    pub fn touch(&self, paths: Vec<PathBuf>) {
        let _ = self.inner.tx.send(Msg::Touch(paths));
    }

    /// Whether every folder below `dir` is fully indexed (`dir` inside a ready root).
    pub fn covers(&self, dir: &Path) -> bool {
        let st = self.inner.shared.status.lock();
        let dir = soyuz_fs::strip_verbatim(dir);
        st.ready_roots.iter().any(|r| same_path(r, &dir) || eval::is_under(&dir, r))
    }

    /// Run a query (blocking; call it off the UI thread — see [`Index::search_async`]).
    pub fn search(&self, req: &SearchRequest) -> Result<Vec<EntryInfo>, IndexError> {
        search_with(&self.inner, req)
    }

    /// Run a query on a short-lived thread and stream the result like a `soyuz-search`
    /// walk (`Hits` chunks, then `Done`), so the app's search plumbing can show it.
    pub fn search_async(&self, req: SearchRequest, wake: Arc<dyn Fn() + Send + Sync>) -> SearchHandle {
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let ctl = OpControl::new();
        let done = Arc::new(AtomicBool::new(false));
        let handle = SearchHandle::new(soyuz_search::next_search_id(), rx, ctl.clone(), done.clone());
        let inner = self.inner.clone();
        let spawned = std::thread::Builder::new().name("soyuz-index-query".into()).spawn(move || {
            let t0 = Instant::now();
            let limit = req.limit;
            let mut stats = SearchStats::default();
            match search_with(&inner, &req) {
                Ok(hits) => {
                    stats.hits = hits.len() as u64;
                    stats.scanned = stats.hits;
                    stats.truncated = limit > 0 && hits.len() >= limit;
                    let mut it = hits.into_iter().peekable();
                    while it.peek().is_some() {
                        if ctl.is_cancelled() {
                            stats.truncated = true;
                            break;
                        }
                        let chunk: Vec<_> = it.by_ref().take(4096).map(|e| (e, Vec::new())).collect();
                        if tx.send(SearchEvent::Hits(chunk)).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(SearchEvent::Error(
                        PathBuf::new(),
                        FsError::with_message(FsErrorKind::Io, "index_search", None, e.to_string()),
                    ));
                }
            }
            stats.elapsed = t0.elapsed();
            let _ = tx.send(SearchEvent::Done(stats));
            done.store(true, Ordering::SeqCst);
            wake();
        });
        if spawned.is_err() {
            handle.cancel();
        }
        handle
    }
}

impl Drop for Index {
    fn drop(&mut self) {
        self.inner.shared.stop.store(true, Ordering::SeqCst);
        let _ = self.inner.tx.send(Msg::Shutdown);
        // Bounded: a crawl stuck in a slow folder must not hang the UI.
        let _ = self.done.recv_timeout(Duration::from_millis(1500));
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { a == b }
}

/// Canonical form of a folder (true case, no verbatim prefix); the input when it can't
/// be resolved.
pub fn canonical(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => soyuz_fs::strip_verbatim(&c),
        Err(_) => p.to_path_buf(),
    }
}

/// `In` paths and scopes in canonical form, so they match the stored paths.
fn canonical_expr(e: &Expr) -> Expr {
    match e {
        Expr::And(v) => Expr::And(v.iter().map(canonical_expr).collect()),
        Expr::Or(v) => Expr::Or(v.iter().map(canonical_expr).collect()),
        Expr::Not(x) => Expr::Not(Box::new(canonical_expr(x))),
        Expr::Term(Term::In(p)) => Expr::Term(Term::In(canonical(p))),
        Expr::Term(t) => Expr::Term(t.clone()),
    }
}

fn search_with(inner: &Inner, req: &SearchRequest) -> Result<Vec<EntryInfo>, IndexError> {
    let conn = match inner.readers.lock().pop() {
        Some(c) => c,
        None => db::open_reader(&inner.db_path)?,
    };
    let now = query::unix_secs(req.now);
    let opts = sql::SqlOptions {
        now,
        utc_offset: query::local_offset(now),
        include_hidden: req.include_hidden,
        scope: req.scope.iter().map(|p| canonical(p)).collect(),
        limit: req.limit,
    };
    let q = sql::select(&canonical_expr(&req.expr), &opts);
    let res = db::run(&conn, &q);
    let mut pool = inner.readers.lock();
    if pool.len() < 4 {
        pool.push(conn);
    }
    Ok(res?)
}

#[cfg(test)]
mod tests;
