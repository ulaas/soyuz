//! The Snapshots page: saved system states, and comparing two of them or one with now.
//! Files are read and written on `Loader` workers; comparing runs on one too.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use soyuz_sysmon::fetch::Loader;
use soyuz_sysmon::snapshot::{self, Snapshot, SnapshotMeta};
use soyuz_sysmon::{Change, Comparison, DiffSort, Frame, MatchBy, Tolerance, compare, sort_diffs};

use super::data_dir;

/// The right side of a comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompareTarget {
    /// The newest frame, at the moment Compare was pressed.
    Live,
    Snapshot(String),
}

/// The page.
pub struct SnapshotsState {
    /// Newest first.
    pub metas: Vec<SnapshotMeta>,
    list_loader: Option<Loader<std::io::Result<Vec<SnapshotMeta>>>>,
    /// Read the list again at the next pump.
    pub stale: bool,
    /// The row under the cursor.
    pub cursor: usize,
    /// Left side of the comparison.
    pub a: Option<String>,
    pub b: CompareTarget,
    pub match_by: MatchBy,
    pub sort: DiffSort,
    pub descending: bool,
    /// List unchanged processes too.
    pub show_same: bool,
    /// The comparison's plain text filter.
    pub filter: String,
    pub comparison: Option<Arc<Comparison>>,
    /// Indices into `comparison.procs`, sorted and filtered.
    pub shown: Vec<usize>,
    /// Row under the cursor in `shown`.
    pub diff_cursor: usize,
    /// A comparison is being computed.
    pub comparing: bool,
    /// Why the last load failed.
    pub error: Option<String>,
    /// Editing the label of a snapshot (`id`, text).
    pub renaming: Option<(String, String)>,
    /// Snapshots read from disk (their files can be large).
    cache: HashMap<String, Arc<Snapshot>>,
    compare_loader: Option<Loader<Result<Comparison, String>>>,
}

impl Default for SnapshotsState {
    fn default() -> Self {
        SnapshotsState {
            metas: Vec::new(),
            list_loader: None,
            stale: true,
            cursor: 0,
            a: None,
            b: CompareTarget::Live,
            match_by: MatchBy::Program,
            sort: DiffSort::CpuDelta,
            descending: true,
            show_same: false,
            filter: String::new(),
            comparison: None,
            shown: Vec::new(),
            diff_cursor: 0,
            comparing: false,
            error: None,
            renaming: None,
            cache: HashMap::new(),
            compare_loader: None,
        }
    }
}

impl SnapshotsState {
    pub fn pump(&mut self, _frame: Option<&Frame>, wake: &soyuz_sysmon::Wake) {
        if let Some(l) = self.list_loader.as_mut()
            && let Some(result) = l.poll()
        {
            self.list_loader = None;
            match result {
                Ok(m) => {
                    self.metas = m;
                    self.cursor = self.cursor.min(self.metas.len().saturating_sub(1));
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if self.stale && self.list_loader.is_none() {
            self.stale = false;
            let dir = data_dir();
            self.list_loader = Some(Loader::spawn("snapshots", wake.clone(), move || snapshot::list(&dir)));
        }
        if let Some(l) = self.compare_loader.as_mut()
            && let Some(result) = l.poll()
        {
            self.compare_loader = None;
            self.comparing = false;
            match result {
                Ok(c) => {
                    self.comparison = Some(Arc::new(c));
                    self.error = None;
                    self.diff_cursor = 0;
                    self.reshow();
                }
                Err(e) => self.error = Some(e),
            }
        }
    }

    /// Compare `a` with `b`, on a worker.
    pub fn start_compare(&mut self, live: Option<Arc<Frame>>, wake: &soyuz_sysmon::Wake) {
        let Some(a_id) = self.a.clone() else { return };
        let b = self.b.clone();
        let match_by = self.match_by;
        let cached_a = self.cache.get(&a_id).cloned();
        let cached_b = match &b {
            CompareTarget::Snapshot(id) => self.cache.get(id).cloned(),
            CompareTarget::Live => None,
        };
        let dir = data_dir();
        self.comparing = true;
        self.compare_loader = Some(Loader::spawn("snapshot-compare", wake.clone(), move || {
            let load = |id: &str, cached: Option<Arc<Snapshot>>| -> Result<Arc<Snapshot>, String> {
                match cached {
                    Some(s) => Ok(s),
                    None => snapshot::load(&dir, id).map(Arc::new).map_err(|e| e.to_string()),
                }
            };
            let a = load(&a_id, cached_a)?;
            let bf: Arc<Frame> = match &b {
                CompareTarget::Live => live.ok_or_else(|| tr!("monitor-no-data").to_string())?,
                CompareTarget::Snapshot(id) => Arc::new(load(id, cached_b)?.frame.clone()),
            };
            Ok(compare(&a.frame, &bf, match_by, Tolerance::default()))
        }));
    }

    /// Apply the sort, filter and "show unchanged" to the comparison.
    pub fn reshow(&mut self) {
        let Some(c) = self.comparison.clone() else {
            self.shown.clear();
            return;
        };
        let mut sorted = c.procs.clone();
        sort_diffs(&mut sorted, self.sort, self.descending);
        let filter = self.filter.to_lowercase();
        // `sort_diffs` works on a copy, so map each back by position in the sorted copy.
        let c2 = Comparison { procs: sorted, totals: c.totals.clone(), ..*c };
        self.comparison = Some(Arc::new(c2));
        let c = self.comparison.as_ref().unwrap();
        self.shown = c
            .procs
            .iter()
            .enumerate()
            .filter(|(_, d)| self.show_same || d.change != Change::Same)
            .filter(|(_, d)| filter.is_empty() || d.name.to_lowercase().contains(&filter))
            .map(|(i, _)| i)
            .collect();
        self.diff_cursor = self.diff_cursor.min(self.shown.len().saturating_sub(1));
    }

    /// The label of `id`.
    pub fn label_of(&self, id: &str) -> Option<&str> {
        self.metas.iter().find(|m| m.id == id).map(|m| m.label.as_str())
    }
}

/// Where exports go (`<monitor data>/exports`).
pub fn export_dir() -> PathBuf {
    data_dir().join("exports")
}
