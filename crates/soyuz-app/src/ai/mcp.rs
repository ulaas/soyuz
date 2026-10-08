//! External MCP clients inside the assistant's UI model (roadmap A3 + A6, unified):
//!
//! * [`PendingMcpApproval`] — a write / destructive call of an MCP client waiting for the
//!   user (created by the bridge's `Backend::request_approval` on a server thread).
//! * [`McpApprovals`] — turns each request into a [`PlanCard`] (the assistant's card
//!   widget: summary, tree / rename diff, Approve / Edit / Reject / "Approve all of this
//!   kind for this client"), answers the server thread, follows the resulting job
//!   (live pane highlights, card state) and expires cards whose call went away.
//! * Job labels `MCP <client> · <summary>` tag MCP jobs in the activity panel and let
//!   "Undo everything this client did in this session" find them in the history.
//!
//! Everything here is plain state (no egui, no app): the bridge and the tests drive it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use serde_json::Value;
use soyuz_ai::tools::Plan;
use soyuz_core::OpControl;
use soyuz_jobs::{HistoryId, JobEngine, JobId, JobState};
use soyuz_mcp::{ApprovalDecision, Transport};

use super::agent::Decision;
use super::state::{CardOrigin, CardState, PlanCard};
use super::touch::{self, TouchKind};
use super::{CancelFlag, TaskId};

/// Label prefix of jobs started by MCP clients (`MCP claude-desktop · Move 3 items …`).
pub const JOB_PREFIX: &str = "MCP ";

/// Job label for a plan an MCP client ran.
pub fn job_label(client: &str, summary: &str) -> String {
    format!("{JOB_PREFIX}{client} \u{00b7} {}", summary.trim_start_matches("AI: ").trim())
}

/// The client of an MCP job label.
pub fn label_client(label: &str) -> Option<&str> {
    let rest = label.strip_prefix(JOB_PREFIX)?;
    let (client, _) = rest.split_once(" \u{00b7} ")?;
    (!client.is_empty() && !client.contains(' ')).then_some(client)
}

/// A plan an MCP client wants to run, waiting for the user.
pub struct PendingMcpApproval {
    pub client: String,
    pub transport: Transport,
    pub tool: String,
    pub args: Value,
    pub plan: Plan,
    /// Cancelled when the client gives up (cancelled or timed out).
    pub ctl: Arc<OpControl>,
    pub received: Instant,
    /// When the server stops waiting.
    pub deadline: Instant,
    /// Identifies the server call (matches the later `execute` of the approved plan).
    pub call_key: usize,
    reply: Sender<ApprovalDecision>,
}

impl PendingMcpApproval {
    pub fn new(
        req: soyuz_mcp::ApprovalRequest,
        ctl: Arc<OpControl>,
        timeout: Duration,
        call_key: usize,
        reply: Sender<ApprovalDecision>,
    ) -> Self {
        let now = Instant::now();
        PendingMcpApproval {
            client: req.client,
            transport: req.transport,
            tool: req.tool,
            args: req.args,
            plan: req.plan,
            ctl,
            received: now,
            deadline: now + timeout,
            call_key,
            reply,
        }
    }

    /// Answer the request (only the first answer counts).
    pub fn decide(&self, d: ApprovalDecision) {
        let _ = self.reply.try_send(d);
    }
}

/// One MCP card and its live link to the waiting server call.
pub struct McpCard {
    pub card: PlanCard,
    pub call_key: usize,
    pending: Option<PendingMcpApproval>,
    /// The job the approved plan became (live highlights, card state).
    pub job: Option<JobId>,
    pub resolved: Option<Instant>,
}

impl McpCard {
    pub fn client(&self) -> &str {
        self.card.origin.mcp_client().unwrap_or_default()
    }

    pub fn is_pending(&self) -> bool {
        self.card.state == CardState::Pending
    }
}

/// A job an MCP client started without a card (`require_approval = false`).
struct TrackedJob {
    job: JobId,
    paths: Vec<PathBuf>,
    changed: Vec<PathBuf>,
}

/// MCP approval cards of this window.
#[derive(Default)]
pub struct McpApprovals {
    pub cards: Vec<McpCard>,
    next_id: u64,
    /// `(client, tool)` pairs approved with "Approve all of this kind for this client".
    pub approve_kinds: HashSet<(String, String)>,
    jobs: Vec<TrackedJob>,
    /// A request arrived since the UI last looked (opens the panel / the modal card).
    pub arrived: bool,
}

/// Most finished cards kept below the pending ones.
const KEEP_RESOLVED: usize = 6;

fn changed_paths(plan: &Plan) -> Vec<PathBuf> {
    plan.items.iter().filter_map(|i| i.dst.clone().or_else(|| i.src.clone())).collect()
}

impl McpApprovals {
    /// Show a request as a card. Returns its id. Requests of a kind the user approved for
    /// this client are approved at once (the card says so).
    pub fn push(&mut self, p: PendingMcpApproval) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let card = PlanCard {
            id,
            task: TaskId(0),
            tool: p.tool.clone(),
            args: p.args.clone(),
            expanded: p.plan.items.len() <= 12,
            plan: p.plan.clone(),
            tainted: false,
            state: CardState::Pending,
            edit: None,
            tree: true,
            origin: CardOrigin::Mcp { client: p.client.clone(), via: p.transport.label().to_string() },
            expires: Some(p.deadline),
        };
        touch::mark_mcp(&card.paths(), TouchKind::Planned);
        let auto = self.approve_kinds.contains(&(p.client.clone(), p.tool.clone()))
            && !crate::ui::ai::cards::irreversible(&p.tool)
            && p.plan.executable;
        let call_key = p.call_key;
        self.cards.push(McpCard { card, call_key, pending: Some(p), job: None, resolved: None });
        if auto {
            self.answer(id, ApprovalDecision::Approved, CardState::Approved { auto: true });
        } else {
            self.arrived = true;
        }
        id
    }

    pub fn card(&self, id: u64) -> Option<&McpCard> {
        self.cards.iter().find(|c| c.card.id == id)
    }

    pub fn card_mut(&mut self, id: u64) -> Option<&mut McpCard> {
        self.cards.iter_mut().find(|c| c.card.id == id)
    }

    pub fn pending(&self) -> usize {
        self.cards.iter().filter(|c| c.is_pending()).count()
    }

    fn answer(&mut self, id: u64, d: ApprovalDecision, state: CardState) {
        let Some(c) = self.card_mut(id) else { return };
        if let Some(p) = c.pending.take() {
            p.decide(d);
        }
        let done = !matches!(state, CardState::Approved { .. });
        if done {
            touch::unmark(&c.card.paths());
            c.resolved = Some(Instant::now());
            c.card.expanded = false;
        }
        c.card.state = state;
        c.card.edit = None;
    }

    /// The user answered a card (Approve / Edit / Reject / "Approve all of this kind").
    pub fn decide(&mut self, id: u64, d: Decision) {
        if !self.card(id).is_some_and(McpCard::is_pending) {
            return;
        }
        match d {
            Decision::Approve | Decision::ApproveKind => {
                self.answer(id, ApprovalDecision::Approved, CardState::Approved { auto: false })
            }
            Decision::Reject(why) => {
                let why = if why.trim().is_empty() { "the user rejected the plan".to_string() } else { why };
                self.answer(id, ApprovalDecision::Rejected(why), CardState::Rejected)
            }
            // The server plans the edited arguments again and sends a new card.
            Decision::Edit(args) => self.answer(id, ApprovalDecision::Edited(args), CardState::Edited),
        }
    }

    /// "Approve all of this kind for this client": this card and the client's other
    /// pending cards of the same tool, plus every later one until restart.
    pub fn approve_kind(&mut self, id: u64) {
        let Some(c) = self.card(id) else { return };
        let (client, tool) = (c.client().to_string(), c.card.tool.clone());
        if crate::ui::ai::cards::irreversible(&tool) {
            self.decide(id, Decision::Approve);
            return;
        }
        self.approve_kinds.insert((client.clone(), tool.clone()));
        let ids: Vec<u64> = self
            .cards
            .iter()
            .filter(|c| c.is_pending() && c.client() == client && c.card.tool == tool && c.card.plan.executable)
            .map(|c| c.card.id)
            .collect();
        for i in ids {
            let state = CardState::Approved { auto: i != id };
            self.answer(i, ApprovalDecision::Approved, state);
        }
    }

    /// Reject every pending card (window closing).
    pub fn reject_all(&mut self, why: &str) {
        let ids: Vec<u64> = self.cards.iter().filter(|c| c.is_pending()).map(|c| c.card.id).collect();
        for id in ids {
            self.answer(id, ApprovalDecision::Rejected(why.to_string()), CardState::Rejected);
        }
    }

    /// The approved plan of call `key` was submitted (`job` = its job, if any). Plans
    /// from clients that need no approval have no card: they are tracked for highlights.
    pub fn on_submitted(&mut self, key: usize, plan: &Plan, job: Option<JobId>) {
        let card = self
            .cards
            .iter_mut()
            .rev()
            .find(|c| c.call_key == key && matches!(c.card.state, CardState::Approved { .. }));
        match card {
            Some(c) => {
                touch::mark_mcp(&c.card.paths(), TouchKind::Changing);
                match job {
                    Some(j) => {
                        c.job = Some(j);
                        c.card.state = CardState::Running(Some(j));
                    }
                    None => {
                        touch::unmark(&c.card.paths());
                        touch::mark_mcp(&changed_paths(&c.card.plan), TouchKind::Done);
                        c.card.state = CardState::Done { ok: true, message: tr!("ai-exec-done").into() };
                        c.resolved = Some(Instant::now());
                        c.card.expanded = false;
                    }
                }
            }
            None => {
                let paths: Vec<PathBuf> =
                    plan.items.iter().flat_map(|i| i.src.iter().chain(i.dst.iter()).cloned()).collect();
                touch::mark_mcp(&paths, TouchKind::Changing);
                if let Some(job) = job {
                    self.jobs.push(TrackedJob { job, paths, changed: changed_paths(plan) });
                } else {
                    touch::unmark(&paths);
                }
            }
        }
    }

    /// Submitting the approved plan of call `key` failed.
    pub fn on_failed(&mut self, key: usize, message: &str) {
        if let Some(c) = self
            .cards
            .iter_mut()
            .rev()
            .find(|c| c.call_key == key && matches!(c.card.state, CardState::Approved { .. }))
        {
            touch::unmark(&c.card.paths());
            c.card.state = CardState::Done { ok: false, message: message.to_string() };
            c.resolved = Some(Instant::now());
            c.card.expanded = false;
        }
    }

    /// Per frame: expire cards whose call went away, follow jobs, drop old cards.
    /// `job_state` looks a job up (the engine or the app's job list).
    pub fn tick(&mut self, job_state: impl Fn(JobId) -> Option<JobState>) {
        let now = Instant::now();
        for c in &mut self.cards {
            if c.is_pending() && c.pending.as_ref().is_some_and(|p| p.ctl.is_cancelled()) {
                let timed_out = c.pending.as_ref().is_some_and(|p| now + Duration::from_millis(500) >= p.deadline);
                c.pending = None;
                touch::unmark(&c.card.paths());
                c.card.state = CardState::Withdrawn(
                    if timed_out { tr!("ai-mcp-timed-out") } else { tr!("ai-mcp-cancelled-by-client") }.to_string(),
                );
                c.resolved = Some(now);
                c.card.expanded = false;
            }
            if let (CardState::Running(_), Some(job)) = (&c.card.state, c.job) {
                let finished = match job_state(job) {
                    None => Some((false, tr!("ai-mcp-job-gone").to_string())),
                    Some(JobState::Done(s)) if s.items_failed == 0 => {
                        Some((true, tr!("ai-mcp-job-done", ok = s.items_ok, skipped = s.items_skipped)))
                    }
                    Some(JobState::Done(s)) => {
                        Some((false, tr!("ai-mcp-job-errors", ok = s.items_ok, failed = s.items_failed)))
                    }
                    Some(JobState::Failed(e)) => Some((false, tr!("ai-mcp-job-failed", error = e.to_string()))),
                    Some(JobState::Cancelled) => Some((false, tr!("ai-mcp-job-cancelled").to_string())),
                    Some(_) => None,
                };
                if let Some((ok, message)) = finished {
                    touch::unmark(&c.card.paths());
                    if ok {
                        touch::mark_mcp(&changed_paths(&c.card.plan), TouchKind::Done);
                    }
                    c.card.state = CardState::Done { ok, message };
                    c.resolved = Some(now);
                    c.card.expanded = false;
                }
            }
        }
        self.jobs.retain(|t| match job_state(t.job) {
            Some(JobState::Done(_)) => {
                touch::unmark(&t.paths);
                touch::mark_mcp(&t.changed, TouchKind::Done);
                false
            }
            Some(JobState::Failed(_) | JobState::Cancelled) | None => {
                touch::unmark(&t.paths);
                false
            }
            Some(_) => true,
        });
        // Keep the pending / running cards and the newest few finished ones.
        let resolved = self.cards.iter().filter(|c| c.resolved.is_some()).count();
        if resolved > KEEP_RESOLVED {
            let mut drop = resolved - KEEP_RESOLVED;
            self.cards.retain(|c| {
                if drop > 0 && c.resolved.is_some() {
                    drop -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }

    /// Forget finished cards (the "Clear" button).
    pub fn clear_resolved(&mut self) {
        self.cards.retain(|c| c.resolved.is_none());
    }

    /// Whether anything needs repainting soon (countdowns, running jobs).
    pub fn busy(&self) -> bool {
        !self.jobs.is_empty() || self.cards.iter().any(|c| c.resolved.is_none())
    }
}

/// Undoable history entries an MCP client created in this session, newest first.
pub fn client_history(history: &[soyuz_jobs::HistoryEntry], client: &str) -> Vec<HistoryId> {
    let mut v: Vec<HistoryId> = history
        .iter()
        .filter(|h| h.this_session && !h.undone && h.undoable)
        .filter(|h| h.options.as_ref().and_then(|o| o.label.as_deref()).and_then(label_client) == Some(client))
        .map(|h| h.id)
        .collect();
    v.sort_by_key(|h| std::cmp::Reverse(h.0));
    v
}

/// Clients with something to undo in this session (for the approvals area).
pub fn clients_with_history(history: &[soyuz_jobs::HistoryEntry]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for h in history.iter().filter(|h| h.this_session && !h.undone && h.undoable) {
        if let Some(c) = h.options.as_ref().and_then(|o| o.label.as_deref()).and_then(label_client) {
            match out.iter_mut().find(|(n, _)| n == c) {
                Some((_, k)) => *k += 1,
                None => out.push((c.to_string(), 1)),
            }
        }
    }
    out
}

/// "Undo everything this client did in this session": newest first, one at a time.
/// Returns `(undone, failed)`.
pub fn undo_client(
    engine: &JobEngine,
    client: &str,
    cancel: &CancelFlag,
    on_job: impl FnMut(JobId),
) -> (usize, Vec<String>) {
    super::exec::undo_entries(engine, client_history(&engine.history(), client), cancel, on_job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_round_trip() {
        let l = job_label("claude-desktop", "AI: Move 3 items into C:\\x");
        assert_eq!(l, "MCP claude-desktop \u{00b7} Move 3 items into C:\\x");
        assert_eq!(label_client(&l), Some("claude-desktop"));
        assert_eq!(label_client("Move things"), None);
        assert_eq!(label_client("AI #3 \u{00b7} x"), None);
    }
}
