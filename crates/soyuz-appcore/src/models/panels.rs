//! The bottom panel (terminal / activity tabs) and the activity panel's state and rules:
//! visibility, the active tab, the panel height, where the activity panel is placed,
//! the status-bar job indicator and the history wording. `ui::panels` draws them.

use std::collections::HashSet;
use std::path::PathBuf;

use soyuz_core::config::ActivityPlacement;
use soyuz_jobs::{CreateKind, DeleteMode, HistoryEntry, JobId, JobKind, JobSnapshot, JobState};

use crate::core::AppCore;
use crate::ops::fileops::names;
use crate::services::jobs::{JobMeta, OpKind};

/// Default bottom panel height (the GUI token `size::TERMINAL_DEFAULT`).
pub const DEFAULT_HEIGHT: f32 = 260.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomTab {
    Terminal,
    Activity,
}

impl BottomTab {
    pub fn label(self) -> &'static str {
        match self {
            BottomTab::Terminal => tr!("panels-tab-terminal"),
            BottomTab::Activity => tr!("panels-tab-activity"),
        }
    }
}

pub struct BottomPanel {
    pub open: bool,
    pub tab: BottomTab,
    pub height: f32,
    /// Apply `height` on the next frame even if the user resized the panel before
    /// (`[layout] terminal_height` changed).
    pub reset_height: bool,
    /// `[layout] activity_panel`: a bottom-panel tab, a right side panel or a window.
    pub activity_placement: ActivityPlacement,
    /// The activity panel is shown (right / floating placements).
    pub activity_open: bool,
}

impl Default for BottomPanel {
    fn default() -> Self {
        BottomPanel {
            open: false,
            tab: BottomTab::Terminal,
            height: DEFAULT_HEIGHT,
            reset_height: false,
            activity_placement: ActivityPlacement::Bottom,
            activity_open: false,
        }
    }
}

impl BottomPanel {
    /// Toggle `tab`: opens the panel on it, switches to it, or closes when already shown.
    pub fn toggle(&mut self, tab: BottomTab) {
        if tab == BottomTab::Activity && self.activity_placement != ActivityPlacement::Bottom {
            self.activity_open = !self.activity_open;
            return;
        }
        if self.open && self.tab == tab {
            self.open = false;
        } else {
            self.open = true;
            self.tab = tab;
        }
    }

    /// Show the activity panel wherever it is placed.
    pub fn open_activity(&mut self) {
        if self.activity_placement == ActivityPlacement::Bottom {
            self.open = true;
            self.tab = BottomTab::Activity;
        } else {
            self.activity_open = true;
        }
    }

    pub fn activity_shown(&self) -> bool {
        match self.activity_placement {
            ActivityPlacement::Bottom => self.open && self.tab == BottomTab::Activity,
            _ => self.activity_open,
        }
    }

    /// Move the activity panel (keeps it visible if it was).
    pub fn set_activity_placement(&mut self, placement: ActivityPlacement) {
        if placement == self.activity_placement {
            return;
        }
        let shown = self.activity_shown();
        if self.activity_placement == ActivityPlacement::Bottom && shown {
            self.open = false;
            self.tab = BottomTab::Terminal;
        }
        self.activity_placement = placement;
        self.activity_open = false;
        if shown {
            self.open_activity();
        }
    }

    /// The tabs of the bottom panel (the activity tab only in the bottom placement).
    pub fn tabs(&self) -> &'static [BottomTab] {
        if self.activity_placement == ActivityPlacement::Bottom {
            &[BottomTab::Terminal, BottomTab::Activity]
        } else {
            &[BottomTab::Terminal]
        }
    }

    /// Show the terminal tab.
    pub fn show_terminal(&mut self) {
        self.open = true;
        self.tab = BottomTab::Terminal;
    }

    /// The terminal tab is on screen.
    pub fn terminal_shown(&self) -> bool {
        self.open && self.tab == BottomTab::Terminal
    }
}

// ---------------------------------------------------------------------------------------
// Activity panel.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityTab {
    #[default]
    Jobs,
    History,
}

#[derive(Default)]
pub struct ActivityState {
    pub tab: ActivityTab,
    pub expanded: HashSet<JobId>,
}

/// Badge for jobs whose copies are hash-verified (copy / move / sync): "Verifying"
/// while running, "Verified" when done without failures.
pub fn verify_badge(kind: soyuz_jobs::JobKindBrief, verify: bool, state: &JobState) -> Option<&'static str> {
    use soyuz_jobs::JobKindBrief as K;
    if !verify || !matches!(kind, K::Copy | K::Move | K::Sync) {
        return None;
    }
    match state {
        JobState::Done(sum) if sum.items_failed == 0 => Some(tr!("panels-verified")),
        JobState::Done(_) | JobState::Failed(_) | JobState::Cancelled => None,
        _ => Some(tr!("panels-verifying")),
    }
}

/// Overall fraction done, `None` while unknown.
pub fn fraction(s: &JobSnapshot) -> Option<f32> {
    if matches!(s.state, JobState::Done(_)) {
        return Some(1.0);
    }
    let pr = &s.progress;
    if pr.bytes_total > 0 {
        Some((pr.bytes_done as f64 / pr.bytes_total as f64).clamp(0.0, 1.0) as f32)
    } else if pr.items_total > 0 && pr.totals_known {
        Some((pr.items_done as f64 / pr.items_total as f64).clamp(0.0, 1.0) as f32)
    } else {
        None
    }
}

/// Data for the status-bar indicator.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Indicator {
    pub pending: usize,
    pub progress: Option<f32>,
    pub waiting: bool,
    pub paused: bool,
}

pub fn indicator(app: &AppCore) -> Indicator {
    let mut ind = Indicator::default();
    let (mut done, mut total) = (0u64, 0u64);
    for s in app.fileops.jobs.pending() {
        ind.pending += 1;
        ind.waiting |= s.pending_prompt.is_some();
        ind.paused |= s.state == JobState::Paused;
        done += s.progress.bytes_done;
        total += s.progress.bytes_total;
    }
    ind.progress = (total > 0).then(|| (done as f64 / total as f64) as f32);
    ind
}

/// Operation kind and past-tense title of a history entry.
pub fn describe(job: &JobKind) -> (OpKind, String) {
    let (op, meta) = match job {
        JobKind::Copy { sources, dest_dir, .. } => {
            let same = sources.iter().all(|s| s.parent() == Some(dest_dir.as_path()));
            let op = if same { OpKind::Duplicate } else { OpKind::Copy };
            (op, JobMeta::new(op, sources, Some(dest_dir.clone())))
        }
        JobKind::Move { sources, dest_dir, .. } => {
            (OpKind::Move, JobMeta::new(OpKind::Move, sources, Some(dest_dir.clone())))
        }
        JobKind::Delete { paths, mode } => {
            let op = match mode {
                DeleteMode::Trash => OpKind::Trash,
                DeleteMode::Permanent => OpKind::Delete,
                DeleteMode::Wipe { .. } => OpKind::Wipe,
            };
            (op, JobMeta::new(op, paths, None))
        }
        JobKind::Rename { pairs } => {
            let from: Vec<PathBuf> = pairs.iter().map(|p| p.0.clone()).collect();
            let dest = (pairs.len() == 1).then(|| PathBuf::from(soyuz_core::entry::display_name(&pairs[0].1)));
            (OpKind::Rename, JobMeta::new(OpKind::Rename, &from, dest))
        }
        JobKind::Create { path, kind } => {
            let op = match kind {
                CreateKind::Dir => OpKind::NewFolder,
                CreateKind::File { .. } => OpKind::NewFile,
                _ => OpKind::Link,
            };
            (op, JobMeta::new(op, std::slice::from_ref(path), None))
        }
        JobKind::RestoreTrash { items, .. } => {
            let paths: Vec<PathBuf> = items.iter().map(|i| i.original.clone()).collect();
            (OpKind::Restore, JobMeta::new(OpKind::Restore, &paths, None))
        }
        JobKind::EmptyTrash => (OpKind::EmptyTrash, JobMeta::new(OpKind::EmptyTrash, &[], None)),
        JobKind::Media { src, dest, op } => {
            let op = OpKind::Media(crate::models::media_tools::MediaTool::of_op(op));
            (op, JobMeta::new(op, std::slice::from_ref(src), Some(dest.clone())))
        }
        other => (OpKind::Other, JobMeta::new(OpKind::Other, &other.touched_roots(), None)),
    };
    (op, names::job_title(&meta, false, None, ""))
}

/// A copy / move / sync that ran with hash verification and had no failures.
pub fn is_verified(h: &HistoryEntry) -> bool {
    let verify = h.options.as_ref().is_some_and(|o| o.copy.verify.is_some());
    let done = soyuz_jobs::JobState::Done(h.summary.clone());
    verify_badge(h.job.brief(), verify, &done).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_and_placement() {
        let mut b = BottomPanel::default();
        b.toggle(BottomTab::Terminal);
        assert!(b.terminal_shown());
        b.toggle(BottomTab::Activity);
        assert!(b.activity_shown());
        b.toggle(BottomTab::Activity);
        assert!(!b.open);
        b.open_activity();
        b.set_activity_placement(ActivityPlacement::Right);
        assert!(!b.open && b.activity_open && b.activity_shown());
        assert_eq!(b.tabs(), &[BottomTab::Terminal]);
    }

    #[test]
    fn describes_history_entries() {
        let (op, t) = describe(&JobKind::Move { sources: vec!["/a/x".into()], dest_dir: "/b".into(), rename_to: None });
        assert_eq!(op, OpKind::Move);
        assert_eq!(t, "Moved \u{201c}x\u{201d} to /b");
        let (op, _) = describe(&JobKind::Copy { sources: vec!["/a/x".into()], dest_dir: "/a".into(), rename_to: None });
        assert_eq!(op, OpKind::Duplicate);
        let (op, t) = describe(&JobKind::Rename { pairs: vec![("/a/x".into(), "/a/y".into())] });
        assert_eq!(op, OpKind::Rename);
        assert_eq!(t, "Renamed \u{201c}x\u{201d} to \u{201c}y\u{201d}");
        let (op, _) = describe(&JobKind::Delete { paths: vec!["/a".into()], mode: DeleteMode::Permanent });
        assert_eq!(op, OpKind::Delete);
    }

    #[test]
    fn verified_badge_rules() {
        use soyuz_jobs::{JobKindBrief as K, JobState, JobSummary};
        let ok = JobState::Done(JobSummary::default());
        let bad = JobState::Done(JobSummary { items_failed: 1, ..JobSummary::default() });
        assert_eq!(verify_badge(K::Copy, true, &ok), Some("Verified"));
        assert_eq!(verify_badge(K::Sync, true, &JobState::Running), Some("Verifying"));
        assert_eq!(verify_badge(K::Copy, true, &bad), None, "failures: no badge");
        assert_eq!(verify_badge(K::Copy, false, &ok), None);
        assert_eq!(verify_badge(K::Delete, true, &ok), None, "only copying jobs");
        let mut h = HistoryEntry {
            id: soyuz_jobs::HistoryId(1),
            job: JobKind::Copy { sources: vec!["/a".into()], dest_dir: "/b".into(), rename_to: None },
            summary: JobSummary::default(),
            when: std::time::SystemTime::now(),
            records: Vec::new(),
            undone: false,
            undoable: true,
            stamps: Vec::new(),
            options: None,
            this_session: true,
        };
        assert!(!is_verified(&h), "old entries without options");
        let mut o = soyuz_jobs::JobOptions::default();
        o.copy.verify = Some(soyuz_core::HashAlgo::Blake3);
        h.options = Some(o);
        assert!(is_verified(&h));
    }
}
