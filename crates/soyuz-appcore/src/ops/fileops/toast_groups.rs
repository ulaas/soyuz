//! Operations of the same kind share one toast, so deleting a hundred files one by one
//! does not leave a hundred toasts queueing long after the work ended.
//!
//! A group starts with a job's first registration and stays open while one of its jobs
//! runs or its toast is on screen; a matching job started in that time joins it. One job
//! alone shows the toast it always did. From the second on, the toast counts them ("Moving
//! 7/12 items to the trash", no timer) and ends with the outcome and an Undo for them all,
//! or with a warning and Details when something failed.

use std::path::PathBuf;

use soyuz_core::config::Action;
use soyuz_jobs::{HistoryId, JobId, JobState, JobSummary};

use super::names;
use crate::models::toasts::{Toast, ToastKind, Toasts};
use crate::services::jobs::{JobMeta, JobOrigin, OpKind};

/// Jobs merge when all of this is the same. The destination keeps "copied to D:" and
/// "copied to E:" apart, the starter keeps a rule's or a client's work out of an Undo that
/// is about the user's own operations.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GroupKey {
    op: OpKind,
    dest: Option<PathBuf>,
    origin: Option<JobOrigin>,
    ai_task: Option<u64>,
    mcp_client: Option<String>,
}

impl GroupKey {
    fn of(meta: &JobMeta) -> Option<GroupKey> {
        let merges = matches!(
            meta.op,
            OpKind::Copy
                | OpKind::Move
                | OpKind::Duplicate
                | OpKind::Trash
                | OpKind::Delete
                | OpKind::Wipe
                | OpKind::SetMeta
                | OpKind::Restore
                | OpKind::Undo
                | OpKind::Redo
        );
        (merges && meta.toast).then(|| GroupKey {
            op: meta.op,
            dest: meta.dest.clone(),
            origin: meta.origin,
            ai_task: meta.ai_task,
            mcp_client: meta.mcp_client.clone(),
        })
    }

    /// Undo and redo jobs do not know their items: they count as operations.
    fn counts_operations(&self) -> bool {
        matches!(self.op, OpKind::Undo | OpKind::Redo)
    }
}

/// How a job of a group ended.
pub struct Finished<'a> {
    pub state: &'a JobState,
    pub summary: &'a JobSummary,
    /// Its history entry, while it can be undone.
    pub undo: Option<HistoryId>,
    /// The toast the job shows when it is alone.
    pub toast: Option<Toast>,
}

struct Group {
    key: GroupKey,
    /// Jobs still running, with their weight in `total`.
    running: Vec<(JobId, u64)>,
    jobs: usize,
    /// Top-level items of all its jobs.
    total: u64,
    ok: u64,
    /// Items whose job failed, or that it reported failed.
    failed: u64,
    /// The engine's own counts, for the body line.
    summary: JobSummary,
    /// Finish order: oldest first.
    undo: Vec<(HistoryId, u64)>,
    /// The toast on screen. Still set but no longer open: the user closed it.
    toast: Option<u64>,
    /// The last finished job's own toast (what shows when it was the only one).
    single: Option<Toast>,
}

impl Group {
    fn new(key: GroupKey) -> Group {
        Group {
            key,
            running: Vec::new(),
            jobs: 0,
            total: 0,
            ok: 0,
            failed: 0,
            summary: JobSummary::default(),
            undo: Vec::new(),
            toast: None,
            single: None,
        }
    }

    fn title(&self, active: bool, what: String) -> String {
        names::group_title(self.key.op, self.key.dest.as_deref(), active, what)
    }

    fn progress(&self) -> Toast {
        let done = self.total - self.running.iter().map(|(_, w)| w).sum::<u64>();
        let what = if self.key.counts_operations() {
            tr!("ops-group-progress-ops", done = done, total = self.total)
        } else {
            tr!("ops-group-progress", done = done, total = self.total)
        };
        Toast::info(self.title(true, what)).sticky()
    }

    /// `done` of everything: "12 items", or "9 of 12 items" when some did not make it.
    fn outcome(&self, done: u64) -> String {
        match (self.key.counts_operations(), done == self.total) {
            (false, true) => tr!("common-items", count = done),
            (false, false) => tr!("ops-group-partial", done = done, total = self.total),
            (true, true) => tr!("ops-group-ops", count = done),
            (true, false) => tr!("ops-group-partial-ops", done = done, total = self.total),
        }
    }

    /// The toast once nothing runs any more.
    fn outcome_toast(&mut self) -> Option<Toast> {
        if self.jobs <= 1 {
            return self.single.take();
        }
        if self.total == 0 {
            return None;
        }
        let what = self.outcome(self.ok);
        let body = names::summary_line(&self.summary);
        let entries: Vec<HistoryId> = self.undo.iter().map(|(h, _)| *h).collect();
        if self.failed > 0 {
            let mut t =
                Toast::warning(self.title(false, what)).body(body).action(tr!("ops-details"), Action::JobsShowHistory);
            // The button shows the failures; the key still undoes what worked.
            t.group_undo = entries;
            return Some(t);
        }
        if self.ok == 0 {
            // Nothing to show for it: all cancelled or skipped.
            let mut t = Toast::info(self.title(false, what));
            if self.summary.items_skipped > 0 {
                t = t.body(body);
            }
            return Some(t);
        }
        let mut t = if self.ok == self.total {
            Toast::success(self.title(false, what))
        } else {
            Toast::info(self.title(false, what))
        }
        .body(body);
        match entries.as_slice() {
            [] => {}
            [one] => t = t.undo_button(tr!("common-undo"), *one),
            _ => {
                let items: u64 = self.undo.iter().map(|(_, w)| w).sum();
                t = t.undo_group_button(tr!("ops-undo-count", count = items), entries);
            }
        }
        if !self.undo.is_empty() {
            t.duration = Some(7.0);
        }
        Some(t)
    }

    /// Show `toast`: in place of the group's toast when that is still up.
    fn show(&mut self, toast: Toast, toasts: &mut Toasts) {
        match self.toast {
            Some(id) if toasts.update(id, toast.clone()) => {}
            _ => self.toast = Some(toasts.push(toast)),
        }
    }

    fn is_open(&self, toasts: &Toasts) -> bool {
        self.toast.is_some_and(|id| toasts.is_open(id))
    }

    /// Bring the toast up to date after a job left the running list.
    fn refresh(&mut self, toasts: &mut Toasts) {
        let open = self.is_open(toasts);
        if !self.running.is_empty() {
            // Closed by the user: the rest finishes quietly, unless it goes wrong.
            if open {
                self.show(self.progress(), toasts);
            }
            return;
        }
        let closed = self.toast.is_some() && !open;
        match self.outcome_toast() {
            Some(t) if closed && !matches!(t.kind, ToastKind::Warning | ToastKind::Error) => {}
            Some(t) => self.show(t, toasts),
            None => {
                if let Some(id) = self.toast.filter(|_| open) {
                    toasts.dismiss(id);
                }
            }
        }
    }
}

/// Top-level items a job counts for: undo and redo jobs have none, they count once.
fn weight(meta: &JobMeta) -> u64 {
    (meta.items as u64).max(1)
}

#[derive(Default)]
pub struct ToastGroups {
    groups: Vec<Group>,
}

impl ToastGroups {
    /// A job was registered: it joins the open group of its kind or starts one.
    pub fn started(&mut self, id: JobId, meta: &JobMeta, toasts: &mut Toasts) {
        let Some(key) = GroupKey::of(meta) else { return };
        self.groups.retain(|g| !g.running.is_empty() || g.is_open(toasts));
        let i = match self.groups.iter().position(|g| g.key == key) {
            Some(i) => i,
            None => {
                self.groups.push(Group::new(key));
                self.groups.len() - 1
            }
        };
        let g = &mut self.groups[i];
        let w = weight(meta);
        g.running.push((id, w));
        g.jobs += 1;
        g.total += w;
        if g.jobs >= 2 {
            g.show(g.progress(), toasts);
        }
    }

    /// A job ended. `Some` hands its toast back when the job is in no group.
    pub fn finished(&mut self, id: JobId, f: Finished<'_>, toasts: &mut Toasts) -> Option<Toast> {
        let found = self.groups.iter().position(|g| g.running.iter().any(|(j, _)| *j == id));
        let Some(i) = found else { return f.toast };
        let g = &mut self.groups[i];
        let w = g.running.iter().find(|(j, _)| *j == id).map_or(1, |(_, w)| *w);
        g.running.retain(|(j, _)| *j != id);
        match f.state {
            JobState::Done(_) => {
                let failed = f.summary.items_failed.min(w);
                g.failed += failed;
                if f.summary.items_ok > 0 || f.summary.items_skipped == 0 {
                    g.ok += w - failed;
                }
                g.summary.items_ok += f.summary.items_ok;
                g.summary.items_skipped += f.summary.items_skipped;
                g.summary.items_failed += f.summary.items_failed;
                g.summary.elapsed += f.summary.elapsed;
            }
            JobState::Failed(_) => {
                g.failed += w;
                g.summary.items_failed += w;
            }
            _ => {}
        }
        if let Some(h) = f.undo {
            g.undo.push((h, w));
        }
        g.single = f.toast;
        g.refresh(toasts);
        None
    }

    /// A job went away without reporting how it ended: it no longer counts.
    pub fn abandoned(&mut self, id: JobId, toasts: &mut Toasts) {
        let Some(g) = self.groups.iter_mut().find(|g| g.running.iter().any(|(j, _)| *j == id)) else { return };
        let w = g.running.iter().find(|(j, _)| *j == id).map_or(1, |(_, w)| *w);
        g.running.retain(|(j, _)| *j != id);
        g.jobs -= 1;
        g.total -= w;
        g.refresh(toasts);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use soyuz_core::FsError;
    use soyuz_jobs::JobId;

    use super::*;
    use crate::models::toasts::Toasts;

    fn trash(n: usize) -> JobMeta {
        let items: Vec<PathBuf> = (0..n).map(|i| PathBuf::from(format!("/a/{i}"))).collect();
        JobMeta::new(OpKind::Trash, &items, None)
    }

    fn done(ok: u64, failed: u64) -> (JobState, JobSummary) {
        let s =
            JobSummary { items_ok: ok, items_failed: failed, elapsed: Duration::from_secs(1), ..Default::default() };
        (JobState::Done(s.clone()), s)
    }

    /// The job's toast when alone, with an Undo.
    fn alone(h: u64) -> Toast {
        Toast::success("Moved it").undo_button("Undo", HistoryId(h))
    }

    fn finish(g: &mut ToastGroups, t: &mut Toasts, id: u64, ok: u64, failed: u64, undo: Option<u64>) -> Option<Toast> {
        let (state, summary) = done(ok, failed);
        let f = Finished { state: &state, summary: &summary, undo: undo.map(HistoryId), toast: undo.map(alone) };
        g.finished(JobId(id), f, t)
    }

    fn titles(t: &Toasts) -> Vec<String> {
        t.live.iter().filter(|l| l.closing.is_none()).map(|l| l.toast.title.clone()).collect()
    }

    #[test]
    fn one_job_alone_keeps_its_own_toast() {
        let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
        g.started(JobId(1), &trash(1), &mut t);
        assert!(titles(&t).is_empty(), "no progress toast next to the activity panel");
        finish(&mut g, &mut t, 1, 1, 0, Some(1));
        assert_eq!(titles(&t), ["Moved it"]);
        assert_eq!(t.live[0].toast.undoes, Some(HistoryId(1)));
    }

    #[test]
    fn many_jobs_share_one_toast_that_counts_them() {
        let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
        for id in 1..=3 {
            g.started(JobId(id), &trash(1), &mut t);
        }
        assert_eq!(titles(&t), ["Moving 0/3 items to the trash"]);
        assert!(t.live[0].remaining.is_none(), "no timer while it counts");
        finish(&mut g, &mut t, 1, 1, 0, Some(1));
        finish(&mut g, &mut t, 2, 1, 0, Some(2));
        assert_eq!(titles(&t), ["Moving 2/3 items to the trash"]);
        finish(&mut g, &mut t, 3, 1, 0, Some(3));
        assert_eq!(titles(&t), ["Moved 3 items to the trash"]);
        let l = &t.live[0];
        assert!(l.remaining.is_some(), "the timer starts at the end");
        assert_eq!(l.toast.action.as_ref().unwrap().0, "Undo 3");
        assert_eq!(l.toast.group_undo, [HistoryId(1), HistoryId(2), HistoryId(3)]);
        assert!(matches!(
            l.toast.command(),
            Some(crate::cmd::Cmd::FileOp(super::super::FileOpCmd::UndoEntries(e))) if e.len() == 3
        ));
    }

    #[test]
    fn a_job_joins_while_the_toast_is_up_and_starts_over_after_it_closed() {
        let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
        g.started(JobId(1), &trash(2), &mut t);
        finish(&mut g, &mut t, 1, 2, 0, Some(1));
        g.started(JobId(2), &trash(1), &mut t);
        assert_eq!(titles(&t), ["Moving 2/3 items to the trash"], "took over the toast");
        finish(&mut g, &mut t, 2, 1, 0, Some(2));
        assert_eq!(t.live.len(), 1);

        assert!(t.dismiss_all());
        g.started(JobId(3), &trash(1), &mut t);
        assert_eq!(titles(&t), Vec::<String>::new(), "a new group: one job, no count yet");
        finish(&mut g, &mut t, 3, 1, 0, Some(3));
        assert_eq!(titles(&t), ["Moved it"]);
    }

    #[test]
    fn a_failure_stays_visible() {
        let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
        g.started(JobId(1), &trash(1), &mut t);
        g.started(JobId(2), &trash(1), &mut t);
        finish(&mut g, &mut t, 1, 1, 0, Some(1));
        let (state, summary) = (JobState::Failed(FsError::unsupported("trash", None)), JobSummary::default());
        g.finished(JobId(2), Finished { state: &state, summary: &summary, undo: None, toast: None }, &mut t);
        assert_eq!(titles(&t), ["Moved 1 of 2 items to the trash"]);
        let l = &t.live[0].toast;
        assert_eq!(l.kind, ToastKind::Warning);
        assert_eq!(l.action.as_ref().unwrap().1, Action::JobsShowHistory);
        assert_eq!(l.group_undo, [HistoryId(1)], "the key can still undo what worked");
        assert!(l.body.as_deref().unwrap().contains("1 failed"));
    }

    #[test]
    fn closing_the_counter_keeps_quiet_unless_something_fails() {
        for failed in [0, 1] {
            let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
            g.started(JobId(1), &trash(1), &mut t);
            g.started(JobId(2), &trash(1), &mut t);
            assert!(t.dismiss_all());
            t.live.clear();
            finish(&mut g, &mut t, 1, 1, 0, Some(1));
            assert!(titles(&t).is_empty());
            finish(&mut g, &mut t, 2, 1 - failed, failed, None);
            assert_eq!(titles(&t).len() as u64, failed, "failed = {failed}");
        }
    }

    #[test]
    fn kinds_destinations_and_starters_stay_apart() {
        let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
        let src = [PathBuf::from("/a/x")];
        g.started(JobId(1), &JobMeta::new(OpKind::Copy, &src, Some("/d".into())), &mut t);
        g.started(JobId(2), &JobMeta::new(OpKind::Copy, &src, Some("/e".into())), &mut t);
        g.started(JobId(3), &JobMeta::new(OpKind::Move, &src, Some("/d".into())), &mut t);
        g.started(JobId(4), &trash(1), &mut t);
        g.started(JobId(5), &JobMeta::new(OpKind::Delete, &src, None), &mut t);
        g.started(JobId(6), &trash(1).origin(JobOrigin::Automation), &mut t);
        assert!(titles(&t).is_empty(), "six jobs, six groups of one");
        g.started(JobId(7), &trash(1).quiet(), &mut t);
        assert!(titles(&t).is_empty(), "a quiet job is in no group");
        let handed_back = finish(&mut g, &mut t, 7, 1, 0, Some(7));
        assert!(handed_back.is_some(), "and the caller shows what it has");
    }

    #[test]
    fn undo_jobs_count_operations() {
        let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
        let undo = || JobMeta::new(OpKind::Undo, &[], None);
        g.started(JobId(1), &undo(), &mut t);
        g.started(JobId(2), &undo(), &mut t);
        assert_eq!(titles(&t), ["Undoing: 0/2 operations"]);
        finish(&mut g, &mut t, 1, 1, 0, None);
        finish(&mut g, &mut t, 2, 1, 0, None);
        assert_eq!(titles(&t), ["Undone: 2 operations"]);
    }

    #[test]
    fn a_job_that_vanishes_does_not_leave_the_counter_up() {
        let (mut g, mut t) = (ToastGroups::default(), Toasts::default());
        g.started(JobId(1), &trash(1), &mut t);
        g.started(JobId(2), &trash(1), &mut t);
        g.abandoned(JobId(1), &mut t);
        g.abandoned(JobId(2), &mut t);
        assert!(titles(&t).is_empty());
        g.abandoned(JobId(9), &mut t);
    }
}
