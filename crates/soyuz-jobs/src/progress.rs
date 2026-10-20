//! Live progress and snapshots polled by the UI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::job::{JobId, JobKindBrief, JobState, Priority};
use crate::prompt::JobPrompt;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub items_done: u64,
    pub items_total: u64,
    /// Planner finished counting.
    pub totals_known: bool,
    pub current: Option<PathBuf>,
    pub current_bytes: u64,
    pub current_total: u64,
}

/// Cheap-to-clone view of one job.
#[derive(Clone, Debug)]
pub struct JobSnapshot {
    pub id: JobId,
    pub kind: JobKindBrief,
    pub label: String,
    pub state: JobState,
    pub priority: Priority,
    pub queue_pos: Option<usize>,
    pub progress: Progress,
    pub started: Option<Instant>,
    pub elapsed: Duration,
    /// Exponential moving average.
    pub speed_bps: f64,
    pub eta: Option<Duration>,
    /// Ring buffer of 120 × 0.5 s samples `(secs, bps)` for the speed graph.
    pub samples: Arc<[(f32, f64)]>,
    pub pending_prompt: Option<JobPrompt>,
    pub warnings: u32,
    pub log_len: usize,
    /// Copies of this job are verified by hashing (`JobOptions::copy.verify`).
    pub verify: Option<soyuz_core::HashAlgo>,
    /// Where the time went and why a queued job waits ("Why is this slow?").
    pub io: IoStats,
}

/// Why a queued job has not started yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// Another job uses the same disk(s) (per-device queue); the job, when known.
    SameDevice(Option<JobId>),
    /// Another job works in the same folders.
    SameFolders(Option<JobId>),
    /// `max_parallel` jobs already run.
    Parallel(usize),
    /// The disks of its paths are still being looked up.
    Probing,
}

/// Live I/O statistics of a job. The time buckets are charged by the engine's sampler
/// (every `sample_interval`), so they are accurate to about one interval.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IoStats {
    /// Submitted → started (or until now while it waits).
    pub queued: Duration,
    /// Counting / planning before the first byte.
    pub planning: Duration,
    /// Running with bytes or items moving.
    pub moving: Duration,
    /// Running while nothing moved (a slow or busy disk, the network, a file held by
    /// another program or an antivirus scan).
    pub stalled: Duration,
    pub paused: Duration,
    /// Waiting for an answer to a question (conflict, error).
    pub waiting_for_user: Duration,
    /// Items per second (moving average).
    pub files_per_sec: f64,
    /// Bytes per second over the time spent running (moving + stalled).
    pub avg_bps: f64,
    /// Fastest sample.
    pub peak_bps: f64,
    /// Set while the job is queued.
    pub blocked: Option<Blocked>,
    /// Per-device queue: whether it reads and writes the same disk (`None` = not known
    /// or not a per-device job).
    pub same_device: Option<bool>,
}
