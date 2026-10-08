//! `soyuz-jobs`: the background operation engine — queue with per-device lanes,
//! pause/resume/cancel, conflict/error prompts, history and undo (PLAN §2.4).
//!
//! **M0 status:** every public type is final; [`JobEngine`] constructs and answers
//! queries, mutating operations are `todo!()` until M2.
#![forbid(unsafe_code)]

pub mod engine;
pub mod history;
pub mod job;
pub mod kinds;
pub mod planner;
pub mod policy;
pub mod progress;
pub mod prompt;
pub mod scheduler;
pub mod undo;

pub use engine::{EngineConfig, JobEngine, JobEvent};
pub use history::{HistoryEntry, HistoryId};
pub use job::{
    CreateKind, DeleteMode, JobId, JobKind, JobKindBrief, JobOptions, JobSpec, JobState, JobSummary, LogLevel, LogLine,
    Priority, QueueMode, SyncAction, SyncPlan,
};
pub use progress::{JobSnapshot, Progress};
pub use prompt::{ConflictInfo, ConflictKind, JobPrompt, PromptResponse};
pub use undo::UndoRecord;
