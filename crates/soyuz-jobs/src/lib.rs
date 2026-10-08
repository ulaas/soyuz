//! `soyuz-jobs`: the background operation engine — queue with per-device lanes,
//! pause/resume/cancel, conflict/error prompts, history and undo (PLAN §2.4).
//!
//! Drive it from the UI with [`JobEngine::submit`], poll [`JobEngine::snapshots`] while
//! [`JobEngine::has_active`], drain [`JobEngine::events`] each frame, answer
//! [`JobEvent::Prompt`] with [`JobEngine::respond`], and call [`JobEngine::shutdown`] on exit.
#![forbid(unsafe_code)]

mod ctx;
pub mod engine;
pub mod history;
pub mod job;
pub mod journal;
pub mod kinds;
pub mod planner;
pub mod policy;
pub mod progress;
pub mod prompt;
pub mod scheduler;
pub mod undo;

#[cfg(test)]
mod tests;

pub use engine::{EngineConfig, JobEngine, JobEvent};
pub use history::{HistoryEntry, HistoryId, UndoStamp};
pub use job::{
    CreateKind, DeleteMode, JobId, JobKind, JobKindBrief, JobOptions, JobSpec, JobState, JobSummary, LogLevel, LogLine,
    Priority, QueueMode, RemoteOp, SyncAction, SyncPlan,
};
pub use journal::{JournalJob, JournalSlot, Recovered};
pub use progress::{JobSnapshot, Progress};
pub use prompt::{ConflictInfo, ConflictKind, JobPrompt, PromptResponse};
pub use undo::UndoRecord;
