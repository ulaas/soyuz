//! `soyuz-search`: streaming file search, folder sizes, duplicate finder and folder
//! compare.
//!
//! Entry points: [`start`] (streaming search), [`folder_sizes`] / [`size_tree`],
//! [`find_duplicates`] and [`compare_dirs`]. Fuzzy hits are streamed unsorted; rank them
//! with [`fuzzy_score`] / [`FuzzyScorer`].
#![forbid(unsafe_code)]

pub mod compare;
pub mod content;
pub mod dupes;
pub mod matcher;
pub mod query;
pub mod sizes;
pub mod walker;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use soyuz_core::{EntryInfo, FsError, OpControl, PatternError, SearchId};

pub use compare::{CompareResult, compare_dirs};
pub use dupes::{DupeEvent, find_duplicates};
pub use matcher::{FuzzyScorer, fuzzy_score};
pub use query::{ContentPatternSpec, SavedSearch, SearchQuery};
pub use sizes::{SizeEvent, TreeNode, folder_sizes, size_tree};

/// One content hit inside a file.
#[derive(Clone, Debug)]
pub struct ContentMatch {
    /// 1-based line number.
    pub line: u64,
    /// 0-based byte column.
    pub col: usize,
    /// The matching line (trimmed to a sane length).
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchStats {
    pub scanned: u64,
    pub hits: u64,
    pub elapsed: Duration,
    /// `max_results` reached or cancelled.
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub enum SearchEvent {
    /// Batched every 50 ms or 256 hits.
    Hits(Vec<(EntryInfo, Vec<ContentMatch>)>),
    Progress {
        scanned: u64,
        dirs: u64,
        current: PathBuf,
    },
    Error(PathBuf, FsError),
    Done(SearchStats),
}

/// A running search; dropping it cancels the search. `Send`.
#[derive(Debug)]
pub struct SearchHandle {
    pub id: SearchId,
    /// Bounded (1024).
    pub rx: crossbeam_channel::Receiver<SearchEvent>,
    ctl: Arc<OpControl>,
    done: Arc<AtomicBool>,
}

impl SearchHandle {
    /// Assemble a handle (used by the implementation and by tests).
    pub fn new(
        id: SearchId,
        rx: crossbeam_channel::Receiver<SearchEvent>,
        ctl: Arc<OpControl>,
        done: Arc<AtomicBool>,
    ) -> Self {
        SearchHandle { id, rx, ctl, done }
    }

    pub fn cancel(&self) {
        self.ctl.cancel();
    }

    /// The worker sent `Done` (or exited).
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    pub fn control(&self) -> Arc<OpControl> {
        self.ctl.clone()
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.ctl.cancel();
    }
}

static NEXT_SEARCH_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a process-unique [`SearchId`].
pub fn next_search_id() -> SearchId {
    SearchId(NEXT_SEARCH_ID.fetch_add(1, Ordering::Relaxed))
}

/// Start a search (`threads` 0 = available parallelism). Patterns are compiled up front
/// so invalid globs/regexes fail synchronously. Walks with `ignore::WalkBuilder::build_parallel`.
pub fn start(q: SearchQuery, threads: usize, wake: Arc<dyn Fn() + Send + Sync>) -> Result<SearchHandle, PatternError> {
    let compiled = matcher::Compiled::new(&q)?;
    Ok(walker::spawn_search(q, compiled, threads, wake))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_cancel_on_drop() {
        let (_tx, rx) = crossbeam_channel::bounded(1024);
        let ctl = OpControl::new();
        let h = SearchHandle::new(next_search_id(), rx, ctl.clone(), Arc::default());
        assert!(!h.is_done());
        assert!(!ctl.is_cancelled());
        drop(h);
        assert!(ctl.is_cancelled());
        assert_ne!(next_search_id(), next_search_id());
    }

    #[test]
    fn invalid_pattern_fails_fast() {
        let q = SearchQuery {
            name: Some(soyuz_core::NamePatternSpec::new(soyuz_core::PatternKind::Regex, "(")),
            ..Default::default()
        };
        assert!(start(q, 0, Arc::new(|| {})).is_err());
    }

    #[test]
    fn handle_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SearchHandle>();
    }
}
