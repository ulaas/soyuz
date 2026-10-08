//! Job helpers shared by the headless backend and the app bridge: JSON views of
//! snapshots / history for tool results, and waiting for a submitted job with progress.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use soyuz_core::OpControl;
use soyuz_jobs::{HistoryEntry, JobEngine, JobId, JobPrompt, JobSnapshot, JobState, PromptResponse};

use crate::server::ProgressFn;

pub fn state_name(s: &JobState) -> &'static str {
    match s {
        JobState::Queued => "queued",
        JobState::Planning => "planning",
        JobState::Running => "running",
        JobState::Paused => "paused",
        JobState::WaitingForUser => "waiting_for_user",
        JobState::Finishing => "finishing",
        JobState::Done(_) => "done",
        JobState::Failed(_) => "failed",
        JobState::Cancelled => "cancelled",
    }
}

/// Model-facing view of a job.
pub fn snapshot_json(s: &JobSnapshot) -> Value {
    let mut v = json!({
        "job_id": s.id.0,
        "kind": s.kind,
        "label": s.label,
        "state": state_name(&s.state),
        "bytes_done": s.progress.bytes_done,
        "bytes_total": s.progress.bytes_total,
        "items_done": s.progress.items_done,
        "items_total": s.progress.items_total,
        "waiting_for_user": s.pending_prompt.is_some(),
    });
    match &s.state {
        JobState::Done(sum) => {
            v["summary"] = json!({
                "items_ok": sum.items_ok,
                "items_skipped": sum.items_skipped,
                "items_failed": sum.items_failed,
                "bytes": sum.bytes,
                "errors": sum.errors.iter().take(10).map(|(p, e)| json!({"path": p, "error": e.to_string()})).collect::<Vec<_>>(),
            });
        }
        JobState::Failed(e) => v["error"] = json!(e.to_string()),
        _ => {}
    }
    v
}

/// Model-facing view of the undo history (newest first).
pub fn history_json(entries: &[HistoryEntry], limit: usize) -> Value {
    let list: Vec<Value> = entries
        .iter()
        .rev()
        .take(limit)
        .map(|h| {
            json!({
                "history_id": h.id.0,
                "label": h.options.as_ref().and_then(|o| o.label.clone()),
                "kind": h.job.brief(),
                "when": soyuz_core::time::serde_rfc3339::to_string(&h.when),
                "items_ok": h.summary.items_ok,
                "items_failed": h.summary.items_failed,
                "undone": h.undone,
                "undoable": h.undoable && !h.undone,
            })
        })
        .collect();
    json!({"total": entries.len(), "entries": list})
}

/// Answer for a prompt when nobody can be asked (headless): never overwrite, never
/// delete permanently, skip what fails.
pub fn headless_answer(p: &JobPrompt) -> PromptResponse {
    match p {
        JobPrompt::Conflict(_) => {
            PromptResponse::Conflict { resolution: soyuz_core::ConflictResolution::Skip, apply_to_all: true }
        }
        JobPrompt::Error { .. } => {
            PromptResponse::Error { action: soyuz_core::ErrorAction::SkipAll, apply_to_all: true }
        }
        JobPrompt::NotEnoughSpace { .. } => PromptResponse::Space { proceed: false },
        JobPrompt::TrashUnsupported { .. } => PromptResponse::TrashUnsupported { permanent: false, apply_to_all: true },
        JobPrompt::ReadOnlyItem { .. } => PromptResponse::ReadOnly { force: false, apply_to_all: true },
        JobPrompt::Password { .. } => PromptResponse::Password(None),
    }
}

/// Wait until `id` finishes (or `max_wait` passes), reporting progress. `answer_prompts`
/// answers job prompts with [`headless_answer`] (headless only; the app asks the user).
/// Cancelling `ctl` cancels the job.
pub fn wait_job(
    engine: &JobEngine,
    id: JobId,
    ctl: &OpControl,
    progress: &ProgressFn<'_>,
    max_wait: Duration,
    answer_prompts: bool,
) -> Value {
    let start = Instant::now();
    let mut last = None;
    loop {
        // Keep the event channel from growing when nobody else drains it.
        if answer_prompts {
            while engine.events().try_recv().is_ok() {}
        }
        let Some(snap) = engine.snapshot(id) else {
            return json!({"job_id": id.0, "state": "unknown"});
        };
        if snap.state.is_terminal() {
            return snapshot_json(&snap);
        }
        if ctl.is_cancelled() {
            engine.cancel(id);
            return json!({"job_id": id.0, "state": "cancelled"});
        }
        if answer_prompts && let Some(p) = &snap.pending_prompt {
            engine.respond(id, headless_answer(p));
        }
        let done = (snap.progress.bytes_done, snap.progress.items_done);
        if last != Some(done) {
            last = Some(done);
            let (cur, total) = if snap.progress.bytes_total > 0 {
                (snap.progress.bytes_done as f64, Some(snap.progress.bytes_total as f64))
            } else {
                (
                    snap.progress.items_done as f64,
                    (snap.progress.items_total > 0).then_some(snap.progress.items_total as f64),
                )
            };
            progress(cur, total, Some(snap.label.clone()));
        }
        if start.elapsed() > max_wait {
            let mut v = snapshot_json(&snap);
            v["note"] = json!("still running in Soyuz; poll job_status with this job_id");
            return v;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}
