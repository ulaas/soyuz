//! How approved plans reach the jobs engine. The agent calls an [`Executor`]: the app's
//! submits on the UI thread through `JobsService` (so jobs get AI-tagged metadata, the
//! configured copy options and the normal prompt / toast handling); tests use
//! [`EngineExecutor`] on a bare `JobEngine`. Waiting for a job and "undo everything the
//! assistant did" only need the engine, so they live here as plain functions.

use std::sync::Arc;
use std::time::{Duration, Instant};

use soyuz_ai::tools::{Plan, PlanAction};
use soyuz_jobs::{JobEngine, JobId, JobSpec, JobState, JobSummary};

use super::{CancelFlag, TaskId, job_label, label_task};

/// Result of submitting an approved plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Submitted {
    /// The job, for plans that run one.
    pub job: Option<JobId>,
    /// What happened for app actions (tags, custom commands, undo).
    pub note: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done(JobSummary),
    Failed(String),
    Cancelled,
    /// Still running when the task ran out of time (the job keeps running).
    TimedOut,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        matches!(self, Outcome::Done(s) if s.items_failed == 0)
    }

    /// One line for the chat and the model.
    pub fn describe(&self) -> String {
        match self {
            Outcome::Done(s) if s.items_failed == 0 => {
                format!("done: {} item(s), {} skipped", s.items_ok, s.items_skipped)
            }
            Outcome::Done(s) => format!(
                "finished with errors: {} ok, {} failed{}",
                s.items_ok,
                s.items_failed,
                s.errors.first().map(|(p, e)| format!(" (first: {}: {e})", p.display())).unwrap_or_default()
            ),
            Outcome::Failed(e) => format!("failed: {e}"),
            Outcome::Cancelled => "cancelled".into(),
            Outcome::TimedOut => "still running (time limit reached)".into(),
        }
    }
}

/// Runs approved plans.
pub trait Executor: Send + Sync {
    /// Submit an approved plan of `task`.
    fn submit(&self, task: TaskId, plan: &Plan) -> Result<Submitted, String>;
    /// Wait for a job to finish (cancellable; gives up at `deadline`).
    fn wait(&self, job: JobId, cancel: &CancelFlag, deadline: Instant) -> Outcome;
    fn cancel(&self, job: JobId);
}

/// The plan's job spec relabelled for `task` (so history entries can be found again).
pub fn task_spec(task: TaskId, plan: &Plan) -> Option<JobSpec> {
    let mut spec = plan.job()?.clone();
    let summary = spec.options.label.clone().unwrap_or_else(|| plan.summary.clone());
    spec.options.label = Some(job_label(task, &summary));
    spec.options.journal = true;
    Some(spec)
}

/// Poll the engine until `job` reaches a terminal state.
pub fn wait_engine(engine: &JobEngine, job: JobId, cancel: &CancelFlag, deadline: Instant) -> Outcome {
    let mut cancelled_job = false;
    loop {
        match engine.snapshot(job).map(|s| s.state) {
            None => return Outcome::Failed("the job disappeared from the activity list".into()),
            Some(JobState::Done(s)) => return Outcome::Done(s),
            Some(JobState::Failed(e)) => return Outcome::Failed(e.to_string()),
            Some(JobState::Cancelled) => return Outcome::Cancelled,
            Some(_) => {}
        }
        if cancel.is_cancelled() && !cancelled_job {
            engine.cancel(job);
            cancelled_job = true;
        }
        if Instant::now() >= deadline {
            return Outcome::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// Executor straight on an engine (tests, headless runs). App actions (tags, custom
/// commands, undo) are recorded, not performed.
pub struct EngineExecutor {
    pub engine: Arc<JobEngine>,
    pub app_actions: parking_lot::Mutex<Vec<(String, serde_json::Value)>>,
}

impl EngineExecutor {
    pub fn new(engine: Arc<JobEngine>) -> Self {
        EngineExecutor { engine, app_actions: parking_lot::Mutex::new(Vec::new()) }
    }
}

impl Executor for EngineExecutor {
    fn submit(&self, task: TaskId, plan: &Plan) -> Result<Submitted, String> {
        if !plan.executable {
            return Err("the plan has blocking conflicts".into());
        }
        match &plan.action {
            PlanAction::Job(_) => {
                let spec = task_spec(task, plan).ok_or("no job")?;
                Ok(Submitted { job: Some(self.engine.submit(spec)), note: String::new() })
            }
            PlanAction::App { op, args } => {
                self.app_actions.lock().push((op.clone(), args.clone()));
                Ok(Submitted { job: None, note: format!("{op} done") })
            }
            PlanAction::None => Ok(Submitted { job: None, note: "nothing to do".into() }),
        }
    }

    fn wait(&self, job: JobId, cancel: &CancelFlag, deadline: Instant) -> Outcome {
        wait_engine(&self.engine, job, cancel, deadline)
    }

    fn cancel(&self, job: JobId) {
        self.engine.cancel(job);
    }
}

/// History entries created by `task` in this session that can still be undone, newest
/// first.
pub fn task_history(engine: &JobEngine, task: TaskId) -> Vec<soyuz_jobs::HistoryId> {
    let mut v: Vec<_> = engine
        .history()
        .into_iter()
        .filter(|h| h.this_session && !h.undone && h.undoable)
        .filter(|h| h.options.as_ref().and_then(|o| o.label.as_deref()).and_then(label_task) == Some(task))
        .map(|h| h.id)
        .collect();
    v.sort_by_key(|h| std::cmp::Reverse(h.0));
    v
}

/// Undo every job of `task`, newest first, one after another (later jobs may depend on
/// earlier ones). `on_job` sees each undo job as it starts. Returns `(undone, failed)`.
pub fn undo_task(
    engine: &JobEngine,
    task: TaskId,
    cancel: &CancelFlag,
    mut on_job: impl FnMut(JobId),
) -> (usize, Vec<String>) {
    let mut undone = 0;
    let mut failed = Vec::new();
    for h in task_history(engine, task) {
        if cancel.is_cancelled() {
            break;
        }
        let Some(job) = engine.undo(h) else {
            failed.push(format!("history entry {} cannot be undone now", h.0));
            continue;
        };
        on_job(job);
        match wait_engine(engine, job, cancel, Instant::now() + Duration::from_secs(600)) {
            Outcome::Done(s) if s.items_failed == 0 => undone += 1,
            o => failed.push(format!("history entry {}: {}", h.0, o.describe())),
        }
    }
    (undone, failed)
}
