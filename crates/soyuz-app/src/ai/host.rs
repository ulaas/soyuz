//! [`AppToolHost`]: the app's `soyuz_ai::tools::ToolHost`. File reads and dry-run plans
//! go through `FsToolHost` (soyuz-fs / soyuz-search, never the UI thread); app-state reads
//! (selection, location, tabs, history, jobs) come from an [`AppSnapshot`] the UI
//! refreshes; UI-control tools (navigate, open_tab, select, reveal, preview, show_toast,
//! open_terminal_at) are sent to the UI thread over a [`UiLink`] and answered there.
//! `run_custom_command`, `undo` and `tag` are plans that need approval like file changes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;
use serde_json::{Value, json};
use soyuz_ai::tools::{CallContext, FsToolHost, Plan, Scope, ToolError, ToolHost, ToolInput};
use soyuz_jobs::{JobEngine, JobId};

use super::exec::{Executor, Outcome, Submitted, wait_engine};
use super::{CancelFlag, TaskId, Wake};

/// App state the host reads (refreshed by the UI thread while a task runs).
#[derive(Clone, Debug, Default)]
pub struct AppSnapshot {
    pub location: Option<PathBuf>,
    pub location_label: String,
    pub selection: Vec<PathBuf>,
    /// `(title, folder, active)`.
    pub tabs: Vec<(String, Option<PathBuf>, bool)>,
    /// Recent history entries as model-facing JSON (newest first).
    pub history: Vec<Value>,
    /// Jobs as model-facing JSON.
    pub jobs: Vec<Value>,
    /// Names of the user's custom commands.
    pub commands: Vec<String>,
}

/// Requests from worker threads to the UI thread.
pub enum UiRequest {
    /// A UI-control or meta tool (`navigate`, `cancel_job`, …).
    Tool { input: ToolInput, reply: Sender<Result<Value, ToolError>> },
    /// Submit an approved plan through `JobsService` (AI-tagged).
    Submit { task: TaskId, plan: Box<Plan>, reply: Sender<Result<Submitted, String>> },
    /// Register an engine-created job (undo-all) as belonging to `task`.
    Track { job: JobId, task: TaskId },
}

/// Sends [`UiRequest`]s and waits for the answer (never called on the UI thread).
#[derive(Clone)]
pub struct UiLink {
    tx: Sender<UiRequest>,
    wake: Wake,
}

impl UiLink {
    pub fn new(tx: Sender<UiRequest>, wake: Wake) -> Self {
        UiLink { tx, wake }
    }

    /// A link with its receiving end (tests drive the UI side by hand).
    pub fn pair(wake: Wake) -> (UiLink, Receiver<UiRequest>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (UiLink { tx, wake }, rx)
    }

    pub fn send(&self, req: UiRequest) {
        let _ = self.tx.send(req);
        (self.wake)();
    }

    fn call<T>(&self, make: impl FnOnce(Sender<T>) -> UiRequest, timeout: Duration) -> Option<T> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.send(make(tx));
        rx.recv_timeout(timeout).ok()
    }
}

pub struct AppToolHost {
    pub base: FsToolHost,
    pub snapshot: Arc<RwLock<AppSnapshot>>,
    pub ui: Option<UiLink>,
}

impl AppToolHost {
    pub fn new(base: FsToolHost, snapshot: Arc<RwLock<AppSnapshot>>, ui: Option<UiLink>) -> Self {
        AppToolHost { base, snapshot, ui }
    }

    fn limit(n: Option<u32>, default: usize) -> usize {
        n.map_or(default, |n| n as usize).clamp(1, 500)
    }
}

impl ToolHost for AppToolHost {
    fn scope(&self) -> &Scope {
        &self.base.scope
    }

    fn read(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        let s = self.snapshot.read();
        match input {
            ToolInput::GetSelection(_) => {
                let items: Vec<Value> = s
                    .selection
                    .iter()
                    .filter(|p| !self.base.scope.is_blocked(p))
                    .take(500)
                    .map(|p| json!({"path": p}))
                    .collect();
                Ok(json!({"count": items.len(), "items": items}))
            }
            ToolInput::GetCurrentLocation(_) => Ok(json!({
                "path": s.location,
                "label": s.location_label,
            })),
            ToolInput::ListTabs(_) => Ok(json!({
                "tabs": s.tabs.iter().enumerate().map(|(i, (title, path, active))| {
                    json!({"index": i, "title": title, "path": path, "active": active})
                }).collect::<Vec<_>>()
            })),
            ToolInput::HistoryList(a) => {
                let n = Self::limit(a.limit, 50);
                Ok(json!({"entries": s.history.iter().take(n).cloned().collect::<Vec<_>>()}))
            }
            _ => {
                drop(s);
                self.base.read(input, ctx)
            }
        }
    }

    fn plan(&self, input: &ToolInput, ctx: &CallContext) -> Result<Plan, ToolError> {
        if let ToolInput::RunCustomCommand(a) = input
            && !self.snapshot.read().commands.iter().any(|c| c == &a.name)
        {
            return Err(ToolError::Failed(format!("no custom command named “{}”", a.name)));
        }
        self.base.plan(input, ctx)
    }

    fn ui(&self, input: &ToolInput, _ctx: &CallContext) -> Result<Value, ToolError> {
        let Some(link) = &self.ui else { return Err(ToolError::Unsupported(input.name().into())) };
        let input = input.clone();
        link.call(|reply| UiRequest::Tool { input, reply }, Duration::from_secs(5))
            .unwrap_or_else(|| Err(ToolError::Failed("the window did not answer in time".into())))
    }

    fn meta(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        match input {
            ToolInput::JobStatus(a) => {
                let s = self.snapshot.read();
                let jobs: Vec<Value> = s
                    .jobs
                    .iter()
                    .filter(|j| a.job_id.is_none_or(|id| j.get("job_id").and_then(Value::as_u64) == Some(id)))
                    .cloned()
                    .collect();
                Ok(json!({"jobs": jobs}))
            }
            ToolInput::CancelJob(_) => self.ui(input, ctx),
            other => Err(ToolError::Unsupported(other.name().into())),
        }
    }
}

/// The app's [`Executor`]: submission on the UI thread, waiting on the engine.
pub struct AppExecutor {
    pub engine: Arc<JobEngine>,
    pub ui: UiLink,
}

impl Executor for AppExecutor {
    fn submit(&self, task: TaskId, plan: &Plan) -> Result<Submitted, String> {
        let plan = Box::new(plan.clone());
        self.ui
            .call(|reply| UiRequest::Submit { task, plan, reply }, Duration::from_secs(15))
            .unwrap_or_else(|| Err("the window did not accept the job in time".into()))
    }

    fn wait(&self, job: JobId, cancel: &CancelFlag, deadline: Instant) -> Outcome {
        wait_engine(&self.engine, job, cancel, deadline)
    }

    fn cancel(&self, job: JobId) {
        self.engine.cancel(job);
    }
}
