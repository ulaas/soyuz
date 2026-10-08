//! Parallel walker over `ignore::WalkBuilder::build_parallel` + `soyuz_fs::stat` (M6).
//!
//! Layout of a running search: N `ignore` worker threads match entries and push hits
//! into an internal bounded channel; one coordinator thread batches them into
//! [`SearchEvent::Hits`] (first hit immediately, then every 50 ms or 256 hits), emits
//! [`SearchEvent::Progress`] every 100 ms and finally [`SearchEvent::Done`]. Every
//! blocking send polls the [`OpControl`] so a cancel never deadlocks on a full channel.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, SendTimeoutError, Sender};
use ignore::{WalkBuilder, WalkState};
use parking_lot::Mutex;
use soyuz_core::{EntryInfo, FsError, FsErrorKind, OpControl, SearchId};

use crate::content::search_file;
use crate::matcher::{Compiled, is_file_like};
use crate::query::SearchQuery;
use crate::{ContentMatch, SearchEvent, SearchHandle, SearchStats};

/// Batch flush interval / size and progress interval.
pub(crate) const BATCH_INTERVAL: Duration = Duration::from_millis(50);
pub(crate) const BATCH_MAX: usize = 256;
pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Capacity of [`SearchHandle::rx`].
pub(crate) const EVENT_CAPACITY: usize = 1024;

/// Send that gives up when `ctl` is cancelled or the receiver is gone. `true` = sent.
pub(crate) fn send_cancellable<T>(tx: &Sender<T>, mut item: T, ctl: &OpControl) -> bool {
    loop {
        if ctl.is_cancelled() {
            return false;
        }
        match tx.send_timeout(item, Duration::from_millis(25)) {
            Ok(()) => return true,
            Err(SendTimeoutError::Timeout(back)) => item = back,
            Err(SendTimeoutError::Disconnected(_)) => return false,
        }
    }
}

/// Thread count for `threads == 0`.
pub(crate) fn default_threads(threads: usize) -> usize {
    if threads > 0 { threads } else { std::thread::available_parallelism().map_or(4, |n| n.get()).min(16) }
}

/// A walk builder with every filter off (sizes, duplicates): hidden files included,
/// ignore files not consulted, symlinks not followed.
pub(crate) fn raw_builder<P: AsRef<Path>>(roots: &[P], threads: usize) -> Option<WalkBuilder> {
    let (first, rest) = roots.split_first()?;
    let mut b = WalkBuilder::new(first);
    for r in rest {
        b.add(r);
    }
    b.standard_filters(false).hidden(false).follow_links(false).threads(default_threads(threads));
    Some(b)
}

/// Convert an `ignore` error into the workspace error type, with its path if known.
pub(crate) fn convert_error(err: &ignore::Error, fallback: &Path) -> (PathBuf, FsError) {
    fn path_of(err: &ignore::Error) -> Option<&Path> {
        match err {
            ignore::Error::WithPath { path, .. } => Some(path),
            ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => path_of(err),
            ignore::Error::Loop { child, .. } => Some(child),
            ignore::Error::Partial(errs) => errs.first().and_then(path_of),
            _ => None,
        }
    }
    let path = path_of(err).unwrap_or(fallback).to_path_buf();
    let fe = match (err.io_error(), err) {
        (Some(io), _) => FsError::from_io("search", Some(&path), io),
        (None, ignore::Error::Loop { .. }) => {
            FsError::with_message(FsErrorKind::Loop, "search", Some(&path), err.to_string())
        }
        (None, _) => FsError::with_message(FsErrorKind::Io, "search", Some(&path), err.to_string()),
    };
    (path, fe)
}

/// `(dev, ino)` of a multiply-linked regular file (unix hardlink dedupe); `None` elsewhere.
#[cfg(unix)]
pub(crate) fn hardlink_id(md: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    (md.is_file() && md.nlink() > 1).then(|| (md.dev(), md.ino()))
}

#[cfg(not(unix))]
pub(crate) fn hardlink_id(_md: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

enum Internal {
    Hit(EntryInfo, Vec<ContentMatch>),
    Error(PathBuf, FsError),
}

struct Shared {
    ctl: Arc<OpControl>,
    compiled: Compiled,
    max_results: u64,
    hits: AtomicU64,
    scanned: AtomicU64,
    dirs: AtomicU64,
    /// `max_results` reached: workers stop, result is truncated.
    limit_hit: AtomicBool,
    current: Mutex<PathBuf>,
}

impl Shared {
    fn stop(&self) -> bool {
        self.limit_hit.load(Ordering::Relaxed) || self.ctl.checkpoint().is_err()
    }
}

/// Spawn the search threads and return the handle (patterns already validated).
pub(crate) fn spawn_search(
    q: SearchQuery,
    compiled: Compiled,
    threads: usize,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> SearchHandle {
    let id = crate::next_search_id();
    let (tx, rx) = crossbeam_channel::bounded(EVENT_CAPACITY);
    let ctl = OpControl::new();
    let done = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Shared {
        ctl: ctl.clone(),
        compiled,
        max_results: q.max_results as u64,
        hits: AtomicU64::new(0),
        scanned: AtomicU64::new(0),
        dirs: AtomicU64::new(0),
        limit_hit: AtomicBool::new(false),
        current: Mutex::new(q.roots.first().cloned().unwrap_or_default()),
    });
    let done2 = done.clone();
    let spawned = std::thread::Builder::new().name(format!("soyuz-search-{}", id.0)).spawn(move || {
        coordinate(id, q, threads, shared, tx, wake.as_ref());
        done2.store(true, Ordering::SeqCst);
        wake();
    });
    if let Err(e) = spawned {
        log::error!("cannot spawn search thread: {e}");
        done.store(true, Ordering::SeqCst);
    }
    SearchHandle::new(id, rx, ctl, done)
}

fn coordinate(
    id: SearchId,
    q: SearchQuery,
    threads: usize,
    shared: Arc<Shared>,
    tx: Sender<SearchEvent>,
    wake: &(dyn Fn() + Send + Sync),
) {
    let started = Instant::now();
    let (itx, irx) = crossbeam_channel::bounded::<Internal>(4 * BATCH_MAX);
    let walker = {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name(format!("soyuz-search-walk-{}", id.0))
            .spawn(move || run_walk(&q, threads, &shared, &itx))
    };
    let walker = match walker {
        Ok(h) => Some(h),
        Err(e) => {
            log::error!("cannot spawn search walker: {e}");
            None
        }
    };
    let ctl = shared.ctl.clone();
    let send = |ev: SearchEvent| -> bool {
        let ok = send_cancellable(&tx, ev, &ctl);
        if ok {
            wake();
        } else {
            // Receiver dropped (handle gone) or cancelled: make sure workers stop.
            ctl.cancel();
        }
        ok
    };

    let mut batch: Vec<(EntryInfo, Vec<ContentMatch>)> = Vec::new();
    let mut delivered = 0u64;
    // First hit goes out immediately.
    let mut last_flush = started.checked_sub(BATCH_INTERVAL).unwrap_or(started);
    let mut last_progress = started;
    let flush = |batch: &mut Vec<_>, last_flush: &mut Instant, delivered: &mut u64| {
        if batch.is_empty() {
            return;
        }
        let n = batch.len() as u64;
        if send(SearchEvent::Hits(std::mem::take(batch))) {
            *delivered += n;
        }
        *last_flush = Instant::now();
    };
    if walker.is_some() {
        loop {
            let wait = BATCH_INTERVAL.min(PROGRESS_INTERVAL);
            match irx.recv_timeout(wait) {
                Ok(Internal::Hit(e, m)) => batch.push((e, m)),
                Ok(Internal::Error(p, e)) => {
                    flush(&mut batch, &mut last_flush, &mut delivered);
                    send(SearchEvent::Error(p, e));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if !batch.is_empty() && (batch.len() >= BATCH_MAX || last_flush.elapsed() >= BATCH_INTERVAL) {
                flush(&mut batch, &mut last_flush, &mut delivered);
            }
            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                last_progress = Instant::now();
                let ev = SearchEvent::Progress {
                    scanned: shared.scanned.load(Ordering::Relaxed),
                    dirs: shared.dirs.load(Ordering::Relaxed),
                    current: shared.current.lock().clone(),
                };
                // Progress is droppable: never block on it.
                if tx.try_send(ev).is_ok() {
                    wake();
                }
            }
        }
    }
    if let Some(h) = walker {
        let _ = h.join();
    }
    flush(&mut batch, &mut last_flush, &mut delivered);
    let stats = SearchStats {
        scanned: shared.scanned.load(Ordering::Relaxed),
        hits: delivered,
        elapsed: started.elapsed(),
        truncated: shared.limit_hit.load(Ordering::Relaxed) || ctl.is_cancelled(),
    };
    // Done must not block forever on a cancelled search nobody drains.
    let done = SearchEvent::Done(stats);
    let sent = if ctl.is_cancelled() { tx.try_send(done).is_ok() } else { send_cancellable(&tx, done, &ctl) };
    if sent {
        wake();
    }
}

fn run_walk(q: &SearchQuery, threads: usize, shared: &Arc<Shared>, itx: &Sender<Internal>) {
    let Some((first, rest)) = q.roots.split_first() else { return };
    let mut b = WalkBuilder::new(first);
    for r in rest {
        b.add(r);
    }
    let gi = q.respect_gitignore;
    b.hidden(!q.include_hidden)
        .ignore(gi)
        .git_ignore(gi)
        .git_global(gi)
        .git_exclude(gi)
        .parents(gi)
        .require_git(false)
        .follow_links(q.follow_symlinks)
        .max_depth(q.max_depth)
        .threads(default_threads(threads));
    b.build_parallel().run(|| {
        let shared = shared.clone();
        let itx = itx.clone();
        let mut scorer = shared.compiled.scorer();
        let mut buf = Vec::new();
        Box::new(move |res| {
            if shared.stop() {
                return WalkState::Quit;
            }
            let dent = match res {
                Ok(d) => d,
                Err(err) => {
                    let (p, e) = convert_error(&err, Path::new(""));
                    return if send_cancellable(&itx, Internal::Error(p, e), &shared.ctl) {
                        WalkState::Continue
                    } else {
                        WalkState::Quit
                    };
                }
            };
            let is_dir = dent.file_type().map(|t| t.is_dir());
            if is_dir == Some(true) {
                shared.dirs.fetch_add(1, Ordering::Relaxed);
                if let Some(mut cur) = shared.current.try_lock() {
                    cur.clear();
                    cur.push(dent.path());
                }
            }
            // The roots themselves are never results.
            if dent.depth() == 0 {
                return WalkState::Continue;
            }
            shared.scanned.fetch_add(1, Ordering::Relaxed);
            let name = dent.file_name().to_string_lossy();
            if !shared.compiled.pre_match(&name, is_dir, scorer.as_mut()) {
                return WalkState::Continue;
            }
            let entry = match soyuz_fs::stat(dent.path(), false) {
                Ok(e) => e,
                Err(e) => {
                    let p = dent.path().to_path_buf();
                    return if send_cancellable(&itx, Internal::Error(p, e), &shared.ctl) {
                        WalkState::Continue
                    } else {
                        WalkState::Quit
                    };
                }
            };
            if !shared.compiled.meta_match(&entry) {
                return WalkState::Continue;
            }
            let mut matches = Vec::new();
            if let Some(cm) = &shared.compiled.content {
                // Symlinks to files report the link's own size; use the target's.
                let size = if is_file_like(entry.kind) && entry.kind != soyuz_core::EntryKind::File {
                    std::fs::metadata(dent.path()).map_or(u64::MAX, |m| m.len())
                } else {
                    entry.size
                };
                if size > shared.compiled.content_max_size {
                    return WalkState::Continue;
                }
                match search_file(cm, dent.path(), shared.compiled.content_max_size, &mut buf) {
                    Ok(Some(m)) if !m.is_empty() => matches = m,
                    Ok(_) => return WalkState::Continue,
                    Err(e) => {
                        let p = dent.path().to_path_buf();
                        let fe = FsError::from_io("search", Some(&p), &e);
                        return if send_cancellable(&itx, Internal::Error(p, fe), &shared.ctl) {
                            WalkState::Continue
                        } else {
                            WalkState::Quit
                        };
                    }
                }
                if buf.capacity() > 1 << 20 {
                    buf = Vec::new();
                }
            }
            let n = shared.hits.fetch_add(1, Ordering::Relaxed) + 1;
            if shared.max_results > 0 && n > shared.max_results {
                shared.limit_hit.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            if !send_cancellable(&itx, Internal::Hit(entry, matches), &shared.ctl) {
                return WalkState::Quit;
            }
            if shared.max_results > 0 && n == shared.max_results {
                shared.limit_hit.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            WalkState::Continue
        })
    });
}
