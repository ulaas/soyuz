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
}
