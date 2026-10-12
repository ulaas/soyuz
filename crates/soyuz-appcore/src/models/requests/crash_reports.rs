//! Crash reports offered on the next start (`FrontendCmd::CrashReports`): list them, view
//! one, delete them, open their folder or restart in safe mode. Reports never leave the
//! computer. The choices go to `services::crash::request` (carried out by `crash::pump`).

use crate::services::crash::{CrashReport, RecoveryAction, request};

/// Shown text is capped (reports with huge log lines stay readable).
pub const VIEW_CAP: usize = 64 * 1024;
/// Reports listed at most.
pub const MAX_LISTED: usize = 8;

/// The crash reports dialog while it is open.
#[derive(Debug)]
pub struct CrashReportsModel {
    pub title: String,
    pub reports: Vec<CrashReport>,
    /// Index of the report being viewed and its text.
    pub viewing: Option<(usize, String)>,
}

impl CrashReportsModel {
    pub fn new(reports: Vec<CrashReport>) -> Self {
        let title = tr!("dialogs-crash-title", count = reports.len());
        CrashReportsModel { title, reports, viewing: None }
    }

    /// The reports listed (newest first, at most [`MAX_LISTED`]).
    pub fn listed(&self) -> &[CrashReport] {
        &self.reports[..self.reports.len().min(MAX_LISTED)]
    }

    /// Report `i` is being viewed.
    pub fn is_viewing(&self, i: usize) -> bool {
        self.viewing.as_ref().is_some_and(|(v, _)| *v == i)
    }

    /// View report `i` (read from disk, capped at [`VIEW_CAP`]), or hide it when it is
    /// the one shown.
    pub fn toggle_view(&mut self, i: usize) {
        if self.is_viewing(i) {
            self.viewing = None;
            return;
        }
        let Some(r) = self.reports.get(i) else { return };
        self.viewing = Some((i, read_capped(&r.path)));
    }

    /// Delete every report (the dialog closes).
    pub fn delete(&self) {
        request(RecoveryAction::DeleteReports(self.reports.clone()));
    }

    /// Keep the reports and never offer them again (Close).
    pub fn keep(&self) {
        request(RecoveryAction::KeepReports(self.reports.clone()));
    }

    /// Keep the reports and open the crashes folder in a new tab.
    pub fn open_folder(&self) {
        self.keep();
        request(RecoveryAction::OpenCrashFolder);
    }

    /// Keep the reports and restart in safe mode.
    pub fn restart_safe_mode(&self) {
        self.keep();
        request(RecoveryAction::RestartSafeMode);
    }
}

/// A report's text, cut at [`VIEW_CAP`] on a character boundary.
pub fn read_capped(path: &std::path::Path) -> String {
    let mut text = std::fs::read_to_string(path).unwrap_or_else(|e| format!("({e})"));
    if text.len() > VIEW_CAP {
        let mut cut = VIEW_CAP;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push('\n');
        text.push_str(tr!("dialogs-crash-truncated"));
    }
    text
}

/// When the report was written (short date and time; empty when unknown).
pub fn when(r: &CrashReport) -> String {
    r.when
        .map(|t| soyuz_core::format_time(t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Short)))
        .unwrap_or_default()
}

/// The report's panic line, or its file name.
pub fn summary(r: &CrashReport) -> String {
    if r.summary.is_empty() { r.name.clone() } else { r.summary.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_toggles_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("crash-1.txt");
        std::fs::write(&p, format!("Panic: boom\n{}", "x".repeat(VIEW_CAP + 10))).unwrap();
        let r = CrashReport { path: p, name: "crash-1.txt".into(), when: None, summary: String::new() };
        let mut m = CrashReportsModel::new(vec![r.clone()]);
        assert_eq!(summary(&r), "crash-1.txt");
        m.toggle_view(0);
        let (i, text) = m.viewing.clone().unwrap();
        assert_eq!(i, 0);
        assert!(text.starts_with("Panic: boom") && text.len() < VIEW_CAP + 200);
        m.toggle_view(0);
        assert!(m.viewing.is_none());
        m.toggle_view(7);
        assert!(m.viewing.is_none());
    }
}
