//! Headless backend: `soyuz --mcp` when no Soyuz window is running. Read tools run on
//! [`FsToolHost`]; approved plans run on a private [`JobEngine`] (same `history.jsonl`, so
//! the window can undo them later). Nobody can approve here: write tools are refused
//! unless the client is configured with `require_approval = false`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde_json::{Value, json};
use soyuz_ai::tools::{
    CallContext, FsToolHost, LimitArg, MediaAccess, Plan, PlanAction, Scope, ToolError, ToolHost, ToolInput,
};
use soyuz_core::OpControl;
use soyuz_core::config::{Ai, AiMcpClient};
use soyuz_jobs::{EngineConfig, HistoryId, JobEngine, JobId};

use crate::jobs;
use crate::server::{ApprovalDecision, ApprovalRequest, Backend, Mode, ProgressFn};
use crate::trust::{self, Transport};

/// Longest a headless tool call waits for its job.
pub const MAX_JOB_WAIT: Duration = Duration::from_secs(30 * 60);

pub struct HeadlessHost {
    fs: FsToolHost,
    engine: Arc<JobEngine>,
}

impl HeadlessHost {
    pub fn new(ai: &Ai, engine: Arc<JobEngine>) -> Self {
        HeadlessHost { fs: FsToolHost::from_config(&ai.scope), engine }
    }

    /// The same host with `[media]` settings for the media tools.
    pub fn with_media(self, media: MediaAccess) -> Self {
        HeadlessHost { fs: self.fs.with_media(media), ..self }
    }
}

impl ToolHost for HeadlessHost {
    fn scope(&self) -> &Scope {
        &self.fs.scope
    }

    fn read(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        match input {
            ToolInput::HistoryList(LimitArg { limit }) => {
                Ok(jobs::history_json(&self.engine.history(), limit.unwrap_or(50).min(500) as usize))
            }
            ToolInput::GetSelection(_) | ToolInput::GetCurrentLocation(_) | ToolInput::ListTabs(_) => {
                Err(ToolError::Unsupported(format!("{} (no Soyuz window is open; Soyuz runs headless)", input.name())))
            }
            _ => self.fs.read(input, ctx),
        }
    }

    fn plan(&self, input: &ToolInput, ctx: &CallContext) -> Result<Plan, ToolError> {
        self.fs.plan(input, ctx)
    }

    fn ui(&self, input: &ToolInput, _ctx: &CallContext) -> Result<Value, ToolError> {
        Err(ToolError::Unsupported(format!("{} (no Soyuz window is open; Soyuz runs headless)", input.name())))
    }

    fn meta(&self, input: &ToolInput, _ctx: &CallContext) -> Result<Value, ToolError> {
        match input {
            ToolInput::JobStatus(a) => match a.job_id {
                Some(id) => self
                    .engine
                    .snapshot(JobId(id))
                    .map(|s| jobs::snapshot_json(&s))
                    .ok_or_else(|| ToolError::Failed(format!("no job {id}"))),
                None => {
                    Ok(json!({"jobs": self.engine.snapshots().iter().map(jobs::snapshot_json).collect::<Vec<_>>()}))
                }
            },
            ToolInput::CancelJob(a) => {
                self.engine.cancel(JobId(a.job_id));
                Ok(json!({"job_id": a.job_id, "cancel_requested": true}))
            }
            other => Err(ToolError::Unsupported(other.name().to_string())),
        }
    }
}

pub struct HeadlessBackend {
    host: Arc<HeadlessHost>,
    engine: Arc<JobEngine>,
    ai: RwLock<Ai>,
    /// `config.toml` to register pending clients in (`None` = memory only, tests).
    config_path: Option<PathBuf>,
    /// `ai-audit.jsonl`.
    audit_path: PathBuf,
}

impl HeadlessBackend {
    /// `history`: `history.jsonl` to share with the window (`None` = in memory);
    /// `audit_path`: usually [`soyuz_ai::audit::default_path`].
    pub fn new(ai: Ai, config_path: Option<PathBuf>, history: Option<PathBuf>, audit_path: PathBuf) -> Arc<Self> {
        Self::with_media(ai, MediaAccess::default(), config_path, history, audit_path)
    }

    /// [`HeadlessBackend::new`] with `[media]` settings: the media tools (and their jobs)
    /// load the FFmpeg libraries from `media.dirs`, as nothing else in this process does.
    pub fn with_media(
        ai: Ai,
        media: MediaAccess,
        config_path: Option<PathBuf>,
        history: Option<PathBuf>,
        audit_path: PathBuf,
    ) -> Arc<Self> {
        let cfg = EngineConfig { history_path: history, media_dirs: media.dirs.clone(), ..EngineConfig::default() };
        let engine = JobEngine::new(cfg, Arc::new(soyuz_fs::RealProbe), Arc::new(|| {}));
        Arc::new(HeadlessBackend {
            host: Arc::new(HeadlessHost::new(&ai, engine.clone()).with_media(media)),
            engine,
            ai: RwLock::new(ai),
            config_path,
            audit_path,
        })
    }

    pub fn engine(&self) -> &Arc<JobEngine> {
        &self.engine
    }
}

impl Backend for HeadlessBackend {
    fn mode(&self) -> Mode {
        Mode::Headless
    }

    fn host(&self) -> Arc<dyn ToolHost> {
        self.host.clone()
    }

    fn ai(&self) -> Ai {
        self.ai.read().clone()
    }

    fn request_approval(&self, _req: ApprovalRequest, _ctl: &OpControl) -> ApprovalDecision {
        ApprovalDecision::Unavailable(
            "This change needs the user's approval in the Soyuz window, but Soyuz is running headless (no window is \
             open). Ask the user to open Soyuz and retry, or call the tool with \"dry_run\": true to show the plan."
                .into(),
        )
    }

    fn execute(&self, plan: &Plan, ctx: &CallContext, progress: &ProgressFn<'_>) -> Result<Value, ToolError> {
        execute_on_engine(&self.engine, plan, &ctx.ctl, progress, true)
    }

    fn audit(&self, rec: soyuz_ai::audit::AuditRecord) {
        if let Err(e) = soyuz_ai::audit::append(&self.audit_path, &rec) {
            log::warn!("{}: {e}", self.audit_path.display());
        }
    }

    fn register_pending(&self, name: &str, _transport: Transport) {
        let entry = AiMcpClient { name: name.to_string(), pending: true, ..Default::default() };
        self.ai.write().mcp_clients.push(entry.clone());
        if let Some(path) = &self.config_path
            && let Err(e) = register_in_file(path, entry)
        {
            log::warn!("cannot register MCP client “{name}” in {}: {e}", path.display());
        }
    }
}

/// Submit a plan's job (or undo) on `engine` and wait for it.
pub fn execute_on_engine(
    engine: &JobEngine,
    plan: &Plan,
    ctl: &OpControl,
    progress: &ProgressFn<'_>,
    answer_prompts: bool,
) -> Result<Value, ToolError> {
    let id = match &plan.action {
        PlanAction::Job(spec) => engine.submit(spec.clone()),
        PlanAction::App { op, args } if op == "undo" => {
            let h =
                args.get("history_id").and_then(Value::as_u64).ok_or_else(|| ToolError::Failed("bad undo".into()))?;
            engine.undo(HistoryId(h)).ok_or_else(|| ToolError::Failed(format!("history entry {h} cannot be undone")))?
        }
        PlanAction::App { op, .. } => {
            return Err(ToolError::Unsupported(format!("“{op}” without the Soyuz window")));
        }
        PlanAction::None => return Ok(json!({"status": "nothing to do"})),
    };
    let v = jobs::wait_job(engine, id, ctl, progress, MAX_JOB_WAIT, answer_prompts);
    if v.get("state").and_then(Value::as_str) == Some("cancelled") && ctl.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    Ok(v)
}

/// Append a pending `[[ai.mcp_clients]]` entry to `config.toml`, keeping its formatting.
pub fn register_in_file(path: &std::path::Path, entry: AiMcpClient) -> Result<(), String> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut config = if existing.trim().is_empty() {
        soyuz_core::config::Config::default()
    } else {
        soyuz_core::config::load_or_default(path).map_err(|e| e.to_string())?.config
    };
    let name = trust::sanitize_name(&entry.name);
    if config.ai.mcp_clients.iter().any(|c| trust::sanitize_name(&c.name) == name) {
        return Ok(());
    }
    config.ai.mcp_clients.push(entry);
    let text = soyuz_core::config::to_string_preserving(&existing, &config).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        soyuz_fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    soyuz_fs::write_atomic(path, text.as_bytes()).map_err(|e| e.to_string())
}
