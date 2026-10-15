//! The job engine (the threading rules below are binding for the implementation).
//!
//! # Threads
//! * one scheduler/sampler thread (`soyuz-jobs-sched`): admits queued jobs, computes
//!   per-device lanes (probe I/O happens outside every lock), samples speeds every
//!   `sample_interval`, reaps finished worker threads. It is woken early ("kicked") by
//!   submit / resume / finish / reorder so admission does not wait for a tick.
//! * one worker thread per admitted job (`soyuz-job-<id>`), see `ctx.rs`.
//!
//! # Lock ordering (deadlock freedom)
//! Locks are only ever acquired in this order; a thread holding a later lock never
//! acquires an earlier one:
//!
//! 1. `Inner::history_io` (held while writing `history.jsonl`; never by a UI-thread call
//!    path except `shutdown`, which holds nothing else when taking it)
//! 2. `Inner::state` (the single engine mutex)
//! 3. `JobRt::progress` (per job; writers hold it only to write fields)
//!
//! Leaf locks (nothing is acquired while they are held): `Inner::shared_policy`,
//! `Inner::workers`, `JobCtx::{own_memory, out}`, `JobEngine::scheduler`, and the pause
//! condvar mutex inside `OpControl` (pause/resume/cancel may be called under `state`;
//! a worker parked in `checkpoint()` holds no engine lock).
//!
//! # Semantics decided here (beyond the threading rules)
//! * `has_active()` = any job that is not terminal (queued, paused, waiting included).
//! * Only *active* jobs occupy a lane; `touched_roots` are held by every started,
//!   unfinished job (see `scheduler.rs`). `resume()` continues immediately even if
//!   that temporarily exceeds `max_parallel` (the worker already exists).
//! * `cancel()` of a paused / waiting job shows `Finishing` until its worker exits
//!   (milliseconds), then `Cancelled`.
//! * History: only `Done` jobs with `journal == true` create entries; undo jobs never do.
//!   An undo reports every record it reversed; when all records of the entry are reversed
//!   it is marked `undone`, otherwise (some failed, were refused or the undo was cancelled)
//!   exactly the reversed records are dropped from the entry, so the next undo never
//!   replays them. `redo()` re-runs the original kind with the original options and
//!   updates the same entry in place. `retry_failed()` re-submits the failed sources /
//!   paths / pairs with the original options (copy, move and extract pre-answer conflicts
//!   with `Skip`). Entries record `UndoStamp`s of what they created (taken on the worker
//!   after the job) and whether they come from this session.
//! * `log(id, from)` uses absolute line indices (`JobSnapshot::log_len` = next index).
//!
//! No channel send, no `wake()` and no filesystem I/O happens while `state` is held:
//! `Inner::update` collects events under the lock and emits them after unlocking.
//! The only blocking waits are a worker on its private prompt channel (holding no
//! lock) and `OpControl::checkpoint` (holding no engine lock).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::{Condvar, Mutex};
use soyuz_core::{DeviceId, FsError, FsErrorKind, OpControl};
use soyuz_fs::FsProbe;

use crate::history::{self, HistoryEntry, HistoryId, UndoStamp};
use crate::job::{
    JobId, JobKind, JobKindBrief, JobOptions, JobSpec, JobState, JobSummary, LogLevel, LogLine, Priority, QueueMode,
};
use crate::policy::PolicyMemory;
use crate::progress::{JobSnapshot, Progress};
use crate::prompt::{JobPrompt, PromptResponse};
use crate::undo::UndoRecord;

/// Discrete transitions only (no per-chunk progress: the UI polls `snapshots()`).
#[derive(Clone, Debug)]
pub enum JobEvent {
    Added(JobId),
    StateChanged(JobId, JobState),
    Prompt(JobId),
    Finished(JobId, JobSummary),
    Removed(JobId),
    HistoryChanged,
    Log(JobId, LogLine),
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Jobs in `Planning | Running | Finishing` at once (default 4).
    pub max_parallel: usize,
    pub default_queue: QueueMode,
    pub reset_apply_all_between_jobs: bool,
    /// Speed-sample / scheduler tick (default 500 ms).
    pub sample_interval: Duration,
    pub history_limit: usize,
    /// `history.jsonl`; `None` = in-memory only.
    pub history_path: Option<PathBuf>,
    /// Per-job log ring size (default 1000).
    pub log_lines_per_job: usize,
    /// Crash journal of in-flight jobs (`jobs.journal`, see [`crate::journal`]); `None` = off.
    pub journal_path: Option<PathBuf>,
    /// Folders `Media` jobs may load the FFmpeg libraries from when none are loaded yet
    /// (`soyuz_media::search_dirs`); empty = only an already loaded runtime is used
    /// (`[media] ffmpeg = false`, or a resumed job before the app loaded them).
    pub media_dirs: Vec<PathBuf>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            max_parallel: 4,
            default_queue: QueueMode::PerDevice,
            reset_apply_all_between_jobs: true,
            sample_interval: Duration::from_millis(500),
            history_limit: 500,
            history_path: None,
            log_lines_per_job: 1000,
            journal_path: None,
            media_dirs: Vec::new(),
        }
    }
}

/// Number of speed samples kept for the graph.
pub(crate) const SAMPLE_RING: usize = 120;
/// EMA smoothing factor for `speed_bps`.
const EMA_ALPHA: f64 = 0.3;

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

/// What a job executes.
#[derive(Clone, Debug)]
pub(crate) enum Exec {
    Kind(JobKind),
    /// Undo of a history entry: records in the order they must be undone, plus the
    /// identities of what the original job created.
    Undo {
        records: Vec<UndoRecord>,
        stamps: Vec<UndoStamp>,
    },
}

/// Serialization lane of a job.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Lane {
    Global,
    Devices(DeviceId, DeviceId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LaneState {
    /// `PerDevice` lane not computed yet (needs probe I/O on the scheduler thread).
    Unknown,
    Known(Option<Lane>),
}

/// Shared between the engine and a job's worker thread.
#[derive(Debug)]
pub(crate) struct JobRt {
    pub ctl: Arc<OpControl>,
    pub progress: Mutex<Progress>,
    pub prompt_tx: Sender<PromptResponse>,
    pub prompt_rx: Receiver<PromptResponse>,
}

#[derive(Debug, Default)]
struct Speed {
    last_bytes: u64,
    last_at: Option<Instant>,
    ema: f64,
    /// Items/s EMA, used for the ETA of jobs without byte totals.
    item_ema: f64,
    last_items: u64,
    ring: VecDeque<(f32, f64)>,
    arc: Option<Arc<[(f32, f64)]>>,
}

pub(crate) struct JobEntry {
    id: JobId,
    brief: JobKindBrief,
    label: String,
    exec: Exec,
    options: JobOptions,
    roots: Vec<PathBuf>,
    lane: LaneState,
    state: JobState,
    /// State to return to on resume / after a prompt.
    resume_to: JobState,
    /// A worker thread was spawned for this job.
    started: bool,
    pause_pending: bool,
    force: bool,
    priority: Priority,
    rt: Arc<JobRt>,
    pending_prompt: Option<JobPrompt>,
    started_at: Option<Instant>,
    paused_since: Option<Instant>,
    paused_total: Duration,
    finished_elapsed: Option<Duration>,
    speed: Speed,
    log: VecDeque<LogLine>,
    log_base: usize,
    warnings: u32,
    auto_remove: bool,
    undo_of: Option<HistoryId>,
    redo_of: Option<HistoryId>,
}

impl JobEntry {
    fn elapsed(&self, now: Instant) -> Duration {
        if let Some(e) = self.finished_elapsed {
            return e;
        }
        let Some(start) = self.started_at else { return Duration::ZERO };
        let paused =
            self.paused_total + self.paused_since.map(|p| now.saturating_duration_since(p)).unwrap_or_default();
        now.saturating_duration_since(start).saturating_sub(paused)
    }

    /// Holds its `touched_roots` (started and not finished: includes paused / waiting).
    fn holds_roots(&self) -> bool {
        self.started && !self.state.is_terminal()
    }

    fn mark_paused(&mut self, now: Instant) {
        if self.paused_since.is_none() {
            self.paused_since = Some(now);
        }
    }

    fn mark_unpaused(&mut self, now: Instant) {
        if let Some(p) = self.paused_since.take() {
            self.paused_total += now.saturating_duration_since(p);
        }
    }
}

pub(crate) struct EngineState {
    next_id: u64,
    jobs: BTreeMap<JobId, JobEntry>,
    /// Jobs waiting for admission (Queued, or Paused before they ever started), in
    /// admission order: `(Priority desc, seq asc)` plus `move_in_queue`/`start_now` overrides.
    queue: Vec<JobId>,
    history: Vec<HistoryEntry>,
    next_history: u64,
    threads: Vec<JoinHandle<()>>,
    shutting_down: bool,
}

pub(crate) struct Inner {
    pub cfg: EngineConfig,
    pub probe: Arc<dyn FsProbe>,
    wake: Arc<dyn Fn() + Send + Sync>,
    events_tx: Sender<JobEvent>,
    kick_tx: Sender<()>,
    stop: AtomicBool,
    pub state: Mutex<EngineState>,
    history_io: Mutex<()>,
    pub shared_policy: Mutex<PolicyMemory>,
    workers: Mutex<usize>,
    workers_cv: Condvar,
    /// Lock order: its locks are taken with no engine lock held (see `journal.rs`).
    pub(crate) journal: Option<crate::journal::JournalWriter>,
}

impl Inner {
    /// Run `f` under the state lock; afterwards (unlocked) emit the collected events and wake the UI.
    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut EngineState, &mut Vec<JobEvent>) -> R) -> R {
        let mut events = Vec::new();
        let r = {
            let mut st = self.state.lock();
            f(&mut st, &mut events)
        };
        self.emit(events);
        r
    }

    fn emit(&self, events: Vec<JobEvent>) {
        if events.is_empty() {
            return;
        }
        for e in events {
            let _ = self.events_tx.send(e);
        }
        (self.wake)();
    }

    pub(crate) fn kick(&self) {
        let _ = self.kick_tx.send(());
    }

    fn add_job(
        &self,
        exec: Exec,
        brief: JobKindBrief,
        label: String,
        roots: Vec<PathBuf>,
        options: JobOptions,
    ) -> JobId {
        self.add_job_ext(exec, brief, label, roots, options, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_job_ext(
        &self,
        exec: Exec,
        brief: JobKindBrief,
        label: String,
        roots: Vec<PathBuf>,
        options: JobOptions,
        undo_of: Option<HistoryId>,
        redo_of: Option<HistoryId>,
    ) -> JobId {
        let mode = if options.queue == QueueMode::PerDevice { self.cfg.default_queue } else { options.queue };
        let lane = match mode {
            QueueMode::Serial => LaneState::Known(Some(Lane::Global)),
            QueueMode::Parallel => LaneState::Known(None),
            QueueMode::PerDevice if roots.is_empty() => LaneState::Known(None),
            QueueMode::PerDevice => LaneState::Unknown,
        };
        let (prompt_tx, prompt_rx) = crossbeam_channel::unbounded();
        let rt =
            Arc::new(JobRt { ctl: OpControl::new(), progress: Mutex::new(Progress::default()), prompt_tx, prompt_rx });
        let id = self.update(|st, ev| {
            st.next_id += 1;
            let id = JobId(st.next_id);
            let state = if st.shutting_down {
                JobState::Cancelled
            } else if options.start_paused {
                JobState::Paused
            } else {
                JobState::Queued
            };
            let entry = JobEntry {
                id,
                brief,
                label,
                exec,
                priority: options.priority,
                options,
                roots,
                lane,
                state: state.clone(),
                resume_to: JobState::Running,
                started: false,
                pause_pending: false,
                force: false,
                rt,
                pending_prompt: None,
                started_at: None,
                paused_since: None,
                paused_total: Duration::ZERO,
                finished_elapsed: None,
                speed: Speed::default(),
                log: VecDeque::new(),
                log_base: 0,
                warnings: 0,
                auto_remove: false,
                undo_of,
                redo_of,
            };
            let priority = entry.priority;
            st.jobs.insert(id, entry);
            ev.push(JobEvent::Added(id));
            if state.is_terminal() {
                ev.push(JobEvent::StateChanged(id, state));
                ev.push(JobEvent::Finished(id, JobSummary::default()));
            } else {
                st.insert_by_priority(id, priority);
            }
            id
        });
        self.kick();
        id
    }

    // ----- called by worker threads -------------------------------------------------

    /// Worker-driven transition (`Running`, `Finishing`). Deferred while paused / waiting.
    pub(crate) fn worker_state(&self, id: JobId, s: JobState) {
        self.update(|st, ev| {
            let Some(e) = st.jobs.get_mut(&id) else { return };
            if e.rt.ctl.is_cancelled() {
                return;
            }
            match e.state {
                ref t if t.is_terminal() => {}
                JobState::Paused | JobState::WaitingForUser => e.resume_to = s,
                ref cur if *cur == s => {}
                _ => {
                    e.state = s.clone();
                    ev.push(JobEvent::StateChanged(id, s));
                }
            }
        });
    }

    pub(crate) fn push_log(&self, id: JobId, line: LogLine) {
        let cap = self.cfg.log_lines_per_job.max(1);
        self.update(|st, ev| {
            let Some(e) = st.jobs.get_mut(&id) else { return };
            if line.level != LogLevel::Info {
                e.warnings += 1;
            }
            if e.log.len() >= cap {
                e.log.pop_front();
                e.log_base += 1;
            }
            e.log.push_back(line.clone());
            ev.push(JobEvent::Log(id, line));
        });
    }

    /// Publish a prompt; returns `false` if the job is already cancelled.
    pub(crate) fn begin_prompt(&self, id: JobId, prompt: JobPrompt) -> bool {
        self.update(|st, ev| {
            let Some(e) = st.jobs.get_mut(&id) else { return false };
            if e.rt.ctl.is_cancelled() || e.state.is_terminal() {
                return false;
            }
            // Stale answers (e.g. a double click) must not answer this prompt.
            while e.rt.prompt_rx.try_recv().is_ok() {}
            e.pending_prompt = Some(prompt);
            if e.state != JobState::Paused {
                e.resume_to = e.state.clone();
            }
            if e.state == JobState::Paused {
                // Paused while running into the prompt: show the prompt, resume afterwards paused.
                e.pause_pending = true;
                e.rt.ctl.resume();
                e.mark_unpaused(Instant::now());
            }
            e.state = JobState::WaitingForUser;
            ev.push(JobEvent::StateChanged(id, JobState::WaitingForUser));
            ev.push(JobEvent::Prompt(id));
            true
        })
    }

    /// Called by the worker after its prompt receiver yielded (state was already restored
    /// by `respond`/`cancel`; this is the safety net for every other wake-up).
    pub(crate) fn end_prompt(&self, id: JobId) {
        self.update(|st, ev| {
            let Some(e) = st.jobs.get_mut(&id) else { return };
            if e.state == JobState::WaitingForUser {
                e.pending_prompt = None;
                restore_after_prompt(e, ev);
            }
        });
    }

    fn finish_job(&self, id: JobId, result: Result<(), FsError>, outcome: crate::ctx::Outcome, cancelled: bool) {
        let crate::ctx::Outcome { mut summary, records, undone, stamps } = outcome;
        let mut history_changed = false;
        self.update(|st, ev| {
            let now = Instant::now();
            let Some(e) = st.jobs.get_mut(&id) else { return };
            e.mark_unpaused(now);
            summary.elapsed = e.elapsed(now);
            e.finished_elapsed = Some(summary.elapsed);
            let final_state = match result {
                _ if cancelled => JobState::Cancelled,
                Ok(()) => JobState::Done(summary.clone()),
                Err(err) if err.is_cancel() => JobState::Cancelled,
                Err(err) => JobState::Failed(err),
            };
            e.state = final_state.clone();
            e.pending_prompt = None;
            e.pause_pending = false;
            let (undo_of, redo_of, journal, auto_remove) = (e.undo_of, e.redo_of, e.options.journal, e.auto_remove);
            let options = e.options.clone();
            let job_kind = match &e.exec {
                Exec::Kind(k) => Some(k.clone()),
                Exec::Undo { .. } => None,
            };
            ev.push(JobEvent::StateChanged(id, final_state.clone()));
            ev.push(JobEvent::Finished(id, summary.clone()));
            if let Some(h) = undo_of {
                // Whatever the final state: records that were reversed are never replayed.
                if let Some(he) = st.history.iter_mut().find(|he| he.id == h) {
                    let remaining = without(&he.records, &undone);
                    if remaining.is_empty() {
                        he.undone = true;
                        history_changed = true;
                    } else if remaining.len() != he.records.len() {
                        he.undoable = history::is_undoable(&remaining);
                        he.records = remaining;
                        history_changed = true;
                    }
                }
            } else if let JobState::Done(_) = final_state {
                if let Some(h) = redo_of {
                    if let Some(he) = st.history.iter_mut().find(|he| he.id == h) {
                        he.undone = false;
                        he.undoable = history::is_undoable(&records);
                        he.records = records;
                        he.stamps = stamps;
                        he.summary = summary.clone();
                        he.when = SystemTime::now();
                        he.this_session = true;
                        history_changed = true;
                    }
                } else if let (Some(kind), true) = (job_kind, journal) {
                    st.next_history += 1;
                    let entry = HistoryEntry {
                        id: HistoryId(st.next_history),
                        job: kind,
                        summary: summary.clone(),
                        when: SystemTime::now(),
                        undoable: history::is_undoable(&records),
                        records,
                        undone: false,
                        stamps,
                        options: Some(options),
                        this_session: true,
                    };
                    st.history.push(entry);
                    let limit = self.cfg.history_limit;
                    if st.history.len() > limit {
                        let drop_n = st.history.len() - limit;
                        st.history.drain(..drop_n);
                    }
                    history_changed = true;
                }
            }
            if history_changed {
                ev.push(JobEvent::HistoryChanged);
            }
            if auto_remove {
                st.jobs.remove(&id);
                ev.push(JobEvent::Removed(id));
            }
        });
        if history_changed {
            self.persist_history();
        }
        // After the history: a crash in between shows the job as interrupted, never lost.
        if let Some(j) = &self.journal {
            j.end(id.0);
        }
        self.kick();
    }

    /// Rewrite `history.jsonl` atomically (lock order: history_io → state).
    pub(crate) fn persist_history(&self) {
        let Some(path) = self.cfg.history_path.clone() else { return };
        let _io = self.history_io.lock();
        let entries = self.state.lock().history.clone();
        if let Err(e) = history::save(&path, &entries) {
            log::warn!("soyuz-jobs: cannot write history {}: {e}", path.display());
        }
    }

    // ----- scheduler ------------------------------------------------------------------

    /// Compute `PerDevice` lanes of queued jobs (probe I/O outside the lock).
    fn compute_lanes(&self) {
        let todo: Vec<(JobId, Vec<PathBuf>)> = {
            let st = self.state.lock();
            st.queue
                .iter()
                .filter_map(|id| st.jobs.get(id))
                .filter(|e| e.lane == LaneState::Unknown)
                .map(|e| (e.id, e.roots.clone()))
                .collect()
        };
        if todo.is_empty() {
            return;
        }
        let computed: Vec<(JobId, Lane)> = todo
            .into_iter()
            .map(|(id, roots)| {
                let real: Vec<&PathBuf> = roots
                    .iter()
                    .filter(|r| !crate::job::is_synthetic_root(r) && !crate::job::is_remote_root(r))
                    .collect();
                let src = real.first().map(|p| device_for(&*self.probe, p)).unwrap_or(DeviceId(0));
                let dst = real.last().map(|p| device_for(&*self.probe, p)).unwrap_or(DeviceId(0));
                (id, Lane::Devices(src, dst))
            })
            .collect();
        let mut st = self.state.lock();
        for (id, lane) in computed {
            if let Some(e) = st.jobs.get_mut(&id) {
                e.lane = LaneState::Known(Some(lane));
            }
        }
    }

    /// One admission pass; returns the jobs that must get a worker thread.
    fn admit(&self) -> Vec<JobId> {
        self.update(|st, ev| {
            if st.shutting_down {
                return Vec::new();
            }
            let now = Instant::now();
            let mut active = st.jobs.values().filter(|e| e.state.is_active()).count();
            let mut held_roots: Vec<PathBuf> =
                st.jobs.values().filter(|e| e.holds_roots()).flat_map(|e| e.roots.iter().cloned()).collect();
            let mut busy_lanes: HashSet<Lane> = st
                .jobs
                .values()
                .filter(|e| e.state.is_active())
                .filter_map(|e| match e.lane {
                    LaneState::Known(Some(l)) => Some(l),
                    _ => None,
                })
                .collect();
            let mut blocked_lanes: HashSet<Lane> = HashSet::new();
            let mut blocked_roots: Vec<PathBuf> = Vec::new();
            let mut start = Vec::new();
            for id in st.queue.clone() {
                let Some(e) = st.jobs.get_mut(&id) else { continue };
                if e.state != JobState::Queued {
                    continue; // paused before start: user-held, blocks nothing
                }
                let overlaps =
                    |set: &[PathBuf]| e.roots.iter().any(|r| set.iter().any(|h| crate::job::roots_overlap(r, h)));
                let roots_ok = !overlaps(&held_roots) && !overlaps(&blocked_roots);
                let (lane_ok, lane) = match e.lane {
                    LaneState::Unknown => (false, None),
                    LaneState::Known(None) => (true, None),
                    LaneState::Known(Some(l)) => {
                        (e.force || (!busy_lanes.contains(&l) && !blocked_lanes.contains(&l)), Some(l))
                    }
                };
                let cap_ok = e.force || active < self.cfg.max_parallel.max(1);
                if roots_ok && lane_ok && cap_ok {
                    e.force = false;
                    e.started = true;
                    e.state = JobState::Planning;
                    e.started_at = Some(now);
                    active += 1;
                    if let Some(l) = lane {
                        busy_lanes.insert(l);
                    }
                    held_roots.extend(e.roots.iter().cloned());
                    ev.push(JobEvent::StateChanged(id, JobState::Planning));
                    start.push(id);
                } else {
                    blocked_roots.extend(e.roots.iter().cloned());
                    if let Some(l) = lane {
                        blocked_lanes.insert(l);
                    }
                }
            }
            st.queue.retain(|id| !start.contains(id));
            start
        })
    }

    fn spawn_worker(self: &Arc<Self>, id: JobId) {
        let Some((rt, exec, options, label)) = ({
            let st = self.state.lock();
            st.jobs.get(&id).map(|e| (e.rt.clone(), e.exec.clone(), e.options.clone(), e.label.clone()))
        }) else {
            return;
        };
        if let (Some(j), Exec::Kind(kind)) = (&self.journal, &exec)
            && crate::journal::journals(kind)
        {
            // Before the worker starts: a crash from here on is recoverable.
            j.begin(id.0, label, kind, &options, rt.clone());
        }
        *self.workers.lock() += 1;
        let inner = self.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("soyuz-job-{}", id.0))
            .spawn(move || worker_main(inner, id, rt, exec, options));
        match spawned {
            Ok(h) => self.state.lock().threads.push(h),
            Err(e) => {
                self.worker_exited();
                let err =
                    FsError::with_message(FsErrorKind::Io, "spawn", None, format!("cannot start job thread: {e}"));
                self.finish_job(id, Err(err), crate::ctx::Outcome::default(), false);
            }
        }
    }

    fn worker_exited(&self) {
        let mut n = self.workers.lock();
        *n = n.saturating_sub(1);
        self.workers_cv.notify_all();
    }

    fn sample(&self) {
        let now = Instant::now();
        let mut st = self.state.lock();
        for e in st.jobs.values_mut() {
            if !e.started || e.state.is_terminal() {
                continue;
            }
            let (bytes, items) = {
                let p = e.rt.progress.lock();
                (p.bytes_done, p.items_done)
            };
            let elapsed = e.elapsed(now);
            let sp = &mut e.speed;
            let Some(last) = sp.last_at else {
                sp.last_at = Some(now);
                sp.last_bytes = bytes;
                sp.last_items = items;
                continue;
            };
            let dt = now.saturating_duration_since(last).as_secs_f64();
            if dt <= 0.0 {
                continue;
            }
            let running = matches!(e.state, JobState::Running | JobState::Planning | JobState::Finishing);
            let bps = if running { bytes.saturating_sub(sp.last_bytes) as f64 / dt } else { 0.0 };
            let ips = if running { items.saturating_sub(sp.last_items) as f64 / dt } else { 0.0 };
            if running {
                sp.ema = if sp.ring.is_empty() { bps } else { EMA_ALPHA * bps + (1.0 - EMA_ALPHA) * sp.ema };
                sp.item_ema = if sp.ring.is_empty() { ips } else { EMA_ALPHA * ips + (1.0 - EMA_ALPHA) * sp.item_ema };
            }
            sp.last_at = Some(now);
            sp.last_bytes = bytes;
            sp.last_items = items;
            if sp.ring.len() >= SAMPLE_RING {
                sp.ring.pop_front();
            }
            sp.ring.push_back((elapsed.as_secs_f32(), bps));
            sp.arc = None;
        }
    }

    fn reap_threads(&self) {
        let finished: Vec<JoinHandle<()>> = {
            let mut st = self.state.lock();
            let (done, running): (Vec<_>, Vec<_>) = st.threads.drain(..).partition(|h| h.is_finished());
            st.threads = running;
            done
        };
        for h in finished {
            let _ = h.join();
        }
    }
}

impl EngineState {
    fn insert_by_priority(&mut self, id: JobId, p: Priority) {
        let pos = self
            .queue
            .iter()
            .position(|q| self.jobs.get(q).map(|e| e.priority < p).unwrap_or(false))
            .unwrap_or(self.queue.len());
        self.queue.insert(pos, id);
    }
}

/// After a prompt was answered: back to the pre-prompt state or to `Paused`.
fn restore_after_prompt(e: &mut JobEntry, ev: &mut Vec<JobEvent>) {
    let next = if e.pause_pending {
        e.pause_pending = false;
        e.rt.ctl.pause();
        e.mark_paused(Instant::now());
        JobState::Paused
    } else {
        e.resume_to.clone()
    };
    e.state = next.clone();
    ev.push(JobEvent::StateChanged(e.id, next));
}

/// Device of `p`, or of its nearest existing ancestor (destinations may not exist yet).
fn device_for(probe: &dyn FsProbe, p: &Path) -> DeviceId {
    let mut cur = Some(p);
    while let Some(c) = cur {
        if let Ok(d) = probe.device_of(c) {
            return d;
        }
        cur = c.parent();
    }
    DeviceId(0)
}

struct WorkerGuard(Arc<Inner>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.worker_exited();
    }
}

fn worker_main(inner: Arc<Inner>, id: JobId, rt: Arc<JobRt>, exec: Exec, options: JobOptions) {
    let _guard = WorkerGuard(inner.clone());
    let ctx = crate::ctx::JobCtx::new(id, inner.clone(), rt, options);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::kinds::run(&ctx, &exec)));
    let result = match result {
        Ok(r) => r,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            Err(FsError::with_message(FsErrorKind::Io, "job", None, format!("internal error: {msg}")))
        }
    };
    let cancelled = ctx.rt.ctl.is_cancelled();
    match &result {
        Ok(()) if !cancelled => ctx.log(LogLevel::Info, None, "finished".to_string()),
        Err(e) if !cancelled && !e.is_cancel() => ctx.log(LogLevel::Error, e.path.clone(), format!("failed: {e}")),
        _ => ctx.log(LogLevel::Warn, None, "cancelled".to_string()),
    }
    let mut outcome = ctx.take_outcome();
    if matches!(exec, Exec::Kind(_)) && ctx.opts.journal {
        // I/O on the worker, never under the engine lock.
        outcome.stamps = crate::undo::stamp_records(&outcome.records);
    }
    inner.finish_job(id, result, outcome, cancelled);
}

fn scheduler_main(inner: Arc<Inner>, kick_rx: Receiver<()>) {
    let interval = inner.cfg.sample_interval.max(Duration::from_millis(10));
    let mut last_sample = Instant::now();
    loop {
        let timeout = interval.saturating_sub(last_sample.elapsed());
        let _ = kick_rx.recv_timeout(timeout);
        while kick_rx.try_recv().is_ok() {}
        if inner.stop.load(Ordering::SeqCst) {
            break;
        }
        inner.compute_lanes();
        for id in inner.admit() {
            inner.spawn_worker(id);
        }
        if last_sample.elapsed() >= interval {
            last_sample = Instant::now();
            inner.sample();
            inner.reap_threads();
            if let Some(j) = &inner.journal {
                j.tick();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// `Send + Sync`; all state behind one `parking_lot::Mutex<EngineState>` plus per-job
/// `Arc<Mutex<Progress>>` (see the module docs for threads and lock ordering).
pub struct JobEngine {
    inner: Arc<Inner>,
    events_rx: Receiver<JobEvent>,
    scheduler: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for JobEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobEngine").field("cfg", &self.inner.cfg).finish_non_exhaustive()
    }
}

impl JobEngine {
    /// `wake` = `ctx.request_repaint`; `probe` = `RealProbe` in the app, a fake in tests.
    ///
    /// Loads `cfg.history_path` if it exists (unparsable lines are skipped with a warning).
    pub fn new(cfg: EngineConfig, probe: Arc<dyn FsProbe>, wake: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        let (events_tx, events_rx) = crossbeam_channel::unbounded();
        let (kick_tx, kick_rx) = crossbeam_channel::unbounded();
        let mut history = cfg.history_path.as_deref().map(history::load).unwrap_or_default();
        if history.len() > cfg.history_limit {
            let drop_n = history.len() - cfg.history_limit;
            history.drain(..drop_n);
        }
        let next_history = history.iter().map(|h| h.id.0).max().unwrap_or(0);
        let journal = cfg.journal_path.clone().map(crate::journal::JournalWriter::new);
        let inner = Arc::new(Inner {
            journal,
            cfg,
            probe,
            wake,
            events_tx,
            kick_tx,
            stop: AtomicBool::new(false),
            state: Mutex::new(EngineState {
                next_id: 0,
                jobs: BTreeMap::new(),
                queue: Vec::new(),
                history,
                next_history,
                threads: Vec::new(),
                shutting_down: false,
            }),
            history_io: Mutex::new(()),
            shared_policy: Mutex::new(PolicyMemory::default()),
            workers: Mutex::new(0),
            workers_cv: Condvar::new(),
        });
        let sched_inner = inner.clone();
        let handle = std::thread::Builder::new()
            .name("soyuz-jobs-sched".into())
            .spawn(move || scheduler_main(sched_inner, kick_rx))
            .expect("spawn soyuz-jobs-sched");
        Arc::new(JobEngine { inner, events_rx, scheduler: Mutex::new(Some(handle)) })
    }

    /// Never blocks.
    pub fn submit(&self, spec: JobSpec) -> JobId {
        let JobSpec { kind, options } = spec;
        let brief = kind.brief();
        let label = options.label.clone().unwrap_or_else(|| crate::job::default_label(&kind));
        let roots = kind.touched_roots();
        self.inner.add_job(Exec::Kind(kind), brief, label, roots, options)
    }

    pub fn pause(&self, id: JobId) {
        self.inner.update(|st, ev| {
            if let Some(e) = st.jobs.get_mut(&id) {
                pause_entry(e, ev);
            }
        });
    }

    pub fn resume(&self, id: JobId) {
        self.inner.update(|st, ev| {
            if let Some(e) = st.jobs.get_mut(&id) {
                resume_entry(e, ev);
            }
        });
        self.inner.kick();
    }

    pub fn cancel(&self, id: JobId) {
        self.inner.update(|st, ev| cancel_entry(st, id, ev));
        self.inner.kick();
    }

    pub fn pause_all(&self) {
        self.inner.update(|st, ev| {
            for e in st.jobs.values_mut() {
                pause_entry(e, ev);
            }
        });
    }

    pub fn resume_all(&self) {
        self.inner.update(|st, ev| {
            for e in st.jobs.values_mut() {
                resume_entry(e, ev);
            }
        });
        self.inner.kick();
    }

    pub fn cancel_all(&self) {
        self.inner.update(|st, ev| {
            let ids: Vec<JobId> = st.jobs.keys().copied().collect();
            for id in ids {
                cancel_entry(st, id, ev);
            }
        });
        self.inner.kick();
    }

    /// Queued jobs only; running jobs are not preempted.
    pub fn set_priority(&self, id: JobId, p: Priority) {
        self.inner.update(|st, _| {
            let Some(e) = st.jobs.get_mut(&id) else { return };
            if e.started || e.state.is_terminal() {
                return;
            }
            e.priority = p;
            st.queue.retain(|q| *q != id);
            st.insert_by_priority(id, p);
        });
        (self.inner.wake)();
        self.inner.kick();
    }

    /// Move a waiting job `delta` places in the admission queue (negative = earlier).
    pub fn move_in_queue(&self, id: JobId, delta: i32) {
        self.inner.update(|st, _| {
            let Some(pos) = st.queue.iter().position(|q| *q == id) else { return };
            let new = (pos as i64 + delta as i64).clamp(0, st.queue.len() as i64 - 1) as usize;
            let id = st.queue.remove(pos);
            st.queue.insert(new, id);
        });
        (self.inner.wake)();
        self.inner.kick();
    }

    /// Bypass lanes + `max_parallel` once (overlapping `touched_roots` are still serialized).
    pub fn start_now(&self, id: JobId) {
        self.inner.update(|st, ev| {
            let Some(pos) = st.queue.iter().position(|q| *q == id) else { return };
            let Some(e) = st.jobs.get_mut(&id) else { return };
            e.force = true;
            if e.state == JobState::Paused {
                e.state = JobState::Queued;
                ev.push(JobEvent::StateChanged(id, JobState::Queued));
            }
            st.queue.remove(pos);
            st.queue.insert(0, id);
        });
        self.inner.kick();
    }

    /// Ignored if no prompt is pending.
    pub fn respond(&self, id: JobId, r: PromptResponse) {
        self.inner.update(|st, ev| {
            let Some(e) = st.jobs.get_mut(&id) else { return };
            if e.state != JobState::WaitingForUser || e.pending_prompt.is_none() {
                return;
            }
            e.pending_prompt = None;
            let _ = e.rt.prompt_tx.send(r);
            restore_after_prompt(e, ev);
        });
    }

    pub fn snapshot(&self, id: JobId) -> Option<JobSnapshot> {
        let mut st = self.inner.state.lock();
        let now = Instant::now();
        let pos = st.queue.iter().position(|q| *q == id);
        st.jobs.get_mut(&id).map(|e| make_snapshot(e, pos, now))
    }

    pub fn snapshots(&self) -> Vec<JobSnapshot> {
        let mut st = self.inner.state.lock();
        let now = Instant::now();
        let positions: BTreeMap<JobId, usize> = st.queue.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        st.jobs.values_mut().map(|e| make_snapshot(e, positions.get(&e.id).copied(), now)).collect()
    }

    /// Any job not yet terminal (queued, running, paused or waiting for the user).
    pub fn has_active(&self) -> bool {
        self.inner.state.lock().jobs.values().any(|e| !e.state.is_terminal())
    }

    /// Log lines with absolute index `>= from` (indices keep counting when the ring drops
    /// old lines; `JobSnapshot::log_len` is the next index).
    pub fn log(&self, id: JobId, from: usize) -> Vec<LogLine> {
        let st = self.inner.state.lock();
        let Some(e) = st.jobs.get(&id) else { return Vec::new() };
        let skip = from.saturating_sub(e.log_base);
        e.log.iter().skip(skip).cloned().collect()
    }

    /// Unbounded; the UI drains it each frame with `try_iter()`.
    pub fn events(&self) -> &Receiver<JobEvent> {
        &self.events_rx
    }

    /// Remove every terminal job from the list (history is unaffected).
    pub fn clear_finished(&self) {
        self.inner.update(|st, ev| {
            let done: Vec<JobId> = st.jobs.values().filter(|e| e.state.is_terminal()).map(|e| e.id).collect();
            for id in done {
                st.jobs.remove(&id);
                ev.push(JobEvent::Removed(id));
            }
        });
    }

    /// Remove a job from the list; a job that is not finished is cancelled first and
    /// removed once its thread has exited.
    pub fn remove(&self, id: JobId) {
        self.inner.update(|st, ev| {
            let Some(e) = st.jobs.get_mut(&id) else { return };
            if e.started && !e.state.is_terminal() {
                e.auto_remove = true;
                cancel_entry(st, id, ev);
                return;
            }
            st.queue.retain(|q| *q != id);
            st.jobs.remove(&id);
            ev.push(JobEvent::Removed(id));
        });
        self.inner.kick();
    }

    /// Oldest first.
    pub fn history(&self) -> Vec<HistoryEntry> {
        self.inner.state.lock().history.clone()
    }

    /// Submit a job that reverses history entry `h`. `None` if the entry is unknown,
    /// already undone, not undoable, or an undo/redo of it is in flight.
    pub fn undo(&self, h: HistoryId) -> Option<JobId> {
        let (records, stamps, label) = {
            let st = self.inner.state.lock();
            if history_busy(&st, h) {
                return None;
            }
            let he = st.history.iter().find(|he| he.id == h)?;
            if he.undone || !he.undoable {
                return None;
            }
            (
                crate::undo::undo_order(&he.records),
                he.stamps.clone(),
                format!("Undo: {}", crate::job::default_label(&he.job)),
            )
        };
        let roots = crate::undo::touched_roots(&records);
        let options = JobOptions { journal: false, queue: QueueMode::Parallel, ..JobOptions::default() };
        let exec = Exec::Undo { records, stamps };
        Some(self.inner.add_job_ext(exec, JobKindBrief::Undo, label, roots, options, Some(h), None))
    }

    /// Re-run the original `JobKind` of an undone entry; on success the entry is updated
    /// in place (`undone = false`, fresh records).
    pub fn redo(&self, h: HistoryId) -> Option<JobId> {
        let (kind, original) = {
            let st = self.inner.state.lock();
            if history_busy(&st, h) {
                return None;
            }
            let he = st.history.iter().find(|he| he.id == h)?;
            if !he.undone {
                return None;
            }
            (he.job.clone(), he.options.clone())
        };
        let label = format!("Redo: {}", crate::job::default_label(&kind));
        let roots = kind.touched_roots();
        let brief = kind.brief();
        let options = rerun_options(original);
        Some(self.inner.add_job_ext(Exec::Kind(kind), brief, label, roots, options, None, Some(h)))
    }

    /// Re-submit the failed part of entry `h` (conflicts with what already arrived are
    /// skipped). `None` if the entry had no failures.
    pub fn retry_failed(&self, h: HistoryId) -> Option<JobId> {
        let (kind, original) = {
            let st = self.inner.state.lock();
            let he = st.history.iter().find(|he| he.id == h)?;
            if he.summary.items_failed == 0 {
                return None;
            }
            (crate::kinds::retry_kind(&he.job, &he.summary)?, he.options.clone())
        };
        let mut spec = JobSpec { kind, options: rerun_options(original) };
        if matches!(
            spec.kind,
            JobKind::Copy { .. }
                | JobKind::Move { .. }
                | JobKind::Extract { .. }
                | JobKind::Remote(crate::job::RemoteOp::Transfer { .. })
        ) {
            spec.options.conflict = Some(soyuz_core::ConflictResolution::Skip);
        }
        Some(self.submit(spec))
    }

    /// `cancel_all` + join with timeout + flush history. Jobs submitted afterwards are
    /// cancelled immediately. Idempotent.
    pub fn shutdown(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        self.inner.update(|st, _| st.shutting_down = true);
        self.cancel_all();
        {
            let mut n = self.inner.workers.lock();
            while *n > 0 {
                if self.inner.workers_cv.wait_until(&mut n, deadline).timed_out() {
                    break;
                }
            }
        }
        let all_exited = *self.inner.workers.lock() == 0;
        let handles: Vec<JoinHandle<()>> = std::mem::take(&mut self.inner.state.lock().threads);
        for h in handles {
            // A worker decrements the counter as its very last action, so joining is
            // immediate once the counter is zero; stragglers past the deadline are detached.
            if all_exited || h.is_finished() {
                let _ = h.join();
            }
        }
        self.inner.stop.store(true, Ordering::SeqCst);
        self.inner.kick();
        if let Some(h) = self.scheduler.lock().take() {
            let _ = h.join();
        }
        self.inner.persist_history();
        if let Some(j) = &self.inner.journal {
            // Jobs that did not stop in time stay journaled (shown as interrupted next start).
            j.flush();
        }
    }

    /// Resume an interrupted job from a recovered journal: what is left of it
    /// ([`crate::journal::resume_kind`]) with its original options; copy-like kinds skip
    /// whatever already arrived. `None` when nothing is left to do.
    pub fn resume_journaled(&self, job: &crate::journal::JournalJob) -> Option<JobId> {
        let kind = crate::journal::resume_kind(&job.kind)?;
        let mut options = rerun_options(Some(job.options.clone()));
        if matches!(
            kind,
            JobKind::Copy { .. }
                | JobKind::Move { .. }
                | JobKind::Extract { .. }
                | JobKind::Remote(crate::job::RemoteOp::Transfer { .. })
        ) {
            options.conflict = Some(soyuz_core::ConflictResolution::Skip);
        }
        options.label = Some(format!("Resume: {}", job.label));
        Some(self.submit(JobSpec { kind, options }))
    }

    /// Roll back the completed parts of an interrupted job (an undo of its journaled
    /// records, identity-checked like any undo). `None` when nothing was journaled.
    pub fn rollback_journaled(&self, job: &crate::journal::JournalJob) -> Option<JobId> {
        if !job.can_roll_back() {
            return None;
        }
        let records: Vec<UndoRecord> = crate::undo::undo_order(&job.records)
            .into_iter()
            .filter(|r| !matches!(r, UndoRecord::Irreversible(_)))
            .collect();
        let roots = crate::undo::touched_roots(&records);
        let options = JobOptions { journal: false, queue: QueueMode::Parallel, ..JobOptions::default() };
        let exec = Exec::Undo { records, stamps: job.stamps.clone() };
        let label = format!("Roll back: {}", job.label);
        Some(self.inner.add_job(exec, JobKindBrief::Undo, label, roots, options))
    }

    /// Live worker threads (test hook for shutdown).
    #[cfg(test)]
    pub(crate) fn live_workers(&self) -> usize {
        *self.inner.workers.lock()
    }

    /// Submit an undo job for arbitrary records (tests of record kinds that no job in the
    /// baseline fs can produce, e.g. `Extracted`).
    #[cfg(test)]
    pub(crate) fn submit_undo_records(&self, records: Vec<UndoRecord>) -> JobId {
        let roots = crate::undo::touched_roots(&records);
        let options = JobOptions { journal: false, queue: QueueMode::Parallel, ..JobOptions::default() };
        let exec = Exec::Undo { records, stamps: Vec::new() };
        self.inner.add_job(exec, JobKindBrief::Undo, "undo".into(), roots, options)
    }
}

impl Drop for JobEngine {
    fn drop(&mut self) {
        if self.scheduler.lock().is_some() {
            self.shutdown(Duration::ZERO);
        }
    }
}

/// `records` minus `undone` (multiset difference, order kept).
fn without(records: &[UndoRecord], undone: &[UndoRecord]) -> Vec<UndoRecord> {
    let mut pool: Vec<&UndoRecord> = undone.iter().collect();
    records
        .iter()
        .filter(|r| match pool.iter().position(|u| *u == *r) {
            Some(i) => {
                pool.swap_remove(i);
                false
            }
            None => true,
        })
        .cloned()
        .collect()
}

/// Options for re-running a history entry (redo / retry): the original ones when known
/// (older files: the defaults), journaled, not paused, with the default label.
fn rerun_options(original: Option<JobOptions>) -> JobOptions {
    let mut o = original.unwrap_or_default();
    o.journal = true;
    o.start_paused = false;
    o.label = None;
    o
}

fn history_busy(st: &EngineState, h: HistoryId) -> bool {
    st.jobs.values().any(|e| !e.state.is_terminal() && (e.undo_of == Some(h) || e.redo_of == Some(h)))
}

fn pause_entry(e: &mut JobEntry, ev: &mut Vec<JobEvent>) {
    match e.state {
        JobState::Queued => {
            e.state = JobState::Paused;
            ev.push(JobEvent::StateChanged(e.id, JobState::Paused));
        }
        JobState::Planning | JobState::Running | JobState::Finishing => {
            e.resume_to = e.state.clone();
            e.rt.ctl.pause();
            e.mark_paused(Instant::now());
            e.state = JobState::Paused;
            ev.push(JobEvent::StateChanged(e.id, JobState::Paused));
        }
        JobState::WaitingForUser => e.pause_pending = true,
        _ => {}
    }
}

fn resume_entry(e: &mut JobEntry, ev: &mut Vec<JobEvent>) {
    match e.state {
        JobState::Paused if !e.started => {
            e.state = JobState::Queued;
            ev.push(JobEvent::StateChanged(e.id, JobState::Queued));
        }
        JobState::Paused => {
            e.mark_unpaused(Instant::now());
            e.rt.ctl.resume();
            e.state = e.resume_to.clone();
            ev.push(JobEvent::StateChanged(e.id, e.state.clone()));
        }
        JobState::WaitingForUser => e.pause_pending = false,
        _ => {}
    }
}

fn cancel_entry(st: &mut EngineState, id: JobId, ev: &mut Vec<JobEvent>) {
    let Some(e) = st.jobs.get_mut(&id) else { return };
    if e.state.is_terminal() {
        return;
    }
    e.rt.ctl.cancel();
    if !e.started {
        e.state = JobState::Cancelled;
        e.finished_elapsed = Some(Duration::ZERO);
        ev.push(JobEvent::StateChanged(id, JobState::Cancelled));
        ev.push(JobEvent::Finished(id, JobSummary::default()));
        st.queue.retain(|q| *q != id);
        return;
    }
    if e.pending_prompt.take().is_some() {
        // Unblock the worker waiting on its prompt channel.
        let _ = e.rt.prompt_tx.send(PromptResponse::Cancelled);
    }
    e.pause_pending = false;
    if matches!(e.state, JobState::Paused | JobState::WaitingForUser) {
        e.mark_unpaused(Instant::now());
        // The worker wakes (cancel wins over pause) and reports `Cancelled` when it exits;
        // meanwhile show it as winding down.
        e.state = JobState::Finishing;
        ev.push(JobEvent::StateChanged(id, JobState::Finishing));
    }
}

fn make_snapshot(e: &mut JobEntry, queue_pos: Option<usize>, now: Instant) -> JobSnapshot {
    let progress = e.rt.progress.lock().clone();
    let elapsed = e.elapsed(now);
    let running = !e.state.is_terminal() && e.started;
    let speed_bps = if running { e.speed.ema } else { 0.0 };
    let eta = if !running || !progress.totals_known {
        None
    } else if progress.bytes_total > 0 && speed_bps > 1.0 {
        Some(Duration::from_secs_f64(progress.bytes_total.saturating_sub(progress.bytes_done) as f64 / speed_bps))
    } else if progress.bytes_total == 0 && progress.items_total > 0 && e.speed.item_ema > 0.01 {
        Some(Duration::from_secs_f64(
            progress.items_total.saturating_sub(progress.items_done) as f64 / e.speed.item_ema,
        ))
    } else {
        None
    };
    let samples = e.speed.arc.get_or_insert_with(|| e.speed.ring.iter().copied().collect::<Vec<_>>().into()).clone();
    JobSnapshot {
        id: e.id,
        kind: e.brief,
        label: e.label.clone(),
        state: e.state.clone(),
        priority: e.priority,
        queue_pos,
        progress,
        started: e.started_at,
        elapsed,
        speed_bps,
        eta,
        samples,
        pending_prompt: e.pending_prompt.clone(),
        warnings: e.warnings,
        log_len: e.log_base + e.log.len(),
        verify: e.options.copy.verify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_fs::RealProbe;

    #[test]
    fn engine_constructs_and_is_idle() {
        let e = JobEngine::new(EngineConfig::default(), Arc::new(RealProbe), Arc::new(|| {}));
        assert!(!e.has_active());
        assert!(e.snapshots().is_empty());
        assert!(e.events().try_recv().is_err());
        e.shutdown(Duration::from_millis(10));
    }

    #[test]
    fn engine_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<JobEngine>();
    }
}
