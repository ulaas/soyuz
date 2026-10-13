//! The duplicate finder's model (Phase T7, agent C), moved from the GUI's
//! `ui/tools/dupes.rs`: roots and a minimum size, then `soyuz_search::find_duplicates`
//! streams groups of identical files (size → head hash → full BLAKE3) while it runs. Each
//! group offers keep-one helpers (newest, oldest, shortest path); marked files go to the
//! trash through the jobs engine as one undoable job. A group can never have all of its
//! copies marked. Files leave the list only once the trash job has finished and they are
//! really gone (failed / skipped items stay, still marked).
//!
//! Both frontends draw [`DupesModel`] (the GUI's `DupesWindow`, the terminal UI's
//! full-screen duplicate finder). Adding a root does no disk check (the scan reports
//! errors). Dropping the model stops the scan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::Receiver;
use soyuz_core::{EntryInfo, OpControl, SizeStyle, format_size};
use soyuz_jobs::{DeleteMode, JobId, JobKind};
use soyuz_search::DupeEvent;

use crate::core::AppCore;
use crate::host::Wake;
use crate::models::toasts::Toast;
use crate::ops::fileops::names::show_path;
use crate::services::jobs::{JobMeta, JobsService, OpKind};

/// How long a refused mark shows "Keep at least one copy".
pub const REFUSED_FLASH: Duration = Duration::from_millis(1200);

/// Which copy a helper keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    Newest,
    Oldest,
    ShortestPath,
}

impl Keep {
    pub const ALL: [Keep; 3] = [Keep::Newest, Keep::Oldest, Keep::ShortestPath];

    pub fn label(self) -> &'static str {
        match self {
            Keep::Newest => tr!("tools-dupes-keep-newest"),
            Keep::Oldest => tr!("tools-dupes-keep-oldest"),
            Keep::ShortestPath => tr!("tools-dupes-keep-shortest"),
        }
    }

    /// Short chip text in a group header.
    pub fn short_label(self) -> &'static str {
        match self {
            Keep::Newest => tr!("tools-dupes-newest"),
            Keep::Oldest => tr!("tools-dupes-oldest"),
            Keep::ShortestPath => tr!("tools-dupes-shortest"),
        }
    }
}

/// Index of the file to keep in a group.
pub fn keeper(files: &[EntryInfo], keep: Keep) -> usize {
    let t = |e: &EntryInfo| e.modified.unwrap_or(SystemTime::UNIX_EPOCH);
    let len = |e: &EntryInfo| e.path.as_os_str().len();
    let idx = 0..files.len();
    match keep {
        Keep::Newest => idx.max_by(|&a, &b| t(&files[a]).cmp(&t(&files[b])).then(b.cmp(&a))),
        Keep::Oldest => idx.min_by(|&a, &b| t(&files[a]).cmp(&t(&files[b])).then(a.cmp(&b))),
        Keep::ShortestPath => idx.min_by(|&a, &b| len(&files[a]).cmp(&len(&files[b])).then(a.cmp(&b))),
    }
    .unwrap_or(0)
}

/// A group of identical files.
#[derive(Clone, Debug)]
pub struct Group {
    pub files: Vec<EntryInfo>,
    pub marked: Vec<bool>,
    pub collapsed: bool,
}

impl Group {
    pub fn size(&self) -> u64 {
        self.files.first().map_or(0, |e| e.size)
    }

    pub fn wasted(&self) -> u64 {
        self.size() * self.files.len().saturating_sub(1) as u64
    }

    /// Mark everything except the keeper.
    pub fn apply(&mut self, keep: Keep) {
        let k = keeper(&self.files, keep);
        for (i, m) in self.marked.iter_mut().enumerate() {
            *m = i != k;
        }
    }

    /// Toggle a mark, refusing to mark the last unmarked copy. Returns false when refused.
    pub fn toggle(&mut self, i: usize) -> bool {
        if !self.marked[i] && self.marked.iter().filter(|m| !**m).count() <= 1 {
            return false;
        }
        self.marked[i] = !self.marked[i];
        true
    }
}

/// Minimum sizes; an empty label is "Any" ([`size_label`]).
pub const SIZES: [(&str, u64); 5] =
    [("", 1), ("≥ 100 KB", 100_000), ("≥ 1 MB", 1_000_000), ("≥ 10 MB", 10_000_000), ("≥ 100 MB", 100_000_000)];

/// The minimum size chosen by default (≥ 100 KB).
pub const DEFAULT_MIN: usize = 1;

/// The label of minimum size `i`.
pub fn size_label(i: usize) -> &'static str {
    match SIZES.get(i) {
        Some(("", _)) | None => tr!("tools-dupes-size-any"),
        Some((label, _)) => label,
    }
}

/// A running search.
struct Running {
    rx: Receiver<DupeEvent>,
    ctl: Arc<OpControl>,
    started: Instant,
    hashed: u64,
    total: u64,
    bytes: u64,
}

/// What the list area says while it has no groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Empty {
    /// Searching, nothing confirmed yet.
    Running,
    /// Every duplicate was moved to the trash.
    Handled,
    /// The search found nothing.
    None,
    /// Nothing searched yet.
    Start,
}

impl Empty {
    pub fn title(self) -> &'static str {
        match self {
            Empty::Running => tr!("tools-dupes-empty-running"),
            Empty::Handled => tr!("tools-dupes-empty-handled"),
            Empty::None => tr!("tools-dupes-empty-none"),
            Empty::Start => tr!("tools-dupes-empty-start"),
        }
    }

    pub fn body(self) -> &'static str {
        match self {
            Empty::Running => tr!("tools-dupes-empty-running-body"),
            Empty::Handled => tr!("tools-dupes-empty-handled-body"),
            Empty::None => tr!("tools-dupes-empty-none-body"),
            Empty::Start => tr!("tools-dupes-empty-start-body"),
        }
    }
}

/// The duplicate finder.
pub struct DupesModel {
    pub roots: Vec<PathBuf>,
    /// Index into [`SIZES`].
    pub min_idx: usize,
    running: Option<Running>,
    pub groups: Vec<Group>,
    /// The last search ended: groups, wasted bytes, seconds (0 when stopped by hand).
    pub done: Option<(u64, u64, f32)>,
    /// The trash confirmation shows.
    pub confirm: bool,
    /// A refused mark (group, file, when) for the "keep one" flash.
    pub refused: Option<(usize, usize, Instant)>,
    /// Files sent to the trash from this finder.
    pub removed: usize,
    /// Trash jobs in flight and the paths each was asked to remove.
    trashing: Vec<(JobId, Vec<PathBuf>)>,
    /// Existence checks of finished trash jobs, running on worker threads.
    gone_rx: Vec<Receiver<HashSet<PathBuf>>>,
    wake: Wake,
}

impl std::fmt::Debug for DupesModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DupesModel")
            .field("roots", &self.roots)
            .field("min_idx", &self.min_idx)
            .field("groups", &self.groups.len())
            .field("running", &self.running.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for DupesModel {
    fn drop(&mut self) {
        self.stop();
    }
}

impl DupesModel {
    /// The finder on `roots`; the search starts at once when there are roots (`wake`
    /// redraws the frontend as results come in).
    pub fn new(roots: Vec<PathBuf>, wake: Wake) -> Self {
        let mut m = DupesModel {
            roots,
            min_idx: DEFAULT_MIN,
            running: None,
            groups: Vec::new(),
            done: None,
            confirm: false,
            refused: None,
            removed: 0,
            trashing: Vec::new(),
            gone_rx: Vec::new(),
            wake,
        };
        if !m.roots.is_empty() {
            m.start();
        }
        m
    }

    /// Search (again) with the current roots and minimum size.
    pub fn start(&mut self) {
        self.stop();
        self.removed = 0;
        self.groups.clear();
        self.done = None;
        self.confirm = false;
        self.refused = None;
        if self.roots.is_empty() {
            return;
        }
        let min = SIZES[self.min_idx.min(SIZES.len() - 1)].1;
        let (rx, ctl) = soyuz_search::find_duplicates(self.roots.clone(), min, self.wake.clone());
        self.running = Some(Running { rx, ctl, started: Instant::now(), hashed: 0, total: 0, bytes: 0 });
    }

    /// Cancel the search (no summary).
    pub fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            r.ctl.cancel();
        }
    }

    /// The Stop button: cancel and keep what was found, sorted.
    pub fn stop_search(&mut self) {
        if self.running.is_some() {
            self.stop();
            self.done = Some((self.groups.len() as u64, 0, 0.0));
            self.groups.sort_by_key(|g| std::cmp::Reverse(g.wasted()));
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// A trash job or its existence check is in flight.
    pub fn busy(&self) -> bool {
        !self.trashing.is_empty() || !self.gone_rx.is_empty()
    }

    /// Take the search's results.
    pub fn poll(&mut self) {
        let Some(r) = &mut self.running else { return };
        let mut finished = None;
        loop {
            match r.rx.try_recv() {
                Ok(DupeEvent::Progress { hashed, total, bytes }) => {
                    r.hashed = hashed;
                    r.total = total;
                    r.bytes = bytes;
                }
                Ok(DupeEvent::Group(mut files)) => {
                    files.sort_by(|a, b| a.path.cmp(&b.path));
                    let n = files.len();
                    self.groups.push(Group { files, marked: vec![false; n], collapsed: false });
                }
                Ok(DupeEvent::Done { groups, wasted_bytes }) => {
                    finished = Some((groups, wasted_bytes, r.started.elapsed().as_secs_f32()));
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    finished = Some((self.groups.len() as u64, 0, r.started.elapsed().as_secs_f32()));
                    break;
                }
            }
        }
        if let Some(f) = finished {
            self.running = None;
            self.done = Some(f);
            self.groups.sort_by_key(|g| std::cmp::Reverse(g.wasted()));
        }
    }

    /// Drop the files of finished trash jobs that are really gone (one existence check
    /// per file, once per finished job, off the UI thread).
    pub fn poll_trash(&mut self, jobs: &JobsService) {
        let finished: Vec<usize> = (0..self.trashing.len())
            .filter(|&i| jobs.snapshot(self.trashing[i].0).is_none_or(|s| s.state.is_terminal()))
            .collect();
        for i in finished.into_iter().rev() {
            let (_, paths) = self.trashing.remove(i);
            let (tx, rx) = crossbeam_channel::bounded(1);
            let wake = self.wake.clone();
            let _ = std::thread::Builder::new().name("soyuz-dupes-gone".into()).spawn(move || {
                let gone: HashSet<PathBuf> = paths.into_iter().filter(|p| !soyuz_fs::exists_nofollow(p)).collect();
                let _ = tx.send(gone);
                wake();
            });
            self.gone_rx.push(rx);
        }
        let mut landed = Vec::new();
        self.gone_rx.retain(|rx| match rx.try_recv() {
            Ok(g) => {
                landed.push(g);
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(crossbeam_channel::TryRecvError::Disconnected) => false,
        });
        for gone in landed {
            self.forget(&gone);
        }
    }

    /// Remove `gone` from the groups (groups left with one file go).
    fn forget(&mut self, gone: &HashSet<PathBuf>) {
        self.removed += gone.len();
        for g in &mut self.groups {
            let keep: Vec<(EntryInfo, bool)> =
                g.files.drain(..).zip(g.marked.drain(..)).filter(|(e, _)| !gone.contains(&e.path)).collect();
            for (e, m) in keep {
                g.files.push(e);
                g.marked.push(m);
            }
        }
        self.groups.retain(|g| g.files.len() > 1);
        self.refused = None;
    }

    /// The marked files: count, bytes, paths.
    pub fn marked(&self) -> (usize, u64, Vec<PathBuf>) {
        let mut n = 0;
        let mut bytes = 0;
        let mut paths = Vec::new();
        for g in &self.groups {
            for (i, m) in g.marked.iter().enumerate() {
                if *m {
                    n += 1;
                    bytes += g.files[i].size;
                    paths.push(g.files[i].path.clone());
                }
            }
        }
        (n, bytes, paths)
    }

    /// Bytes the groups waste in all.
    pub fn wasted(&self) -> u64 {
        self.groups.iter().map(Group::wasted).sum()
    }

    /// "Select in every group": keep one copy by `keep`, mark the rest.
    pub fn keep_all(&mut self, keep: Keep) {
        for g in &mut self.groups {
            g.apply(keep);
        }
        self.confirm = false;
    }

    /// Keep one copy of group `g` by `keep`.
    pub fn keep_in(&mut self, g: usize, keep: Keep) {
        if let Some(g) = self.groups.get_mut(g) {
            g.apply(keep);
        }
    }

    /// Clear every mark.
    pub fn clear_marks(&mut self) {
        for g in &mut self.groups {
            g.marked.iter_mut().for_each(|m| *m = false);
        }
        self.confirm = false;
    }

    /// Toggle the mark of file `i` of group `g`; a refused toggle (the last unmarked copy)
    /// starts the "keep one" flash and returns false.
    pub fn toggle(&mut self, g: usize, i: usize) -> bool {
        let Some(group) = self.groups.get_mut(g) else { return false };
        if i >= group.files.len() {
            return false;
        }
        if group.toggle(i) {
            true
        } else {
            self.refused = Some((g, i, Instant::now()));
            false
        }
    }

    /// File `i` of group `g` shows the "keep one" flash now.
    pub fn refused_now(&self, g: usize, i: usize) -> bool {
        self.refused.is_some_and(|(a, b, t)| a == g && b == i && t.elapsed() < REFUSED_FLASH)
    }

    /// A "keep one" flash is showing (redraw until it ends).
    pub fn flashing(&self) -> bool {
        self.refused.is_some_and(|(_, _, t)| t.elapsed() < REFUSED_FLASH)
    }

    /// Add a root typed by the user (trimmed; no disk check: the search reports errors).
    /// Returns whether it was added.
    pub fn add_root(&mut self, text: &str) -> bool {
        let t = text.trim();
        if t.is_empty() {
            return false;
        }
        self.add_path(PathBuf::from(t))
    }

    /// Add root `p` unless it is listed already.
    pub fn add_path(&mut self, p: PathBuf) -> bool {
        if self.roots.contains(&p) {
            return false;
        }
        self.roots.push(p);
        true
    }

    pub fn remove_root(&mut self, i: usize) {
        if i < self.roots.len() {
            self.roots.remove(i);
        }
    }

    /// Pick minimum size `i` (used by the next search).
    pub fn set_min(&mut self, i: usize) {
        if i < SIZES.len() {
            self.min_idx = i;
        }
    }

    /// The window's subtitle: the root, or "3 folders".
    pub fn subtitle(&self) -> String {
        match self.roots.as_slice() {
            [] => String::new(),
            [one] => show_path(one),
            many => tr!("tools-dupes-folders", count = many.len()),
        }
    }

    /// The running search's progress: hashed, total, and its line.
    pub fn progress(&self) -> Option<(u64, u64, String)> {
        let r = self.running.as_ref()?;
        let text = tr!(
            "tools-dupes-hashing",
            done = r.hashed,
            total = r.total,
            size = format_size(r.bytes, SizeStyle::Binary),
            groups = self.groups.len(),
            secs = r.started.elapsed().as_secs()
        );
        Some((r.hashed, r.total, text))
    }

    /// The summary of the finished search ("42 groups · 3.4 GiB reclaimable · 2.3 s").
    pub fn summary(&self) -> Option<String> {
        let (_, _, secs) = self.done?;
        let groups = self.groups.len();
        let size = format_size(self.wasted(), SizeStyle::Binary);
        Some(if secs >= 1.0 {
            let secs = soyuz_core::locale::format_decimal(f64::from(secs), 1);
            tr!("tools-dupes-summary-secs", groups = groups, size = size, secs = secs)
        } else if secs > 0.0 {
            let ms = (secs * 1000.0).round() as u64;
            tr!("tools-dupes-summary-ms", groups = groups, size = size, ms = ms)
        } else {
            tr!("tools-dupes-summary", groups = groups, size = size)
        })
    }

    /// What the list says while there are no groups (`None`: groups show).
    pub fn empty_state(&self) -> Option<Empty> {
        if !self.groups.is_empty() {
            return None;
        }
        Some(if self.running.is_some() {
            Empty::Running
        } else if self.done.is_some() && self.removed > 0 {
            Empty::Handled
        } else if self.done.is_some() {
            Empty::None
        } else {
            Empty::Start
        })
    }

    /// The footer's text: the marked count and size, or the hint.
    pub fn marked_text(&self) -> String {
        let (n, bytes, _) = self.marked();
        if n == 0 {
            tr!("tools-dupes-mark-hint").to_string()
        } else {
            tr!("tools-dupes-marked", count = n, size = format_size(bytes, SizeStyle::Binary))
        }
    }

    /// The trash job for the marked files (`None` when nothing is marked): one
    /// `JobKind::Delete { Trash }` with `OpKind::Trash` meta (undoable). Pure: nothing is
    /// submitted.
    pub fn trash_request(&self) -> Option<(JobKind, Vec<PathBuf>, JobMeta)> {
        let (n, _, paths) = self.marked();
        if n == 0 {
            return None;
        }
        let kind = JobKind::Delete { paths: paths.clone(), mode: DeleteMode::Trash };
        let meta = JobMeta::new(OpKind::Trash, &paths, None);
        Some((kind, paths, meta))
    }

    /// Submit [`Self::trash_request`] to the jobs engine (with a toast); the files leave
    /// the list once the job ended and they are gone.
    pub fn trash_marked(&mut self, core: &mut AppCore) -> Option<JobId> {
        if self.busy() {
            return None;
        }
        let (kind, paths, meta) = self.trash_request()?;
        let n = paths.len();
        let spec = core.fileops.jobs.spec(kind);
        let id = core.fileops.jobs.submit(spec, meta);
        self.trashing.push((id, paths));
        self.confirm = false;
        core.ui.toasts.push(Toast::info(tr!("tools-dupes-toast-moving", count = n)));
        Some(id)
    }
}

/// A folder shown relative to the search root it lives in (`root\sub\dir`).
pub fn short_folder(roots: &[PathBuf], folder: &Path) -> String {
    for r in roots {
        if let Ok(rel) = folder.strip_prefix(r) {
            let name = soyuz_core::entry::display_name(r);
            return if rel.as_os_str().is_empty() {
                name
            } else {
                format!("{name}{}{}", std::path::MAIN_SEPARATOR, rel.display())
            };
        }
    }
    show_path(folder)
}

/// The other pane's folder in a dual-pane tab ("Add other pane").
pub fn other_pane_dir(core: &AppCore) -> Option<PathBuf> {
    let tab = core.ws.tab();
    if !tab.dual || tab.panes.len() < 2 {
        return None;
    }
    match &tab.panes[1 - tab.focused.min(1)].location {
        soyuz_core::Location::Dir(d) => Some(d.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, secs: u64) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from(path), soyuz_core::EntryKind::File);
        e.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs));
        e.size = 100;
        e
    }

    #[test]
    fn keep_helpers_pick_one_and_mark_the_rest() {
        let files = vec![f("/a/long/path/x.jpg", 50), f("/b/x.jpg", 10), f("/c/d/x.jpg", 90)];
        assert_eq!(keeper(&files, Keep::Newest), 2);
        assert_eq!(keeper(&files, Keep::Oldest), 1);
        assert_eq!(keeper(&files, Keep::ShortestPath), 1);
        let mut g = Group { files, marked: vec![false; 3], collapsed: false };
        g.apply(Keep::Newest);
        assert_eq!(g.marked, vec![true, true, false]);
        assert_eq!(g.wasted(), 200);
        // The last unmarked copy cannot be marked.
        assert!(!g.toggle(2));
        assert!(g.toggle(0));
        assert!(g.toggle(2));
        assert_eq!(g.marked, vec![false, true, true]);
    }

    fn wait(m: &mut DupesModel) {
        let t = Instant::now();
        while m.is_running() && t.elapsed().as_secs() < 30 {
            m.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn finds_marks_refuses_and_builds_one_trash_job() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let same = vec![7u8; 200_000];
        for n in ["a.bin", "b.bin", "c.bin"] {
            std::fs::write(root.join(n), &same).unwrap();
        }
        std::fs::write(root.join("other.bin"), vec![1u8; 200_000]).unwrap();
        let mut m = DupesModel::new(vec![root.clone()], crate::host::no_wake());
        assert_eq!(m.empty_state(), Some(Empty::Running));
        wait(&mut m);
        assert_eq!(m.groups.len(), 1, "one group of three");
        assert_eq!(m.groups[0].files.len(), 3);
        assert!(m.summary().is_some());
        assert_eq!(m.trash_request().map(|r| r.1), None, "nothing marked");
        m.keep_all(Keep::Newest);
        assert_eq!(m.marked().0, 2);
        // The last unmarked copy is refused (and flashes).
        let keeper = m.groups[0].marked.iter().position(|x| !x).unwrap();
        assert!(!m.toggle(0, keeper));
        assert!(m.refused_now(0, keeper) && m.flashing());
        let (kind, paths, meta) = m.trash_request().unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(kind, JobKind::Delete { paths: paths.clone(), mode: DeleteMode::Trash });
        assert_eq!((meta.op, meta.items), (OpKind::Trash, 2));
        // Gone files leave the list; a group of one goes.
        let gone: HashSet<PathBuf> = paths.into_iter().collect();
        m.forget(&gone);
        assert!(m.groups.is_empty());
        assert_eq!(m.empty_state(), Some(Empty::Handled));
        m.clear_marks();
        assert!(root.join("a.bin").exists() && root.join("b.bin").exists(), "nothing was submitted");
    }

    #[test]
    fn roots_and_sizes() {
        let mut m = DupesModel::new(Vec::new(), crate::host::no_wake());
        assert_eq!(m.empty_state(), Some(Empty::Start));
        assert!(!m.is_running(), "no roots, no search");
        assert!(!m.add_root("   "));
        assert!(m.add_root(" /no/such/folder "), "no disk check");
        assert!(!m.add_path(PathBuf::from("/no/such/folder")), "no duplicates");
        assert_eq!(m.subtitle(), show_path(Path::new("/no/such/folder")));
        m.remove_root(0);
        assert!(m.roots.is_empty());
        m.set_min(9);
        assert_eq!(m.min_idx, DEFAULT_MIN);
        m.set_min(0);
        assert_eq!(size_label(0), tr!("tools-dupes-size-any"));
        assert_eq!(size_label(2), "≥ 1 MB");
        let roots = [PathBuf::from("/data/pics")];
        let f = Path::new("/data/pics/2019");
        assert_eq!(short_folder(&roots, f), format!("pics{}2019", std::path::MAIN_SEPARATOR));
    }
}
