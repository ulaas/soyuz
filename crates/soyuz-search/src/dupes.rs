//! Duplicate finder: size → 4 KiB head blake3 → full blake3.
//!
//! 0. Roots are canonicalized: a root equal to or nested in another root (also through
//!    junctions, symlinks, `subst` drives or different spellings) is dropped.
//! 1. One parallel walk over the remaining roots collects regular files ≥
//!    `max(min_size, 1)` (symlinks not followed, each path once).
//! 2. Files are grouped by size; inside each group files are deduplicated by file
//!    identity (`(dev, inode)` / volume + NTFS file id, on every OS), so one file reached
//!    through two names (hardlinks, aliased paths) is never its own duplicate — trashing
//!    "the copy" would otherwise delete the only one. Singletons are dropped.
//! 3. Size groups are processed in parallel (largest size first): blake3 of the first
//!    4 KiB splits each group, then (for files > 4 KiB) a full blake3 confirms it. Every
//!    confirmed group is streamed as a [`DupeEvent::Group`] as soon as it is known.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use ignore::WalkState;
use parking_lot::Mutex;
use rayon::prelude::*;
use soyuz_core::{EntryInfo, EntryKind, FsResult, OpControl};

use crate::walker::{raw_builder, send_cancellable};

/// Bytes hashed in the partial (head) pass.
pub const HEAD_BYTES: usize = 4 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const READ_CHUNK: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub enum DupeEvent {
    Progress {
        hashed: u64,
        total: u64,
        bytes: u64,
    },
    /// One group of identical files.
    Group(Vec<EntryInfo>),
    Done {
        groups: u64,
        wasted_bytes: u64,
    },
}

/// Find duplicate files (≥ `min_size` bytes) under `roots` in the background.
///
/// `Progress::total` is the number of hash operations planned so far (it grows when a
/// head match requires a full hash); `hashed` counts completed ones and `bytes` the bytes
/// read. After a cancel no `Done` is sent and the channel disconnects.
pub fn find_duplicates(
    roots: Vec<PathBuf>,
    min_size: u64,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> (crossbeam_channel::Receiver<DupeEvent>, Arc<OpControl>) {
    let (tx, rx) = crossbeam_channel::bounded(256);
    let ctl = OpControl::new();
    let ctl2 = ctl.clone();
    let spawned = std::thread::Builder::new().name("soyuz-dupes".into()).spawn(move || {
        let job = Job {
            ctl: ctl2,
            tx,
            wake,
            hashed: AtomicU64::new(0),
            total: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            groups: AtomicU64::new(0),
            wasted: AtomicU64::new(0),
            last_progress: Mutex::new(Instant::now()),
        };
        job.run(roots, min_size.max(1));
    });
    if let Err(e) = spawned {
        log::error!("cannot spawn duplicate-finder thread: {e}");
    }
    (rx, ctl)
}

struct Cand {
    path: PathBuf,
    size: u64,
}

struct Job {
    ctl: Arc<OpControl>,
    tx: Sender<DupeEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    hashed: AtomicU64,
    total: AtomicU64,
    bytes: AtomicU64,
    groups: AtomicU64,
    wasted: AtomicU64,
    last_progress: Mutex<Instant>,
}

impl Job {
    fn run(&self, roots: Vec<PathBuf>, min_size: u64) {
        let roots = effective_roots(roots);
        let cands = self.collect(&roots, min_size);
        if self.ctl.is_cancelled() {
            return;
        }
        let mut by_size: HashMap<u64, Vec<Cand>> = HashMap::new();
        for c in cands {
            by_size.entry(c.size).or_default().push(c);
        }
        let mut groups: Vec<(u64, Vec<Cand>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
        groups.sort_by_key(|g| std::cmp::Reverse(g.0));
        self.total.store(groups.iter().map(|g| g.1.len() as u64).sum(), Ordering::Relaxed);
        self.progress(true);
        let work = || {
            groups.par_iter().for_each(|(size, files)| {
                if self.ctl.checkpoint().is_ok() {
                    self.process_size_group(*size, files);
                }
            })
        };
        // A private pool: hashing blocks on I/O and must not starve the global rayon pool.
        match rayon::ThreadPoolBuilder::new().num_threads(crate::walker::default_threads(0)).build() {
            Ok(pool) => pool.install(work),
            Err(_) => work(),
        }
        if self.ctl.is_cancelled() {
            return;
        }
        self.progress(true);
        let done = DupeEvent::Done {
            groups: self.groups.load(Ordering::Relaxed),
            wasted_bytes: self.wasted.load(Ordering::Relaxed),
        };
        if send_cancellable(&self.tx, done, &self.ctl) {
            (self.wake)();
        }
    }

    fn collect(&self, roots: &[PathBuf], min_size: u64) -> Vec<Cand> {
        let Some(builder) = raw_builder(roots, 0) else { return Vec::new() };
        let found: Mutex<Vec<Cand>> = Mutex::default();
        builder.build_parallel().run(|| {
            let found = &found;
            let ctl = &self.ctl;
            Box::new(move |res| {
                if ctl.checkpoint().is_err() {
                    return WalkState::Quit;
                }
                let Ok(dent) = res else { return WalkState::Continue };
                if !dent.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                if let Ok(md) = dent.metadata()
                    && md.is_file()
                    && md.len() >= min_size
                {
                    found.lock().push(Cand { path: dent.into_path(), size: md.len() });
                }
                WalkState::Continue
            })
        });
        let mut paths = HashSet::new();
        found.into_inner().into_iter().filter(|c| paths.insert(c.path.clone())).collect()
    }

    fn process_size_group(&self, size: u64, files: &[Cand]) {
        let unique = dedup_by_identity(files);
        let dropped = (files.len() - unique.len()) as u64;
        if dropped > 0 {
            self.total.fetch_sub(dropped, Ordering::Relaxed);
        }
        if unique.len() < 2 {
            return;
        }
        let files = unique;
        // Head pass.
        let mut by_head: HashMap<blake3::Hash, Vec<&Cand>> = HashMap::new();
        for c in files.iter().copied() {
            if self.ctl.is_cancelled() {
                return;
            }
            if let Ok(h) = self.hash(&c.path, Some(HEAD_BYTES as u64)) {
                by_head.entry(h).or_default().push(c);
            }
        }
        for (_, group) in by_head.into_iter().filter(|(_, v)| v.len() > 1) {
            if size <= HEAD_BYTES as u64 {
                self.emit(size, &group);
                continue;
            }
            self.total.fetch_add(group.len() as u64, Ordering::Relaxed);
            let mut by_full: HashMap<blake3::Hash, Vec<&Cand>> = HashMap::new();
            for c in group {
                if self.ctl.is_cancelled() {
                    return;
                }
                if let Ok(h) = self.hash(&c.path, None) {
                    by_full.entry(h).or_default().push(c);
                }
            }
            for (_, g) in by_full.into_iter().filter(|(_, v)| v.len() > 1) {
                self.emit(size, &g);
            }
        }
    }

    fn emit(&self, size: u64, group: &[&Cand]) {
        let mut entries: Vec<EntryInfo> = group
            .iter()
            .map(|c| {
                soyuz_fs::stat(&c.path, false).unwrap_or_else(|_| {
                    let mut e = EntryInfo::minimal(c.path.clone(), EntryKind::File);
                    e.size = size;
                    e
                })
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let n = entries.len() as u64;
        if send_cancellable(&self.tx, DupeEvent::Group(entries), &self.ctl) {
            self.groups.fetch_add(1, Ordering::Relaxed);
            self.wasted.fetch_add(size * (n - 1), Ordering::Relaxed);
            (self.wake)();
        }
    }

    /// blake3 of the first `limit` bytes (or the whole file).
    fn hash(&self, path: &Path, limit: Option<u64>) -> FsResult<blake3::Hash> {
        let r = hash_file_prefix(path, limit, &self.ctl, &self.bytes);
        self.hashed.fetch_add(1, Ordering::Relaxed);
        self.progress(false);
        r
    }

    fn progress(&self, force: bool) {
        {
            let Some(mut last) = self.last_progress.try_lock() else { return };
            if !force && last.elapsed() < PROGRESS_INTERVAL {
                return;
            }
            *last = Instant::now();
        }
        let ev = DupeEvent::Progress {
            hashed: self.hashed.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        };
        if self.tx.try_send(ev).is_ok() {
            (self.wake)();
        }
    }
}

/// Identity key of a candidate: file identity, else its canonical path, else the path.
#[derive(PartialEq, Eq, Hash)]
enum IdKey {
    Id(soyuz_fs::FileIdentity),
    Path(PathBuf),
}

/// One candidate per filesystem object (first spelling wins).
fn dedup_by_identity(files: &[Cand]) -> Vec<&Cand> {
    let mut seen = HashSet::new();
    files
        .iter()
        .filter(|c| {
            let key = match soyuz_fs::file_identity(&c.path, false) {
                Ok(id) => IdKey::Id(id),
                Err(_) => IdKey::Path(std::fs::canonicalize(&c.path).unwrap_or_else(|_| c.path.clone())),
            };
            seen.insert(key)
        })
        .collect()
}

/// Roots without duplicates and without roots nested in another root, compared after
/// resolving links / aliases (`canonicalize`; a root that cannot be resolved is compared
/// lexically after normalization). Order of the kept roots is preserved.
pub(crate) fn effective_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let canon: Vec<PathBuf> =
        roots.iter().map(|r| std::fs::canonicalize(r).unwrap_or_else(|_| soyuz_fs::normalize(r))).collect();
    // Shortest first, so an outer root is kept before anything nested in it.
    let mut order: Vec<usize> = (0..roots.len()).collect();
    order.sort_by_key(|&i| (canon[i].components().count(), i));
    let mut kept: Vec<usize> = Vec::new();
    for i in order {
        if !kept.iter().any(|&k| canon[i].starts_with(&canon[k])) {
            kept.push(i);
        }
    }
    kept.sort_unstable();
    kept.into_iter().map(|i| roots[i].clone()).collect()
}

pub(crate) fn hash_file_prefix(
    path: &Path,
    limit: Option<u64>,
    ctl: &OpControl,
    bytes: &AtomicU64,
) -> FsResult<blake3::Hash> {
    let io = |e: std::io::Error| soyuz_core::FsError::from_io("hash", Some(path), &e);
    let f = File::open(path).map_err(io)?;
    let mut reader: Box<dyn Read> = match limit {
        Some(n) => Box::new(f.take(n)),
        None => Box::new(f),
    };
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; READ_CHUNK.min(limit.unwrap_or(u64::MAX) as usize)];
    loop {
        ctl.checkpoint()?;
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io(e)),
        };
        hasher.update(&buf[..n]);
        bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(hasher.finalize())
}
