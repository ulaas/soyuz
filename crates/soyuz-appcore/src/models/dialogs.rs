//! Prompts the core raises (Phase T0): job prompts (conflict, error, not enough space,
//! trash unsupported, read-only item, archive password), delete confirmations, the
//! "new link" and "Copy to / Move to" forms, the quit-with-running-jobs question and the
//! MCP / script approval cards.
//!
//! Each [`DialogModel`] variant holds the prompt's state and turns the user's choice
//! into [`Cmd`]s (or a direct answer) through plain methods; no drawing here. Core code
//! pushes models onto [`DialogStack`] (`AppCore::dialogs`); a frontend draws the
//! top-most one (the GUI in `ui::dialogs`) and calls [`DialogStack::close_top`] when it
//! goes away. Closing without an answer is [`DialogModel::dismiss`]: it defers a job
//! prompt, rejects an MCP request and cancels everything else. Models whose answer is
//! shared with another thread also answer on drop (MCP approval, quit flag, job prompt
//! liveness), as the old dialogs did.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use soyuz_ai::tools::Risk;
use soyuz_core::{ConflictResolution, ErrorAction};
use soyuz_fs::TrashItem;
use soyuz_jobs::kinds::extract::ARCHIVE_LIMIT_OP;
use soyuz_jobs::{ConflictInfo, ConflictKind, DeleteMode, JobId, JobKindBrief, JobPrompt, PromptResponse};
use soyuz_script::PlannedJob;

use crate::ai::agent::Decision;
use crate::ai::mcp::McpApprovals;
use crate::automation::AutoCmd;
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::ops::fileops::drop::DropEffect;

/// Name as shown in prompts: hidden control characters made visible.
fn shown(name: &str) -> String {
    soyuz_core::naming::reveal_controls(name).into_owned()
}

/// Ordering stamp shared by the model stack and a frontend's own dialog stack, so the
/// most recently opened dialog of either kind is the top-most one.
pub fn next_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// A prompt raised by core logic.
pub enum DialogModel {
    /// A job waits for an answer (conflict, error, space, trash, read-only, password).
    Prompt(Box<PromptModel>),
    DeleteConfirm(DeleteConfirmModel),
    NewLink(LinkModel),
    Transfer(TransferModel),
    Quit(QuitModel),
    McpApproval(McpApprovalModel),
    ScriptApproval(ScriptApprovalModel),
}

impl DialogModel {
    pub fn title(&self) -> &str {
        match self {
            DialogModel::Prompt(m) => &m.title,
            DialogModel::DeleteConfirm(m) => &m.title,
            DialogModel::NewLink(_) => tr!("dialogs-link-title"),
            DialogModel::Transfer(m) => m.title(),
            DialogModel::Quit(m) => &m.title,
            DialogModel::McpApproval(m) => &m.title,
            DialogModel::ScriptApproval(m) => &m.title,
        }
    }

    /// Confirmations of destructive or blocking choices (exposed as alert dialogs).
    pub fn alert(&self) -> bool {
        matches!(self, DialogModel::Prompt(_) | DialogModel::DeleteConfirm(_) | DialogModel::Quit(_))
    }

    /// Closed without an answer (Esc, click outside, window close): defer a job prompt,
    /// reject an MCP request, cancel everything else.
    pub fn dismiss(&mut self) {
        match self {
            DialogModel::Prompt(m) => m.dismiss(),
            DialogModel::McpApproval(m) => m.reject(),
            DialogModel::ScriptApproval(m) => m.cancel(),
            DialogModel::DeleteConfirm(_)
            | DialogModel::NewLink(_)
            | DialogModel::Transfer(_)
            | DialogModel::Quit(_) => {}
        }
    }
}

macro_rules! into_model {
    ($($t:ident => $v:ident),* $(,)?) => {
        $(impl From<$t> for DialogModel {
            fn from(m: $t) -> Self {
                DialogModel::$v(m)
            }
        })*
    };
}

impl From<PromptModel> for DialogModel {
    fn from(m: PromptModel) -> Self {
        DialogModel::Prompt(Box::new(m))
    }
}

into_model!(
    DeleteConfirmModel => DeleteConfirm,
    LinkModel => NewLink,
    TransferModel => Transfer,
    QuitModel => Quit,
    McpApprovalModel => McpApproval,
    ScriptApprovalModel => ScriptApproval,
);

/// Open model dialogs, top-most last.
#[derive(Default)]
pub struct DialogStack {
    stack: Vec<(u64, DialogModel)>,
}

impl DialogStack {
    pub fn push(&mut self, m: impl Into<DialogModel>) {
        self.stack.push((next_seq(), m.into()));
    }

    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn top(&self) -> Option<&DialogModel> {
        self.stack.last().map(|(_, m)| m)
    }

    pub fn top_mut(&mut self) -> Option<&mut DialogModel> {
        self.stack.last_mut().map(|(_, m)| m)
    }

    /// Ordering stamp of the top-most model (see [`next_seq`]).
    pub fn top_seq(&self) -> Option<u64> {
        self.stack.last().map(|(s, _)| *s)
    }

    pub fn iter(&self) -> impl Iterator<Item = &DialogModel> {
        self.stack.iter().map(|(_, m)| m)
    }

    /// Close the top-most dialog: [`DialogModel::dismiss`] (a no-op once answered),
    /// then drop it.
    pub fn close_top(&mut self) {
        if let Some((_, mut m)) = self.stack.pop() {
            m.dismiss();
        }
    }
}

// ---------------------------------------------------------------------------------
// Job prompts
// ---------------------------------------------------------------------------------

/// State shared between the prompt model and `ops::fileops` (which owns the prompt queue).
#[derive(Clone, Debug, Default)]
pub struct PromptShared {
    /// Still on the dialog stack.
    pub alive: bool,
    /// An answer (or cancel) was sent.
    pub answered: bool,
    /// The job no longer waits (cancelled / answered elsewhere): close immediately.
    pub stale: bool,
    /// Prompts waiting to be shown, this one included.
    pub total: usize,
}

/// What the user decided for a job prompt.
#[derive(Clone, Debug)]
pub enum PromptAnswer {
    Respond(PromptResponse),
    CancelJob,
}

/// One job prompt. Esc / closing defers it (the job's activity card offers "Resolve…").
pub struct PromptModel {
    pub job: JobId,
    pub prompt: JobPrompt,
    /// Title of the job that asks.
    pub job_title: String,
    pub title: String,
    shared: Rc<RefCell<PromptShared>>,
    /// "Apply to all" (conflicts, trash unsupported, read-only items).
    pub apply_all: bool,
    /// Rename-on-conflict: the new name being typed (`None` = not renaming).
    pub rename: Option<String>,
    /// Ask the frontend to focus the rename field once.
    pub focus_rename: bool,
    /// Archive password being typed.
    pub password: String,
    /// Ask the frontend to focus the password field once.
    pub focus_password: bool,
    /// Validate rename-on-conflict names with the Windows rules.
    pub windows_rules: bool,
}

impl PromptModel {
    pub fn new(job: JobId, prompt: JobPrompt, job_title: String, shared: Rc<RefCell<PromptShared>>) -> Self {
        let title = match &prompt {
            JobPrompt::Conflict(c) => match c.kind {
                ConflictKind::FileExists => tr!("dialogs-conflict-title-file"),
                ConflictKind::DirExists => tr!("dialogs-conflict-title-dir"),
                ConflictKind::FileVsDir => tr!("dialogs-conflict-title-kind"),
                ConflictKind::SameFile => tr!("dialogs-conflict-title-same"),
                ConflictKind::MoveIntoSelf => tr!("dialogs-conflict-title-into-self"),
                ConflictKind::CaseOnly => tr!("dialogs-conflict-title-case"),
            },
            JobPrompt::Error { error, .. } if error.op == ARCHIVE_LIMIT_OP => tr!("dialogs-prompt-title-archive-limit"),
            JobPrompt::Error { .. } => tr!("dialogs-prompt-title-error"),
            JobPrompt::NotEnoughSpace { .. } => tr!("dialogs-prompt-title-space"),
            JobPrompt::TrashUnsupported { .. } => tr!("dialogs-prompt-title-trash"),
            JobPrompt::ReadOnlyItem { .. } => tr!("dialogs-prompt-title-read-only"),
            JobPrompt::Password { .. } => tr!("dialogs-prompt-title-password"),
        }
        .to_string();
        PromptModel {
            job,
            prompt,
            job_title,
            title,
            shared,
            apply_all: false,
            rename: None,
            focus_rename: false,
            password: String::new(),
            focus_password: true,
            windows_rules: cfg!(windows),
        }
    }

    /// Validate rename-on-conflict names with the Windows rules (target on a
    /// Windows-family volume, or strict names).
    pub fn windows_rules(mut self, on: bool) -> Self {
        self.windows_rules = on;
        self
    }

    /// Prompts waiting to be shown, this one included.
    pub fn total(&self) -> usize {
        self.shared.borrow().total
    }

    /// Whether the prompt should stay open: `false` once the job no longer waits
    /// (cancelled / answered elsewhere), which counts as answered.
    pub fn poll(&mut self) -> bool {
        let stale = self.shared.borrow().stale;
        if stale {
            self.shared.borrow_mut().answered = true;
        }
        !stale
    }

    /// Closed without an answer: the prompt is deferred (see `ops::fileops`).
    pub fn dismiss(&mut self) {}

    /// The command that carries `answer` to the job; marks the prompt answered.
    pub fn answer(&mut self, answer: PromptAnswer) -> Cmd {
        self.shared.borrow_mut().answered = true;
        Cmd::FileOp(match answer {
            PromptAnswer::Respond(response) => FileOpCmd::Respond { job: self.job, response },
            PromptAnswer::CancelJob => FileOpCmd::CancelJob(self.job),
        })
    }

    fn conflict_info(&self) -> Option<&ConflictInfo> {
        match &self.prompt {
            JobPrompt::Conflict(c) => Some(c),
            _ => None,
        }
    }

    /// A conflict choice, with "apply to all" when it is ticked and makes sense (never
    /// for a rename or a move into itself).
    pub fn resolve(&self, resolution: ConflictResolution) -> PromptAnswer {
        let blocked = self.conflict_info().is_some_and(|c| c.kind == ConflictKind::MoveIntoSelf);
        let all = self.apply_all && !blocked && !matches!(resolution, ConflictResolution::Rename(_));
        PromptAnswer::Respond(PromptResponse::Conflict { resolution, apply_to_all: all })
    }

    /// Start typing a new name for the conflicting item (the suggested name first).
    pub fn start_rename(&mut self) {
        if let Some(c) = self.conflict_info() {
            self.rename = Some(c.suggested_name.clone());
            self.focus_rename = true;
        }
    }

    pub fn cancel_rename(&mut self) {
        self.rename = None;
    }

    /// The typed name is a valid file name.
    pub fn rename_valid(&self) -> bool {
        self.rename.as_ref().is_some_and(|t| soyuz_core::validate_name_with(t.trim(), self.windows_rules).is_ok())
    }

    /// The typed name is valid and differs from the existing item's name.
    pub fn rename_differs(&self) -> bool {
        self.rename_valid()
            && match (self.rename.as_ref(), self.conflict_info()) {
                (Some(t), Some(c)) => t.trim() != c.dst.name,
                _ => false,
            }
    }

    /// "Rename" with the typed name (`None` while it is invalid).
    pub fn commit_rename(&self) -> Option<PromptAnswer> {
        let text = self.rename.as_ref()?;
        self.rename_valid().then(|| self.resolve(ConflictResolution::Rename(text.trim().to_string())))
    }

    /// Error prompt: retry / skip / skip all / abort (never applied to all).
    pub fn error(&self, action: ErrorAction) -> PromptAnswer {
        PromptAnswer::Respond(PromptResponse::Error { action, apply_to_all: false })
    }

    /// Not enough space: continue anyway or cancel.
    pub fn space(&self, proceed: bool) -> PromptAnswer {
        PromptAnswer::Respond(PromptResponse::Space { proceed })
    }

    /// Trash unsupported: delete permanently or skip (with "apply to all").
    pub fn trash_unsupported(&self, permanent: bool) -> PromptAnswer {
        PromptAnswer::Respond(PromptResponse::TrashUnsupported { permanent, apply_to_all: self.apply_all })
    }

    /// Read-only item: change anyway or skip (with "apply to all").
    pub fn read_only(&self, force: bool) -> PromptAnswer {
        PromptAnswer::Respond(PromptResponse::ReadOnly { force, apply_to_all: self.apply_all })
    }

    /// Unlock with the typed password (`None` while it is empty).
    pub fn submit_password(&self) -> Option<PromptAnswer> {
        (!self.password.is_empty())
            .then(|| PromptAnswer::Respond(PromptResponse::Password(Some(self.password.clone()))))
    }

    pub fn cancel_password(&self) -> PromptAnswer {
        PromptAnswer::Respond(PromptResponse::Password(None))
    }

    /// The answer of Enter (outside text fields): replace for a plain file conflict,
    /// abort for the archive size guard, retry (or skip) for an error.
    pub fn default_answer(&self) -> Option<PromptAnswer> {
        match &self.prompt {
            JobPrompt::Conflict(c) if c.kind == ConflictKind::FileExists && self.rename.is_none() => {
                Some(self.resolve(ConflictResolution::Overwrite))
            }
            JobPrompt::Error { error, .. } if error.op == ARCHIVE_LIMIT_OP => Some(self.error(ErrorAction::Abort)),
            JobPrompt::Error { retryable, .. } => {
                Some(self.error(if *retryable { ErrorAction::Retry } else { ErrorAction::Skip }))
            }
            _ => None,
        }
    }
}

impl Drop for PromptModel {
    fn drop(&mut self) {
        self.shared.borrow_mut().alive = false;
    }
}

// ---------------------------------------------------------------------------------
// Delete confirmation
// ---------------------------------------------------------------------------------

pub enum ConfirmKind {
    Delete { paths: Vec<PathBuf>, mode: DeleteMode },
    Purge(Vec<TrashItem>),
    EmptyTrash(usize),
}

/// Move to trash (when `general.confirm_delete`), permanent delete (Shift+Delete),
/// secure wipe, permanent delete of trashed items and "Empty trash".
pub struct DeleteConfirmModel {
    pub kind: ConfirmKind,
    pub title: String,
    done: bool,
}

impl DeleteConfirmModel {
    pub fn new(paths: Vec<PathBuf>, mode: DeleteMode) -> Self {
        let title = match paths.as_slice() {
            [one] => {
                let name = shown(&soyuz_core::entry::display_name(one));
                match mode {
                    DeleteMode::Trash => tr!("dialogs-delete-trash-one", name = name),
                    DeleteMode::Permanent => tr!("dialogs-delete-permanent-one", name = name),
                    DeleteMode::Wipe { .. } => tr!("dialogs-delete-wipe-one", name = name),
                }
            }
            many => {
                let count = many.len();
                match mode {
                    DeleteMode::Trash => tr!("dialogs-delete-trash-many", count = count),
                    DeleteMode::Permanent => tr!("dialogs-delete-permanent-many", count = count),
                    DeleteMode::Wipe { .. } => tr!("dialogs-delete-wipe-many", count = count),
                }
            }
        };
        DeleteConfirmModel { kind: ConfirmKind::Delete { paths, mode }, title, done: false }
    }

    pub fn purge(items: Vec<TrashItem>) -> Self {
        let title = tr!("dialogs-delete-permanent-many", count = items.len());
        DeleteConfirmModel { kind: ConfirmKind::Purge(items), title, done: false }
    }

    pub fn empty_trash(n: usize) -> Self {
        DeleteConfirmModel {
            kind: ConfirmKind::EmptyTrash(n),
            title: tr!("dialogs-delete-empty-title").into(),
            done: false,
        }
    }

    /// Display names of the items (empty for "Empty trash").
    pub fn names(&self) -> Vec<String> {
        match &self.kind {
            ConfirmKind::Delete { paths, .. } => paths.iter().map(|p| soyuz_core::entry::display_name(p)).collect(),
            ConfirmKind::Purge(items) => items.iter().map(|i| soyuz_core::entry::display_name(&i.original)).collect(),
            ConfirmKind::EmptyTrash(_) => Vec::new(),
        }
    }

    /// The trash (not a permanent delete).
    pub fn to_trash(&self) -> bool {
        matches!(self.kind, ConfirmKind::Delete { mode: DeleteMode::Trash, .. })
    }

    /// `(message, confirm button, destructive)`.
    pub fn texts(&self) -> (String, &'static str, bool) {
        match &self.kind {
            ConfirmKind::Delete { mode: DeleteMode::Trash, .. } => {
                (tr!("dialogs-delete-trash-body").to_string(), tr!("dialogs-delete-trash-confirm"), false)
            }
            ConfirmKind::Delete { mode: DeleteMode::Permanent, .. } => {
                (tr!("dialogs-delete-permanent-body").to_string(), tr!("common-delete"), true)
            }
            ConfirmKind::Delete { mode: DeleteMode::Wipe { .. }, .. } => {
                (tr!("dialogs-delete-wipe-body").to_string(), tr!("dialogs-delete-wipe-confirm"), true)
            }
            ConfirmKind::Purge(_) => (tr!("dialogs-delete-purge-body").to_string(), tr!("common-delete"), true),
            ConfirmKind::EmptyTrash(n) => {
                (tr!("dialogs-delete-empty-body", count = *n), tr!("dialogs-delete-empty-confirm"), true)
            }
        }
    }

    /// Confirmed: the delete command (only once).
    pub fn confirm(&mut self) -> Option<Cmd> {
        if std::mem::replace(&mut self.done, true) {
            return None;
        }
        let cmd = match std::mem::replace(&mut self.kind, ConfirmKind::EmptyTrash(0)) {
            ConfirmKind::Delete { paths, mode } => FileOpCmd::Delete { paths, mode },
            ConfirmKind::Purge(items) => FileOpCmd::TrashPurge(items),
            ConfirmKind::EmptyTrash(_) => FileOpCmd::EmptyTrash,
        };
        Some(Cmd::FileOp(cmd))
    }

    /// Cancelled: nothing is deleted.
    pub fn cancel(&mut self) {
        self.done = true;
    }
}

// ---------------------------------------------------------------------------------
// New link, Copy to / Move to
// ---------------------------------------------------------------------------------

/// The kind of link a "new link" form creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkChoice {
    Symlink,
    Hardlink,
    Junction,
}

impl LinkChoice {
    pub fn label(self) -> &'static str {
        match self {
            LinkChoice::Symlink => tr!("dialogs-link-symlink"),
            LinkChoice::Hardlink => tr!("dialogs-link-hardlink"),
            LinkChoice::Junction => tr!("dialogs-link-junction"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            LinkChoice::Symlink if cfg!(windows) => tr!("dialogs-link-symlink-hint"),
            LinkChoice::Symlink => tr!("dialogs-link-symlink-hint-unix"),
            LinkChoice::Hardlink => tr!("dialogs-link-hardlink-hint"),
            LinkChoice::Junction => tr!("dialogs-link-junction-hint"),
        }
    }

    /// The choices offered on this platform (junctions are NTFS-only).
    pub fn offered() -> impl Iterator<Item = LinkChoice> {
        [LinkChoice::Symlink, LinkChoice::Hardlink, LinkChoice::Junction]
            .into_iter()
            .filter(|k| *k != LinkChoice::Junction || cfg!(windows))
    }
}

/// Create a symbolic link / hard link / junction named `name` in `dir`.
pub struct LinkModel {
    pub dir: PathBuf,
    pub target: String,
    pub name: String,
    pub kind: LinkChoice,
    existing: Vec<String>,
    /// Ask the frontend to focus the target field once.
    pub focus: bool,
    name_touched: bool,
    /// Validate the name with the Windows rules (see `rename::windows_rules_for`).
    windows_rules: bool,
}

impl LinkModel {
    pub fn new(dir: PathBuf, target: Option<PathBuf>, kind: LinkChoice, existing: Vec<String>) -> Self {
        let target_s = target.as_ref().map(|t| soyuz_fs::strip_verbatim(t).display().to_string()).unwrap_or_default();
        let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
        let name = target.as_ref().map(|t| crate::ops::fileops::names::link_name(t, &refs)).unwrap_or_default();
        LinkModel {
            dir,
            target: target_s,
            name,
            kind,
            existing,
            focus: true,
            name_touched: false,
            windows_rules: cfg!(windows),
        }
    }

    /// Validate names with the Windows rules (Windows-family volume or strict names).
    pub fn windows_rules(mut self, on: bool) -> Self {
        self.windows_rules = on;
        self
    }

    /// The target text changed: the name follows it until the user edits the name.
    pub fn target_edited(&mut self) {
        if !self.name_touched {
            let refs: Vec<&str> = self.existing.iter().map(String::as_str).collect();
            self.name = crate::ops::fileops::names::link_name(Path::new(self.target.trim()), &refs);
        }
    }

    /// The user edited the name.
    pub fn name_edited(&mut self) {
        self.name_touched = true;
    }

    /// Why the link cannot be created yet.
    pub fn error(&self) -> Option<String> {
        let name = self.name.trim();
        if self.target.trim().is_empty() {
            Some(tr!("dialogs-link-need-target").to_string())
        } else if let Err(e) = soyuz_core::validate_name_with(name, self.windows_rules) {
            Some(tr!("dialogs-props-invalid-name", error = e.to_string()))
        } else if self.existing.iter().any(|e| e.eq_ignore_ascii_case(name)) {
            Some(tr!("dialogs-link-exists", name = name))
        } else {
            None
        }
    }

    /// "Create": the command, or `None` while the form is invalid.
    pub fn create(&self) -> Option<Cmd> {
        if self.error().is_some() {
            return None;
        }
        Some(Cmd::FileOp(FileOpCmd::CreateLink {
            dir: self.dir.clone(),
            name: self.name.trim().to_string(),
            target: PathBuf::from(self.target.trim()),
            kind: self.kind,
        }))
    }
}

/// "Copy to…" / "Move to…" with a destination folder.
pub struct TransferModel {
    pub paths: Vec<PathBuf>,
    pub dest: String,
    pub mv: bool,
    /// Ask the frontend to focus the destination field once.
    pub focus: bool,
}

impl TransferModel {
    pub fn new(paths: Vec<PathBuf>, start: PathBuf, mv: bool) -> Self {
        TransferModel { paths, dest: soyuz_fs::strip_verbatim(&start).display().to_string(), mv, focus: true }
    }

    pub fn title(&self) -> &'static str {
        if self.mv { tr!("dialogs-transfer-move-title") } else { tr!("dialogs-transfer-copy-title") }
    }

    /// "Copy “a.txt” to:" / "Move 3 items to:".
    pub fn message(&self) -> String {
        let n = self.paths.len();
        if n == 1 {
            let name = shown(&soyuz_core::entry::display_name(&self.paths[0]));
            if self.mv {
                tr!("dialogs-transfer-move-one", name = name)
            } else {
                tr!("dialogs-transfer-copy-one", name = name)
            }
        } else if self.mv {
            tr!("dialogs-transfer-move-many", count = n)
        } else {
            tr!("dialogs-transfer-copy-many", count = n)
        }
    }

    /// Confirmed: the transfer command (`None` without a destination).
    pub fn confirm(&self) -> Option<Cmd> {
        if self.dest.trim().is_empty() {
            return None;
        }
        let dest = PathBuf::from(soyuz_fs::expand_env(self.dest.trim()));
        let effect = if self.mv { DropEffect::Move } else { DropEffect::Copy };
        Some(Cmd::FileOp(FileOpCmd::Transfer { paths: self.paths.clone(), dest, effect }))
    }
}

// ---------------------------------------------------------------------------------
// Quit with running jobs
// ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitChoice {
    KeepOpen,
    /// The safe default (Enter).
    QuitWhenFinished,
    CancelAndQuit,
}

/// Closing the window cancels pending jobs, so ask first.
pub struct QuitModel {
    pub title: String,
    /// Shared with `QuitState::prompting`; cleared when the model goes away.
    open: Arc<AtomicBool>,
}

impl QuitModel {
    pub fn new(pending: usize, open: Arc<AtomicBool>) -> Self {
        QuitModel { title: tr!("dialogs-quit-title", count = pending), open }
    }

    pub fn choose(&self, c: QuitChoice) -> Option<Cmd> {
        match c {
            QuitChoice::KeepOpen => None,
            QuitChoice::QuitWhenFinished => Some(Cmd::Quit { when_idle: true }),
            QuitChoice::CancelAndQuit => Some(Cmd::Quit { when_idle: false }),
        }
    }
}

impl Drop for QuitModel {
    fn drop(&mut self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------------
// MCP approval
// ---------------------------------------------------------------------------------

/// A plan an MCP client wants to run (`[ai.mcp] approval_ui = "dialog"`): one card of
/// [`McpApprovals`]. Closing it without an answer rejects the request.
pub struct McpApprovalModel {
    pub id: u64,
    pub approvals: Arc<Mutex<McpApprovals>>,
    /// Shared with the bridge; cleared when the model goes away.
    open: Arc<AtomicBool>,
    pub title: String,
    pub destructive: bool,
}

impl McpApprovalModel {
    pub fn new(id: u64, approvals: Arc<Mutex<McpApprovals>>, open: Arc<AtomicBool>) -> Self {
        let (title, destructive) = match approvals.lock().card(id) {
            Some(c) if c.card.plan.risk == Risk::Destructive => {
                (tr!("dialogs-mcp-title-destructive", client = c.client()), true)
            }
            Some(c) => (tr!("dialogs-mcp-title", client = c.client()), false),
            None => (tr!("dialogs-mcp-title-unknown").to_string(), false),
        };
        McpApprovalModel { id, approvals, open, title, destructive }
    }

    /// The card still waits for an answer (not answered, withdrawn or timed out).
    pub fn is_pending(&self) -> bool {
        self.approvals.lock().card(self.id).is_some_and(|c| c.is_pending())
    }

    /// Approve / Edit / Reject (a no-op once answered).
    pub fn decide(&self, d: Decision) {
        self.approvals.lock().decide(self.id, d);
    }

    /// "Approve all of this kind for this client".
    pub fn approve_kind(&self) {
        self.approvals.lock().approve_kind(self.id);
    }

    /// Dismissed: reject (a no-op once answered).
    pub fn reject(&self) {
        self.decide(Decision::Reject("the user dismissed the approval card".into()));
    }
}

impl Drop for McpApprovalModel {
    fn drop(&mut self) {
        self.reject();
        self.open.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------------
// Script approval
// ---------------------------------------------------------------------------------

/// The changes a script action planned (`[scripting] approve_writes`): Run submits them
/// as normal jobs, Cancel (or closing) drops them.
pub struct ScriptApprovalModel {
    pub action: String,
    pub jobs: Vec<PlannedJob>,
    pub title: String,
    pub destructive: bool,
}

impl ScriptApprovalModel {
    pub fn new(action: String, jobs: Vec<PlannedJob>) -> Self {
        let destructive = jobs.iter().any(|j| matches!(j.kind.brief(), JobKindBrief::Trash | JobKindBrief::Delete));
        let title = tr!("automation-approval-title", action = &action, count = jobs.len());
        ScriptApprovalModel { action, jobs, title, destructive }
    }

    /// Run: submit the planned jobs (only once).
    pub fn run(&mut self) -> Option<Cmd> {
        if self.jobs.is_empty() {
            return None;
        }
        Some(Cmd::Automation(AutoCmd::SubmitPlanned {
            title: self.action.clone(),
            jobs: std::mem::take(&mut self.jobs),
        }))
    }

    /// Cancel: the planned jobs are dropped.
    pub fn cancel(&mut self) {
        self.jobs.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use soyuz_core::{EntryInfo, EntryKind};
    use soyuz_jobs::JobKind;

    use super::*;

    fn shared() -> Rc<RefCell<PromptShared>> {
        Rc::new(RefCell::new(PromptShared { alive: true, total: 1, ..Default::default() }))
    }

    fn conflict(kind: ConflictKind) -> JobPrompt {
        JobPrompt::Conflict(ConflictInfo {
            src: EntryInfo::minimal(PathBuf::from("src").join("a.txt"), EntryKind::File),
            dst: EntryInfo::minimal(PathBuf::from("dst").join("a.txt"), EntryKind::File),
            kind,
            suggested_name: "a (2).txt".into(),
        })
    }

    fn job() -> JobId {
        JobId(7)
    }

    /// `(resolution, apply_to_all)` of a conflict answer.
    fn conf(a: Option<PromptAnswer>) -> Option<(ConflictResolution, bool)> {
        match a? {
            PromptAnswer::Respond(PromptResponse::Conflict { resolution, apply_to_all }) => {
                Some((resolution, apply_to_all))
            }
            _ => None,
        }
    }

    fn respond(cmd: Cmd) -> PromptResponse {
        match cmd {
            Cmd::FileOp(FileOpCmd::Respond { job: j, response }) => {
                assert_eq!(j, job());
                response
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn conflict_choice_and_apply_to_all() {
        let s = shared();
        let mut m = PromptModel::new(job(), conflict(ConflictKind::FileExists), "Copy".into(), s.clone());
        assert_eq!(conf(Some(m.resolve(ConflictResolution::Skip))), Some((ConflictResolution::Skip, false)));
        m.apply_all = true;
        let a = m.resolve(ConflictResolution::KeepBoth);
        assert!(!s.borrow().answered);
        let r = respond(m.answer(a));
        assert!(matches!(r, PromptResponse::Conflict { resolution: ConflictResolution::KeepBoth, apply_to_all: true }));
        assert!(s.borrow().answered);
        // Enter replaces, with apply-to-all.
        assert_eq!(conf(m.default_answer()), Some((ConflictResolution::Overwrite, true)));
    }

    #[test]
    fn conflict_rename_never_applies_to_all() {
        let mut m = PromptModel::new(job(), conflict(ConflictKind::FileExists), "Copy".into(), shared());
        m.apply_all = true;
        assert!(m.commit_rename().is_none(), "not renaming yet");
        m.start_rename();
        assert_eq!(m.rename.as_deref(), Some("a (2).txt"));
        assert!(m.focus_rename);
        assert!(m.default_answer().is_none(), "Enter belongs to the name field while renaming");
        assert!(m.rename_differs());
        m.rename = Some("a.txt".into());
        assert!(m.rename_valid() && !m.rename_differs());
        m.rename = Some(" b.txt ".into());
        assert_eq!(conf(m.commit_rename()), Some((ConflictResolution::Rename("b.txt".into()), false)));
        m.rename = Some(String::new());
        assert!(m.commit_rename().is_none(), "invalid name");
        m.cancel_rename();
        assert!(m.rename.is_none());
    }

    #[test]
    fn move_into_self_skips_only_this_one() {
        let mut m = PromptModel::new(job(), conflict(ConflictKind::MoveIntoSelf), "Move".into(), shared());
        m.apply_all = true;
        assert_eq!(conf(Some(m.resolve(ConflictResolution::Skip))), Some((ConflictResolution::Skip, false)));
        assert!(m.default_answer().is_none());
    }

    #[test]
    fn prompt_cancel_job_stale_and_drop() {
        let s = shared();
        let mut m = PromptModel::new(job(), conflict(ConflictKind::DirExists), "Copy".into(), s.clone());
        assert!(matches!(m.answer(PromptAnswer::CancelJob), Cmd::FileOp(FileOpCmd::CancelJob(j)) if j == job()));
        assert!(m.poll());
        s.borrow_mut().stale = true;
        assert!(!m.poll());
        drop(m);
        assert!(!s.borrow().alive);

        // Dismissed without an answer: deferred (not answered).
        let s = shared();
        let mut stack = DialogStack::default();
        stack.push(PromptModel::new(job(), conflict(ConflictKind::FileExists), "Copy".into(), s.clone()));
        assert!(stack.is_open());
        stack.close_top();
        assert!(!stack.is_open());
        let s = s.borrow();
        assert!(!s.alive && !s.answered);
    }

    #[test]
    fn error_prompt_answers() {
        let error = soyuz_core::FsError::new(soyuz_core::FsErrorKind::PermissionDenied, "read", None);
        let p = JobPrompt::Error { path: PathBuf::from("x"), error, retryable: true };
        let mut m = PromptModel::new(job(), p, "Copy".into(), shared());
        m.apply_all = true;
        assert!(matches!(
            m.default_answer(),
            Some(PromptAnswer::Respond(PromptResponse::Error { action: ErrorAction::Retry, apply_to_all: false }))
        ));
        assert!(matches!(
            m.trash_unsupported(true),
            PromptAnswer::Respond(PromptResponse::TrashUnsupported { permanent: true, apply_to_all: true })
        ));
        assert!(m.submit_password().is_none());
        m.password = "pw".into();
        assert!(matches!(
            m.submit_password(),
            Some(PromptAnswer::Respond(PromptResponse::Password(Some(ref p)))) if p == "pw"
        ));
    }

    #[test]
    fn delete_confirm_and_cancel() {
        let paths = vec![PathBuf::from("a"), PathBuf::from("b")];
        let mut m = DeleteConfirmModel::new(paths.clone(), DeleteMode::Permanent);
        assert!(!m.to_trash());
        assert_eq!(m.names(), vec!["a".to_string(), "b".to_string()]);
        assert!(m.texts().2, "permanent delete is destructive");
        match m.confirm() {
            Some(Cmd::FileOp(FileOpCmd::Delete { paths: p, mode: DeleteMode::Permanent })) => assert_eq!(p, paths),
            other => panic!("unexpected {other:?}"),
        }
        assert!(m.confirm().is_none(), "only once");

        let mut m = DeleteConfirmModel::new(vec![PathBuf::from("a")], DeleteMode::Trash);
        assert!(m.to_trash() && !m.texts().2);
        m.cancel();
        assert!(m.confirm().is_none(), "cancelled");

        let mut m = DeleteConfirmModel::empty_trash(3);
        assert!(m.names().is_empty());
        assert!(matches!(m.confirm(), Some(Cmd::FileOp(FileOpCmd::EmptyTrash))));
    }

    #[test]
    fn quit_choices_and_flag() {
        let flag = Arc::new(AtomicBool::new(true));
        let mut stack = DialogStack::default();
        stack.push(QuitModel::new(2, flag.clone()));
        let Some(DialogModel::Quit(m)) = stack.top_mut() else { panic!() };
        assert!(m.choose(QuitChoice::KeepOpen).is_none());
        assert!(matches!(m.choose(QuitChoice::QuitWhenFinished), Some(Cmd::Quit { when_idle: true })));
        assert!(matches!(m.choose(QuitChoice::CancelAndQuit), Some(Cmd::Quit { when_idle: false })));
        assert!(flag.load(Ordering::SeqCst));
        stack.close_top();
        assert!(!flag.load(Ordering::SeqCst), "the prompting flag clears when the dialog goes away");
    }

    fn mcp_request(
        approvals: &Arc<Mutex<McpApprovals>>,
        tool: &str,
    ) -> (u64, crossbeam_channel::Receiver<soyuz_mcp::ApprovalDecision>) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let plan = soyuz_ai::tools::Plan {
            tool: tool.into(),
            summary: "Move 1 item".into(),
            risk: Risk::Write,
            items: Vec::new(),
            total_bytes: 0,
            total_entries: 0,
            conflicts: 0,
            executable: true,
            warnings: Vec::new(),
            action: soyuz_ai::tools::PlanAction::None,
        };
        let req = soyuz_mcp::ApprovalRequest {
            client: "test-client".into(),
            transport: soyuz_mcp::Transport::Memory,
            tool: tool.into(),
            args: serde_json::Value::Null,
            plan,
        };
        let p =
            crate::ai::mcp::PendingMcpApproval::new(req, soyuz_core::OpControl::new(), Duration::from_secs(60), 1, tx);
        let id = approvals.lock().push(p);
        (id, rx)
    }

    #[test]
    fn mcp_approve_deny_and_drop() {
        let approvals = Arc::new(Mutex::new(McpApprovals::default()));
        let flag = Arc::new(AtomicBool::new(true));

        let (id, rx) = mcp_request(&approvals, "move");
        let m = McpApprovalModel::new(id, approvals.clone(), flag.clone());
        assert!(m.is_pending() && !m.destructive);
        m.decide(Decision::Approve);
        assert!(!m.is_pending());
        assert_eq!(rx.try_recv().ok(), Some(soyuz_mcp::ApprovalDecision::Approved));
        drop(m); // already answered: dropping changes nothing
        assert!(rx.try_recv().is_err());
        assert!(!flag.load(Ordering::SeqCst));

        let (id, rx) = mcp_request(&approvals, "move");
        let m = McpApprovalModel::new(id, approvals.clone(), flag.clone());
        m.decide(Decision::Reject("no".into()));
        assert_eq!(rx.try_recv().ok(), Some(soyuz_mcp::ApprovalDecision::Rejected("no".into())));

        // Closing without an answer rejects.
        let (id, rx) = mcp_request(&approvals, "move");
        flag.store(true, Ordering::SeqCst);
        let mut stack = DialogStack::default();
        stack.push(McpApprovalModel::new(id, approvals.clone(), flag.clone()));
        stack.close_top();
        assert!(matches!(rx.try_recv().ok(), Some(soyuz_mcp::ApprovalDecision::Rejected(_))));
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[test]
    fn script_approval_run_once() {
        let jobs = vec![PlannedJob {
            kind: JobKind::Delete { paths: vec![PathBuf::from("x")], mode: DeleteMode::Trash },
            summary: "x".into(),
        }];
        let mut m = ScriptApprovalModel::new("Tidy".into(), jobs);
        assert!(m.destructive);
        assert!(
            matches!(m.run(), Some(Cmd::Automation(AutoCmd::SubmitPlanned { ref title, ref jobs })) if title == "Tidy" && jobs.len() == 1)
        );
        assert!(m.run().is_none());
    }

    #[test]
    fn link_and_transfer_forms() {
        let dir = std::env::temp_dir();
        let mut m = LinkModel::new(dir.clone(), None, LinkChoice::Symlink, vec!["taken".into()]);
        assert!(m.create().is_none() && m.error().is_some(), "needs a target");
        m.target = "some/where/file.txt".into();
        m.target_edited();
        assert!(!m.name.is_empty());
        m.name = "TAKEN".into();
        m.name_edited();
        assert!(m.create().is_none(), "name exists");
        m.target = "other.txt".into();
        m.target_edited();
        assert_eq!(m.name, "TAKEN", "a typed name is kept");
        m.name = "new link".into();
        match m.create() {
            Some(Cmd::FileOp(FileOpCmd::CreateLink { name, kind: LinkChoice::Symlink, .. })) => {
                assert_eq!(name, "new link")
            }
            other => panic!("unexpected {other:?}"),
        }

        let mut t = TransferModel::new(vec![PathBuf::from("a")], dir, true);
        assert!(matches!(t.confirm(), Some(Cmd::FileOp(FileOpCmd::Transfer { effect: DropEffect::Move, .. }))));
        t.dest = "  ".into();
        assert!(t.confirm().is_none());
    }

    #[test]
    fn stack_order() {
        let mut stack = DialogStack::default();
        let a = next_seq();
        stack.push(DeleteConfirmModel::empty_trash(1));
        assert!(stack.top_seq().unwrap() > a);
        assert_eq!(stack.len(), 1);
        assert!(stack.top().is_some_and(DialogModel::alert));
    }
}
