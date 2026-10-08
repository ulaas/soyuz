//! The in-app agent harness (roadmap A5 / A7 / A8).
//!
//! * [`agent`] — the agent loop on a worker thread: context → model call with the tool
//!   catalog filtered by scope and risk → streamed text → read tools run automatically
//!   within limits → write tools become dry-run plans that pause for approval → approved
//!   plans run as AI-tagged jobs → job results go back to the model → repeat. Step,
//!   token and wall-clock limits, cancellation, context compaction and model routing.
//! * [`host`] — [`host::AppToolHost`], the app's `soyuz_ai::tools::ToolHost`: read tools
//!   through `FsToolHost`, app-state reads from a snapshot, UI-control tools on the UI
//!   thread.
//! * [`exec`] — how approved plans reach the jobs engine ([`exec::Executor`]), job
//!   waiting and "undo everything the assistant did".
//! * [`model`] / [`fake`] — the model abstraction, provider routing (fast vs strong) and
//!   the scripted fake provider (`SOYUZ_AI_FAKE=<transcript.toml>`, tests, demos).
//! * [`safety`] — per-folder permissions, redaction, prompt-injection heuristics.
//! * [`context`] — the task context, the system prompt, compaction and cost estimates.
//! * [`recipe`] — "save as recipe" and re-running recipes without a model.
//! * [`audit`] — the assistant's records in the shared `ai-audit.jsonl` (`soyuz_ai::audit`).
//! * [`touch`] — live highlights of the files the assistant or MCP clients read or change.
//! * [`mcp`] — external MCP clients' requests as plan cards, their jobs and undo.
//! * [`vision`] — "Suggest names" inputs: text excerpts and downscaled pictures.
//! * [`smart_search`] — natural language → editable structured query pills.
//! * [`state`] — the UI-side state and the per-frame pump.
//!
//! Models never touch the disk: every change is a `JobSpec` inside a `soyuz_ai` plan
//! that the user approves; jobs are labelled `AI #<task> · …` so the activity panel tags
//! them and the history can undo a whole task.

pub mod agent;
pub mod audit;
pub mod context;
pub mod exec;
pub mod fake;
pub mod host;
pub mod mcp;
pub mod model;
pub mod recipe;
pub mod safety;
pub mod smart_search;
pub mod state;
pub mod touch;
pub mod vision;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub use state::{AiCmd, AiState, apply_cmd, execute_action, handles, pump};

/// Repaint / wake-up callback.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// One assistant task (a user message and everything the agent did for it).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskId(pub u64);

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

static NEXT_TASK: AtomicU64 = AtomicU64::new(1);

impl TaskId {
    pub fn next() -> TaskId {
        TaskId(NEXT_TASK.fetch_add(1, Ordering::Relaxed))
    }
}

/// Label prefix of jobs started by the assistant (`AI #3 · Move 4 items into …`).
pub const JOB_PREFIX: &str = "AI #";

/// Job label for a plan approved in `task`.
pub fn job_label(task: TaskId, summary: &str) -> String {
    let s = summary.trim_start_matches("AI: ").trim();
    format!("{JOB_PREFIX}{} \u{00b7} {s}", task.0)
}

/// The task of an AI job label.
pub fn label_task(label: &str) -> Option<TaskId> {
    let rest = label.strip_prefix(JOB_PREFIX)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().map(TaskId)
}

/// Cooperative cancellation flag shared by the UI, the agent and its tool calls.
#[derive(Clone, Default, Debug)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_round_trip() {
        let l = job_label(TaskId(12), "AI: Move 3 items into C:\\x");
        assert_eq!(l, "AI #12 \u{00b7} Move 3 items into C:\\x");
        assert_eq!(label_task(&l), Some(TaskId(12)));
        assert_eq!(label_task("Move things"), None);
        assert_eq!(label_task("Undo: AI #4 \u{00b7} x"), None);
    }
}
