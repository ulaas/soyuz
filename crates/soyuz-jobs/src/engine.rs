//! The job engine (PLAN §2.4 threading rules are binding for the implementation).
//!
//! **M0 status:** construction, the event channel and read-only queries work (they
//! report an empty engine); every mutating operation is `todo!()` until M2.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use soyuz_fs::FsProbe;

use crate::history::{HistoryEntry, HistoryId};
use crate::job::{JobId, JobSpec, JobState, JobSummary, LogLine, Priority, QueueMode};
use crate::progress::JobSnapshot;
use crate::prompt::PromptResponse;

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
        }
    }
}

/// `Send + Sync`; all state behind one `parking_lot::Mutex<EngineState>` plus per-job
/// `Arc<Mutex<Progress>>` (implementation detail, M2).
pub struct JobEngine {
    cfg: EngineConfig,
    probe: Arc<dyn FsProbe>,
    wake: Arc<dyn Fn() + Send + Sync>,
    events_tx: Sender<JobEvent>,
    events_rx: Receiver<JobEvent>,
}

impl std::fmt::Debug for JobEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobEngine").field("cfg", &self.cfg).finish_non_exhaustive()
    }
}

impl JobEngine {
    /// `wake` = `ctx.request_repaint`; `probe` = `RealProbe` in the app, a fake in tests.
    pub fn new(cfg: EngineConfig, probe: Arc<dyn FsProbe>, wake: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        let (events_tx, events_rx) = crossbeam_channel::unbounded();
        Arc::new(JobEngine { cfg, probe, wake, events_tx, events_rx })
    }

    /// Never blocks.
    pub fn submit(&self, spec: JobSpec) -> JobId {
        let _ = (&self.probe, &self.wake, &self.events_tx, spec);
        todo!("M2: JobEngine::submit")
    }

    pub fn pause(&self, id: JobId) {
        todo!("M2: JobEngine::pause({id:?})")
    }

    pub fn resume(&self, id: JobId) {
        todo!("M2: JobEngine::resume({id:?})")
    }

    pub fn cancel(&self, id: JobId) {
        todo!("M2: JobEngine::cancel({id:?})")
    }

    pub fn pause_all(&self) {
        todo!("M2: JobEngine::pause_all")
    }

    pub fn resume_all(&self) {
        todo!("M2: JobEngine::resume_all")
    }

    pub fn cancel_all(&self) {
        todo!("M2: JobEngine::cancel_all")
    }

    /// Queued jobs only; running jobs are not preempted.
    pub fn set_priority(&self, id: JobId, p: Priority) {
        todo!("M2: JobEngine::set_priority({id:?}, {p:?})")
    }

    pub fn move_in_queue(&self, id: JobId, delta: i32) {
        todo!("M2: JobEngine::move_in_queue({id:?}, {delta})")
    }

    /// Bypass lanes + `max_parallel` once.
    pub fn start_now(&self, id: JobId) {
        todo!("M2: JobEngine::start_now({id:?})")
    }

    /// Ignored if no prompt is pending.
    pub fn respond(&self, id: JobId, r: PromptResponse) {
        todo!("M2: JobEngine::respond({id:?}, {r:?})")
    }

    pub fn snapshot(&self, id: JobId) -> Option<JobSnapshot> {
        let _ = id;
        None
    }

    pub fn snapshots(&self) -> Vec<JobSnapshot> {
        Vec::new()
    }

    pub fn has_active(&self) -> bool {
        false
    }

    pub fn log(&self, id: JobId, from: usize) -> Vec<LogLine> {
        let _ = (id, from);
        Vec::new()
    }

    /// Unbounded; the UI drains it each frame with `try_iter()`.
    pub fn events(&self) -> &Receiver<JobEvent> {
        &self.events_rx
    }

    pub fn clear_finished(&self) {}

    pub fn remove(&self, id: JobId) {
        let _ = id;
    }

    pub fn history(&self) -> Vec<HistoryEntry> {
        Vec::new()
    }

    pub fn undo(&self, h: HistoryId) -> Option<JobId> {
        todo!("M2: JobEngine::undo({h:?})")
    }

    pub fn redo(&self, h: HistoryId) -> Option<JobId> {
        todo!("M2: JobEngine::redo({h:?})")
    }

    pub fn retry_failed(&self, h: HistoryId) -> Option<JobId> {
        todo!("M2: JobEngine::retry_failed({h:?})")
    }

    /// `cancel_all` + join with timeout + flush history.
    pub fn shutdown(&self, wait: Duration) {
        let _ = wait;
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
