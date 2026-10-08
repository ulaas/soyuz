//! Recursive search and folder sizes for the app (PLAN M6).
//!
//! * Searches are registered under a `SearchId` that becomes the pane's
//!   `Location::Search(id)`; `ops::start_listing` calls [`attach`], which (re)starts the
//!   query for that pane generation (refresh / history re-runs it). [`pump`] drains every
//!   running `SearchHandle` each frame and streams hits into the pane through the normal
//!   listing path, so results are sortable and actionable like any folder.
//! * Hits are named by their path relative to the search root, which keeps selection keys
//!   unique across folders and doubles as a compact location hint (the Path column shows
//!   the full folder).
//! * Fuzzy name searches are ranked app-side with `soyuz_search::FuzzyScorer`.
//! * Folder sizes (`soyuz_search::folder_sizes`) stream partial results into a size cache
//!   the details view reads, so the Size column animates while sizes are computed.
//!
//! All state is UI-thread-local (search handles are polled, never forwarded).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use soyuz_core::{Column, ColumnSpec, EntryInfo, EntryKindSet, NamePatternSpec, OpControl, PatternKind, SearchId};
use soyuz_search::{ContentPatternSpec, FuzzyScorer, SearchEvent, SearchHandle, SearchQuery, SizeEvent};

use crate::state::pane::{ListingStatus, Pane, PaneId};
use crate::state::workspace::Workspace;

/// A registered search: the query plus how to present / rank it.
#[derive(Clone, Debug)]
pub struct SearchSpec {
    pub query: SearchQuery,
    /// Short title for headers ("“*.rs” in src").
    pub title: String,
}

/// Live progress of a search.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub scanned: u64,
    pub dirs: u64,
    pub hits: u64,
    pub errors: u64,
    pub current: PathBuf,
    pub elapsed: Duration,
    pub done: bool,
    pub truncated: bool,
    pub cancelled: bool,
    /// Fuzzy results are currently ordered by relevance.
    pub ranked: bool,
}

struct Active {
    pane: PaneId,
    generation: u64,
    handle: SearchHandle,
    root: PathBuf,
    started: Instant,
    progress: Progress,
    fuzzy: Option<FuzzyScorer>,
    /// Sort spec in effect when ranking was applied; a user sort turns ranking off.
    ranked_sort: Option<soyuz_core::SortSpec>,
    rank_dirty: bool,
    last_rank: Instant,
    /// Fuzzy score per hit, computed once when the hit arrives, so re-ranking a large
    /// result set is a plain sort instead of re-scoring every name each time.
    scores: HashMap<PathBuf, u32>,
}

struct SizeJob {
    pane: PaneId,
    rx: crossbeam_channel::Receiver<SizeEvent>,
    ctl: Arc<OpControl>,
    pending: usize,
}

#[derive(Default)]
struct Registry {
    specs: HashMap<SearchId, SearchSpec>,
    active: HashMap<SearchId, Active>,
    /// Finished searches keep their final progress for the status strip.
    finished: HashMap<SearchId, Progress>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    threads: usize,
    sizes: Vec<SizeJob>,
    /// Folder size cache: path → (bytes, done).
    size_cache: HashMap<PathBuf, (u64, bool)>,
    /// Errors to surface (bad patterns, …).
    notices: Vec<String>,
    /// Registration order of `specs`, oldest first (bounded, see [`MAX_SPECS`]).
    order: std::collections::VecDeque<SearchId>,
}

/// Registered searches kept for history / refresh; older ones are forgotten (their
/// panes then show "This search is no longer available").
const MAX_SPECS: usize = 64;

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Install the repaint hook and thread count (idempotent; called every frame by the UI).
pub fn init(ctx: &egui::Context, threads: usize) {
    REG.with_borrow_mut(|r| {
        if r.wake.is_none() {
            let c = ctx.clone();
            r.wake = Some(Arc::new(move || c.request_repaint()));
        }
        r.threads = threads;
    });
}

/// Register a search; navigate a pane to `Location::Search(id)` to run it.
pub fn register(spec: SearchSpec) -> SearchId {
    let id = soyuz_search::next_search_id();
    REG.with_borrow_mut(|r| {
        r.specs.insert(id, spec);
        r.order.push_back(id);
        // Bound the registry: every search used to leak its spec and final progress.
        // Running searches are never evicted.
        while r.order.len() > MAX_SPECS {
            let Some(pos) = r.order.iter().position(|i| !r.active.contains_key(i)) else { break };
            if let Some(old) = r.order.remove(pos) {
                r.specs.remove(&old);
                r.finished.remove(&old);
            }
        }
    });
    id
}

pub fn spec(id: SearchId) -> Option<SearchSpec> {
    REG.with_borrow(|r| r.specs.get(&id).cloned())
}

/// Default columns for result panes: name (relative path), folder, size, modified.
pub fn result_columns() -> Vec<ColumnSpec> {
    let mut path = ColumnSpec::new(Column::Path);
    path.width = 260.0;
    vec![ColumnSpec::new(Column::Name), path, ColumnSpec::new(Column::Size), ColumnSpec::new(Column::Modified)]
}

/// (Re)start the search behind `Location::Search(id)` for this pane generation.
pub fn attach(pane: &mut Pane, id: SearchId) {
    let Some(spec) = spec(id) else {
        pane.status = ListingStatus::Failed(soyuz_core::FsError::with_message(
            soyuz_core::FsErrorKind::NotFound,
            "search",
            None,
            "This search is no longer available. Run it again from the search bar.",
        ));
        return;
    };
    if !pane.view.columns.iter().any(|c| c.column == Column::Path && c.visible) {
        pane.view.columns = result_columns();
    }
    pane.view.group = soyuz_core::GroupBy::None;
    pane.view.mode = soyuz_core::ViewMode::Details;
    pane.status = ListingStatus::Loading { since: Instant::now() };
    let fuzzy = spec
        .query
        .name
        .as_ref()
        .filter(|n| n.kind == PatternKind::Fuzzy)
        .map(|n| FuzzyScorer::new(&n.text, n.case_sensitive));
    REG.with_borrow_mut(|r| {
        r.active.remove(&id);
        r.finished.remove(&id);
        let wake = r.wake.clone().unwrap_or_else(|| Arc::new(|| {}));
        match soyuz_search::start(spec.query.clone(), r.threads, wake) {
            Ok(handle) => {
                let root = spec.query.roots.first().cloned().unwrap_or_default();
                r.active.insert(
                    id,
                    Active {
                        pane: pane.id,
                        generation: pane.generation,
                        handle,
                        root,
                        started: Instant::now(),
                        progress: Progress { ranked: fuzzy.is_some(), ..Default::default() },
                        fuzzy,
                        ranked_sort: None,
                        rank_dirty: false,
                        last_rank: Instant::now(),
                        scores: HashMap::new(),
                    },
                );
            }
            Err(e) => {
                pane.status = ListingStatus::Failed(soyuz_core::FsError::with_message(
                    soyuz_core::FsErrorKind::InvalidName,
                    "search",
                    None,
                    format!("Invalid pattern: {e}"),
                ));
                r.notices.push(e.to_string());
            }
        }
    });
}

/// Cancel a running search (results so far stay).
pub fn cancel(id: SearchId) {
    REG.with_borrow_mut(|r| {
        if let Some(a) = r.active.get_mut(&id) {
            a.handle.cancel();
            a.progress.cancelled = true;
        }
    });
}

pub fn progress(id: SearchId) -> Option<Progress> {
    REG.with_borrow(|r| {
        r.active
            .get(&id)
            .map(|a| {
                let mut p = a.progress.clone();
                if !p.done {
                    p.elapsed = a.started.elapsed();
                }
                p
            })
            .or_else(|| r.finished.get(&id).cloned())
    })
}

pub fn is_running(id: SearchId) -> bool {
    REG.with_borrow(|r| r.active.get(&id).is_some_and(|a| !a.progress.done))
}

/// Toggle relevance ordering for a fuzzy search.
pub fn set_ranked(id: SearchId, on: bool) {
    REG.with_borrow_mut(|r| {
        if let Some(a) = r.active.get_mut(&id) {
            a.progress.ranked = on && a.fuzzy.is_some();
            a.ranked_sort = None;
            a.rank_dirty = true;
        }
    });
}

/// Rename a hit to its path relative to the search root (unique keys, compact hint).
pub fn relative_name(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().into_owned(),
        _ => soyuz_core::entry::display_name(path),
    }
}

fn file_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Order entries by fuzzy score (best first), ties by shorter then alphabetical name.
pub fn rank(entries: &mut Vec<EntryInfo>, scorer: &mut FuzzyScorer) {
    rank_with(entries, |e| fuzzy_score(scorer, e));
}

fn fuzzy_score(scorer: &mut FuzzyScorer, e: &EntryInfo) -> u32 {
    scorer.score(file_name(&e.name)).unwrap_or(0)
}

/// [`rank`] with a caller-provided score (e.g. cached); moves entries, no clones.
fn rank_with(entries: &mut Vec<EntryInfo>, mut score: impl FnMut(&EntryInfo) -> u32) {
    let mut keyed: Vec<(u32, EntryInfo)> = std::mem::take(entries).into_iter().map(|e| (score(&e), e)).collect();
    keyed.sort_unstable_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| a.1.name.len().cmp(&b.1.name.len())).then_with(|| a.1.name.cmp(&b.1.name))
    });
    *entries = keyed.into_iter().map(|(_, e)| e).collect();
}

/// Minimum pause between re-rankings while results stream in; grows with the result
/// count so a 100k-hit search doesn't re-sort on the UI thread several times a second.
fn rank_interval(n: usize) -> Duration {
    Duration::from_millis((300 + n as u64 / 50).min(3_000))
}

/// Drain running searches and folder-size jobs into their panes. Returns notices.
pub fn pump(ws: &mut Workspace) -> Vec<String> {
    REG.with_borrow_mut(|r| {
        let mut finished = Vec::new();
        for (id, a) in r.active.iter_mut() {
            let pane = ws.find_pane_mut(a.pane).filter(|p| p.generation == a.generation);
            let Some(pane) = pane else {
                // The pane navigated away: stop the walk.
                a.handle.cancel();
                finished.push(*id);
                continue;
            };
            let mut chunk = Vec::new();
            let mut done = None;
            let deadline = Instant::now() + Duration::from_millis(6);
            while let Ok(ev) = a.handle.rx.try_recv() {
                match ev {
                    SearchEvent::Hits(hits) => {
                        for (mut e, _matches) in hits {
                            e.name = relative_name(&a.root, &e.path);
                            if let Some(scorer) = a.fuzzy.as_mut() {
                                a.scores.insert(e.path.clone(), fuzzy_score(scorer, &e));
                            }
                            chunk.push(e);
                        }
                    }
                    SearchEvent::Progress { scanned, dirs, current } => {
                        a.progress.scanned = scanned;
                        a.progress.dirs = dirs;
                        a.progress.current = current;
                    }
                    SearchEvent::Error(..) => a.progress.errors += 1,
                    SearchEvent::Done(stats) => {
                        done = Some(stats);
                        break;
                    }
                }
                if Instant::now() > deadline {
                    break;
                }
            }
            a.progress.hits += chunk.len() as u64;
            if !chunk.is_empty() {
                a.rank_dirty = true;
            }
            let finished_now = done.is_some() || (a.handle.is_done() && a.handle.rx.is_empty());
            if let Some(stats) = done {
                a.progress.scanned = a.progress.scanned.max(stats.scanned);
                a.progress.truncated = stats.truncated;
                a.progress.elapsed = stats.elapsed;
            }
            if !chunk.is_empty() || finished_now {
                pane.on_listing(chunk, finished_now, None);
            }
            // Fuzzy relevance ordering (throttled while streaming).
            if a.progress.ranked
                && let Some(scorer) = a.fuzzy.as_mut()
            {
                if a.ranked_sort.as_ref().is_some_and(|s| *s != pane.view.sort) {
                    a.progress.ranked = false; // the user sorted by a column
                } else if a.rank_dirty && (finished_now || a.last_rank.elapsed() > rank_interval(pane.entries.len())) {
                    pane.flush_incoming();
                    let mut entries = std::mem::take(&mut pane.entries);
                    let scores = &a.scores;
                    rank_with(&mut entries, |e| scores.get(&e.path).copied().unwrap_or_else(|| fuzzy_score(scorer, e)));

                    pane.set_entries_ordered(entries);
                    a.ranked_sort = Some(pane.view.sort.clone());
                    a.rank_dirty = false;
                    a.last_rank = Instant::now();
                }
            }
            if finished_now {
                a.progress.done = true;
                if a.progress.elapsed.is_zero() {
                    a.progress.elapsed = a.started.elapsed();
                }
                finished.push(*id);
            }
        }
        for id in finished {
            if let Some(a) = r.active.remove(&id) {
                r.finished.insert(id, a.progress);
            }
        }
        pump_sizes(r, ws);
        std::mem::take(&mut r.notices)
    })
}

fn pump_sizes(r: &mut Registry, ws: &mut Workspace) {
    let mut keep = Vec::new();
    for mut job in std::mem::take(&mut r.sizes) {
        let mut disconnected = false;
        let mut updates: Vec<(PathBuf, u64, bool)> = Vec::new();
        loop {
            match job.rx.try_recv() {
                Ok(SizeEvent::Partial { path, bytes, .. }) => updates.push((path, bytes, false)),
                Ok(SizeEvent::Done { path, bytes, .. }) => {
                    updates.push((path, bytes, true));
                    job.pending = job.pending.saturating_sub(1);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if !updates.is_empty() {
            let pane = ws.find_pane_mut(job.pane);
            let mut resort = false;
            if let Some(pane) = pane {
                for (path, bytes, done) in &updates {
                    if let Some(e) = pane.entries.iter_mut().find(|e| &e.path == path) {
                        e.size = *bytes;
                        resort |= *done;
                    }
                }
                if resort && pane.view.sort.column == Column::Size {
                    pane.resort();
                }
                pane.dirty = true;
            }
            for (path, bytes, done) in updates {
                r.size_cache.insert(path, (bytes, done));
            }
        }
        if !disconnected && job.pending > 0 {
            keep.push(job);
        } else {
            job.ctl.cancel();
        }
    }
    r.sizes = keep;
    if r.size_cache.len() > 50_000 {
        r.size_cache.clear();
    }
}

/// Compute recursive sizes of `dirs` (shown in the Size column as they arrive).
pub fn start_folder_sizes(pane: PaneId, dirs: Vec<PathBuf>) -> usize {
    if dirs.is_empty() {
        return 0;
    }
    REG.with_borrow_mut(|r| {
        for d in &dirs {
            r.size_cache.insert(d.clone(), (0, false));
        }
        let wake = r.wake.clone().unwrap_or_else(|| Arc::new(|| {}));
        let n = dirs.len();
        let (rx, ctl) = soyuz_search::folder_sizes(dirs, wake);
        r.sizes.push(SizeJob { pane, rx, ctl, pending: n });
        n
    })
}

/// Whether folder sizes are being computed (for a status hint).
pub fn sizing() -> bool {
    REG.with_borrow(|r| !r.sizes.is_empty())
}

/// Cached recursive size of a folder: `(bytes, done)`.
pub fn cached_size(path: &Path) -> Option<(u64, bool)> {
    REG.with_borrow(|r| r.size_cache.get(path).copied())
}

/// Size column text for a folder (empty when unknown).
pub fn dir_size_text(path: &Path, style: soyuz_core::SizeStyle) -> String {
    match cached_size(path) {
        Some((b, true)) => soyuz_core::format_size(b, style),
        Some((b, false)) => format!("{}…", soyuz_core::format_size(b, style)),
        None => String::new(),
    }
}

// ---- query building -----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NameMode {
    #[default]
    Substring,
    Glob,
    Regex,
    Fuzzy,
}

impl NameMode {
    pub const ALL: [NameMode; 4] = [NameMode::Substring, NameMode::Glob, NameMode::Regex, NameMode::Fuzzy];
    pub fn label(self) -> &'static str {
        match self {
            NameMode::Substring => "Contains",
            NameMode::Glob => "Glob",
            NameMode::Regex => "Regex",
            NameMode::Fuzzy => "Fuzzy",
        }
    }
    fn kind(self) -> PatternKind {
        match self {
            NameMode::Substring => PatternKind::Substring,
            NameMode::Glob => PatternKind::Glob,
            NameMode::Regex => PatternKind::Regex,
            NameMode::Fuzzy => PatternKind::Fuzzy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DateRange {
    #[default]
    Any,
    Today,
    Week,
    Month,
    Year,
    OlderThanYear,
}

impl DateRange {
    pub const ALL: [DateRange; 6] = [
        DateRange::Any,
        DateRange::Today,
        DateRange::Week,
        DateRange::Month,
        DateRange::Year,
        DateRange::OlderThanYear,
    ];
    pub fn label(self) -> &'static str {
        match self {
            DateRange::Any => "Any time",
            DateRange::Today => "Last 24 hours",
            DateRange::Week => "Last 7 days",
            DateRange::Month => "Last 30 days",
            DateRange::Year => "Last 12 months",
            DateRange::OlderThanYear => "Older than a year",
        }
    }
    /// `(after, before)` bounds relative to `now`.
    pub fn bounds(self, now: SystemTime) -> Option<(Option<SystemTime>, Option<SystemTime>)> {
        let day = Duration::from_secs(86_400);
        let ago = |d: Duration| now.checked_sub(d);
        match self {
            DateRange::Any => None,
            DateRange::Today => Some((ago(day), None)),
            DateRange::Week => Some((ago(day * 7), None)),
            DateRange::Month => Some((ago(day * 30), None)),
            DateRange::Year => Some((ago(day * 365), None)),
            DateRange::OlderThanYear => Some((None, ago(day * 365))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KindFilter {
    #[default]
    Any,
    Files,
    Folders,
    Images,
    Videos,
    Audio,
    Documents,
    Archives,
    Code,
}

impl KindFilter {
    pub const ALL: [KindFilter; 9] = [
        KindFilter::Any,
        KindFilter::Files,
        KindFilter::Folders,
        KindFilter::Images,
        KindFilter::Videos,
        KindFilter::Audio,
        KindFilter::Documents,
        KindFilter::Archives,
        KindFilter::Code,
    ];
    pub fn label(self) -> &'static str {
        match self {
            KindFilter::Any => "Any kind",
            KindFilter::Files => "Files",
            KindFilter::Folders => "Folders",
            KindFilter::Images => "Images",
            KindFilter::Videos => "Videos",
            KindFilter::Audio => "Audio",
            KindFilter::Documents => "Documents",
            KindFilter::Archives => "Archives",
            KindFilter::Code => "Code",
        }
    }
    fn extensions(self) -> &'static [&'static str] {
        match self {
            KindFilter::Images => {
                &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "tif", "tiff", "heic", "raw"]
            }
            KindFilter::Videos => &["mp4", "mkv", "mov", "avi", "webm", "wmv", "m4v", "mpg", "mpeg"],
            KindFilter::Audio => &["mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma"],
            KindFilter::Documents => {
                &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "txt", "md", "rtf", "epub"]
            }
            KindFilter::Archives => &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst"],
            KindFilter::Code => &[
                "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "kt", "c", "h", "cpp", "hpp", "cs", "rb", "php",
                "swift", "sh", "ps1", "toml", "json", "yaml", "yml", "html", "css",
            ],
            _ => &[],
        }
    }
}

/// The search bar's option pills, turned into a [`SearchQuery`] by [`SearchForm::build`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SearchForm {
    pub text: String,
    pub mode: NameMode,
    pub case_sensitive: bool,
    /// The main text searches file contents (the name field becomes an optional glob).
    pub content: bool,
    pub content_regex: bool,
    pub name_glob: String,
    pub min_size: String,
    pub max_size: String,
    pub date: DateRange,
    pub kind: KindFilter,
    pub include_hidden: bool,
    pub respect_gitignore: bool,
    pub root: PathBuf,
}

/// Parse `10`, `512k`, `1.5 MB`, `2GiB` (binary multiples) into bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let t = s.trim().to_ascii_lowercase().replace(' ', "");
    if t.is_empty() {
        return None;
    }
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult: f64 = match unit.trim_end_matches("ib").trim_end_matches('b') {
        "" => 1.0,
        "k" => 1024.0,
        "m" => 1024f64.powi(2),
        "g" => 1024f64.powi(3),
        "t" => 1024f64.powi(4),
        _ => return None,
    };
    (n >= 0.0).then(|| (n * mult).round() as u64)
}

impl SearchForm {
    /// Whether the form would search for anything.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
            && self.name_glob.trim().is_empty()
            && self.min_size.trim().is_empty()
            && self.max_size.trim().is_empty()
            && self.date == DateRange::Any
            && self.kind == KindFilter::Any
    }

    pub fn build(&self, now: SystemTime, content_max: u64) -> Result<SearchQuery, String> {
        if self.root.as_os_str().is_empty() {
            return Err("Choose a folder to search in".into());
        }
        let mut q = SearchQuery { roots: vec![self.root.clone()], ..SearchQuery::default() };
        let text = self.text.trim();
        if self.content {
            if !text.is_empty() {
                q.content = Some(ContentPatternSpec {
                    regex: self.content_regex,
                    text: text.to_string(),
                    case_sensitive: self.case_sensitive,
                });
            }
            let g = self.name_glob.trim();
            if !g.is_empty() {
                let kind = if g.contains(['*', '?', '[']) { PatternKind::Glob } else { PatternKind::Substring };
                q.name = Some(NamePatternSpec { kind, text: g.to_string(), case_sensitive: false });
            }
        } else if !text.is_empty() {
            // A plain "Contains" text with wildcards is clearly meant as a glob.
            let mode =
                if self.mode == NameMode::Substring && text.contains(['*', '?']) { NameMode::Glob } else { self.mode };
            q.name = Some(NamePatternSpec {
                kind: mode.kind(),
                text: text.to_string(),
                case_sensitive: self.case_sensitive,
            });
        }
        let min = match self.min_size.trim() {
            "" => None,
            s => Some(parse_size(s).ok_or_else(|| format!("Can't read the size “{s}”"))?),
        };
        let max = match self.max_size.trim() {
            "" => None,
            s => Some(parse_size(s).ok_or_else(|| format!("Can't read the size “{s}”"))?),
        };
        if let (Some(a), Some(b)) = (min, max)
            && a > b
        {
            return Err("The minimum size is larger than the maximum".into());
        }
        if min.is_some() || max.is_some() {
            q.size = Some((min, max));
        }
        q.modified = self.date.bounds(now);
        match self.kind {
            KindFilter::Any => {}
            KindFilter::Files => q.kinds = Some(EntryKindSet::FILE),
            KindFilter::Folders => q.kinds = Some(EntryKindSet::DIR),
            k => {
                q.kinds = Some(EntryKindSet::FILE);
                q.extensions = k.extensions().iter().map(|s| s.to_string()).collect();
            }
        }
        if q.content.is_some() || q.size.is_some() {
            // Size and content filters only apply to files.
            q.kinds.get_or_insert(EntryKindSet::FILE);
        }
        q.include_hidden = self.include_hidden;
        q.respect_gitignore = self.respect_gitignore;
        q.content_max_size = content_max;
        if q.name.is_none() && q.content.is_none() && q.size.is_none() && q.modified.is_none() && q.kinds.is_none() {
            return Err("Type something to search for".into());
        }
        Ok(q)
    }

    /// Header title: `“text” in folder`.
    pub fn title(&self) -> String {
        let what =
            if self.text.trim().is_empty() { "Everything".to_string() } else { format!("“{}”", self.text.trim()) };
        let r = soyuz_fs::strip_verbatim(&self.root);
        let folder = soyuz_core::entry::display_name(&r);
        format!("{what} in {folder}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(text: &str) -> SearchForm {
        SearchForm { text: text.into(), root: PathBuf::from("/data"), ..Default::default() }
    }

    #[test]
    fn sizes_parse() {
        assert_eq!(parse_size("10"), Some(10));
        assert_eq!(parse_size("1k"), Some(1024));
        assert_eq!(parse_size("1.5 MB"), Some(1_572_864));
        assert_eq!(parse_size("2GiB"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("ten"), None);
        assert_eq!(parse_size("5 parsecs"), None);
    }

    #[test]
    fn name_modes_build_patterns() {
        let q = form("report").build(SystemTime::now(), 1).unwrap();
        assert_eq!(q.name.as_ref().unwrap().kind, PatternKind::Substring);
        assert_eq!(q.roots, vec![PathBuf::from("/data")]);
        let q = form("*.rs").build(SystemTime::now(), 1).unwrap();
        assert_eq!(q.name.unwrap().kind, PatternKind::Glob, "wildcards promote Contains to Glob");
        let mut f = form("^a.*z$");
        f.mode = NameMode::Regex;
        f.case_sensitive = true;
        let n = f.build(SystemTime::now(), 1).unwrap().name.unwrap();
        assert_eq!((n.kind, n.case_sensitive), (PatternKind::Regex, true));
        let mut f = form("srvmn");
        f.mode = NameMode::Fuzzy;
        assert_eq!(f.build(SystemTime::now(), 1).unwrap().name.unwrap().kind, PatternKind::Fuzzy);
    }

    #[test]
    fn content_size_date_kind_pills() {
        let mut f = form("TODO");
        f.content = true;
        f.name_glob = "*.rs".into();
        f.min_size = "1k".into();
        f.max_size = "1M".into();
        f.date = DateRange::Week;
        f.include_hidden = true;
        f.respect_gitignore = true;
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 86_400);
        let q = f.build(now, 4096).unwrap();
        assert_eq!(q.content.as_ref().unwrap().text, "TODO");
        assert_eq!(q.name.as_ref().unwrap().kind, PatternKind::Glob);
        assert_eq!(q.size, Some((Some(1024), Some(1024 * 1024))));
        assert_eq!(q.modified, Some((Some(SystemTime::UNIX_EPOCH + Duration::from_secs(3 * 86_400)), None)));
        assert_eq!(q.kinds, Some(EntryKindSet::FILE));
        assert!(q.include_hidden && q.respect_gitignore);
        assert_eq!(q.content_max_size, 4096);

        let mut f = form("");
        f.kind = KindFilter::Images;
        let q = f.build(SystemTime::now(), 1).unwrap();
        assert!(q.extensions.contains(&"png".to_string()));
        assert!(q.name.is_none());

        let mut f = form("");
        f.kind = KindFilter::Folders;
        assert_eq!(f.build(SystemTime::now(), 1).unwrap().kinds, Some(EntryKindSet::DIR));
    }

    #[test]
    fn invalid_forms() {
        assert!(form("").build(SystemTime::now(), 1).is_err(), "nothing to search for");
        let mut f = form("x");
        f.root = PathBuf::new();
        assert!(f.build(SystemTime::now(), 1).is_err());
        let mut f = form("x");
        f.min_size = "2M".into();
        f.max_size = "1M".into();
        assert!(f.build(SystemTime::now(), 1).is_err());
        let mut f = form("x");
        f.min_size = "lots".into();
        assert!(f.build(SystemTime::now(), 1).is_err());
    }

    #[test]
    fn relative_names_and_ranking() {
        let root = Path::new("/data");
        assert_eq!(relative_name(root, Path::new("/data/a/b.txt")), Path::new("a/b.txt").to_string_lossy());
        assert_eq!(relative_name(root, Path::new("/other/c.txt")), "c.txt");
        let mk = |n: &str| {
            let mut e = EntryInfo::minimal(PathBuf::from("/data").join(n), soyuz_core::EntryKind::File);
            e.name = n.to_string();
            e
        };
        let mut v = vec![mk("x/some_random_main.rs"), mk("main.rs"), mk("y/maintenance.txt"), mk("z/none.md")];
        let mut s = FuzzyScorer::new("main", false);
        rank(&mut v, &mut s);
        assert_eq!(v[0].name, "main.rs");
        assert_eq!(v.last().unwrap().name, "z/none.md", "non-matches sink");
    }

    #[test]
    fn registered_specs_are_bounded() {
        let mk = || SearchSpec { query: SearchQuery::default(), title: String::new() };
        let first = register(mk());
        let ids: Vec<SearchId> = (0..MAX_SPECS).map(|_| register(mk())).collect();
        assert!(spec(first).is_none(), "the oldest spec is evicted");
        assert!(ids.iter().all(|id| spec(*id).is_some()));
        assert!(REG.with_borrow(|r| r.specs.len() <= MAX_SPECS));
    }

    #[test]
    fn date_bounds() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(400 * 86_400);
        assert_eq!(DateRange::Any.bounds(now), None);
        let (a, b) = DateRange::OlderThanYear.bounds(now).unwrap();
        assert!(a.is_none());
        assert_eq!(b, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(35 * 86_400)));
    }

    #[test]
    fn streams_hits_into_a_pane() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/needle.txt"), "x").unwrap();
        std::fs::write(dir.path().join("a/needle.md"), "x").unwrap();
        std::fs::write(dir.path().join("hay.txt"), "x").unwrap();
        let ctx = egui::Context::default();
        init(&ctx, 2);
        let f = SearchForm { text: "needle".into(), root: dir.path().to_path_buf(), ..Default::default() };
        let id = register(SearchSpec { query: f.build(SystemTime::now(), 1 << 20).unwrap(), title: f.title() });
        let pane = Pane::new(soyuz_core::Location::Search(id), crate::state::view_memory::ViewState::default());
        let mut ws = Workspace::new(crate::state::workspace::Tab::new(pane));
        {
            let p = &mut ws.tabs[0].panes[0];
            p.generation = 1;
            attach(p, id);
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            pump(&mut ws);
            if progress(id).is_some_and(|p| p.done) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let p = &mut ws.tabs[0].panes[0];
        p.refresh_visible(true, true);
        let mut names: Vec<String> = p.entries.iter().map(|e| e.name.replace('\\', "/")).collect();
        names.sort();
        assert_eq!(names, vec!["a/b/needle.txt".to_string(), "a/needle.md".to_string()]);
        assert!(matches!(p.status, ListingStatus::Ready));
        assert_eq!(progress(id).unwrap().hits, 2);
        assert!(p.view.columns.iter().any(|c| c.column == Column::Path));
    }
}
