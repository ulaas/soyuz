//! The agent loop (roadmap A5) on a worker thread. One [`spawn`] = one task:
//!
//! 1. the task context (built on the UI thread from in-memory state) goes in front of
//!    the user's message;
//! 2. the model is called with the tool catalog filtered by the risk ceiling and the
//!    folder permissions; text streams back as [`AgentEvent::Text`];
//! 3. read / UI tools run immediately (at most `max_auto_reads` per task), their results
//!    framed as untrusted data, filtered by folder permissions and redacted for cloud
//!    models;
//! 4. write tools come back from `soyuz_ai::tools::dispatch` as plans; the loop emits
//!    [`AgentEvent::Proposed`] and **pauses** until every plan has a [`Decision`]
//!    (auto-approval only for conflict-free *write* plans when configured, and never once
//!    the task has read file contents or injection-looking text);
//! 5. approved plans are submitted one by one through the [`Executor`] (AI-tagged jobs),
//!    awaited, and the job results go back to the model; repeat.
//!
//! Limits: steps, tokens, wall clock, monthly budget caps; [`AgentHandle::cancel`] (Esc)
//! stops at the next event, cancelling a running job. Old tool results are compacted
//! when the conversation grows. Recipe runs ([`Mode::Recipe`]) use the same plan /
//! approval / execution path without a model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::{Value, json};
use soyuz_ai::tools::{
    self, CallContext, Category, Plan, Risk, ToolHost, ToolInput, ToolResponse, frame_error, frame_untrusted,
};
use soyuz_ai::{AiError, ChatRequest, Collected, Event, Message, Part, PriceTable, Role, Usage, UsageLedger};
use soyuz_core::OpControl;
use soyuz_core::config::{AiAgent, AiApproval, AiPrivacy, AiRiskCeiling};
use soyuz_jobs::JobId;

use super::audit::Audit;
use super::context::{PromptCaps, compact, system_prompt};
use super::exec::{Executor, Outcome};
use super::model::{ChatModel, ModelStream};
use super::safety::{Permissions, looks_like_injection, reads_contents, redact};
use super::{CancelFlag, TaskId, Wake};

/// Per-task limits.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_steps: u32,
    /// 0 = unlimited.
    pub token_budget: u64,
    pub wall_clock: Duration,
    pub max_auto_reads: u32,
    pub compact_after_chars: usize,
    pub keep_recent: usize,
    pub approval: AiApproval,
    pub max_risk: Risk,
}

impl Limits {
    pub fn from_config(a: &AiAgent) -> Self {
        Limits {
            max_steps: a.max_steps.clamp(1, 200),
            token_budget: a.token_budget,
            wall_clock: Duration::from_secs(a.time_limit_secs.clamp(10, 24 * 3600)),
            max_auto_reads: a.max_auto_reads.max(1),
            compact_after_chars: (a.compact_after_kib.max(8) * 1024) as usize,
            keep_recent: a.keep_recent_results as usize,
            approval: a.approval,
            max_risk: match a.max_risk {
                AiRiskCeiling::Read => Risk::Read,
                AiRiskCeiling::Write => Risk::Write,
                AiRiskCeiling::Destructive => Risk::Destructive,
            },
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits::from_config(&AiAgent::default())
    }
}

/// Monthly budget accounting (`ai-usage.json`).
#[derive(Clone, Debug)]
pub struct Budget {
    pub ledger_path: Option<PathBuf>,
    pub prices: PriceTable,
    /// Cap of the profile in USD (0 = none).
    pub cap_usd: f64,
    pub warn_percent: u32,
}

pub enum Mode {
    /// A chat turn: `context` is prepended to `prompt`; `history` is the conversation so far.
    Chat { prompt: String, context: String, history: Vec<Message> },
    /// Re-run recorded steps without a model.
    Recipe { name: String, steps: Vec<(String, Value)> },
}

pub struct TaskSpec {
    pub id: TaskId,
    pub mode: Mode,
    pub limits: Limits,
    pub budget: Option<Budget>,
    /// Redact secrets in data sent to cloud models (`ai.agent.redact_cloud`).
    pub redact_cloud: bool,
    pub today: String,
}

/// Builds the model on the worker thread (credential store reads happen there).
pub type ModelFactory = Box<dyn FnOnce() -> Result<Arc<dyn ChatModel>, String> + Send>;

pub struct Deps {
    pub model: Option<ModelFactory>,
    pub host: Arc<dyn ToolHost>,
    pub exec: Arc<dyn Executor>,
    pub perms: Arc<Permissions>,
    pub audit: Option<Audit>,
    pub wake: Wake,
}

/// A plan waiting for the user.
#[derive(Clone, Debug)]
pub struct ProposedPlan {
    pub id: u64,
    pub call_id: String,
    pub tool: String,
    pub args: Value,
    pub plan: Plan,
    /// Proposed after the task read file contents or injection-looking text: never
    /// auto-approved.
    pub tainted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// The user clicked Approve on this card.
    Approve,
    /// "Approve all of this kind" applied automatically; refused for tainted plans.
    ApproveKind,
    Reject(String),
    /// New arguments; the plan is rebuilt (dry run) and proposed again.
    Edit(Value),
}

pub enum AgentInput {
    Decide(u64, Decision),
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Finish {
    Done,
    Cancelled,
    StepLimit,
    TokenBudget,
    TimeLimit,
    MonthlyCap,
    Error(String),
}

impl Finish {
    pub fn describe(&self) -> String {
        match self {
            Finish::Done => "Done".into(),
            Finish::Cancelled => "Stopped".into(),
            Finish::StepLimit => "Stopped: step limit reached".into(),
            Finish::TokenBudget => "Stopped: token budget for this task used up".into(),
            Finish::TimeLimit => "Stopped: time limit reached".into(),
            Finish::MonthlyCap => "Stopped: monthly budget cap reached".into(),
            Finish::Error(e) => format!("Error: {e}"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Model {
        profile: String,
        model: String,
        local: bool,
    },
    Text(String),
    Reasoning(String),
    /// A model turn ended (the text bubble is complete).
    TurnEnd,
    ToolStarted {
        call_id: String,
        tool: String,
        detail: String,
        paths: Vec<PathBuf>,
    },
    ToolFinished {
        call_id: String,
        ok: bool,
        brief: String,
    },
    /// The model asked for a dry run: shown as a read-only preview card.
    Preview {
        call_id: String,
        tool: String,
        plan: Box<Plan>,
    },
    Proposed(Vec<ProposedPlan>),
    AutoApproved(u64),
    Executing {
        plan: u64,
        job: Option<JobId>,
    },
    Executed {
        plan: u64,
        ok: bool,
        message: String,
    },
    Rejected(u64),
    /// Cumulative usage of the task.
    Usage {
        usage: Usage,
        cost: Option<f64>,
    },
    Notice {
        text: String,
        warn: bool,
    },
    Compacted(usize),
    Finished {
        reason: Finish,
        messages: Vec<Message>,
        executed: Vec<(String, Value)>,
    },
}

pub struct AgentHandle {
    pub id: TaskId,
    pub events: Receiver<AgentEvent>,
    input: Sender<AgentInput>,
    pub cancel: CancelFlag,
    join: Option<JoinHandle<()>>,
}

impl AgentHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
        let _ = self.input.send(AgentInput::Cancel);
    }

    pub fn decide(&self, plan: u64, d: Decision) {
        let _ = self.input.send(AgentInput::Decide(plan, d));
    }

    /// Wait for the worker (tests).
    pub fn join(mut self) {
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }

    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

/// Start a task on a worker thread.
pub fn spawn(spec: TaskSpec, deps: Deps) -> AgentHandle {
    let (etx, erx) = crossbeam_channel::unbounded();
    let (itx, irx) = crossbeam_channel::unbounded();
    let cancel = CancelFlag::new();
    let id = spec.id;
    let flag = cancel.clone();
    let join = std::thread::Builder::new()
        .name(format!("soyuz-ai-task-{}", id.0))
        .spawn(move || {
            let mut run = Run::new(spec, deps, etx, irx, flag);
            run.run();
        })
        .map_err(|e| log::error!("cannot start the assistant task: {e}"))
        .ok();
    AgentHandle { id, events: erx, input: itx, cancel, join }
}

struct PlanResult {
    ok: bool,
    rejected: bool,
    text: String,
}

struct Run {
    id: TaskId,
    mode: Option<Mode>,
    limits: Limits,
    budget: Option<Budget>,
    ledger: Option<UsageLedger>,
    redact_cloud: bool,
    today: String,
    model_factory: Option<ModelFactory>,
    model: Option<Arc<dyn ChatModel>>,
    host: Arc<dyn ToolHost>,
    exec: Arc<dyn Executor>,
    perms: Arc<Permissions>,
    audit: Option<Audit>,
    wake: Wake,
    tx: Sender<AgentEvent>,
    rx: Receiver<AgentInput>,
    cancel: CancelFlag,
    ctl: Arc<OpControl>,
    deadline: Instant,
    usage: Usage,
    cost: Option<f64>,
    reads: u32,
    tainted: bool,
    next_plan: u64,
    executed: Vec<(String, Value)>,
    messages: Vec<Message>,
    warned_budget: bool,
    early_decisions: HashMap<u64, Decision>,
}

impl Run {
    fn new(spec: TaskSpec, deps: Deps, tx: Sender<AgentEvent>, rx: Receiver<AgentInput>, cancel: CancelFlag) -> Self {
        Run {
            id: spec.id,
            mode: Some(spec.mode),
            deadline: Instant::now() + spec.limits.wall_clock,
            limits: spec.limits,
            budget: spec.budget,
            ledger: None,
            redact_cloud: spec.redact_cloud,
            today: spec.today,
            model_factory: deps.model,
            model: None,
            host: deps.host,
            exec: deps.exec,
            perms: deps.perms,
            audit: deps.audit,
            wake: deps.wake,
            tx,
            rx,
            cancel,
            ctl: OpControl::new(),
            usage: Usage::default(),
            cost: None,
            reads: 0,
            tainted: false,
            next_plan: 1,
            executed: Vec::new(),
            messages: Vec::new(),
            warned_budget: false,
            early_decisions: HashMap::new(),
        }
    }

    fn emit(&self, ev: AgentEvent) {
        let _ = self.tx.send(ev);
        (self.wake)();
    }

    fn audit(&self, kind: &str, tool: Option<&str>, detail: &str, job: Option<u64>) {
        if let Some(a) = &self.audit {
            a.log(kind, tool, detail, job);
        }
    }

    fn redacting(&self) -> bool {
        self.redact_cloud && self.model.as_ref().is_some_and(|m| !m.is_local())
    }

    fn run(&mut self) {
        let reason = match self.mode.take() {
            Some(Mode::Chat { prompt, context, history }) => self.chat(prompt, context, history),
            Some(Mode::Recipe { name, steps }) => self.recipe(&name, steps),
            None => Finish::Done,
        };
        if reason != Finish::Done {
            self.ctl.cancel();
        }
        self.audit("finish", None, &reason.describe(), None);
        let messages = std::mem::take(&mut self.messages);
        let executed = std::mem::take(&mut self.executed);
        self.emit(AgentEvent::Finished { reason, messages, executed });
    }

    /// Stop conditions checked between events.
    fn check(&self) -> Result<(), Finish> {
        if self.cancel.is_cancelled() {
            return Err(Finish::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(Finish::TimeLimit);
        }
        if self.limits.token_budget > 0 && self.usage.total() >= self.limits.token_budget {
            return Err(Finish::TokenBudget);
        }
        Ok(())
    }

    fn drain_inputs(&mut self) {
        while let Ok(i) = self.rx.try_recv() {
            match i {
                AgentInput::Cancel => self.cancel.cancel(),
                AgentInput::Decide(id, d) => {
                    self.early_decisions.insert(id, d);
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------
    // Chat mode

    fn chat(&mut self, prompt: String, context: String, history: Vec<Message>) -> Finish {
        let Some(factory) = self.model_factory.take() else { return Finish::Error("no model".into()) };
        let model = match factory() {
            Ok(m) => m,
            Err(e) => return Finish::Error(e),
        };
        self.emit(AgentEvent::Model {
            profile: model.profile().to_string(),
            model: model.model().to_string(),
            local: model.is_local(),
        });
        if let Some(a) = &self.audit {
            a.set_model(&format!("{}/{}", model.profile(), model.model()));
        }
        self.model = Some(model.clone());
        if let Some(b) = &self.budget {
            self.ledger = Some(match &b.ledger_path {
                Some(p) => UsageLedger::load(p).unwrap_or_default(),
                None => UsageLedger::default(),
            });
        }
        let contents = self.perms.any_contents();
        let system = system_prompt(&PromptCaps {
            contents_allowed: contents,
            auto_approve: self.limits.approval == AiApproval::AutoWrite,
            today: self.today.clone(),
        });
        let privacy = if contents { AiPrivacy::Contents } else { AiPrivacy::MetadataOnly };
        let specs = tools::specs(self.limits.max_risk, privacy);
        self.messages = history;
        let context = if self.redacting() { redact(&context).0 } else { context };
        let first = if context.trim().is_empty() { prompt.clone() } else { format!("{context}\n{prompt}") };
        self.messages.push(Message::user(first));
        self.audit("prompt", None, &prompt, None);

        for _step in 0..self.limits.max_steps {
            self.drain_inputs();
            if let Err(f) = self.check() {
                return f;
            }
            if let Err(f) = self.check_monthly(model.as_ref()) {
                return f;
            }
            let n = compact(&system, &mut self.messages, self.limits.compact_after_chars, self.limits.keep_recent);
            if n > 0 {
                self.emit(AgentEvent::Compacted(n));
            }
            let req = ChatRequest {
                model: model.model().to_string(),
                system: system.clone(),
                messages: self.messages.clone(),
                tools: specs.clone(),
                max_tokens: model.max_tokens(),
                temperature: None,
                tool_protocol: model.protocol(),
            };
            let stream = model.stream(req, self.wake.clone());
            let c = match self.collect(&stream) {
                Ok(c) => c,
                Err(f) => return f,
            };
            self.account(model.as_ref(), &c.usage);
            if let Some(e) = &c.error {
                return if *e == AiError::Cancelled { Finish::Cancelled } else { Finish::Error(e.to_string()) };
            }
            self.messages.push(c.to_message());
            self.emit(AgentEvent::TurnEnd);
            if c.tool_calls.is_empty() {
                return Finish::Done;
            }
            // The turn that crossed the token budget does not get its tools run.
            if self.limits.token_budget > 0 && self.usage.total() >= self.limits.token_budget {
                return Finish::TokenBudget;
            }
            match self.run_calls(c.tool_calls) {
                Ok(parts) => self.messages.push(Message { role: Role::User, parts }),
                Err(f) => return f,
            }
        }
        Finish::StepLimit
    }

    fn collect(&mut self, stream: &ModelStream) -> Result<Collected, Finish> {
        let mut c = Collected::default();
        loop {
            self.drain_inputs();
            if let Err(f) = self.check() {
                stream.cancel();
                return Err(f);
            }
            match stream.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ev) => {
                    let term = ev.is_terminal();
                    match &ev {
                        Event::TextDelta(t) => self.emit(AgentEvent::Text(t.clone())),
                        Event::ReasoningDelta(t) => self.emit(AgentEvent::Reasoning(t.clone())),
                        _ => {}
                    }
                    c.push(ev);
                    if term {
                        return Ok(c);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if c.stop.is_none() && c.error.is_none() {
                        c.error = Some(AiError::Network("the response stream ended unexpectedly".into()));
                    }
                    return Ok(c);
                }
            }
        }
    }

    fn check_monthly(&mut self, model: &dyn ChatModel) -> Result<(), Finish> {
        let (Some(b), Some(l)) = (&self.budget, &self.ledger) else { return Ok(()) };
        let month = soyuz_ai::pricing::current_month();
        match l.budget(&month, model.profile(), b.cap_usd, b.warn_percent) {
            soyuz_ai::BudgetStatus::Exceeded => Err(Finish::MonthlyCap),
            soyuz_ai::BudgetStatus::Warn { fraction } if !self.warned_budget => {
                self.warned_budget = true;
                self.emit(AgentEvent::Notice {
                    text: format!(
                        "“{}” has used {:.0} % of its monthly budget (${:.2}).",
                        model.profile(),
                        fraction * 100.0,
                        b.cap_usd
                    ),
                    warn: true,
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn account(&mut self, model: &dyn ChatModel, u: &Usage) {
        if u.total() == 0 {
            return;
        }
        self.usage.add(u);
        let mut step_cost = None;
        if let Some(b) = &self.budget {
            step_cost = b.prices.cost(model.model(), u);
            if let Some(l) = &mut self.ledger {
                l.record(&soyuz_ai::pricing::current_month(), model.profile(), model.model(), u, &b.prices);
                if let Some(p) = &b.ledger_path
                    && let Err(e) = l.save(p)
                {
                    log::warn!("ai usage ledger: {e}");
                }
            }
        }
        if let Some(c) = step_cost {
            *self.cost.get_or_insert(0.0) += c;
        }
        self.emit(AgentEvent::Usage { usage: self.usage, cost: self.cost });
    }

    // -----------------------------------------------------------------------------------
    // Tool calls

    fn ctx(&self, privacy: AiPrivacy) -> CallContext {
        CallContext { max_risk: self.limits.max_risk, privacy, caller: "assistant".into(), ctl: self.ctl.clone() }
    }

    fn needs_plan(name: &str) -> bool {
        tools::def(name).is_some_and(|d| matches!(d.category, Category::Write | Category::Destructive) || d.approval)
    }

    fn run_calls(&mut self, calls: Vec<(String, String, Value, Option<Value>)>) -> Result<Vec<Part>, Finish> {
        let n = calls.len();
        let mut results: Vec<Option<String>> = vec![None; n];
        let mut errors = vec![false; n];
        let mut pending: Vec<(usize, ProposedPlan)> = Vec::new();
        for (i, (call_id, name, args, _)) in calls.iter().enumerate() {
            self.drain_inputs();
            self.check()?;
            let input = ToolInput::parse(name, args.clone()).ok();
            let paths: Vec<PathBuf> =
                input.as_ref().map(|t| t.read_paths().into_iter().chain(t.write_paths()).collect()).unwrap_or_default();
            let planned = Self::needs_plan(name);
            if !planned {
                if self.reads >= self.limits.max_auto_reads {
                    results[i] = Some(frame_untrusted(
                        name,
                        &json!({"error": "read-tool limit for this task reached; summarise what you have and finish"}),
                    ));
                    errors[i] = true;
                    continue;
                }
                self.reads += 1;
            }
            self.emit(AgentEvent::ToolStarted {
                call_id: call_id.clone(),
                tool: name.clone(),
                detail: brief_args(args),
                paths: paths.clone(),
            });
            self.audit("tool_call", Some(name), &args.to_string(), None);
            let ctx = self.ctx(self.perms.call_privacy(input.as_ref()));
            match tools::dispatch(self.host.as_ref(), name, args.clone(), &ctx) {
                Ok(ToolResponse::Data(mut v)) => {
                    let hidden = self.perms.filter_payload(&mut v);
                    let mut text = frame_untrusted(name, &v);
                    if reads_contents(name, args) || looks_like_injection(&text) {
                        if !self.tainted {
                            self.audit("taint", Some(name), "task read untrusted content; auto-approval off", None);
                        }
                        self.tainted = true;
                    }
                    if self.redacting() {
                        text = redact(&text).0;
                    }
                    let brief = summarize_data(&v, hidden);
                    self.emit(AgentEvent::ToolFinished { call_id: call_id.clone(), ok: true, brief });
                    results[i] = Some(text);
                }
                Ok(ToolResponse::Plan(plan)) => {
                    let text = ToolResponse::Plan(plan.clone()).to_model_text(name);
                    self.emit(AgentEvent::ToolFinished {
                        call_id: call_id.clone(),
                        ok: true,
                        brief: plan.summary.clone(),
                    });
                    self.emit(AgentEvent::Preview {
                        call_id: call_id.clone(),
                        tool: name.clone(),
                        plan: Box::new(plan),
                    });
                    results[i] = Some(text);
                }
                Ok(ToolResponse::NeedsApproval(plan)) => {
                    let id = self.next_plan;
                    self.next_plan += 1;
                    self.emit(AgentEvent::ToolFinished {
                        call_id: call_id.clone(),
                        ok: true,
                        brief: plan.summary.clone(),
                    });
                    pending.push((
                        i,
                        ProposedPlan {
                            id,
                            call_id: call_id.clone(),
                            tool: name.clone(),
                            args: args.clone(),
                            plan,
                            tainted: false,
                        },
                    ));
                }
                Err(e) => {
                    self.emit(AgentEvent::ToolFinished { call_id: call_id.clone(), ok: false, brief: e.to_string() });
                    self.audit("error", Some(name), &e.to_string(), None);
                    results[i] = Some(frame_error(name, &e));
                    errors[i] = true;
                }
            }
        }
        if !pending.is_empty() {
            // Taint is decided after every read of this turn ran.
            for (_, pp) in &mut pending {
                pp.tainted = self.tainted;
            }
            let idx: Vec<usize> = pending.iter().map(|(i, _)| *i).collect();
            let outcomes = self.approve_plans(pending.into_iter().map(|(_, p)| p).collect())?;
            for (i, r) in idx.into_iter().zip(outcomes) {
                errors[i] = !r.ok && !r.rejected;
                results[i] = Some(r.text);
            }
        }
        Ok(calls
            .into_iter()
            .zip(results)
            .zip(errors)
            .map(|(((id, name, _, _), content), is_error)| Part::ToolResult {
                id,
                name,
                content: content.unwrap_or_default(),
                is_error,
            })
            .collect())
    }

    fn can_auto(&self, pp: &ProposedPlan) -> bool {
        self.limits.approval == AiApproval::AutoWrite
            && !pp.tainted
            && pp.plan.risk == Risk::Write
            && pp.plan.executable
            && pp.plan.conflicts == 0
            && tools::def(&pp.tool).is_some_and(|d| !d.approval)
    }

    /// Propose plans, wait for every decision, execute approved ones in order. Returns
    /// one result per plan (same order).
    fn approve_plans(&mut self, plans: Vec<ProposedPlan>) -> Result<Vec<PlanResult>, Finish> {
        let mut out: Vec<Option<PlanResult>> = (0..plans.len()).map(|_| None).collect();
        let mut round: Vec<(usize, ProposedPlan)> = plans.into_iter().enumerate().collect();
        for _attempt in 0..4 {
            if round.is_empty() {
                break;
            }
            let auto: Vec<u64> = round.iter().filter(|(_, p)| self.can_auto(p)).map(|(_, p)| p.id).collect();
            for (_, p) in &round {
                self.audit(
                    "plan",
                    Some(&p.tool),
                    &format!(
                        "plan {}: {} ({} items, executable: {}, tainted: {})",
                        p.id,
                        p.plan.summary,
                        p.plan.items.len(),
                        p.plan.executable,
                        p.tainted
                    ),
                    None,
                );
            }
            self.emit(AgentEvent::Proposed(round.iter().map(|(_, p)| p.clone()).collect()));
            for id in &auto {
                self.emit(AgentEvent::AutoApproved(*id));
            }
            let waiting: Vec<u64> = round.iter().map(|(_, p)| p.id).filter(|id| !auto.contains(id)).collect();
            let tainted: HashMap<u64, bool> = round.iter().map(|(_, p)| (p.id, p.tainted)).collect();
            let mut decisions = self.await_decisions(&waiting, &tainted)?;
            let mut next = Vec::new();
            for (idx, pp) in round {
                let d = if auto.contains(&pp.id) {
                    Decision::Approve
                } else {
                    decisions.remove(&pp.id).unwrap_or(Decision::Reject(String::new()))
                };
                match d {
                    Decision::Approve | Decision::ApproveKind => {
                        self.audit(
                            "approval",
                            Some(&pp.tool),
                            &format!(
                                "plan {} {}",
                                pp.id,
                                if auto.contains(&pp.id) { "auto-approved" } else { "approved" }
                            ),
                            None,
                        );
                        out[idx] = Some(self.execute(&pp)?);
                    }
                    Decision::Reject(reason) => {
                        self.audit("rejection", Some(&pp.tool), &format!("plan {}: {reason}", pp.id), None);
                        self.emit(AgentEvent::Rejected(pp.id));
                        let mut v = json!({"status": "rejected_by_user", "plan": pp.plan.summary, "changed": false});
                        if !reason.trim().is_empty() {
                            v["user_comment"] = json!(reason);
                        }
                        out[idx] = Some(PlanResult { ok: false, rejected: true, text: frame_untrusted(&pp.tool, &v) });
                    }
                    Decision::Edit(args) => {
                        self.audit("edit", Some(&pp.tool), &format!("plan {} edited: {args}", pp.id), None);
                        self.emit(AgentEvent::Rejected(pp.id));
                        let input = ToolInput::parse(&pp.tool, args.clone()).ok();
                        let ctx = self.ctx(self.perms.call_privacy(input.as_ref()));
                        let mut args = args;
                        if let Some(m) = args.as_object_mut() {
                            m.remove("dry_run");
                        }
                        match tools::dispatch(self.host.as_ref(), &pp.tool, args.clone(), &ctx) {
                            Ok(ToolResponse::NeedsApproval(plan) | ToolResponse::Plan(plan)) => {
                                let id = self.next_plan;
                                self.next_plan += 1;
                                next.push((idx, ProposedPlan { id, args, plan, ..pp }));
                            }
                            Ok(ToolResponse::Data(_)) => {
                                out[idx] = Some(PlanResult { ok: true, rejected: false, text: String::new() });
                            }
                            Err(e) => {
                                self.emit(AgentEvent::Notice {
                                    text: format!("The edited plan is invalid: {e}"),
                                    warn: true,
                                });
                                out[idx] =
                                    Some(PlanResult { ok: false, rejected: false, text: frame_error(&pp.tool, &e) });
                            }
                        }
                    }
                }
            }
            round = next;
        }
        Ok(out
            .into_iter()
            .map(|r| {
                r.unwrap_or(PlanResult {
                    ok: false,
                    rejected: true,
                    text: frame_untrusted("plan", &json!({"status": "rejected_by_user", "changed": false})),
                })
            })
            .collect())
    }

    fn await_decisions(&mut self, ids: &[u64], tainted: &HashMap<u64, bool>) -> Result<HashMap<u64, Decision>, Finish> {
        let mut got: HashMap<u64, Decision> = HashMap::new();
        let accept = |id: u64, d: Decision, got: &mut HashMap<u64, Decision>, run: &Run| {
            if !ids.contains(&id) {
                return;
            }
            if d == Decision::ApproveKind && tainted.get(&id).copied().unwrap_or(true) {
                run.emit(AgentEvent::Notice {
                    text: "This plan came right after the assistant read file contents, so it needs your explicit approval."
                        .into(),
                    warn: true,
                });
                return;
            }
            got.insert(id, d);
        };
        for (id, d) in std::mem::take(&mut self.early_decisions) {
            accept(id, d, &mut got, self);
        }
        while ids.iter().any(|id| !got.contains_key(id)) {
            if self.cancel.is_cancelled() {
                return Err(Finish::Cancelled);
            }
            if Instant::now() >= self.deadline {
                return Err(Finish::TimeLimit);
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(AgentInput::Cancel) => self.cancel.cancel(),
                Ok(AgentInput::Decide(id, d)) => accept(id, d, &mut got, self),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(Finish::Cancelled),
            }
        }
        Ok(got)
    }

    fn execute(&mut self, pp: &ProposedPlan) -> Result<PlanResult, Finish> {
        if !pp.plan.executable {
            let text = frame_untrusted(
                &pp.tool,
                &json!({"status": "not_executable", "reason": "blocking conflicts", "changed": false}),
            );
            self.emit(AgentEvent::Executed { plan: pp.id, ok: false, message: "Blocked by conflicts".into() });
            return Ok(PlanResult { ok: false, rejected: false, text });
        }
        match self.exec.submit(self.id, &pp.plan) {
            Err(e) => {
                self.audit("error", Some(&pp.tool), &format!("submit failed: {e}"), None);
                self.emit(AgentEvent::Executed { plan: pp.id, ok: false, message: e.clone() });
                Ok(PlanResult {
                    ok: false,
                    rejected: false,
                    text: frame_untrusted(&pp.tool, &json!({"status": "failed", "error": e})),
                })
            }
            Ok(sub) => {
                self.emit(AgentEvent::Executing { plan: pp.id, job: sub.job });
                let (ok, message, job) = match sub.job {
                    Some(job) => {
                        self.audit("job", Some(&pp.tool), &pp.plan.summary, Some(job.0));
                        let o = self.exec.wait(job, &self.cancel, self.deadline);
                        self.audit("job_result", Some(&pp.tool), &o.describe(), Some(job.0));
                        if o == Outcome::Cancelled && self.cancel.is_cancelled() {
                            self.emit(AgentEvent::Executed { plan: pp.id, ok: false, message: o.describe() });
                            return Err(Finish::Cancelled);
                        }
                        (o.ok(), o.describe(), Some(job.0))
                    }
                    None => {
                        self.audit("job", Some(&pp.tool), &format!("{}: {}", pp.plan.summary, sub.note), None);
                        (true, sub.note.clone(), None)
                    }
                };
                if ok {
                    self.executed.push((pp.tool.clone(), pp.args.clone()));
                }
                self.emit(AgentEvent::Executed { plan: pp.id, ok, message: message.clone() });
                let v = json!({
                    "status": if ok { "done" } else { "failed" },
                    "plan": pp.plan.summary,
                    "job_id": job,
                    "result": message,
                });
                Ok(PlanResult { ok, rejected: false, text: frame_untrusted(&pp.tool, &v) })
            }
        }
    }

    // -----------------------------------------------------------------------------------
    // Recipe mode

    fn recipe(&mut self, name: &str, steps: Vec<(String, Value)>) -> Finish {
        self.audit("prompt", None, &format!("run recipe “{name}” ({} steps)", steps.len()), None);
        let n = steps.len();
        for (i, (tool, args)) in steps.into_iter().enumerate() {
            self.drain_inputs();
            if let Err(f) = self.check() {
                return f;
            }
            self.emit(AgentEvent::Notice {
                text: format!("Step {}/{n}: {tool} {}", i + 1, brief_args(&args)),
                warn: false,
            });
            let input = ToolInput::parse(&tool, args.clone()).ok();
            let ctx = self.ctx(self.perms.call_privacy(input.as_ref()));
            self.audit("tool_call", Some(&tool), &args.to_string(), None);
            match tools::dispatch(self.host.as_ref(), &tool, args.clone(), &ctx) {
                Ok(ToolResponse::NeedsApproval(plan) | ToolResponse::Plan(plan)) => {
                    let id = self.next_plan;
                    self.next_plan += 1;
                    let pp = ProposedPlan {
                        id,
                        call_id: format!("step{i}"),
                        tool: tool.clone(),
                        args,
                        plan,
                        tainted: false,
                    };
                    match self.approve_plans(vec![pp]) {
                        Ok(r) if r.first().is_some_and(|r| r.ok) => {}
                        Ok(r) => {
                            let rejected = r.first().is_some_and(|r| r.rejected);
                            self.emit(AgentEvent::Notice {
                                text: if rejected {
                                    format!("Step {} was rejected; the recipe stopped.", i + 1)
                                } else {
                                    format!("Step {} did not complete; the recipe stopped.", i + 1)
                                },
                                warn: !rejected,
                            });
                            return Finish::Done;
                        }
                        Err(f) => return f,
                    }
                }
                Ok(ToolResponse::Data(_)) => {}
                Err(e) => return Finish::Error(format!("step {} ({tool}): {e}", i + 1)),
            }
        }
        Finish::Done
    }
}

/// Make a finished conversation safe to continue: an assistant turn whose tool calls got
/// no results (the task stopped mid-way) is dropped, and the history always ends with an
/// assistant message so the next user message alternates correctly.
pub fn sanitize_history(mut msgs: Vec<Message>) -> Vec<Message> {
    while let Some(last) = msgs.last() {
        let dangling = last.role == Role::Assistant && last.parts.iter().any(|p| matches!(p, Part::ToolUse { .. }));
        if dangling {
            msgs.pop();
            continue;
        }
        break;
    }
    if msgs.last().is_some_and(|m| m.role == Role::User) {
        msgs.push(Message::assistant("(stopped)"));
    }
    msgs
}

/// `path: …\Downloads, dest_dir: …` style summary of tool arguments.
pub fn brief_args(args: &Value) -> String {
    let Some(m) = args.as_object() else { return String::new() };
    let mut parts = Vec::new();
    for (k, v) in m {
        if k == "dry_run" {
            continue;
        }
        let s = match v {
            Value::String(s) => short_path(s),
            Value::Array(a) if a.len() == 1 => a[0].as_str().map(short_path).unwrap_or_else(|| a[0].to_string()),
            Value::Array(a) => format!("{} items", a.len()),
            Value::Null => continue,
            other => other.to_string(),
        };
        parts.push(format!("{k}: {s}"));
    }
    let s = parts.join(", ");
    if s.chars().count() > 120 { format!("{}…", s.chars().take(119).collect::<String>()) } else { s }
}

fn short_path(s: &str) -> String {
    let p = std::path::Path::new(s);
    if p.is_absolute() && p.components().count() > 3 {
        let tail: Vec<String> =
            p.components().rev().take(2).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        format!(
            "\u{2026}{}{}",
            std::path::MAIN_SEPARATOR,
            tail.into_iter().rev().collect::<Vec<_>>().join(std::path::MAIN_SEPARATOR_STR)
        )
    } else {
        s.to_string()
    }
}

/// One line describing a read tool's result for the chat.
fn summarize_data(v: &Value, hidden: usize) -> String {
    let mut s = Vec::new();
    for (key, label) in [
        ("entries", "entries"),
        ("hits", "matches"),
        ("groups", "duplicate groups"),
        ("items", "items"),
        ("sizes", "folders"),
        ("tabs", "tabs"),
    ] {
        if let Some(a) = v.get(key).and_then(Value::as_array) {
            let total = v.get("total").and_then(Value::as_u64).unwrap_or(a.len() as u64);
            s.push(format!("{total} {label}"));
        }
    }
    if let Some(t) = v.get("text").and_then(Value::as_str) {
        s.push(format!("{} lines read", t.lines().count()));
    }
    if v.get("digest").is_some() {
        s.push("checksum computed".into());
    }
    if hidden > 0 {
        s.push(format!("{hidden} hidden by folder permissions"));
    }
    if s.is_empty() { "ok".into() } else { s.join(", ") }
}
