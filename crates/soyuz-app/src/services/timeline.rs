//! The `Timeline` location (F7): files changed today / this week across the folders
//! Soyuz knows about. Candidates come from pluggable [`TimelineSource`]s:
//! * the change journal — file changes reported by the folder watcher while Soyuz runs;
//! * the recent store — opened files, plus a one-level scan of recently visited folders
//!   and the folders open in panes;
//! * any registered extra source, e.g. a search index (`register_source`), asked only
//!   when `[workspace] timeline_use_index` is on. Without one the timeline still works.
//!
//! Everything slow (folder scans, stats, index queries) runs on a worker thread; the
//! result arrives as a normal listing (`UiMsg::Listing`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use soyuz_core::{Column, ColumnSpec, EntryInfo, GroupBy, SortDir};

use super::mailbox::{Postman, UiMsg};
use crate::state::pane::{ListingStatus, Pane};
use crate::state::workspace_timeline as tl;

/// Journal entries kept in memory.
const JOURNAL_MAX: usize = 5000;

/// A provider of `(path, changed at)` candidates. Called on a worker thread.
pub trait TimelineSource: Send + Sync {
    fn name(&self) -> &str;
    /// Files changed at or after `since` (unix seconds), at most `max`.
    fn changed_since(&self, since: i64, max: usize) -> Vec<(PathBuf, i64)>;
}

fn sources() -> &'static Mutex<Vec<Arc<dyn TimelineSource>>> {
    static S: OnceLock<Mutex<Vec<Arc<dyn TimelineSource>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}

/// Plug in an extra source (the search index, for instance). Replaces a source with the
/// same name.
pub fn register_source(src: Arc<dyn TimelineSource>) {
    if let Ok(mut v) = sources().lock() {
        v.retain(|s| s.name() != src.name());
        v.push(src);
    }
}

thread_local! {
    static JOURNAL: RefCell<VecDeque<(PathBuf, i64)>> = const { RefCell::new(VecDeque::new()) };
}

pub fn unix_now() -> i64 {
    super::recent::unix_now() as i64
}

/// The folder watcher saw these paths change (called from `ops::drain_watcher`).
pub fn note_changes(paths: &[PathBuf]) {
    let now = unix_now();
    JOURNAL.with_borrow_mut(|j| {
        for p in paths {
            j.push_back((p.clone(), now));
        }
        while j.len() > JOURNAL_MAX {
            j.pop_front();
        }
    });
}

fn journal_snapshot() -> Vec<(PathBuf, i64)> {
    JOURNAL.with_borrow(|j| j.iter().cloned().collect())
}

/// Local UTC offset in seconds.
pub fn local_offset() -> i64 {
    i64::from(chrono::Local::now().offset().local_minus_utc())
}

/// What the timeline listing needs from the config.
#[derive(Clone, Debug)]
pub struct TimelineOpts {
    pub days: u32,
    pub scan_folders: usize,
    pub max_items: usize,
    pub use_index: bool,
}

impl TimelineOpts {
    pub fn from_config(c: &soyuz_core::config::Config) -> Self {
        let w = &c.workspace;
        TimelineOpts {
            days: w.timeline_days.clamp(1, 90),
            scan_folders: w.timeline_scan_folders.min(500),
            max_items: w.timeline_max_items.clamp(10, 20_000),
            use_index: w.timeline_use_index,
        }
    }
}

thread_local! {
    static OPTS: RefCell<Option<TimelineOpts>> = const { RefCell::new(None) };
}

/// Apply `[workspace] timeline_*` (startup and every config change).
pub fn set_options(opts: TimelineOpts) {
    OPTS.with_borrow_mut(|o| *o = Some(opts));
}

fn options() -> TimelineOpts {
    OPTS.with_borrow(|o| o.clone()).unwrap_or_else(|| TimelineOpts::from_config(&soyuz_core::config::Config::default()))
}

/// Start listing `Location::Timeline` for this pane generation.
pub fn attach(pane: &mut Pane, postman: Postman) {
    let opts = options();
    if !pane.view.columns.iter().any(|c| c.column == Column::Path && c.visible) {
        let mut path = ColumnSpec::new(Column::Path);
        path.width = 300.0;
        let mut modified = ColumnSpec::new(Column::Modified);
        modified.width = 150.0;
        pane.view.columns = vec![ColumnSpec::new(Column::Name), path, modified, ColumnSpec::new(Column::Size)];
        pane.view.sort.column = Column::Modified;
        pane.view.sort.dir = SortDir::Desc;
        pane.view.sort.folders_first = false;
        pane.view.group = GroupBy::Date;
    }
    pane.status = ListingStatus::Loading { since: Instant::now() };
    let now = unix_now();
    let since = tl::range_start(now, local_offset(), opts.days);
    let journal = journal_snapshot();
    let folders: Vec<PathBuf> = super::recent::folders(opts.scan_folders);

    let opened = super::recent::files_since(since as u64);
    let extra: Vec<Arc<dyn TimelineSource>> =
        if opts.use_index { sources().lock().map(|v| v.clone()).unwrap_or_default() } else { Vec::new() };
    let (id, generation) = (pane.id, pane.generation);
    let spawned = std::thread::Builder::new().name("soyuz-timeline".into()).spawn(move || {
        let entries = collect(since, opts.max_items, &folders, journal, opened, &extra);
        postman.send(UiMsg::Listing { pane: id, generation, chunk: entries, done: true, err: None });
    });
    if spawned.is_err() {
        log::warn!("cannot spawn the timeline thread");
    }
}

fn mtime(e: &EntryInfo) -> Option<i64> {
    e.modified.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
}

/// Worker: scan, merge, stat, de-duplicate names.
fn collect(
    since: i64,
    max: usize,
    folders: &[PathBuf],
    journal: Vec<(PathBuf, i64)>,
    opened: Vec<(PathBuf, i64)>,
    extra: &[Arc<dyn TimelineSource>],
) -> Vec<EntryInfo> {
    let started = Instant::now();
    let mut found: HashMap<PathBuf, EntryInfo> = HashMap::new();
    let opts = soyuz_fs::ListOptions { include_hidden: false, stat_links: false, ..Default::default() };
    let mut scanned = Vec::new();
    for dir in folders {
        if started.elapsed() > Duration::from_secs(10) {
            break; // slow shares: show what we have
        }
        let Ok(list) = soyuz_fs::read_dir(dir, &opts) else { continue };
        for e in list {
            if e.is_dir_like() {
                continue;
            }
            if let Some(t) = mtime(&e).filter(|t| *t >= since) {
                scanned.push((e.path.clone(), t));
                found.insert(e.path.clone(), e);
            }
        }
    }
    let mut lists = vec![scanned, journal, opened];
    for s in extra {
        lists.push(s.changed_since(since, max));
    }
    let merged = tl::merge(lists, since, max.saturating_mul(2));
    let mut out = Vec::with_capacity(merged.len().min(max));
    for (path, _) in merged {
        let e = match found.remove(&path) {
            Some(e) => e,
            None => match soyuz_fs::stat(&path, false) {
                Ok(e) if !e.is_dir_like() => e,
                _ => continue, // deleted since, or a folder
            },
        };
        // The journal / index time is when we heard of it; the listing shows the file's
        // own modification time, and only changes inside the range count.
        if mtime(&e).is_some_and(|t| t >= since) {
            out.push(e);
        }
        if out.len() >= max {
            break;
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    for e in &mut out {
        if !seen.insert(e.name.to_lowercase()) {
            let parent = e.path.parent().map(soyuz_core::entry::display_name).unwrap_or_default();
            e.name = format!("{} ({parent})", e.name);
            if !seen.insert(e.name.to_lowercase()) {
                e.name = soyuz_fs::strip_verbatim(&e.path).to_string_lossy().into_owned();
                seen.insert(e.name.to_lowercase());
            }
        }
    }
    out
}

/// Summary for the timeline header: `"12 today · 30 this week"`.
pub fn summary(entries: &[EntryInfo]) -> String {
    let now = unix_now();
    let items: Vec<(PathBuf, i64)> = entries.iter().filter_map(|e| Some((e.path.clone(), mtime(e)?))).collect();
    let c = tl::counts(&items, now, local_offset());
    let week = c[0].1 + c[1].1 + c[2].1;
    format!("{} today · {} this week · {} in total", c[0].1, week, entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<(PathBuf, i64)>);

    impl TimelineSource for Fake {
        fn name(&self) -> &str {
            "fake-index"
        }
        fn changed_since(&self, since: i64, _max: usize) -> Vec<(PathBuf, i64)> {
            self.0.iter().filter(|(_, t)| *t >= since).cloned().collect()
        }
    }

    #[test]
    fn collects_from_scans_journal_and_sources() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let b = sub.join("a.txt");
        let c = sub.join("c.txt");
        for p in [&a, &b, &c] {
            std::fs::write(p, b"x").unwrap();
        }
        let since = unix_now() - 3600;
        let journal = vec![(b.clone(), unix_now())];
        let idx: Arc<dyn TimelineSource> = Arc::new(Fake(vec![(c.clone(), unix_now()), (sub.clone(), unix_now())]));
        let out = collect(since, 100, &[dir.path().to_path_buf()], journal, Vec::new(), &[idx]);
        let mut names: Vec<String> = out.iter().map(|e| e.name.clone()).collect();
        names.sort();
        assert_eq!(names.len(), 3, "{names:?}: folders are skipped, files de-duplicated");
        assert!(names.contains(&"c.txt".to_string()));
        assert!(names.iter().any(|n| n.starts_with("a.txt (")), "{names:?}: same names get their folder");
        assert!(summary(&out).contains("3 today"));
        // Without sources and with a future cutoff nothing qualifies.
        assert!(collect(unix_now() + 3600, 100, &[dir.path().to_path_buf()], Vec::new(), Vec::new(), &[]).is_empty());
    }

    #[test]
    fn journal_is_capped() {
        let many: Vec<PathBuf> = (0..JOURNAL_MAX + 10).map(|i| PathBuf::from(format!("/j{i}"))).collect();
        note_changes(&many);
        assert_eq!(journal_snapshot().len(), JOURNAL_MAX);
    }
}
