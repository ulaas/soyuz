//! The rows of the activity (jobs) panel, without a toolkit: what each job card and each
//! history entry says (title, state line, progress, byte / item counts, speed and ETA),
//! the tones its parts take, and which buttons apply (pause / resume / cancel / resolve /
//! remove / retry failed on jobs, undo / redo / retry failed on history entries).
//!
//! The GUI (`ui::panels::activity`, `activity_history`) draws its cards from these rows;
//! the terminal UI (`soyuz-tui`, `view/jobs.rs`) draws lines. Both carry the buttons out
//! with [`apply_job`] / [`apply_history`].

use std::time::SystemTime;

use soyuz_core::config::Action;
use soyuz_core::{DateFormat, SizeStyle, format_eta, format_size, format_speed};
use soyuz_jobs::{HistoryEntry, HistoryId, JobId, JobSnapshot, JobState, JobSummary, LogLevel, Priority};

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::icon::IconId;
use crate::models::media_tools::MediaTool;
use crate::models::panels::{describe, fraction, is_verified, verify_badge};
use crate::ops::fileops::{FileOpCmd, HistoryAction, history_action, names};
use crate::services::jobs::{JobMeta, OpKind};

/// A colour role of a part of a row (the frontends map it to their palette).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Primary text.
    Text,
    /// Secondary text.
    Muted,
    /// Tertiary text.
    Faint,
    Accent,
    Info,
    Success,
    Warning,
    Danger,
    /// The folder glyph colour.
    Folder,
}

/// A button on a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowButton {
    Pause,
    Resume,
    Cancel,
    /// Answer the job's deferred prompt.
    Resolve,
    /// Remove a finished job from the list.
    Remove,
    /// Run the failed items again.
    RetryFailed,
    Undo,
    Redo,
}

/// What a finished job's icon shows instead of the operation glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoneMark {
    /// Done without failures.
    Check,
    /// Failed.
    Warning,
    /// Cancelled.
    Prohibit,
}

/// Icon of an operation (the GUI's `op_style` glyphs).
pub fn op_icon(op: OpKind) -> IconId {
    match op {
        OpKind::Copy | OpKind::Duplicate => IconId::Copy,
        OpKind::Move => IconId::ArrowBendDownRight,
        OpKind::Trash => IconId::Trash,
        OpKind::Delete | OpKind::Wipe | OpKind::EmptyTrash => IconId::TrashSimple,
        OpKind::Rename => IconId::PencilSimple,
        OpKind::NewFolder => IconId::FolderPlus,
        OpKind::NewFile => IconId::FilePlus,
        OpKind::Link => IconId::Link,
        OpKind::SetMeta => IconId::Sliders,
        OpKind::Restore => IconId::ArrowCounterClockwise,
        OpKind::Undo => IconId::ArrowUUpLeft,
        OpKind::Redo => IconId::ArrowUUpRight,
        OpKind::Retry => IconId::ArrowClockwise,
        OpKind::Media(MediaTool::Convert) => IconId::FilmStrip,
        OpKind::Media(MediaTool::ExtractAudio) => IconId::MusicNotes,
        OpKind::Media(MediaTool::Trim) => IconId::Scissors,
        OpKind::Media(MediaTool::SaveFrame) => IconId::Image,
        OpKind::Other => IconId::GearSix,
    }
}

/// Tone of an operation's icon (the GUI's `op_style` colours).
pub fn op_tone(op: OpKind) -> Tone {
    match op {
        OpKind::Copy
        | OpKind::Duplicate
        | OpKind::Rename
        | OpKind::NewFile
        | OpKind::SetMeta
        | OpKind::Retry
        | OpKind::Media(_) => Tone::Accent,
        OpKind::Move | OpKind::Link => Tone::Info,
        OpKind::Trash => Tone::Warning,
        OpKind::Delete | OpKind::Wipe | OpKind::EmptyTrash => Tone::Danger,
        OpKind::NewFolder => Tone::Folder,
        OpKind::Restore => Tone::Success,
        OpKind::Undo | OpKind::Redo | OpKind::Other => Tone::Muted,
    }
}

/// The label of the jobs tab ("Jobs · 2" while jobs are pending).
pub fn jobs_tab_label(core: &AppCore) -> String {
    let pending = core.fileops.jobs.pending().count();
    if pending > 0 { tr!("panels-tab-jobs-pending", count = pending) } else { tr!("panels-tab-jobs").to_string() }
}

/// The jobs in the panel's order: active first (by queue position), finished last
/// (newest first).
pub fn sorted_snapshots(core: &AppCore) -> Vec<JobSnapshot> {
    let mut snaps: Vec<JobSnapshot> = core.fileops.jobs.snapshots().to_vec();
    snaps.sort_by_key(|s| {
        (
            s.state.is_terminal(),
            if s.state.is_terminal() {
                u64::MAX - s.id.0
            } else {
                s.queue_pos.map_or(0, |q| q as u64 + 1) * 1_000_000 + s.id.0
            },
        )
    });
    snaps
}

/// One job card.
#[derive(Clone, Debug, PartialEq)]
pub struct JobRow {
    pub id: JobId,
    pub op: OpKind,
    pub title: String,
    /// The state line: queue position, the current item, "Paused", the outcome, …
    pub line: String,
    pub line_tone: Tone,
    /// The status colour (progress, ring, done mark).
    pub status: Tone,
    pub done_mark: Option<DoneMark>,
    /// Fraction done, `None` while unknown.
    pub fraction: Option<f32>,
    /// Running without a known fraction: a moving (or, reduced motion, dimmed) bar.
    pub indeterminate: bool,
    /// Bytes and items done (" · "-joined).
    pub stats: String,
    /// Speed and ETA while running, the average speed when done.
    pub speed: String,
    /// "Verifying" / "Verified".
    pub badge: Option<&'static str>,
    /// Not finished.
    pub active: bool,
    /// Waiting for an answer.
    pub waiting: bool,
    /// Done with failed items.
    pub failed: bool,
    /// Not started yet (priority / reorder / start now apply).
    pub queued: bool,
    /// The job's priority (the "More" menu ticks it).
    pub priority: Priority,
    /// The summary of a finished job.
    pub summary: Option<JobSummary>,
    /// The buttons that apply, in reading order.
    pub buttons: Vec<RowButton>,
}

impl JobRow {
    /// The row of snapshot `s`.
    pub fn new(core: &AppCore, s: &JobSnapshot) -> JobRow {
        let meta = core
            .fileops
            .jobs
            .meta(s.id)
            .cloned()
            .unwrap_or_else(|| JobMeta::new(OpKind::from_brief(s.kind), &[], None));
        let active = !s.state.is_terminal();
        let total_items = s.progress.totals_known.then_some(s.progress.items_total);
        let title = names::job_title(&meta, !matches!(s.state, JobState::Done(_)), total_items, &s.label);
        let summary = match &s.state {
            JobState::Done(sum) => Some(sum.clone()),
            _ => None,
        };
        let failed = summary.as_ref().is_some_and(|x| x.items_failed > 0);
        let waiting = s.pending_prompt.is_some();
        let status = match &s.state {
            JobState::Failed(_) => Tone::Danger,
            JobState::Cancelled => Tone::Faint,
            JobState::Done(_) if failed => Tone::Warning,
            JobState::Done(_) => Tone::Success,
            JobState::Paused => Tone::Faint,
            JobState::WaitingForUser => Tone::Warning,
            _ => Tone::Accent,
        };
        let done_mark = match &s.state {
            JobState::Done(_) if !failed => Some(DoneMark::Check),
            JobState::Failed(_) => Some(DoneMark::Warning),
            JobState::Cancelled => Some(DoneMark::Prohibit),
            _ => None,
        };
        let line_tone = match &s.state {
            JobState::WaitingForUser => Tone::Warning,
            JobState::Failed(_) => Tone::Danger,
            JobState::Done(_) if failed => Tone::Warning,
            _ => Tone::Muted,
        };
        let frac = fraction(s);
        let indeterminate = frac.is_none() && active && s.state != JobState::Paused && s.state != JobState::Queued;
        let queued = matches!(s.state, JobState::Queued) || (s.state == JobState::Paused && s.started.is_none());
        let mut buttons = Vec::new();
        if active {
            if waiting && core.fileops.prompts.is_deferred(s.id) {
                buttons.push(RowButton::Resolve);
            }
            if s.state == JobState::Paused {
                buttons.push(RowButton::Resume);
            } else if s.state != JobState::WaitingForUser {
                buttons.push(RowButton::Pause);
            }
            buttons.push(RowButton::Cancel);
        } else {
            if failed {
                buttons.push(RowButton::RetryFailed);
            }
            buttons.push(RowButton::Remove);
        }
        JobRow {
            id: s.id,
            op: meta.op,
            title,
            line: state_line(s),
            line_tone,
            status,
            done_mark,
            fraction: frac,
            indeterminate,
            stats: stats(s, summary.as_ref()),
            speed: speed(s, summary.as_ref()),
            badge: verify_badge(s.kind, s.verify.is_some(), &s.state),
            active,
            waiting,
            failed,
            queued,
            priority: s.priority,
            summary,
            buttons,
        }
    }

    pub fn has(&self, b: RowButton) -> bool {
        self.buttons.contains(&b)
    }
}

/// Every job's row, in the panel's order ([`sorted_snapshots`]).
pub fn job_rows(core: &AppCore) -> Vec<JobRow> {
    sorted_snapshots(core).iter().map(|s| JobRow::new(core, s)).collect()
}

/// The state line of a job.
pub fn state_line(s: &JobSnapshot) -> String {
    match &s.state {
        JobState::Queued => match s.queue_pos {
            Some(q) => tr!("panels-state-queued-at", position = (q + 1).to_string()),
            None => tr!("panels-state-queued").into(),
        },
        JobState::Planning => tr!("panels-state-planning").into(),
        JobState::Running => s
            .progress
            .current
            .as_ref()
            .map(|c| soyuz_core::entry::display_name(c))
            .unwrap_or_else(|| tr!("panels-state-working").into()),
        JobState::Paused => tr!("panels-state-paused").into(),
        JobState::WaitingForUser => tr!("panels-state-waiting").into(),
        JobState::Finishing => tr!("panels-state-finishing").into(),
        JobState::Done(_) | JobState::Failed(_) | JobState::Cancelled => names::outcome(&s.state),
    }
}

/// Bytes and items done ("12 MiB of 40 MiB · 3 of 9 items"). A job cancelled before it
/// did anything shows none ("0 B of 48 MiB" reads like a failure); its state line
/// already says "Cancelled".
fn stats(s: &JobSnapshot, summary: Option<&JobSummary>) -> String {
    let pr = &s.progress;
    if matches!(s.state, JobState::Cancelled) && pr.bytes_done == 0 && pr.items_done == 0 {
        return String::new();
    }
    let mut left = Vec::new();
    if pr.bytes_total > 0 {
        left.push(tr!(
            "panels-progress-of",
            done = format_size(pr.bytes_done, SizeStyle::Binary),
            total = format_size(pr.bytes_total, SizeStyle::Binary)
        ));
    } else if let Some(sum) = summary
        && sum.bytes > 0
    {
        left.push(format_size(sum.bytes, SizeStyle::Binary));
    }
    if pr.items_total > 0 {
        left.push(tr!("panels-items-of", done = pr.items_done, total = pr.items_total));
    }
    left.join(" \u{b7} ")
}

/// Speed and ETA while running, the average speed of a finished job.
fn speed(s: &JobSnapshot, summary: Option<&JobSummary>) -> String {
    if matches!(s.state, JobState::Running) {
        match s.eta {
            Some(eta) => {
                tr!("panels-speed-eta", speed = format_speed(s.speed_bps), eta = format_eta(eta.as_secs_f64()))
            }
            None => format_speed(s.speed_bps),
        }
    } else if let Some(sum) = summary {
        let secs = sum.elapsed.as_secs_f64();
        if sum.bytes > 0 && secs > 0.5 {
            tr!("panels-speed-avg", speed = format_speed(sum.bytes as f64 / secs))
        } else {
            String::new()
        }
    } else {
        String::new()
    }
}

/// Carry out button `b` of job `id`.
pub fn apply_job(core: &mut AppCore, id: JobId, b: RowButton) {
    match b {
        RowButton::Pause => core.fileops.jobs.engine().pause(id),
        RowButton::Resume => core.fileops.jobs.engine().resume(id),
        RowButton::Cancel => core.fileops.jobs.engine().cancel(id),
        RowButton::Remove => core.fileops.jobs.engine().remove(id),
        RowButton::Resolve => core.cmds.push(Cmd::FileOp(FileOpCmd::Resolve(id))),
        RowButton::RetryFailed => {
            let summary = match core.fileops.jobs.snapshot(id).map(|s| &s.state) {
                Some(JobState::Done(sum)) => Some(sum.clone()),
                _ => None,
            };
            let h = summary
                .and_then(|sum| core.fileops.jobs.history().iter().rev().find(|h| h.summary == sum).map(|h| h.id));
            match h {
                Some(h) => history_action(core, h, HistoryAction::Retry),
                None => core.cmds.push(Cmd::Action(Action::JobsShowHistory)),
            }
        }
        RowButton::Undo | RowButton::Redo => {}
    }
}

/// One history entry.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryRow {
    pub id: HistoryId,
    pub op: OpKind,
    /// Past-tense title ("Copied “a.txt” to /b").
    pub title: String,
    /// When, the summary, and "can't be undone".
    pub sub: String,
    /// The summary has failed items (the sub line warns).
    pub failed: bool,
    pub undone: bool,
    pub undoable: bool,
    /// Copies verified by hashing (not when undone).
    pub verified: bool,
    /// The buttons, in reading order, with whether each is enabled.
    pub buttons: Vec<(RowButton, bool)>,
}

impl HistoryRow {
    /// The row of `h`, its time relative to `now`.
    pub fn new(h: &HistoryEntry, now: SystemTime) -> HistoryRow {
        let (op, title) = describe(&h.job);
        let rel = DateFormat::Named(soyuz_core::DateFormatName::Relative);
        let mut sub = format!(
            "{} \u{b7} {}",
            soyuz_core::time::format_time_at(h.when, &rel, now),
            names::summary_line(&h.summary)
        );
        if !h.undoable && !h.undone {
            sub.push_str(" \u{b7} ");
            sub.push_str(tr!("panels-history-not-undoable"));
        }
        let failed = h.summary.items_failed > 0;
        let mut buttons = Vec::new();
        if failed {
            buttons.push((RowButton::RetryFailed, true));
        }
        if h.undone {
            buttons.push((RowButton::Redo, true));
        } else {
            buttons.push((RowButton::Undo, h.undoable));
        }
        HistoryRow {
            id: h.id,
            op,
            title,
            sub,
            failed,
            undone: h.undone,
            undoable: h.undoable,
            verified: !h.undone && is_verified(h),
            buttons,
        }
    }

    /// Whether button `b` is on the row and enabled.
    pub fn enabled(&self, b: RowButton) -> bool {
        self.buttons.iter().any(|&(x, on)| x == b && on)
    }
}

/// Every history entry's row, newest first.
pub fn history_rows(core: &AppCore, now: SystemTime) -> Vec<HistoryRow> {
    core.fileops.jobs.history().iter().rev().map(|h| HistoryRow::new(h, now)).collect()
}

/// Carry out button `b` of history entry `id` (undo, redo, retry failed).
pub fn apply_history(core: &mut AppCore, id: HistoryId, b: RowButton) {
    let what = match b {
        RowButton::Undo => HistoryAction::Undo,
        RowButton::Redo => HistoryAction::Redo,
        RowButton::RetryFailed => HistoryAction::Retry,
        _ => return,
    };
    history_action(core, id, what);
}

// ---- the jobs tab's toolbar, a job's "More" menu, the details ----------------------

/// A button of the jobs tab's toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarButton {
    PauseAll,
    ResumeAll,
    CancelAll,
    /// Remove every finished job from the list.
    ClearFinished,
}

impl ToolbarButton {
    /// Its label (the GUI's tooltip).
    pub fn label(self) -> &'static str {
        match self {
            ToolbarButton::PauseAll => tr!("panels-pause-all"),
            ToolbarButton::ResumeAll => tr!("panels-resume-all"),
            ToolbarButton::CancelAll => tr!("panels-cancel-all"),
            ToolbarButton::ClearFinished => tr!("panels-clear-finished"),
        }
    }

    pub fn icon(self) -> IconId {
        match self {
            ToolbarButton::PauseAll => IconId::Pause,
            ToolbarButton::ResumeAll => IconId::Play,
            ToolbarButton::CancelAll => IconId::XCircle,
            ToolbarButton::ClearFinished => IconId::Broom,
        }
    }
}

/// The toolbar's buttons, left to right, with whether each applies (something pending,
/// something finished).
pub fn toolbar(core: &AppCore) -> Vec<(ToolbarButton, bool)> {
    let pending = core.fileops.jobs.pending().count() > 0;
    let any_done = core.fileops.jobs.snapshots().iter().any(|s| s.state.is_terminal());
    vec![
        (ToolbarButton::PauseAll, pending),
        (ToolbarButton::ResumeAll, pending),
        (ToolbarButton::CancelAll, pending),
        (ToolbarButton::ClearFinished, any_done),
    ]
}

/// Carry out toolbar button `b`.
pub fn apply_toolbar(core: &mut AppCore, b: ToolbarButton) {
    let eng = core.fileops.jobs.engine();
    match b {
        ToolbarButton::PauseAll => eng.pause_all(),
        ToolbarButton::ResumeAll => eng.resume_all(),
        ToolbarButton::CancelAll => eng.cancel_all(),
        ToolbarButton::ClearFinished => eng.clear_finished(),
    }
}

/// What a row of a job's "More" menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobMenuCmd {
    Priority(Priority),
    MoveUp,
    MoveDown,
    /// Start a queued job now, ahead of the queue.
    StartNow,
    Cancel,
}

/// A row of a job's "More" menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobMenuEntry {
    /// The "Priority" submenu: `(command, label, checked)` per priority.
    Priority(Vec<(JobMenuCmd, &'static str, bool)>),
    Item {
        cmd: JobMenuCmd,
        icon: IconId,
        label: &'static str,
        enabled: bool,
    },
    Separator,
}

/// The "More" menu of an active job (priority, reorder and start now apply while it is
/// queued); empty for a finished one.
pub fn job_menu(row: &JobRow) -> Vec<JobMenuEntry> {
    if !row.active {
        return Vec::new();
    }
    let prio = [
        (Priority::High, tr!("panels-priority-high")),
        (Priority::Normal, tr!("panels-priority-normal")),
        (Priority::Low, tr!("panels-priority-low")),
    ]
    .into_iter()
    .map(|(p, label)| (JobMenuCmd::Priority(p), label, row.priority == p))
    .collect();
    let q = row.queued;
    vec![
        JobMenuEntry::Priority(prio),
        JobMenuEntry::Item { cmd: JobMenuCmd::MoveUp, icon: IconId::ArrowUp, label: tr!("panels-move-up"), enabled: q },
        JobMenuEntry::Item {
            cmd: JobMenuCmd::MoveDown,
            icon: IconId::CaretDown,
            label: tr!("panels-move-down"),
            enabled: q,
        },
        JobMenuEntry::Item {
            cmd: JobMenuCmd::StartNow,
            icon: IconId::Lightning,
            label: tr!("panels-start-now"),
            enabled: q,
        },
        JobMenuEntry::Separator,
        JobMenuEntry::Item {
            cmd: JobMenuCmd::Cancel,
            icon: IconId::XCircle,
            label: tr!("common-cancel"),
            enabled: true,
        },
    ]
}

/// Carry out "More" menu command `c` on job `id`.
pub fn apply_menu(core: &mut AppCore, id: JobId, c: JobMenuCmd) {
    let eng = core.fileops.jobs.engine();
    match c {
        JobMenuCmd::Priority(p) => eng.set_priority(id, p),
        JobMenuCmd::MoveUp => eng.move_in_queue(id, -1),
        JobMenuCmd::MoveDown => eng.move_in_queue(id, 1),
        JobMenuCmd::StartNow => eng.start_now(id),
        JobMenuCmd::Cancel => eng.cancel(id),
    }
}

/// Whether job `id`'s details (errors and log) are shown.
pub fn is_expanded(core: &AppCore, id: JobId) -> bool {
    core.fileops.activity.expanded.contains(&id)
}

/// Show or hide job `id`'s details.
pub fn toggle_expanded(core: &mut AppCore, id: JobId) {
    let e = &mut core.fileops.activity.expanded;
    if !e.remove(&id) {
        e.insert(id);
    }
}

/// The details of job `id`, newest log line first: the failed items and notes of its
/// summary, the failure, then up to 300 log lines (`panels-log-empty` when there is
/// nothing). Reads the job's log (cached by the jobs service).
pub fn detail_lines(core: &mut AppCore, id: JobId) -> Vec<(Tone, String)> {
    let mut lines: Vec<(Tone, String)> = Vec::new();
    let state = core.fileops.jobs.snapshot(id).map(|s| s.state.clone());
    if let Some(JobState::Done(sum)) = &state {
        for (path, e) in sum.errors.iter().take(200) {
            lines.push((Tone::Danger, format!("\u{2715} {}  \u{2014}  {e}", soyuz_fs::strip_verbatim(path).display())));
        }
        for n in &sum.notes {
            lines.push((Tone::Faint, format!("\u{2139} {n}")));
        }
    }
    if let Some(JobState::Failed(e)) = &state {
        lines.push((Tone::Danger, format!("\u{2715} {e}")));
    }
    for l in core.fileops.jobs.log(id).iter().rev().take(300) {
        let tone = match l.level {
            LogLevel::Info => Tone::Muted,
            LogLevel::Warn => Tone::Warning,
            LogLevel::Error => Tone::Danger,
        };
        let path = l.path.as_ref().map(|p| format!("  {}", soyuz_fs::strip_verbatim(p).display())).unwrap_or_default();
        lines.push((tone, format!("{:>6.1}s  {}{path}", l.at.as_secs_f32(), l.msg)));
    }
    if lines.is_empty() {
        lines.push((Tone::Faint, tr!("panels-log-empty").into()));
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use soyuz_jobs::{JobKind, JobKindBrief, Priority, Progress};

    use super::*;

    fn snap(state: JobState) -> JobSnapshot {
        JobSnapshot {
            id: JobId(3),
            kind: JobKindBrief::Copy,
            label: "Copy".into(),
            state,
            priority: Priority::Normal,
            queue_pos: None,
            progress: Progress::default(),
            started: None,
            elapsed: Duration::ZERO,
            speed_bps: 0.0,
            eta: None,
            samples: std::sync::Arc::from(Vec::new()),
            pending_prompt: None,
            warnings: 0,
            log_len: 0,
            verify: None,
        }
    }

    #[test]
    fn state_lines_and_stats() {
        let mut s = snap(JobState::Queued);
        s.queue_pos = Some(1);
        assert_eq!(state_line(&s), "Queued · #2 in line");
        s.state = JobState::Paused;
        assert_eq!(state_line(&s), "Paused");
        s.state = JobState::Running;
        s.progress.current = Some("/a/big.bin".into());
        assert_eq!(state_line(&s), "big.bin");
        s.progress.bytes_total = 4096;
        s.progress.bytes_done = 1024;
        s.progress.items_total = 2;
        s.progress.items_done = 1;
        assert_eq!(stats(&s, None), "1 KiB of 4 KiB · 1 of 2 items");
        // Cancelled: what got done stays; nothing done shows nothing.
        s.state = JobState::Cancelled;
        assert_eq!(stats(&s, None), "1 KiB of 4 KiB · 1 of 2 items");
        s.progress.bytes_done = 0;
        s.progress.items_done = 0;
        assert_eq!(stats(&s, None), "");
        s.state = JobState::Running;
        s.speed_bps = 2048.0;
        s.eta = Some(Duration::from_secs(5));
        assert!(speed(&s, None).contains("left"), "{}", speed(&s, None));
        let sum = JobSummary { bytes: 4096, elapsed: Duration::from_secs(2), ..JobSummary::default() };
        s.state = JobState::Done(sum.clone());
        assert!(speed(&s, Some(&sum)).starts_with("avg "));
    }

    #[test]
    fn icons_and_tones() {
        assert_eq!(op_icon(OpKind::Copy), IconId::Copy);
        assert_eq!(op_icon(OpKind::Undo), IconId::ArrowUUpLeft);
        assert_eq!(op_tone(OpKind::Delete), Tone::Danger);
        assert_eq!(op_tone(OpKind::NewFolder), Tone::Folder);
    }

    #[test]
    fn history_buttons() {
        let mut h = HistoryEntry {
            id: HistoryId(9),
            job: JobKind::Copy { sources: vec!["/a/x".into()], dest_dir: "/b".into(), rename_to: None },
            summary: JobSummary::default(),
            when: SystemTime::now(),
            records: Vec::new(),
            undone: false,
            undoable: true,
            stamps: Vec::new(),
            options: None,
            this_session: true,
        };
        let r = HistoryRow::new(&h, SystemTime::now());
        assert_eq!(r.buttons, vec![(RowButton::Undo, true)]);
        assert!(r.title.starts_with("Copied"), "{}", r.title);
        assert!(r.enabled(RowButton::Undo) && !r.enabled(RowButton::Redo));
        h.undoable = false;
        h.summary.items_failed = 2;
        let r = HistoryRow::new(&h, SystemTime::now());
        assert_eq!(r.buttons, vec![(RowButton::RetryFailed, true), (RowButton::Undo, false)]);
        assert!(r.failed && r.sub.ends_with("can't be undone"), "{}", r.sub);
        h.undone = true;
        assert_eq!(HistoryRow::new(&h, SystemTime::now()).buttons[1], (RowButton::Redo, true));
    }
}
