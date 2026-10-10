//! Crash journal for in-flight jobs (`jobs.journal`).
//!
//! While a mutating job runs, the engine keeps a small JSON journal next to the history:
//! the job's kind and options, its label, progress and the undo records it has produced so
//! far (with identity stamps). The file is rewritten atomically (temp + fsync + rename) when
//! a job starts or finishes, when records were added (at the next scheduler tick) and every
//! few seconds for progress; it is removed when nothing is in flight. After a crash or a
//! kill it therefore describes exactly the operations that were interrupted, and the app
//! offers to resume them ([`JobEngine::resume_journaled`]), roll back what they completed
//! ([`JobEngine::rollback_journaled`], an undo of the journaled records) or discard them.
//!
//! # Several instances
//! Every running instance owns one *slot*: `jobs.journal`, `jobs-1.journal`, ... guarded by
//! an OS file lock on `<slot>.lock` that the OS releases when the process dies. At startup
//! [`claim_slot`] takes the first free slot and moves the journal of every slot whose lock
//! is free (its owner is gone) to `jobs-recovered-*.journal`, which [`list_recovered`]
//! reads and nobody writes. A live instance's journal is never touched.
//!
//! # Leftovers
//! [`scan_leftovers`] walks the touched roots of interrupted jobs for temp files the
//! engine and the fs layer create (`.<name>.soyuz-part-*`, `.soyuz-extract-*.part`,
//! `.soyuz-rename-*`, `*.soyuz-join.part`, an archive's `<dest>.part`) so the app can move
//! them to the trash.
//!
//! Archive passwords are never written to the journal.
//!
//! [`JobEngine::resume_journaled`]: crate::JobEngine::resume_journaled
//! [`JobEngine::rollback_journaled`]: crate::JobEngine::rollback_journaled

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use soyuz_core::time::serde_rfc3339;
use soyuz_core::{Control, OpControl};

use crate::engine::JobRt;
use crate::history::UndoStamp;
use crate::job::{JobKind, JobOptions};
use crate::undo::UndoRecord;

/// File format version of `jobs.journal`.
pub const JOURNAL_VERSION: u32 = 1;
/// Base name of slot 0.
pub const JOURNAL_FILE: &str = "jobs.journal";
const RECOVERED_PREFIX: &str = "jobs-recovered-";
const MAX_SLOTS: usize = 64;
/// Progress-only changes are written at most this often.
const PROGRESS_EVERY: Duration = Duration::from_secs(3);

/// One interrupted (or, while running, in-flight) job.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalJob {
    /// Engine job id in the process that wrote it.
    pub id: u64,
    pub label: String,
    pub kind: JobKind,
    pub options: JobOptions,
    #[serde(with = "serde_rfc3339")]
    pub started: SystemTime,
    /// Undo records of the parts that completed (in the order they were done).
    #[serde(default)]
    pub records: Vec<UndoRecord>,
    /// Identities of what `Copied` / `Extracted` / `Created` records produced.
    #[serde(default)]
    pub stamps: Vec<UndoStamp>,
    #[serde(default)]
    pub items_done: u64,
    #[serde(default)]
    pub items_total: u64,
    #[serde(default)]
    pub bytes_done: u64,
    #[serde(default)]
    pub bytes_total: u64,
}

impl JournalJob {
    /// Something to roll back.
    pub fn can_roll_back(&self) -> bool {
        !self.records.is_empty() && !self.records.iter().all(|r| matches!(r, UndoRecord::Irreversible(_)))
    }

    /// Completed fraction (0..=1) when totals are known.
    pub fn fraction(&self) -> Option<f32> {
        if self.bytes_total > 0 {
            Some((self.bytes_done as f64 / self.bytes_total as f64).clamp(0.0, 1.0) as f32)
        } else if self.items_total > 0 {
            Some((self.items_done as f64 / self.items_total as f64).clamp(0.0, 1.0) as f32)
        } else {
            None
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalFileData {
    version: u32,
    pid: u32,
    jobs: Vec<JournalJob>,
}

/// Parse a journal file; unknown / broken files yield `None`.
pub fn read_journal(path: &Path) -> Option<Vec<JournalJob>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<JournalFileData>(&text) {
        Ok(d) if d.version <= JOURNAL_VERSION => Some(d.jobs),
        Ok(d) => {
            log::warn!("soyuz-jobs: {} has journal version {} (newer); ignored", path.display(), d.version);
            None
        }
        Err(e) => {
            log::warn!("soyuz-jobs: unreadable journal {}: {e}", path.display());
            None
        }
    }
}

/// Serialize jobs as a journal file body.
pub(crate) fn to_json(jobs: &[JournalJob]) -> String {
    let data = JournalFileData { version: JOURNAL_VERSION, pid: std::process::id(), jobs: jobs.to_vec() };
    serde_json::to_string(&data).unwrap_or_default()
}

/// Kinds worth journaling (everything that changes the filesystem).
pub(crate) fn journals(kind: &JobKind) -> bool {
    !matches!(kind, JobKind::Hash { .. })
}

/// The kind without secrets (archive passwords).
pub(crate) fn scrub(kind: &JobKind) -> JobKind {
    let mut k = kind.clone();
    match &mut k {
        JobKind::Extract { password, .. } | JobKind::Compress { password, .. } => *password = None,
        _ => {}
    }
    k
}

// ---------------------------------------------------------------------------
// Writer (engine side)
// ---------------------------------------------------------------------------

struct Slot {
    job: JournalJob,
    rt: Arc<JobRt>,
}

/// Lock order: `io` → `jobs` → `JobRt::progress`. Nothing else is acquired while these
/// are held; callers hold no engine lock.
pub(crate) struct JournalWriter {
    path: PathBuf,
    jobs: Mutex<BTreeMap<u64, Slot>>,
    dirty: AtomicBool,
    io: Mutex<Option<Instant>>,
}

impl JournalWriter {
    pub fn new(path: PathBuf) -> Self {
        JournalWriter { path, jobs: Mutex::new(BTreeMap::new()), dirty: AtomicBool::new(false), io: Mutex::new(None) }
    }

    pub fn begin(&self, id: u64, label: String, kind: &JobKind, options: &JobOptions, rt: Arc<JobRt>) {
        let job = JournalJob {
            id,
            label,
            kind: scrub(kind),
            options: options.clone(),
            started: SystemTime::now(),
            records: Vec::new(),
            stamps: Vec::new(),
            items_done: 0,
            items_total: 0,
            bytes_done: 0,
            bytes_total: 0,
        };
        self.jobs.lock().insert(id, Slot { job, rt });
        self.flush();
    }

    pub fn record(&self, id: u64, r: &UndoRecord) {
        // Stamp what the record produced right away (I/O outside the lock).
        let stamp = match r {
            UndoRecord::Copied { dst } | UndoRecord::Extracted { dst } => UndoStamp::take(dst),
            UndoRecord::Created(p) => UndoStamp::take(p),
            _ => None,
        };
        if let Some(s) = self.jobs.lock().get_mut(&id) {
            s.job.records.push(r.clone());
            s.job.stamps.extend(stamp);
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn end(&self, id: u64) {
        let removed = self.jobs.lock().remove(&id).is_some();
        if removed {
            self.flush();
        }
    }

    /// Scheduler tick: write when records changed, or progress is stale.
    pub fn tick(&self) {
        let stale = self.io.lock().is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
        let any = !self.jobs.lock().is_empty();
        if self.dirty.load(Ordering::Acquire) || (any && stale) {
            self.flush();
        }
    }

    /// Rewrite (or remove, when empty) the journal now.
    pub fn flush(&self) {
        let mut io = self.io.lock();
        self.dirty.store(false, Ordering::Release);
        let jobs: Vec<JournalJob> = {
            let mut map = self.jobs.lock();
            map.values_mut()
                .map(|s| {
                    let p = s.rt.progress.lock();
                    s.job.items_done = p.items_done;
                    s.job.items_total = p.items_total;
                    s.job.bytes_done = p.bytes_done;
                    s.job.bytes_total = p.bytes_total;
                    drop(p);
                    s.job.clone()
                })
                .collect()
        };
        *io = Some(Instant::now());
        if jobs.is_empty() {
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => log::warn!("soyuz-jobs: cannot remove {}: {e}", self.path.display()),
            }
            return;
        }
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            let _ = soyuz_fs::create_dir_all(parent);
        }
        if let Err(e) = soyuz_fs::write_atomic(&self.path, to_json(&jobs).as_bytes()) {
            log::warn!("soyuz-jobs: cannot write journal {}: {e}", self.path.display());
        }
    }
}

// ---------------------------------------------------------------------------
// Slots and recovery (app side, at startup)
// ---------------------------------------------------------------------------

/// The journal slot owned by this process (its lock is held until drop / exit).
#[derive(Debug)]
pub struct JournalSlot {
    path: PathBuf,
    _lock: File,
}

impl JournalSlot {
    /// Pass this as `EngineConfig::journal_path`.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn slot_path(dir: &Path, n: usize) -> PathBuf {
    if n == 0 { dir.join(JOURNAL_FILE) } else { dir.join(format!("jobs-{n}.journal")) }
}

fn lock_path(journal: &Path) -> PathBuf {
    let mut s = journal.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

fn try_lock(path: &Path) -> Option<File> {
    let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).ok()?;
    match f.try_lock() {
        Ok(()) => Some(f),
        Err(_) => None,
    }
}

/// Move a dead slot's journal aside so nothing overwrites it before the user decides.
fn adopt(journal: &Path, dir: &Path) {
    if !journal.exists() {
        return;
    }
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let to = dir.join(format!("{RECOVERED_PREFIX}{}-{nanos}.journal", std::process::id()));
    if let Err(e) = std::fs::rename(journal, &to) {
        log::warn!("soyuz-jobs: cannot adopt {}: {e}", journal.display());
    }
}

/// Take the first free journal slot in `dir` and adopt the journals of dead instances.
pub fn claim_slot(dir: &Path) -> std::io::Result<JournalSlot> {
    std::fs::create_dir_all(dir)?;
    let mut mine: Option<JournalSlot> = None;
    for n in 0..MAX_SLOTS {
        let journal = slot_path(dir, n);
        let lock = lock_path(&journal);
        if !lock.exists() && !journal.exists() && mine.is_some() {
            continue;
        }
        let Some(f) = try_lock(&lock) else { continue }; // a live instance owns it
        adopt(&journal, dir);
        if mine.is_none() {
            mine = Some(JournalSlot { path: journal, _lock: f });
        }
        // Other dead slots: adopted; their lock is released when `f` drops here.
    }
    mine.ok_or_else(|| std::io::Error::other("no free journal slot"))
}

/// A journal left behind by an instance that did not shut down cleanly.
#[derive(Clone, Debug)]
pub struct Recovered {
    pub path: PathBuf,
    pub jobs: Vec<JournalJob>,
}

/// Every adopted journal in `dir` (oldest first). Empty / unreadable files are removed.
pub fn list_recovered(dir: &Path) -> Vec<Recovered> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Recovered> = rd
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(RECOVERED_PREFIX) && n.ends_with(".journal")
        })
        .filter_map(|e| {
            let path = e.path();
            match read_journal(&path) {
                Some(jobs) if !jobs.is_empty() => Some(Recovered { path, jobs }),
                _ => {
                    let _ = std::fs::remove_file(&path);
                    None
                }
            }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Forget adopted journals (after the user resumed, rolled back or discarded them).
pub fn dismiss(recovered: &[Recovered]) {
    for r in recovered {
        if let Err(e) = std::fs::remove_file(&r.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("soyuz-jobs: cannot remove {}: {e}", r.path.display());
        }
    }
}

// ---------------------------------------------------------------------------
// Resume
// ---------------------------------------------------------------------------

fn exists(p: &Path) -> bool {
    soyuz_fs::exists_nofollow(p)
}

/// The part of an interrupted job that still makes sense to run: sources that still exist,
/// renames that did not happen yet. `None` when nothing is left. Copy-like kinds are
/// resumed with conflicts pre-answered `Skip` by the engine (what arrived stays).
pub fn resume_kind(kind: &JobKind) -> Option<JobKind> {
    let keep = |v: &[PathBuf]| -> Option<Vec<PathBuf>> {
        let v: Vec<PathBuf> = v.iter().filter(|p| exists(p)).cloned().collect();
        (!v.is_empty()).then_some(v)
    };
    Some(match kind {
        JobKind::Copy { sources, dest_dir, rename_to } => {
            JobKind::Copy { sources: keep(sources)?, dest_dir: dest_dir.clone(), rename_to: rename_to.clone() }
        }
        JobKind::Move { sources, dest_dir, rename_to } => {
            JobKind::Move { sources: keep(sources)?, dest_dir: dest_dir.clone(), rename_to: rename_to.clone() }
        }
        JobKind::Delete { paths, mode } => JobKind::Delete { paths: keep(paths)?, mode: *mode },
        JobKind::SetMeta { paths, times, attrs, mode } => {
            JobKind::SetMeta { paths: keep(paths)?, times: *times, attrs: *attrs, mode: *mode }
        }
        JobKind::Rename { pairs } => {
            let p: Vec<_> = pairs.iter().filter(|(a, b)| a != b && exists(a) && !exists(b)).cloned().collect();
            if p.is_empty() {
                return None;
            }
            JobKind::Rename { pairs: p }
        }
        JobKind::Create { path, .. } if exists(path) => return None,
        JobKind::Extract { archive, .. } if !exists(archive) => return None,
        JobKind::Compress { sources, dest, format, level, password } => JobKind::Compress {
            sources: keep(sources)?,
            dest: dest.clone(),
            format: *format,
            level: *level,
            password: password.clone(),
        },
        JobKind::Hash { .. } => return None,
        other => other.clone(),
    })
}

// ---------------------------------------------------------------------------
// Leftover scan
// ---------------------------------------------------------------------------

/// A temp file name produced by the copy engine, the extractor, the rename scheduler, the
/// case-only rename or the file joiner. (The process id inside some of these names is not
/// used to tell live files apart: Windows reuses ids right away. The scan runs before this
/// process starts any job, and a file another instance still writes is open without
/// delete sharing, so moving it to the trash simply fails.)
pub fn is_leftover_name(name: &str) -> bool {
    name.contains(soyuz_fs::copy::PART_MARKER)
        || (name.starts_with(".soyuz-extract-") && name.ends_with(".part"))
        || name.starts_with(".soyuz-rename-")
        || name.ends_with(".soyuz-join.part")
}

/// Folders to scan for one job: directory roots recursively, the parents of file roots
/// one level deep.
fn scan_roots(kind: &JobKind) -> Vec<(PathBuf, bool)> {
    let mut out: Vec<(PathBuf, bool)> = Vec::new();
    for root in kind.touched_roots() {
        if crate::job::is_synthetic_root(&root) {
            continue;
        }
        if root.is_dir() {
            out.push((root.clone(), true));
        }
        if let Some(parent) = root.parent().filter(|p| !p.as_os_str().is_empty()) {
            out.push((parent.to_path_buf(), false));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Leftover temp files under the touched roots of `jobs` (at most `limit` entries visited,
/// stops when `ctl` is cancelled). Also includes an interrupted archive's `<dest>.part`.
pub fn scan_leftovers(jobs: &[JournalJob], ctl: &OpControl, limit: usize) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut visited = 0usize;
    let push = |p: PathBuf, found: &mut Vec<PathBuf>| {
        if !found.contains(&p) {
            found.push(p);
        }
    };
    for job in jobs {
        if let JobKind::Compress { dest, .. } = &job.kind {
            let mut part = dest.as_os_str().to_owned();
            part.push(".part");
            let part = PathBuf::from(part);
            if exists(&part) {
                push(part, &mut found);
            }
        }
        for (root, deep) in scan_roots(&job.kind) {
            if ctl.is_cancelled() || visited >= limit {
                return found;
            }
            let opts = soyuz_fs::WalkOptions {
                max_depth: Some(if deep { 64 } else { 1 }),
                ..soyuz_fs::WalkOptions::default()
            };
            let _ = soyuz_fs::walk(&root, &opts, ctl, &mut |item| {
                visited += 1;
                if let soyuz_fs::WalkItem::Entry { entry, depth } = item
                    && depth > 0
                    && is_leftover_name(&entry.name)
                {
                    push(entry.path, &mut found);
                }
                if visited >= limit { Control::Stop } else { Control::Continue }
            });
        }
    }
    // A leftover folder (parked directory) also matched its children: keep the top ones.
    let tops: Vec<PathBuf> =
        found.iter().filter(|p| !found.iter().any(|q| q != *p && p.starts_with(q))).cloned().collect();
    tops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::DeleteMode;

    fn job(kind: JobKind) -> JournalJob {
        JournalJob {
            id: 1,
            label: "x".into(),
            kind,
            options: JobOptions::default(),
            started: SystemTime::UNIX_EPOCH,
            records: Vec::new(),
            stamps: Vec::new(),
            items_done: 1,
            items_total: 4,
            bytes_done: 0,
            bytes_total: 0,
        }
    }

    #[test]
    fn journal_round_trip_and_version() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(JOURNAL_FILE);
        let mut j = job(JobKind::Delete { paths: vec!["/a".into()], mode: DeleteMode::Trash });
        j.records.push(UndoRecord::Created("/b".into()));
        std::fs::write(&p, to_json(std::slice::from_ref(&j))).unwrap();
        let back = read_journal(&p).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!((&back[0].kind, &back[0].records, back[0].items_total), (&j.kind, &j.records, 4));
        assert_eq!(j.fraction(), Some(0.25));
        assert!(j.can_roll_back());
        std::fs::write(&p, r#"{"version":99,"pid":1,"jobs":[]}"#).unwrap();
        assert!(read_journal(&p).is_none());
        std::fs::write(&p, "garbage").unwrap();
        assert!(read_journal(&p).is_none());
        assert!(read_journal(&d.path().join("missing")).is_none());
    }

    #[test]
    fn passwords_are_scrubbed() {
        let k = JobKind::Extract {
            archive: "/a.zip".into(),
            entries: None,
            dest_dir: "/d".into(),
            password: Some("hunter2".into()),
            strip_components: 0,
        };
        let s = serde_json::to_string(&scrub(&k)).unwrap();
        assert!(!s.contains("hunter2"));
    }

    #[test]
    fn slots_lock_and_adopt_dead_journals() {
        let d = tempfile::tempdir().unwrap();
        // A crashed instance left slot 0's journal (no live lock).
        std::fs::write(d.path().join(JOURNAL_FILE), to_json(&[job(JobKind::EmptyTrash)])).unwrap();
        let a = claim_slot(d.path()).unwrap();
        assert_eq!(a.path(), d.path().join(JOURNAL_FILE));
        assert!(!a.path().exists(), "adopted before we write");
        let rec = list_recovered(d.path());
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].jobs[0].kind, JobKind::EmptyTrash);
        // A second live instance gets slot 1 and never touches slot 0.
        std::fs::write(a.path(), to_json(&[job(JobKind::EmptyTrash)])).unwrap();
        let b = claim_slot(d.path()).unwrap();
        assert_eq!(b.path(), d.path().join("jobs-1.journal"));
        assert!(a.path().exists(), "live journal untouched");
        assert_eq!(list_recovered(d.path()).len(), 1);
        // `a` dies: the next instance adopts its journal.
        drop(a);
        let c = claim_slot(d.path()).unwrap();
        assert_eq!(c.path(), d.path().join(JOURNAL_FILE));
        let rec = list_recovered(d.path());
        assert_eq!(rec.len(), 2);
        dismiss(&rec);
        assert!(list_recovered(d.path()).is_empty());
        drop((b, c));
    }

    #[test]
    fn leftover_names() {
        assert!(is_leftover_name(".a.txt.soyuz-part-12-3"));
        assert!(is_leftover_name(".soyuz-extract-4-1.part"));
        assert!(is_leftover_name(".soyuz-rename-7-1-123"));
        assert!(is_leftover_name("big.iso.soyuz-join.part"));
        assert!(!is_leftover_name("report.part"));
        assert!(!is_leftover_name("soyuz-part.txt"));
    }

    #[test]
    fn scan_finds_leftovers_under_roots_only() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        let dst = d.path().join("dst");
        let other = d.path().join("other");
        for p in [&src, &dst.join("deep/er"), &other] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(src.join("a.txt"), b"a").unwrap();
        std::fs::write(dst.join("deep/er/.a.txt.soyuz-part-999999999-0"), b"partial").unwrap();
        std::fs::write(dst.join(".soyuz-extract-999999999-1.part"), b"p").unwrap();
        std::fs::write(other.join(".b.soyuz-part-999999999-0"), b"not ours to scan").unwrap();
        std::fs::create_dir(src.join(".soyuz-rename-5-1-1")).unwrap();
        std::fs::write(src.join(".soyuz-rename-5-1-1/inner"), b"x").unwrap();
        let j = job(JobKind::Copy { sources: vec![src.join("a.txt")], dest_dir: dst.clone(), rename_to: None });
        let mut found = scan_leftovers(&[j], &OpControl::new(), 10_000);
        found.sort();
        let mut want = vec![
            dst.join("deep/er/.a.txt.soyuz-part-999999999-0"),
            dst.join(".soyuz-extract-999999999-1.part"),
            src.join(".soyuz-rename-5-1-1"),
        ];
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn resume_keeps_what_is_left() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        std::fs::write(&a, b"a").unwrap();
        let k = JobKind::Copy { sources: vec![a.clone(), b.clone()], dest_dir: d.path().join("d"), rename_to: None };
        assert_eq!(
            resume_kind(&k),
            Some(JobKind::Copy { sources: vec![a.clone()], dest_dir: d.path().join("d"), rename_to: None })
        );
        let gone = JobKind::Delete { paths: vec![b.clone()], mode: DeleteMode::Trash };
        assert_eq!(resume_kind(&gone), None);
        let r = JobKind::Rename { pairs: vec![(a.clone(), b.clone()), (d.path().join("x"), d.path().join("y"))] };
        assert_eq!(resume_kind(&r), Some(JobKind::Rename { pairs: vec![(a.clone(), b.clone())] }));
        assert_eq!(resume_kind(&JobKind::Create { path: a, kind: crate::CreateKind::Dir }), None);
    }
}
