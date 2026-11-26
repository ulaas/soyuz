//! Worker-side job context: progress, log, journal, prompts and error policy.
//!
//! A `JobCtx` lives on its worker thread; it is `Sync` so it can be borrowed by progress
//! sinks and archive `ExtractSink`s. It never holds the engine lock while doing I/O or
//! while blocked on a prompt (see the lock ordering in [`crate::engine`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use soyuz_core::{ByteProgress, ConflictResolution, ErrorAction, FsError, FsErrorKind, FsResult, OpControl};
use soyuz_fs::FsProbe;

use crate::engine::{GonePolicy, Inner, JobRt};
use crate::job::{JobId, JobOptions, JobState, JobSummary, LogLevel, LogLine, MAX_SUMMARY_ERRORS};
use crate::policy::{self, PolicyMemory};
use crate::progress::Progress;
use crate::prompt::{ConflictInfo, InUse, JobPrompt, PromptResponse};
use crate::undo::UndoRecord;

/// Automatic retries for `JobOptions::errors == Some(Retry)` before asking the user.
const AUTO_RETRIES: u32 = 3;
/// How long a program that was asked to close gets before the user is asked again.
const CLOSE_WAIT: Duration = Duration::from_secs(4);
/// How long an ended program gets to disappear.
const END_WAIT: Duration = Duration::from_secs(3);
/// After the last program is gone: the system closes its handles a moment later.
const HANDLES_SETTLE: Duration = Duration::from_millis(60);

/// What a job produced (summary + undo journal).
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub summary: JobSummary,
    pub records: Vec<UndoRecord>,
    /// Undo jobs: the records that were actually reversed.
    pub undone: Vec<UndoRecord>,
    /// Identities of what the records produced (filled by the worker after the job).
    pub stamps: Vec<crate::history::UndoStamp>,
}

pub(crate) struct JobCtx {
    pub id: JobId,
    pub inner: Arc<Inner>,
    pub rt: Arc<JobRt>,
    pub opts: JobOptions,
    preset: PolicyMemory,
    own_memory: Mutex<PolicyMemory>,
    out: Mutex<Outcome>,
    started: Instant,
}

impl JobCtx {
    pub fn new(id: JobId, inner: Arc<Inner>, rt: Arc<JobRt>, opts: JobOptions) -> Self {
        let preset = PolicyMemory::preset(opts.conflict.clone(), opts.errors, opts.copy.overwrite);
        JobCtx {
            id,
            inner,
            rt,
            opts,
            preset,
            own_memory: Mutex::new(PolicyMemory::default()),
            out: Mutex::new(Outcome::default()),
            started: Instant::now(),
        }
    }

    pub fn ctl(&self) -> &OpControl {
        &self.rt.ctl
    }

    pub fn checkpoint(&self) -> FsResult<()> {
        self.rt.ctl.checkpoint()
    }

    pub fn probe(&self) -> &dyn FsProbe {
        &*self.inner.probe
    }

    /// Cancel the job (e.g. the user picked "Cancel" in a dialog) and return the error to propagate.
    pub fn cancel_now(&self) -> FsError {
        self.rt.ctl.cancel();
        FsError::interrupted("job")
    }

    pub fn take_outcome(&self) -> Outcome {
        std::mem::take(&mut *self.out.lock())
    }

    // ----- state / progress -------------------------------------------------------

    pub fn set_running(&self) {
        self.inner.worker_state(self.id, JobState::Running);
    }

    pub fn set_finishing(&self) {
        self.inner.worker_state(self.id, JobState::Finishing);
    }

    pub fn progress<R>(&self, f: impl FnOnce(&mut Progress) -> R) -> R {
        f(&mut self.rt.progress.lock())
    }

    pub fn set_totals(&self, bytes: u64, items: u64) {
        self.progress(|p| {
            p.bytes_total = bytes;
            p.items_total = items;
            p.totals_known = true;
        });
    }

    /// Items discovered while running (e.g. a rename that turned into a cross-device copy).
    pub fn grow_totals(&self, bytes: u64, items: u64) {
        self.progress(|p| {
            p.bytes_total += bytes;
            p.items_total += items;
        });
    }

    pub fn begin_item(&self, path: &Path, total: u64) {
        self.progress(|p| {
            p.current = Some(path.to_path_buf());
            p.current_bytes = 0;
            p.current_total = total;
        });
    }

    /// Byte progress of the current item.
    pub fn on_bytes(&self, bp: ByteProgress) {
        self.progress(|p| {
            p.bytes_done += bp.delta;
            p.current_bytes = bp.done;
            if bp.total > 0 {
                p.current_total = bp.total;
            }
            if p.bytes_done > p.bytes_total {
                p.bytes_total = p.bytes_done;
            }
        });
    }

    /// Credit bytes without a transfer (skipped items) or take them back (`-n`, failed attempts).
    pub fn credit_bytes(&self, n: i64) {
        self.progress(|p| {
            p.bytes_done = if n >= 0 { p.bytes_done + n as u64 } else { p.bytes_done.saturating_sub(n.unsigned_abs()) };
            if p.bytes_done > p.bytes_total {
                p.bytes_total = p.bytes_done;
            }
        });
    }

    fn bump_items(&self, n: u64) {
        self.progress(|p| {
            p.items_done += n;
            if p.items_done > p.items_total {
                p.items_total = p.items_done;
            }
        });
    }

    pub fn item_ok(&self, bytes: u64) {
        self.items_ok(1, bytes);
    }

    pub fn items_ok(&self, n: u64, bytes: u64) {
        self.bump_items(n);
        let mut out = self.out.lock();
        out.summary.items_ok += n;
        out.summary.bytes += bytes;
    }

    pub fn item_skipped(&self, path: &Path, credit: u64, why: &str) {
        self.bump_items(1);
        self.credit_bytes(credit as i64);
        self.out.lock().summary.items_skipped += 1;
        self.log(LogLevel::Info, Some(path.to_path_buf()), format!("skipped: {why}"));
    }

    pub fn item_failed(&self, path: &Path, err: FsError, credit: u64) {
        self.bump_items(1);
        self.credit_bytes(credit as i64);
        self.log(LogLevel::Error, Some(path.to_path_buf()), err.to_string());
        let mut out = self.out.lock();
        out.summary.items_failed += 1;
        if out.summary.errors.len() < MAX_SUMMARY_ERRORS {
            out.summary.errors.push((path.to_path_buf(), err));
        }
    }

    /// Journal an undo record (only when `JobOptions::journal`).
    pub fn record(&self, r: UndoRecord) {
        if self.opts.journal {
            if let Some(j) = &self.inner.journal {
                j.record(self.id.0, &r);
            }
            self.out.lock().records.push(r);
        }
    }

    /// Undo jobs: `r` was reversed (the engine drops it from a partially undone entry).
    pub fn mark_undone(&self, r: UndoRecord) {
        self.out.lock().undone.push(r);
    }

    pub fn note(&self, s: impl Into<String>) {
        let s = s.into();
        let mut out = self.out.lock();
        if !out.summary.notes.contains(&s) {
            out.summary.notes.push(s);
        }
    }

    pub fn summary_counts(&self) -> (u64, u64) {
        let out = self.out.lock();
        (out.summary.items_failed, out.summary.items_skipped)
    }

    pub fn log(&self, level: LogLevel, path: Option<PathBuf>, msg: String) {
        let line = LogLine { at: self.started.elapsed(), level, path, msg };
        self.inner.push_log(self.id, line);
    }

    // ----- prompts --------------------------------------------------------------

    fn memory<R>(&self, f: impl FnOnce(&mut PolicyMemory) -> R) -> R {
        if self.inner.live.lock().reset_apply_all_between_jobs {
            f(&mut self.own_memory.lock())
        } else {
            f(&mut self.inner.shared_policy.lock())
        }
    }

    /// Publish `p` and block until the UI answers or the job is cancelled
    /// (`PromptResponse::Cancelled`). Holds no lock while blocked.
    pub fn prompt(&self, p: JobPrompt) -> PromptResponse {
        // The item the question is about, when it exists now: should it disappear while
        // the job waits, nobody can answer any more (`QueueSettings::item_gone`).
        let watched = prompt_item(&p).filter(|i| soyuz_fs::exists_nofollow(i)).map(Path::to_path_buf);
        let skip = skip_answer(&p);
        if !self.inner.begin_prompt(self.id, p) {
            return PromptResponse::Cancelled;
        }
        let r = loop {
            match self.rt.prompt_rx.recv_timeout(self.inner.cfg.gone_poll) {
                Ok(r) => break r,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break PromptResponse::Cancelled,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
            let Some(item) = watched.as_deref().filter(|i| is_gone(i)) else { continue };
            let policy = self.inner.live.lock().item_gone;
            let (what, answer) = match policy {
                GonePolicy::Wait => continue,
                GonePolicy::Skip => ("skipped", skip.clone()),
                GonePolicy::Cancel => ("cancelled", None),
            };
            self.log(LogLevel::Warn, Some(item.to_path_buf()), format!("{what}: the item no longer exists"));
            match answer {
                Some(a) => break a,
                None => {
                    self.note("Cancelled: the item it was waiting for no longer exists.");
                    self.rt.ctl.cancel();
                    break PromptResponse::Cancelled;
                }
            }
        };
        self.inner.end_prompt(self.id);
        if self.rt.ctl.is_cancelled() { PromptResponse::Cancelled } else { r }
    }

    /// Pre-answer, remembered "apply to all" answer, or ask. `Err` = cancelled.
    pub fn resolve_conflict(&self, info: ConflictInfo) -> FsResult<ConflictResolution> {
        let remembered = self.preset.conflict.clone().or_else(|| self.memory(|m| m.conflict.clone()));
        let r = match remembered {
            Some(r) => r,
            None => match self.prompt(JobPrompt::Conflict(info)) {
                PromptResponse::Conflict { resolution, apply_to_all } => {
                    if apply_to_all {
                        let all = policy::normalize_conflict_for_all(resolution.clone());
                        self.memory(|m| m.conflict = Some(all));
                    }
                    resolution
                }
                PromptResponse::Cancelled => return Err(self.cancel_now()),
                _ => ConflictResolution::Skip,
            },
        };
        if r == ConflictResolution::Cancel {
            return Err(self.cancel_now());
        }
        Ok(r)
    }

    /// Error policy for one failure: `Ok(Retry | Skip)`, or `Err` to abort (the error
    /// itself) / cancel (`Interrupted`).
    pub fn on_error(&self, path: &Path, err: &FsError) -> FsResult<ErrorAction> {
        if self.rt.ctl.is_cancelled() {
            return Err(FsError::interrupted("job"));
        }
        let remembered = self.preset.error.or_else(|| self.memory(|m| m.error));
        match remembered {
            Some(ErrorAction::Abort) => return Err(err.clone()),
            Some(_) => return Ok(ErrorAction::Skip),
            None => {}
        }
        let mut still_open = false;
        loop {
            let in_use = self.in_use(path, err).map(|u| InUse { still_open, ..u });
            // An item in use can be tried again once its program lets go of it.
            let retryable = err.is_retryable() || in_use.is_some();
            let p =
                JobPrompt::Error { path: path.to_path_buf(), error: err.clone(), retryable, in_use: in_use.clone() };
            return match self.prompt(p) {
                PromptResponse::Error { action, apply_to_all } => {
                    if (apply_to_all || action == ErrorAction::SkipAll)
                        && let Some(m) = policy::remembered_error(action)
                    {
                        self.memory(|mem| mem.error = Some(m));
                    }
                    match action {
                        ErrorAction::Retry => Ok(ErrorAction::Retry),
                        ErrorAction::Skip | ErrorAction::SkipAll => Ok(ErrorAction::Skip),
                        ErrorAction::Abort => Err(err.clone()),
                    }
                }
                PromptResponse::CloseBlockers { force } => match in_use {
                    Some(u) if !self.close_blockers(&u, force)? => {
                        // Still running: say so, with what is open now.
                        still_open = true;
                        continue;
                    }
                    _ => Ok(ErrorAction::Retry),
                },
                PromptResponse::Cancelled => Err(self.cancel_now()),
                _ => Ok(ErrorAction::Skip),
            };
        }
    }

    /// The programs that keep the failed item in use, when `err` is the kind of refusal
    /// that causes (`[jobs] show_blocking_programs`).
    fn in_use(&self, path: &Path, err: &FsError) -> Option<InUse> {
        let live = *self.inner.live.lock();
        if !live.find_blockers || !matches!(err.kind, FsErrorKind::PermissionDenied | FsErrorKind::InUse) {
            return None;
        }
        // The path the error names (a destination, an item inside a folder) is the one
        // that was refused.
        let at = err.path.as_deref().unwrap_or(path);
        let found = soyuz_fs::in_use::blockers(at, soyuz_fs::in_use::Limits::default());
        (!found.list.is_empty()).then_some(InUse {
            blockers: found.list,
            complete: found.complete,
            may_end: live.end_blockers,
            still_open: false,
        })
    }

    /// Ask the programs of `u` to close (`force`: end them, when the settings allow it)
    /// and wait for them to go. `Ok(true)` = all are gone, the item is worth another try.
    fn close_blockers(&self, u: &InUse, force: bool) -> FsResult<bool> {
        let force = force && u.may_end;
        let mut asked = Vec::new();
        for b in u.closable() {
            match soyuz_fs::in_use::close(b, force) {
                Ok(()) => {
                    let what = if force { "ended" } else { "asked to close" };
                    self.log(LogLevel::Warn, None, format!("{what} {} (PID {})", b.name, b.pid));
                    asked.push(b);
                }
                Err(e) => self.log(LogLevel::Warn, None, format!("could not close {} (PID {}): {e}", b.name, b.pid)),
            }
        }
        let deadline = Instant::now() + if force { END_WAIT } else { CLOSE_WAIT };
        while asked.iter().any(|b| soyuz_fs::in_use::is_running(b)) {
            self.checkpoint()?;
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(HANDLES_SETTLE);
        // Programs that could not be asked (ours, the system's) still hold the item.
        Ok(asked.len() == u.blockers.len())
    }

    /// Run `op` under the error policy. Outer `Err` = abort/cancel the job; inner `Err` =
    /// the user skipped this item (the caller accounts for it).
    pub fn attempt<T>(&self, path: &Path, mut op: impl FnMut() -> FsResult<T>) -> FsResult<Result<T, FsError>> {
        let mut auto = 0;
        loop {
            self.checkpoint()?;
            match op() {
                Ok(v) => return Ok(Ok(v)),
                Err(e) if self.rt.ctl.is_cancelled() => return Err(e),
                Err(e) => {
                    if self.opts.errors == Some(ErrorAction::Retry) && auto < AUTO_RETRIES {
                        auto += 1;
                        self.log(LogLevel::Warn, Some(path.to_path_buf()), format!("{e}; retrying ({auto})"));
                        std::thread::sleep(Duration::from_millis(50 * auto as u64));
                        continue;
                    }
                    match self.on_error(path, &e)? {
                        ErrorAction::Retry => continue,
                        _ => return Ok(Err(e)),
                    }
                }
            }
        }
    }

    /// Read-only item in the way: `Ok(true)` = clear the flag and continue.
    pub fn read_only_force(&self, path: &Path) -> FsResult<bool> {
        if let Some(b) = self.memory(|m| m.read_only) {
            return Ok(b);
        }
        match self.prompt(JobPrompt::ReadOnlyItem { path: path.to_path_buf() }) {
            PromptResponse::ReadOnly { force, apply_to_all } => {
                if apply_to_all {
                    self.memory(|m| m.read_only = Some(force));
                }
                Ok(force)
            }
            PromptResponse::Cancelled => Err(self.cancel_now()),
            _ => Ok(false),
        }
    }

    /// Trash unavailable for `path`: `Ok(true)` = delete permanently, `Ok(false)` = skip.
    pub fn trash_unsupported(&self, path: &Path, reason: &str) -> FsResult<bool> {
        if let Some(b) = self.memory(|m| m.trash_unsupported) {
            return Ok(b);
        }
        let p = JobPrompt::TrashUnsupported { path: path.to_path_buf(), reason: reason.to_string() };
        match self.prompt(p) {
            PromptResponse::TrashUnsupported { permanent, apply_to_all } => {
                if apply_to_all {
                    self.memory(|m| m.trash_unsupported = Some(permanent));
                }
                Ok(permanent)
            }
            PromptResponse::Cancelled => Err(self.cancel_now()),
            _ => Ok(false),
        }
    }

    /// Free-space check via `FsProbe::space`; an error (incl. `Unsupported`) means "unknown"
    /// and is not a failure. Not enough space asks; declining cancels the job.
    pub fn check_space(&self, needed: u64, dest: &Path) -> FsResult<()> {
        if needed == 0 {
            return Ok(());
        }
        let probe_at = nearest_existing(dest);
        let Ok((_, free)) = self.probe().space(&probe_at) else { return Ok(()) };
        if needed <= free {
            return Ok(());
        }
        self.log(LogLevel::Warn, Some(dest.to_path_buf()), format!("needs {needed} bytes, {free} free"));
        match self.prompt(JobPrompt::NotEnoughSpace { needed, free, dest: dest.to_path_buf() }) {
            PromptResponse::Space { proceed: true } => Ok(()),
            _ => Err(self.cancel_now()),
        }
    }
}

/// The local item a prompt asks about (`None`: nothing that can disappear).
fn prompt_item(p: &JobPrompt) -> Option<&Path> {
    match p {
        JobPrompt::Conflict(c) => Some(&c.src.path),
        JobPrompt::Error { path, .. } | JobPrompt::TrashUnsupported { path, .. } | JobPrompt::ReadOnlyItem { path } => {
            Some(path)
        }
        JobPrompt::Password { archive, .. } => Some(archive),
        JobPrompt::NotEnoughSpace { .. } => None,
    }
}

/// The answer that leaves a prompt's item alone and goes on (`None`: there is none).
fn skip_answer(p: &JobPrompt) -> Option<PromptResponse> {
    Some(match p {
        JobPrompt::Conflict(_) => {
            PromptResponse::Conflict { resolution: ConflictResolution::Skip, apply_to_all: false }
        }
        JobPrompt::Error { .. } => PromptResponse::Error { action: ErrorAction::Skip, apply_to_all: false },
        JobPrompt::TrashUnsupported { .. } => {
            PromptResponse::TrashUnsupported { permanent: false, apply_to_all: false }
        }
        JobPrompt::ReadOnlyItem { .. } => PromptResponse::ReadOnly { force: false, apply_to_all: false },
        JobPrompt::Password { .. } => PromptResponse::Password(None),
        JobPrompt::NotEnoughSpace { .. } => return None,
    })
}

/// `item` is gone while the place it was in is still there: an unplugged drive or a
/// network share that went away does not count, its items may well come back.
fn is_gone(item: &Path) -> bool {
    !soyuz_fs::exists_nofollow(item) && item.parent().is_some_and(soyuz_fs::exists_nofollow)
}

/// `p` or its nearest existing ancestor.
pub(crate) fn nearest_existing(p: &Path) -> PathBuf {
    let mut cur = Some(p);
    while let Some(c) = cur {
        if soyuz_fs::exists_nofollow(c) {
            return c.to_path_buf();
        }
        cur = c.parent();
    }
    p.to_path_buf()
}
