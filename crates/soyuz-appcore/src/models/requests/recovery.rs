//! "Soyuz closed during N operations" (`FrontendCmd::RecoveredJobs`): the jobs found in a
//! crash journal, what each was and how far it got, and the user's choice: Resume (run
//! what is left), Roll back (undo the completed part, through the trash) or Discard.
//! Leftover temp files under the touched folders are scanned on a worker thread and,
//! when [`RecoveryModel::clean`] is on, moved to the trash with the same choice. Closing
//! without a choice ("Decide later") keeps everything for the next start.
//!
//! The choices go to `services::crash::request` and are carried out by `crash::pump`.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use soyuz_core::OpControl;
use soyuz_jobs::JournalJob;
use soyuz_jobs::journal::{self, Recovered};

use crate::host::Wake;
use crate::services::crash::{RecoveryAction, request};

/// Visit at most this many entries while looking for leftovers.
pub const SCAN_LIMIT: usize = 200_000;

/// The recovery offer while it is open.
pub struct RecoveryModel {
    pub title: String,
    recovered: Vec<Recovered>,
    /// Every interrupted job, oldest journal first.
    pub jobs: Vec<JournalJob>,
    /// `None` while the scan runs.
    leftovers: Arc<Mutex<Option<Vec<PathBuf>>>>,
    scan_ctl: Arc<OpControl>,
    /// Trash the leftovers with the choice.
    pub clean: bool,
    /// The leftover list is expanded.
    pub show_leftovers: bool,
}

impl std::fmt::Debug for RecoveryModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryModel").field("jobs", &self.jobs.len()).field("clean", &self.clean).finish()
    }
}

impl RecoveryModel {
    /// Start the leftover scan; `wake` runs when it is done.
    pub fn new(recovered: Vec<Recovered>, wake: Wake) -> Self {
        let jobs: Vec<JournalJob> = recovered.iter().flat_map(|r| r.jobs.iter().cloned()).collect();
        let title = tr!("dialogs-recovery-title", count = jobs.len());
        let leftovers = Arc::new(Mutex::new(None));
        let scan_ctl = OpControl::new();
        let (out, ctl, scan_jobs) = (leftovers.clone(), scan_ctl.clone(), jobs.clone());
        let spawned = std::thread::Builder::new().name("soyuz-recovery-scan".into()).spawn(move || {
            let found = journal::scan_leftovers(&scan_jobs, &ctl, SCAN_LIMIT);
            *out.lock() = Some(found);
            wake();
        });
        if spawned.is_err() {
            *leftovers.lock() = Some(Vec::new());
        }
        RecoveryModel { title, recovered, jobs, leftovers, scan_ctl, clean: true, show_leftovers: false }
    }

    /// The leftover temp files found, `None` while the scan runs.
    pub fn leftovers(&self) -> Option<Vec<PathBuf>> {
        self.leftovers.lock().clone()
    }

    /// What the choice moves to the trash.
    fn cleanup(&self) -> Vec<PathBuf> {
        if !self.clean {
            return Vec::new();
        }
        self.leftovers().unwrap_or_default()
    }

    /// Some job completed parts that can be undone.
    pub fn can_roll_back(&self) -> bool {
        self.jobs.iter().any(JournalJob::can_roll_back)
    }

    /// Resume every job (the dialog closes).
    pub fn resume(&self) {
        request(RecoveryAction::Resume {
            jobs: self.jobs.clone(),
            cleanup: self.cleanup(),
            dismiss: self.recovered.clone(),
        });
    }

    /// Roll back the completed parts; `false` (nothing requested) when there are none.
    pub fn roll_back(&self) -> bool {
        if !self.can_roll_back() {
            return false;
        }
        request(RecoveryAction::RollBack {
            jobs: self.jobs.clone(),
            cleanup: self.cleanup(),
            dismiss: self.recovered.clone(),
        });
        true
    }

    /// Forget the jobs, leaving the files as they are.
    pub fn discard(&self) {
        request(RecoveryAction::Discard { cleanup: self.cleanup(), dismiss: self.recovered.clone() });
    }
}

impl Drop for RecoveryModel {
    fn drop(&mut self) {
        self.scan_ctl.cancel();
    }
}

/// When `job` started (short date and time).
pub fn started(job: &JournalJob) -> String {
    soyuz_core::format_time(job.started, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Short))
}

/// "N% done" or "progress unknown".
pub fn done_text(job: &JournalJob) -> String {
    match job.fraction() {
        Some(f) => tr!("dialogs-recovery-done", percent = format!("{:.0}", f * 100.0)),
        None => tr!("dialogs-recovery-progress-unknown").to_string(),
    }
}

/// The line under a job's label: when it started, how far it got, what can be undone.
pub fn job_line(job: &JournalJob) -> String {
    if job.can_roll_back() {
        tr!("dialogs-recovery-job-line-undo", started = started(job), done = done_text(job), count = job.records.len())
    } else {
        tr!("dialogs-recovery-job-line", started = started(job), done = done_text(job))
    }
}
