//! Shared driver for the assistant harness tests (`ai_harness.rs`) and the task eval
//! suite (`ai_eval.rs`): a real `JobEngine` on a temp folder, the app's tool host and
//! executor, and a scripted "user" that answers plan cards.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde_json::Value;
use soyuz_ai::tools::{FsToolHost, Scope};
use soyuz_app::ai::agent::{self, AgentEvent, Decision, Deps, Finish, Limits, Mode, ProposedPlan, TaskSpec};
use soyuz_app::ai::audit::Audit;
use soyuz_app::ai::exec::EngineExecutor;
use soyuz_app::ai::host::{AppSnapshot, AppToolHost};
use soyuz_app::ai::model::ChatModel;
use soyuz_app::ai::safety::Permissions;
use soyuz_app::ai::{TaskId, Wake};
use soyuz_core::config::{AiAccess, AiFolderRule};
use soyuz_jobs::{EngineConfig, JobEngine};

pub fn engine() -> Arc<JobEngine> {
    JobEngine::new(EngineConfig::default(), Arc::new(soyuz_fs::RealProbe), Arc::new(|| {}))
}

pub fn no_wake() -> Wake {
    Arc::new(|| {})
}

/// Write `files` (relative path → contents) under `root`.
pub fn fixture(root: &Path, files: &[(&str, &str)]) {
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

/// Relative file list under `root` (sorted, `/` separators), folders end with `/`.
pub fn tree(root: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                out.push(format!("{rel}/"));
                walk(base, &p, out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// How the scripted user answers plans.
#[derive(Clone)]
pub enum Policy {
    ApproveAll,
    RejectAll,
    /// Use "approve all of this kind" for every plan (refused for tainted ones, then the
    /// user approves explicitly after the refusal notice).
    ApproveKindThenManual,
    /// Cancel the task when the first plan shows up.
    CancelOnPlan,
    /// Replace the arguments of the first plan.
    EditFirst(Value),
    /// Leave plans pending (tests that the harness waits).
    Never,
}

#[derive(Default, Debug)]
pub struct Outcome {
    pub finish: Option<Finish>,
    pub text: String,
    pub proposed: Vec<ProposedPlan>,
    pub auto_approved: Vec<u64>,
    pub executed: Vec<(u64, bool, String)>,
    pub notices: Vec<String>,
    pub tools: Vec<String>,
    pub steps: Vec<(String, Value)>,
    pub messages: Vec<soyuz_ai::Message>,
    /// Disk state when the first plan was proposed (before any approval).
    pub tree_at_first_plan: Option<Vec<String>>,
    pub usage: soyuz_ai::Usage,
    pub cost: Option<f64>,
    pub model_turns: usize,
}

pub struct Harness {
    pub root: PathBuf,
    pub engine: Arc<JobEngine>,
    pub exec: Arc<EngineExecutor>,
    pub limits: Limits,
    pub rules: Vec<AiFolderRule>,
    pub default_access: AiAccess,
    pub audit: Option<Audit>,
    pub redact_cloud: bool,
    pub budget: Option<soyuz_app::ai::agent::Budget>,
    /// Files attached to the first message ("Suggest names").
    pub attachments: soyuz_app::ai::agent::Attachments,
    /// External MCP tools offered to the model (A4).
    pub external: Option<Arc<dyn soyuz_mcp::client::ExternalTools>>,
}

impl Harness {
    pub fn new(root: &Path) -> Self {
        let engine = engine();
        Harness {
            root: root.to_path_buf(),
            exec: Arc::new(EngineExecutor::new(engine.clone())),
            engine,
            limits: Limits::default(),
            rules: Vec::new(),
            default_access: AiAccess::Metadata,
            audit: None,
            redact_cloud: true,
            budget: None,
            external: None,
            attachments: Default::default(),
        }
    }

    fn host(&self) -> Arc<AppToolHost> {
        let snap = AppSnapshot { location: Some(self.root.clone()), ..Default::default() };
        Arc::new(AppToolHost::new(
            FsToolHost::new(Scope::new(vec![self.root.clone()], &[], false)),
            Arc::new(RwLock::new(snap)),
            None,
        ))
    }

    fn deps(&self, model: Option<Arc<dyn ChatModel>>) -> Deps {
        Deps {
            model: model.map(|m| Box::new(move || Ok(m)) as agent::ModelFactory),
            host: self.host(),
            exec: self.exec.clone(),
            perms: Arc::new(Permissions::new(&self.rules, self.default_access)),
            audit: self.audit.clone(),
            wake: no_wake(),
            external: self.external.clone(),
        }
    }

    pub fn chat(&self, model: Arc<dyn ChatModel>, prompt: &str, policy: Policy) -> (TaskId, Outcome) {
        self.chat_with_context(model, prompt, String::new(), policy)
    }

    pub fn chat_with_context(
        &self,
        model: Arc<dyn ChatModel>,
        prompt: &str,
        context: String,
        policy: Policy,
    ) -> (TaskId, Outcome) {
        let spec = TaskSpec {
            id: TaskId::next(),
            mode: Mode::Chat {
                prompt: prompt.into(),
                context,
                history: Vec::new(),
                attachments: self.attachments.clone(),
            },
            limits: self.limits.clone(),
            budget: self.budget.clone(),
            redact_cloud: self.redact_cloud,
            today: "2026-10-08".into(),
        };
        let id = spec.id;
        (id, self.drive(spec, Some(model), policy))
    }

    pub fn recipe(&self, steps: Vec<(String, Value)>, policy: Policy) -> (TaskId, Outcome) {
        let spec = TaskSpec {
            id: TaskId::next(),
            mode: Mode::Recipe { name: "test".into(), steps },
            limits: self.limits.clone(),
            budget: None,
            redact_cloud: false,
            today: String::new(),
        };
        let id = spec.id;
        (id, self.drive(spec, None, policy))
    }

    fn drive(&self, spec: TaskSpec, model: Option<Arc<dyn ChatModel>>, policy: Policy) -> Outcome {
        let h = agent::spawn(spec, self.deps(model));
        let mut out = Outcome::default();
        let mut edited = false;
        let mut kind_refused: Vec<u64> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            assert!(Instant::now() < deadline, "harness test timed out: {out:?}");
            let Ok(ev) = h.events.recv_timeout(Duration::from_millis(200)) else {
                if matches!(policy, Policy::Never) && out.proposed.len() > out.auto_approved.len() {
                    // Still waiting after a while: the loop must not have moved on.
                    assert!(out.executed.is_empty());
                    h.cancel();
                }
                continue;
            };
            match ev {
                AgentEvent::Text(t) => out.text.push_str(&t),
                AgentEvent::ToolStarted { tool, .. } => out.tools.push(tool),
                AgentEvent::Proposed(plans) => {
                    if out.tree_at_first_plan.is_none() {
                        out.tree_at_first_plan = Some(tree(&self.root));
                    }
                    for p in &plans {
                        out.proposed.push(p.clone());
                    }
                    for p in plans {
                        match &policy {
                            Policy::ApproveAll => h.decide(p.id, Decision::Approve),
                            Policy::RejectAll => h.decide(p.id, Decision::Reject("not like this".into())),
                            Policy::ApproveKindThenManual => h.decide(p.id, Decision::ApproveKind),
                            Policy::CancelOnPlan => h.cancel(),
                            Policy::EditFirst(args) if !edited => {
                                edited = true;
                                h.decide(p.id, Decision::Edit(args.clone()));
                            }
                            Policy::EditFirst(_) => h.decide(p.id, Decision::Approve),
                            Policy::Never => {}
                        }
                    }
                }
                AgentEvent::AutoApproved(id) => out.auto_approved.push(id),
                AgentEvent::Usage { usage, cost } => {
                    out.usage = usage;
                    out.cost = cost;
                }
                AgentEvent::TurnEnd => out.model_turns += 1,
                AgentEvent::Executed { plan, ok, message } => out.executed.push((plan, ok, message)),
                AgentEvent::Notice { text, .. } => {
                    if matches!(policy, Policy::ApproveKindThenManual) && text.contains("explicit approval") {
                        // The refused plan is the newest pending one: approve it by hand.
                        if let Some(p) = out.proposed.iter().rev().find(|p| !kind_refused.contains(&p.id)) {
                            kind_refused.push(p.id);
                            h.decide(p.id, Decision::Approve);
                        }
                    }
                    out.notices.push(text);
                }
                AgentEvent::Finished { reason, messages, executed } => {
                    out.finish = Some(reason);
                    out.steps = executed;
                    out.messages = messages;
                    break;
                }
                _ => {}
            }
        }
        h.join();
        out
    }
}

/// The text of every tool result the model received in the last request.
pub fn tool_results(msgs: &[soyuz_ai::Message]) -> Vec<String> {
    msgs.iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            soyuz_ai::Part::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}
