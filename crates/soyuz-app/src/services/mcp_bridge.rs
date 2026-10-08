//! MCP server inside the running window (roadmap A3).
//!
//! * Serves the local named pipe that `soyuz --mcp` attaches to, and — when enabled —
//!   Streamable HTTP on 127.0.0.1 ([`soyuz_mcp`] does the protocol work on its own
//!   threads).
//! * Tool calls that need the app (selection, current folder, tabs, navigation, toasts,
//!   job status) and approved plans arrive here as requests and are answered on the UI
//!   thread in [`pump`], so jobs show in the activity panel with normal undo.
//! * Write / destructive plans from MCP clients become the assistant's plan cards
//!   ([`crate::ai::mcp::McpApprovals`]): in the assistant panel's approvals area (the
//!   panel opens when a request arrives) or as a modal card (`[ai.mcp] approval_ui`).
//!   Approved plans run as jobs labelled `MCP <client> · …` with live pane highlights;
//!   the area offers "Undo everything this client did".
//! * Every call is appended to the shared `ai-audit.jsonl` (same file and format as the
//!   assistant's records).
//! * Unknown stdio clients are added to `[[ai.mcp_clients]]` as pending (read-only until
//!   approved in Settings → AI assistant → Connections).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::{Mutex, RwLock};
use serde_json::{Value, json};
use soyuz_ai::Secret;
use soyuz_ai::tools::{CallContext, FsToolHost, Plan, PlanAction, Scope, ToolError, ToolHost, ToolInput};
use soyuz_core::config::{Ai, AiMcpApprovalUi, AiMcpClient};
use soyuz_core::{Location, OpControl};
use soyuz_jobs::{HistoryId, JobEngine, JobId};
use soyuz_mcp::http::{HttpServer, HttpSettings};
use soyuz_mcp::pipe::PipeServer;
use soyuz_mcp::{ApprovalDecision, ApprovalRequest, Backend, Mode, ProgressFn, Server, Transport};

use crate::ai::mcp::{McpApprovals, PendingMcpApproval};
use crate::app::SoyuzApp;
use crate::ops::{self, NavOpts};
use crate::services::jobs::{JobMeta, OpKind};
use crate::ui::toasts::Toast;

/// Repaint callback.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// How long a server thread waits for the UI to answer a non-approval request.
const UI_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest a tool call waits for its job before answering "still running".
const MAX_JOB_WAIT: Duration = Duration::from_secs(10 * 60);

/// What a server thread needs from the UI thread.
pub enum UiCall {
    /// App-state read tools (selection, location, tabs, history).
    Read(ToolInput),
    /// UI control tools.
    Ui(ToolInput, String),
    /// Job tools (status, cancel).
    Meta(ToolInput),
    /// Submit an approved plan; answers `{"job_id": …}` or the app action's result.
    /// `(plan, caller, call key)`; the key links the job to its approval card.
    Execute(Plan, String, usize),
}

pub struct UiRequest {
    pub call: UiCall,
    pub reply: Sender<Result<Value, ToolError>>,
}

/// Messages from server threads to the UI thread.
pub enum BridgeMsg {
    Approval(PendingMcpApproval),
    Ui(UiRequest),
    Register(String, Transport),
}

/// Tool host of the window: soyuz-fs for files, the UI thread for app state.
struct AppHost {
    fs: FsToolHost,
    tx: Sender<BridgeMsg>,
    wake: Wake,
}

impl AppHost {
    fn ask(&self, call: UiCall, ctl: &OpControl) -> Result<Value, ToolError> {
        let (reply, rx) = crossbeam_channel::bounded(1);
        self.tx
            .send(BridgeMsg::Ui(UiRequest { call, reply }))
            .map_err(|_| ToolError::Failed("Soyuz is closing".into()))?;
        (self.wake)();
        let deadline = Instant::now() + UI_TIMEOUT;
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => return r,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return Err(ToolError::Failed("Soyuz is closing".into()));
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
            if ctl.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            if Instant::now() > deadline {
                return Err(ToolError::Failed("the Soyuz window did not answer in time".into()));
            }
        }
    }
}

impl ToolHost for AppHost {
    fn scope(&self) -> &Scope {
        &self.fs.scope
    }

    fn read(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        match input {
            ToolInput::GetSelection(_)
            | ToolInput::GetCurrentLocation(_)
            | ToolInput::ListTabs(_)
            | ToolInput::HistoryList(_) => self.ask(UiCall::Read(input.clone()), &ctx.ctl),
            _ => self.fs.read(input, ctx),
        }
    }

    fn plan(&self, input: &ToolInput, ctx: &CallContext) -> Result<Plan, ToolError> {
        self.fs.plan(input, ctx)
    }

    fn ui(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        self.ask(UiCall::Ui(input.clone(), ctx.caller.clone()), &ctx.ctl)
    }

    fn meta(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        self.ask(UiCall::Meta(input.clone()), &ctx.ctl)
    }
}

/// The server-call identity shared by `request_approval` and the later `execute`.
fn call_key(ctl: &OpControl) -> usize {
    ctl as *const OpControl as usize
}

/// The [`Backend`] of the window.
pub struct GuiBackend {
    ai: RwLock<Ai>,
    host: RwLock<Arc<AppHost>>,
    tx: Sender<BridgeMsg>,
    wake: Wake,
    engine: Arc<JobEngine>,
    /// The shared audit log (`ai-audit.jsonl`; a temp file in tests).
    audit_path: PathBuf,
}

impl GuiBackend {
    /// A backend whose UI side is whoever drains `tx`'s receiver (the window's [`pump`],
    /// or a test).
    pub fn new(ai: &Ai, engine: Arc<JobEngine>, tx: Sender<BridgeMsg>, wake: Wake, audit_path: PathBuf) -> Arc<Self> {
        Arc::new(GuiBackend {
            ai: RwLock::new(ai.clone()),
            host: RwLock::new(Arc::new(AppHost {
                fs: FsToolHost::from_config(&ai.scope),
                tx: tx.clone(),
                wake: wake.clone(),
            })),
            tx,
            wake,
            engine,
            audit_path,
        })
    }

    fn set_ai(&self, ai: &Ai) {
        let scope_changed = self.ai.read().scope != ai.scope;
        *self.ai.write() = ai.clone();
        if scope_changed {
            *self.host.write() = Arc::new(AppHost {
                fs: FsToolHost::from_config(&ai.scope),
                tx: self.tx.clone(),
                wake: self.wake.clone(),
            });
        }
    }
}

impl Backend for GuiBackend {
    fn mode(&self) -> Mode {
        Mode::Gui
    }

    fn host(&self) -> Arc<dyn ToolHost> {
        self.host.read().clone()
    }

    fn ai(&self) -> Ai {
        self.ai.read().clone()
    }

    fn request_approval(&self, req: ApprovalRequest, ctl: &OpControl) -> ApprovalDecision {
        let (reply, rx) = crossbeam_channel::bounded(1);
        // The card watches this control; it is cancelled with the call.
        let watch = OpControl::new();
        let timeout = Duration::from_secs(self.ai.read().mcp.approval_timeout_secs.clamp(10, 3600));
        let deadline = Instant::now() + timeout;
        let pending = PendingMcpApproval::new(req, watch.clone(), timeout, call_key(ctl), reply);
        if self.tx.send(BridgeMsg::Approval(pending)).is_err() {
            return ApprovalDecision::Unavailable("Soyuz is closing".into());
        }
        (self.wake)();
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(d) => return d,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return ApprovalDecision::Unavailable("Soyuz is closing".into());
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
            if ctl.is_cancelled() {
                watch.cancel();
                return ApprovalDecision::Cancelled;
            }
            if Instant::now() > deadline {
                watch.cancel();
                (self.wake)();
                return ApprovalDecision::TimedOut;
            }
        }
    }

    fn execute(&self, plan: &Plan, ctx: &CallContext, progress: &ProgressFn<'_>) -> Result<Value, ToolError> {
        let host = self.host.read().clone();
        let v = host.ask(UiCall::Execute(plan.clone(), ctx.caller.clone(), call_key(&ctx.ctl)), &ctx.ctl)?;
        match v.get("job_id").and_then(Value::as_u64) {
            Some(id) => {
                let r = soyuz_mcp::jobs::wait_job(&self.engine, JobId(id), &ctx.ctl, progress, MAX_JOB_WAIT, false);
                if ctx.ctl.is_cancelled() {
                    return Err(ToolError::Cancelled);
                }
                Ok(r)
            }
            None => Ok(v),
        }
    }

    fn register_pending(&self, name: &str, transport: Transport) {
        self.ai.write().mcp_clients.push(AiMcpClient { name: name.to_string(), pending: true, ..Default::default() });
        let _ = self.tx.send(BridgeMsg::Register(name.to_string(), transport));
        (self.wake)();
    }

    fn audit(&self, rec: soyuz_ai::audit::AuditRecord) {
        if let Err(e) = soyuz_ai::audit::append(&self.audit_path, &rec) {
            log::warn!("ai-audit.jsonl: {e}");
        }
    }
}

/// Live status for the Connections page.
#[derive(Clone, Debug, Default)]
pub struct BridgeStatus {
    /// `Ok(pipe name)` / `Err(why not)`; `None` = off.
    pub pipe: Option<Result<String, String>>,
    /// `Ok(address)` / `Err(why not)`; `None` = off.
    pub http: Option<Result<String, String>>,
    /// Calls waiting for approval.
    pub waiting: usize,
}

/// App-owned MCP state (lives in [`crate::services::Services`]).
pub struct McpBridge {
    tx: Sender<BridgeMsg>,
    rx: Receiver<BridgeMsg>,
    backend: Option<Arc<GuiBackend>>,
    server: Option<Arc<Server>>,
    pipe: Option<PipeServer>,
    http: Option<(u16, HttpServer)>,
    http_settings: HttpSettings,
    tokens_rx: Option<Receiver<Vec<(String, Secret)>>>,
    applied: Option<Ai>,
    /// Plan cards of MCP requests (shared with the modal card dialog).
    pub approvals: Arc<Mutex<McpApprovals>>,
    /// A modal card is open (`approval_ui = "dialog"`).
    modal_open: Arc<AtomicBool>,
    pub status: BridgeStatus,
    disabled: bool,
}

impl Default for McpBridge {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        McpBridge {
            tx,
            rx,
            backend: None,
            server: None,
            pipe: None,
            http: None,
            http_settings: HttpSettings::default(),
            tokens_rx: None,
            applied: None,
            approvals: Arc::new(Mutex::new(McpApprovals::default())),
            modal_open: Arc::new(AtomicBool::new(false)),
            status: BridgeStatus::default(),
            disabled: false,
        }
    }
}

impl McpBridge {
    /// Re-resolve HTTP tokens after the Connections page stored a new one.
    pub fn refresh_tokens(&mut self) {
        self.applied = None;
    }

    fn reconfigure(&mut self, ai: &Ai, engine: Arc<JobEngine>, wake: Wake) {
        let tx = self.tx.clone();
        let backend = self
            .backend
            .get_or_insert_with(|| GuiBackend::new(ai, engine, tx, wake.clone(), soyuz_ai::audit::default_path()))
            .clone();
        backend.set_ai(ai);
        let server = self.server.get_or_insert_with(|| Server::new(backend.clone())).clone();

        let want_pipe = ai.enabled && ai.mcp.server_enabled;
        if !want_pipe {
            self.pipe = None;
            self.status.pipe = None;
        } else if self.pipe.is_none() && !matches!(self.status.pipe, Some(Err(_))) {
            let name = soyuz_mcp::pipe::pipe_name();
            match PipeServer::start(&name, server.clone()) {
                Ok(p) => {
                    log::info!("MCP: serving {name}");
                    self.pipe = Some(p);
                    self.status.pipe = Some(Ok(name));
                }
                Err(e) => {
                    log::warn!("MCP: cannot serve {name}: {e} (another Soyuz window may serve it)");
                    self.status.pipe = Some(Err(format!("not available: {e}")));
                }
            }
        }

        *self.http_settings.allowed_origins.write() = ai.mcp.allowed_origins.clone();
        let want_http = ai.enabled && ai.mcp.http_enabled;
        let port = ai.mcp.http_port;
        if !want_http {
            self.http = None;
            self.status.http = None;
        } else if self.http.as_ref().is_none_or(|(p, _)| *p != port) {
            self.http = None;
            match HttpServer::start(server, port, self.http_settings.clone()) {
                Ok(h) => {
                    self.status.http = Some(Ok(format!("http://{}/mcp", h.addr())));
                    self.http = Some((port, h));
                }
                Err(e) => self.status.http = Some(Err(format!("cannot listen on 127.0.0.1:{port}: {e}"))),
            }
        }
        // Tokens come from the credential store: resolve them off the UI thread.
        if want_http {
            let clients: Vec<(String, String)> = ai
                .mcp_clients
                .iter()
                .filter(|c| !c.pending && !c.token.trim().is_empty())
                .map(|c| (soyuz_mcp::trust::sanitize_name(&c.name), c.token.clone()))
                .collect();
            let (tx, rx) = crossbeam_channel::bounded(1);
            let w = wake.clone();
            let _ = std::thread::Builder::new().name("soyuz-mcp-tokens".into()).spawn(move || {
                let mut out = Vec::new();
                for (name, reference) in clients {
                    match soyuz_ai::credentials::resolve(&reference) {
                        Ok(Some(s)) => out.push((name, s)),
                        Ok(None) => log::warn!("MCP client “{name}”: token {reference} is not stored"),
                        Err(e) => log::warn!("MCP client “{name}”: {e}"),
                    }
                }
                let _ = tx.send(out);
                w();
            });
            self.tokens_rx = Some(rx);
        }
    }
}

fn wake_of(ctx: &egui::Context) -> Wake {
    let c = ctx.clone();
    Arc::new(move || c.request_repaint())
}

/// Per frame: (re)start servers on config changes, answer UI requests, show approvals.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    if app.secondary {
        // Secondary / safe-mode windows never serve MCP (the first window does).
        app.services.mcp.disabled = true;
        return;
    }
    if app.services.mcp.applied.as_ref() != Some(&app.config.ai) {
        let ai = app.config.ai.clone();
        let engine = app.fileops.jobs.engine_arc();
        app.services.mcp.reconfigure(&ai, engine, wake_of(ctx));
        app.services.mcp.applied = Some(ai);
    }
    if let Some(rx) = &app.services.mcp.tokens_rx
        && let Ok(tokens) = rx.try_recv()
    {
        *app.services.mcp.http_settings.tokens.write() = tokens;
        app.services.mcp.tokens_rx = None;
    }
    let msgs: Vec<BridgeMsg> = app.services.mcp.rx.try_iter().collect();
    for m in msgs {
        match m {
            BridgeMsg::Approval(p) => {
                app.services.mcp.approvals.lock().push(p);
            }
            BridgeMsg::Ui(req) => {
                let r = answer(app, req.call);
                let _ = req.reply.send(r);
            }
            BridgeMsg::Register(name, transport) => register(app, ctx, name, transport),
        }
    }
    // Cards: expire the ones whose call went away, follow their jobs.
    let approvals = app.services.mcp.approvals.clone();
    let mut a = approvals.lock();
    {
        let jobs = &app.fileops.jobs;
        a.tick(|id| jobs.engine().snapshot(id).map(|s| s.state));
    }
    app.services.mcp.status.waiting = a.pending();
    let arrived = std::mem::take(&mut a.arrived);
    let first_pending = a.cards.iter().find(|c| c.is_pending()).map(|c| c.card.id);
    let busy = a.busy();
    drop(a);
    if arrived {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    match app.config.ai.mcp.approval_ui {
        AiMcpApprovalUi::Panel => {
            if arrived {
                crate::ai::state::show_mcp_requests(app);
            }
        }
        AiMcpApprovalUi::Dialog => {
            if let Some(id) = first_pending
                && !app.services.mcp.modal_open.load(Ordering::SeqCst)
            {
                app.services.mcp.modal_open.store(true, Ordering::SeqCst);
                let flag = app.services.mcp.modal_open.clone();
                app.ui.dialogs.push(crate::ui::dialogs::mcp_approval::McpApprovalDialog::new(id, approvals, flag));
            }
        }
    }
    if busy {
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn register(app: &mut SoyuzApp, ctx: &egui::Context, name: String, transport: Transport) {
    let known = app.config.ai.mcp_clients.iter().any(|c| soyuz_mcp::trust::sanitize_name(&c.name) == name);
    if !known {
        crate::ui::settings::update_config(app, ctx, |c| {
            c.ai.mcp_clients.push(AiMcpClient { name: name.clone(), pending: true, ..Default::default() })
        });
        app.ui.toasts.push(Toast::info(format!("New MCP client “{name}”")).body(format!(
            "Connected over {}; it is read-only until you approve it in Settings → AI assistant → Connections.",
            transport.label()
        )));
    }
}

/// Paths of the selection in the focused pane (inside the AI scope).
fn selection_json(app: &SoyuzApp, scope: &Scope) -> Value {
    let pane = app.ws.pane();
    let entries: Vec<Value> = pane
        .selected_entries()
        .into_iter()
        .filter(|e| scope.check_read(&e.path).is_ok())
        .take(1000)
        .map(soyuz_ai::tools::host::entry_json)
        .collect();
    json!({"location": pane.location.display(), "count": entries.len(), "entries": entries})
}

fn dir_of(path: &std::path::Path) -> Result<(PathBuf, Option<String>), ToolError> {
    let meta = soyuz_fs::stat(path, true).map_err(|e| ToolError::Failed(e.to_string()))?;
    if meta.is_dir_like() {
        Ok((path.to_path_buf(), None))
    } else {
        let parent = path.parent().ok_or_else(|| ToolError::Failed("no parent folder".into()))?;
        Ok((parent.to_path_buf(), path.file_name().map(|n| n.to_string_lossy().into_owned())))
    }
}

fn answer(app: &mut SoyuzApp, call: UiCall) -> Result<Value, ToolError> {
    let scope = Scope::from_config(&app.config.ai.scope);
    match call {
        UiCall::Read(input) => match input {
            ToolInput::GetSelection(_) => Ok(selection_json(app, &scope)),
            ToolInput::GetCurrentLocation(_) => {
                let loc = &app.ws.pane().location;
                let path = crate::services::jobs::location_dir(loc);
                if path.as_ref().is_some_and(|p| scope.check_read(p).is_err()) {
                    return Ok(
                        json!({"location": null, "note": "the current folder is outside the folders AI may use"}),
                    );
                }
                Ok(json!({"location": loc.display(), "path": path, "is_folder": path.is_some()}))
            }
            ToolInput::ListTabs(_) => {
                let tabs: Vec<Value> = app
                    .ws
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let panes: Vec<Value> = t
                            .panes
                            .iter()
                            .enumerate()
                            .take(if t.dual { 2 } else { 1 })
                            .map(|(j, p)| {
                                let visible = crate::services::jobs::location_dir(&p.location)
                                    .is_none_or(|d| scope.check_read(&d).is_ok());
                                json!({"location": if visible { json!(p.location.display()) } else { Value::Null }, "focused": j == t.focused})
                            })
                            .collect();
                        json!({"index": i, "active": i == app.ws.active, "pinned": t.pinned, "panes": panes})
                    })
                    .collect();
                Ok(json!({"tabs": tabs}))
            }
            ToolInput::HistoryList(a) => {
                Ok(soyuz_mcp::jobs::history_json(app.fileops.jobs.history(), a.limit.unwrap_or(50).min(500) as usize))
            }
            other => Err(ToolError::Unsupported(other.name().into())),
        },
        UiCall::Meta(input) => match input {
            ToolInput::JobStatus(a) => match a.job_id {
                Some(id) => app
                    .fileops
                    .jobs
                    .engine()
                    .snapshot(JobId(id))
                    .map(|s| soyuz_mcp::jobs::snapshot_json(&s))
                    .ok_or_else(|| ToolError::Failed(format!("no job {id}"))),
                None => Ok(
                    json!({"jobs": app.fileops.jobs.snapshots().iter().map(soyuz_mcp::jobs::snapshot_json).collect::<Vec<_>>()}),
                ),
            },
            ToolInput::CancelJob(a) => {
                app.fileops.jobs.engine().cancel(JobId(a.job_id));
                Ok(json!({"job_id": a.job_id, "cancel_requested": true}))
            }
            other => Err(ToolError::Unsupported(other.name().into())),
        },
        UiCall::Ui(input, caller) => {
            let pane = app.ws.pane().id;
            match input {
                ToolInput::Navigate(a) => {
                    let (dir, select) = dir_of(&a.path)?;
                    ops::navigate(app, pane, Location::Dir(dir.clone()), NavOpts { select, ..NavOpts::user() });
                    Ok(json!({"navigated": dir}))
                }
                ToolInput::OpenTab(a) => {
                    let (dir, _) = dir_of(&a.path)?;
                    ops::open_in_new_tab(app, Location::Dir(dir.clone()), true);
                    Ok(json!({"opened_tab": dir}))
                }
                ToolInput::Reveal(a) | ToolInput::Preview(a) => {
                    let parent = a.path.parent().ok_or_else(|| ToolError::Failed("no parent folder".into()))?;
                    let name = a.path.file_name().map(|n| n.to_string_lossy().into_owned());
                    ops::navigate(
                        app,
                        pane,
                        Location::Dir(parent.to_path_buf()),
                        NavOpts { select: name, ..NavOpts::user() },
                    );
                    Ok(json!({"revealed": a.path}))
                }
                ToolInput::Select(a) => {
                    let parent = a.paths.first().and_then(|p| p.parent()).map(|p| p.to_path_buf());
                    let Some(parent) = parent else { return Ok(json!({"selected": 0})) };
                    if a.paths.iter().any(|p| p.parent() != Some(parent.as_path())) {
                        return Err(ToolError::InvalidArgs(vec!["all paths must be in the same folder".into()]));
                    }
                    let names: Vec<String> = a
                        .paths
                        .iter()
                        .filter_map(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .collect();
                    ops::navigate(app, pane, Location::Dir(parent), NavOpts::user());
                    let n = names.len();
                    app.ws.pane_mut().pending_selection = names;
                    Ok(json!({"selected": n}))
                }
                ToolInput::ShowToast(a) => {
                    let who = caller.strip_prefix("mcp:").unwrap_or(&caller).to_string();
                    let msg: String = a.message.chars().take(200).collect();
                    app.ui.toasts.push(Toast::info(format!("{who}: {msg}")));
                    Ok(json!({"shown": true}))
                }
                other => Err(ToolError::Unsupported(format!("{} over MCP", other.name()))),
            }
        }
        UiCall::Execute(plan, caller, key) => {
            let r = execute(app, &plan, &caller);
            let mut a = app.services.mcp.approvals.lock();
            match &r {
                Ok(v) => a.on_submitted(key, &plan, v.get("job_id").and_then(Value::as_u64).map(JobId)),
                Err(e) => a.on_failed(key, &e.to_string()),
            }
            r
        }
    }
}

/// The job of an approved MCP plan: the plan's spec with the app's configured copy
/// options, labelled `MCP <client> · <summary>` (activity tag, "undo everything this
/// client did").
pub fn job_spec(base: soyuz_jobs::JobSpec, plan: &Plan, caller: &str) -> soyuz_jobs::JobSpec {
    let Some(spec) = plan.job() else { return base };
    let mut full = base;
    full.options.conflict = spec.options.conflict.clone();
    full.options.errors = spec.options.errors;
    full.options.delete = spec.options.delete.clone();
    full.options.mv = spec.options.mv.clone();
    full.options.journal = true;
    let who = caller.strip_prefix("mcp:").unwrap_or(caller);
    let summary = spec.options.label.clone().unwrap_or_else(|| plan.summary.clone());
    full.options.label = Some(crate::ai::mcp::job_label(who, &summary));
    full
}

fn execute(app: &mut SoyuzApp, plan: &Plan, caller: &str) -> Result<Value, ToolError> {
    match &plan.action {
        PlanAction::Job(spec) => {
            let full = job_spec(app.fileops.jobs.spec(spec.kind.clone()), plan, caller);
            let who = caller.strip_prefix("mcp:").unwrap_or(caller);
            let items: Vec<PathBuf> =
                plan.items.iter().filter_map(|i| i.src.clone().or_else(|| i.dst.clone())).collect();
            let dest = plan.items.iter().find_map(|i| i.dst.as_ref().and_then(|d| d.parent().map(|p| p.to_path_buf())));
            let meta = JobMeta::new(OpKind::from_brief(full.kind.brief()), &items, dest).mcp(who);
            let id = app.fileops.jobs.submit(full, meta);
            Ok(json!({"job_id": id.0}))
        }
        PlanAction::App { op, args } if op.as_str() == "undo" => {
            let h =
                args.get("history_id").and_then(Value::as_u64).ok_or_else(|| ToolError::Failed("bad undo".into()))?;
            let id = app
                .fileops
                .jobs
                .engine()
                .undo(HistoryId(h))
                .ok_or_else(|| ToolError::Failed(format!("history entry {h} cannot be undone")))?;
            let who = caller.strip_prefix("mcp:").unwrap_or(caller);
            app.fileops.jobs.track(id, JobMeta::new(OpKind::Undo, &[], None).mcp(who));
            Ok(json!({"job_id": id.0}))
        }
        PlanAction::App { op, .. } => Err(ToolError::Unsupported(format!("“{op}” over MCP (use Soyuz directly)"))),
        PlanAction::None => Ok(json!({"status": "nothing to do"})),
    }
}
