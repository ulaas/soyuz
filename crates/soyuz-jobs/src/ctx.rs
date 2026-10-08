//! Worker-side job context: progress, log, journal, prompts and error policy.
//!
//! A `JobCtx` lives on its worker thread; it is `Sync` so it can be borrowed by progress
//! sinks and archive `ExtractSink`s. It never holds the engine lock while doing I/O or
//! while blocked on a prompt (see the lock ordering in [`crate::engine`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use soyuz_core::{ByteProgress, ConflictResolution, ErrorAction, FsError, FsResult, OpControl};
use soyuz_fs::FsProbe;

use crate::engine::{Inner, JobRt};
use crate::job::{JobId, JobOptions, JobState, JobSummary, LogLevel, LogLine, MAX_SUMMARY_ERRORS};
use crate::policy::{self, PolicyMemory};
use crate::progress::Progress;
use crate::prompt::{ConflictInfo, JobPrompt, PromptResponse};
use crate::undo::UndoRecord;

/// Automatic retries for `JobOptions::errors == Some(Retry)` before asking the user.
const AUTO_RETRIES: u32 = 3;

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
        if self.inner.cfg.reset_apply_all_between_jobs {
            f(&mut self.own_memory.lock())
        } else {
            f(&mut self.inner.shared_policy.lock())
        }
    }

    /// Publish `p` and block until the UI answers or the job is cancelled
    /// (`PromptResponse::Cancelled`). Holds no lock while blocked.
    pub fn prompt(&self, p: JobPrompt) -> PromptResponse {
        if !self.inner.begin_prompt(self.id, p) {
            return PromptResponse::Cancelled;
        }
        let r = self.rt.prompt_rx.recv().unwrap_or(PromptResponse::Cancelled);
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
        let p = JobPrompt::Error { path: path.to_path_buf(), error: err.clone(), retryable: err.is_retryable() };
        match self.prompt(p) {
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
            PromptResponse::Cancelled => Err(self.cancel_now()),
            _ => Ok(ErrorAction::Skip),
        }
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
