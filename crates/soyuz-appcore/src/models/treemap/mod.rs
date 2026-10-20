//! The disk usage treemap's model (Phase T7, agent C), moved from the GUI's
//! `ui/tools/treemap/`: the scan tree ([`scan`]), the squarified layout ([`layout`]), the
//! legend buckets ([`Bucket`]) and [`TreemapModel`], which both frontends draw (the GUI's
//! `TreemapWindow` with cushions and tooltips, the terminal UI's full-screen treemap in
//! cells).
//!
//! The model owns the scan (a worker fills the tree progressively; deeper levels load on
//! drill-in), the current level, the selection, the side list's tab, the trash
//! confirmation and the tile menu ([`TreemapItem`], applied with the same `Cmd`s in both
//! frontends). Dropping it stops the scan.
//!
//! With a [`diff::Diff`] (Compare as treemap) the folder is also compared with another
//! one and the frontends colour tiles by [`TreemapModel::mark`] instead of their bucket.

pub mod diff;
pub mod layout;
pub mod scan;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::{Location, OpControl, SizeStyle, format_size};
use soyuz_jobs::DeleteMode;

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::file_class::FileClass;
use crate::host::Wake;
use crate::models::tools::{ToolCmd, queue};
use crate::ops::fileops::FileOpCmd;
use crate::ops::fileops::names::show_path;
use diff::{Diff, Mark};
use scan::{ScanMsg, Tree};

/// The legend's colour buckets: the GUI's `class_color` groups (documents, PDFs, sheets,
/// slides and text are one "Documents" colour; fonts, links and the rest "Other").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bucket {
    Folder,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    Docs,
    Exe,
    Other,
}

impl Bucket {
    /// The legend, in order.
    pub const LEGEND: [Bucket; 9] = [
        Bucket::Folder,
        Bucket::Image,
        Bucket::Video,
        Bucket::Audio,
        Bucket::Archive,
        Bucket::Code,
        Bucket::Docs,
        Bucket::Exe,
        Bucket::Other,
    ];

    /// The bucket of a file class.
    pub fn of(class: FileClass) -> Bucket {
        match class {
            FileClass::Folder => Bucket::Folder,
            FileClass::Image => Bucket::Image,
            FileClass::Video => Bucket::Video,
            FileClass::Audio => Bucket::Audio,
            FileClass::Archive => Bucket::Archive,
            FileClass::Code => Bucket::Code,
            FileClass::Pdf | FileClass::Doc | FileClass::Sheet | FileClass::Slides | FileClass::Text => Bucket::Docs,
            FileClass::Exe => Bucket::Exe,
            FileClass::Font | FileClass::Link | FileClass::Other => Bucket::Other,
        }
    }

    /// The legend label.
    pub fn label(self) -> &'static str {
        match self {
            Bucket::Folder => tr!("tools-treemap-legend-folders"),
            Bucket::Image => tr!("tools-treemap-legend-images"),
            Bucket::Video => tr!("tools-treemap-legend-video"),
            Bucket::Audio => tr!("tools-treemap-legend-audio"),
            Bucket::Archive => tr!("tools-treemap-legend-archives"),
            Bucket::Code => tr!("tools-treemap-legend-code"),
            Bucket::Docs => tr!("tools-treemap-legend-documents"),
            Bucket::Exe => tr!("tools-treemap-legend-programs"),
            Bucket::Other => tr!("tools-treemap-legend-other"),
        }
    }
}

/// `12.5%` / `%12,5`.
pub fn percent(v: f64) -> String {
    tr!("tools-percent", pct = soyuz_core::locale::format_decimal(v, 1))
}

/// `part` as a percentage of `of` (0 when `of` is 0).
pub fn share(part: u64, of: u64) -> f64 {
    if of == 0 { 0.0 } else { part as f64 * 100.0 / of as f64 }
}

/// The side list's tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SideTab {
    /// The current level's children.
    #[default]
    Contents,
    /// The largest files below the current level ([`LARGEST`] at most).
    Largest,
}

/// Files listed under [`SideTab::Largest`] at most.
pub const LARGEST: usize = 100;

/// A running scan.
#[derive(Debug)]
struct Scanning {
    ctl: Arc<OpControl>,
    started: Instant,
    total: usize,
    done: usize,
    current: PathBuf,
}

/// The scan line under the toolbar.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Nothing to say.
    Idle,
    /// Measuring: `done` of `total` folders, and the line to show.
    Scanning { done: usize, total: usize, text: String },
    /// The root could not be read.
    Failed(String),
    /// "Scanned in 1.2 s (3 folders could not be read)".
    Done(String),
}

impl Status {
    /// The share of the scan done (`None` while the count is unknown).
    pub fn fraction(&self) -> Option<f32> {
        match self {
            Status::Scanning { done, total, .. } if *total > 0 => Some(*done.min(total) as f32 / *total as f32),
            _ => None,
        }
    }
}

/// An entry of a tile's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreemapItem {
    /// The folded "smaller items" tile: an inert note.
    ManySmall,
    ZoomIn,
    OpenInPane,
    Open,
    Reveal,
    RevealInSystem,
    CopyPath,
    AddToShelf,
    /// Ask to move it to the trash (the confirm bar).
    Trash,
}

impl TreemapItem {
    pub fn label(self) -> &'static str {
        match self {
            TreemapItem::ManySmall => tr!("tools-treemap-many-small"),
            TreemapItem::ZoomIn => tr!("tools-treemap-zoom-in"),
            TreemapItem::OpenInPane => tr!("tools-open-in-pane"),
            TreemapItem::Open => tr!("tools-open"),
            TreemapItem::Reveal => tr!("tools-shelf-reveal"),
            TreemapItem::RevealInSystem => tr!("tools-reveal-in-system"),
            TreemapItem::CopyPath => tr!("tools-copy-path"),
            TreemapItem::AddToShelf => tr!("tools-menu-add-to-shelf"),
            TreemapItem::Trash => tr!("tools-move-to-trash-dialog"),
        }
    }
}

/// The disk usage treemap of one folder.
pub struct TreemapModel {
    root: PathBuf,
    pub tree: Tree,
    /// The level shown.
    pub current: usize,
    tx: Sender<ScanMsg>,
    rx: Receiver<ScanMsg>,
    scanning: Option<Scanning>,
    /// Drill-in scans (cancelled with the window).
    sub_ctl: Arc<OpControl>,
    errors: u32,
    failed: Option<String>,
    pub selected: Option<usize>,
    /// The node the trash confirmation asks about.
    pub confirm_delete: Option<usize>,
    pub side: SideTab,
    finished_in: Option<f32>,
    /// The compare with another folder (Compare as treemap).
    pub diff: Option<Diff>,
    wake: Wake,
}

impl std::fmt::Debug for TreemapModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreemapModel")
            .field("root", &self.root)
            .field("nodes", &self.tree.nodes.len())
            .field("current", &self.current)
            .field("scanning", &self.scanning.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for TreemapModel {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl TreemapModel {
    /// The treemap of `root`; the scan starts at once (`wake` redraws the frontend as
    /// results come in).
    pub fn new(root: PathBuf, wake: Wake) -> Self {
        let (tx, rx) = unbounded();
        let mut m = TreemapModel {
            tree: Tree::new(&root),
            root,
            current: 0,
            tx,
            rx,
            scanning: None,
            sub_ctl: OpControl::new(),
            errors: 0,
            failed: None,
            selected: None,
            confirm_delete: None,
            side: SideTab::Contents,
            finished_in: None,
            diff: None,
            wake,
        };
        m.rescan();
        m
    }

    /// Also compare the folder with `other` and colour the tiles by what differs
    /// (`by_content`: bytes, else dates and sizes).
    pub fn with_diff(mut self, other: PathBuf, by_content: bool) -> Self {
        self.diff = Some(Diff::start(&self.root, other, by_content, self.wake.clone()));
        self
    }

    /// The folder compared with, while the treemap is a diff.
    pub fn diff_other(&self) -> Option<&Path> {
        self.diff.as_ref().map(|d| d.other.as_path())
    }

    /// The diff colour of node `id` (`None`: colour by kind; no diff, not ready or
    /// switched off).
    pub fn mark(&self, id: usize) -> Option<Mark> {
        let d = self.diff.as_ref().filter(|d| d.show)?;
        let marks = d.marks()?;
        let n = &self.tree.nodes[id];
        let rel = n.path.strip_prefix(&self.root).unwrap_or(&n.path);
        Some(if n.other { marks.mark(rel.parent().unwrap_or(rel), false) } else { marks.mark(rel, n.is_dir) })
    }

    /// The diff legend: bytes per mark in the current level (empty while colouring by kind).
    pub fn diff_totals(&mut self) -> Vec<(Mark, u64)> {
        let (root, tree, current) = (&self.root, &self.tree, self.current);
        match self.diff.as_mut().filter(|d| d.show) {
            Some(d) => d.totals(root, tree, current),
            None => Vec::new(),
        }
    }

    /// Switch between the diff colouring and the kind colouring.
    pub fn toggle_diff(&mut self) {
        if let Some(d) = &mut self.diff {
            d.show = !d.show;
        }
    }

    pub fn root_path(&self) -> &Path {
        &self.root
    }

    /// The root scan is running.
    pub fn is_scanning(&self) -> bool {
        self.scanning.is_some()
    }

    /// The root scan or a drill-in is running (redraw while busy).
    pub fn is_busy(&self) -> bool {
        self.scanning.is_some()
            || self.tree.nodes.iter().any(|n| n.loading)
            || self.diff.as_ref().is_some_and(Diff::is_comparing)
    }

    /// Scan again from scratch.
    pub fn rescan(&mut self) {
        self.cancel();
        let (tx, rx) = unbounded();
        self.tx = tx;
        self.rx = rx;
        self.tree = Tree::new(&self.root);
        self.current = 0;
        self.selected = None;
        self.confirm_delete = None;
        self.errors = 0;
        self.failed = None;
        self.finished_in = None;
        self.sub_ctl = OpControl::new();
        if let Some(d) = self.diff.take() {
            let show = d.show;
            let mut fresh = Diff::start(&self.root, d.other.clone(), d.by_content, self.wake.clone());
            fresh.show = show;
            self.diff = Some(fresh);
        }
        let ctl = OpControl::new();
        scan::start_scan(self.root.clone(), self.tx.clone(), ctl.clone(), self.wake.clone());
        self.scanning = Some(Scanning { ctl, started: Instant::now(), total: 0, done: 0, current: PathBuf::new() });
    }

    /// Stop the scan (and the drill-in scans).
    pub fn cancel(&mut self) {
        if let Some(s) = self.scanning.take() {
            s.ctl.cancel();
            self.tree.nodes[0].loading = false;
        }
        self.sub_ctl.cancel();
    }

    /// Take the worker's results.
    pub fn poll(&mut self) {
        if let Some(d) = &mut self.diff {
            d.poll();
        }
        for msg in self.rx.try_iter() {
            match msg {
                ScanMsg::Begin { files, dirs } => {
                    for f in files {
                        self.tree.add_child(0, f);
                    }
                    if let Some(s) = &mut self.scanning {
                        s.total = dirs;
                    }
                }
                ScanMsg::Child(c) => {
                    self.tree.add_child(0, c);
                    if let Some(s) = &mut self.scanning {
                        s.done += 1;
                    }
                }
                ScanMsg::Progress { done, current } => {
                    if let Some(s) = &mut self.scanning {
                        s.done = done;
                        s.current = current;
                    }
                }
                ScanMsg::Done { errors } => {
                    self.errors = errors;
                    let n = &mut self.tree.nodes[0];
                    n.loading = false;
                    n.complete = true;
                    self.finished_in = self.scanning.take().map(|s| s.started.elapsed().as_secs_f32());
                }
                ScanMsg::Failed(e) => {
                    self.failed = Some(e);
                    self.scanning = None;
                    self.tree.nodes[0].loading = false;
                }
                ScanMsg::Subtree { path, result } => {
                    if let Some(id) = self.tree.find(&path) {
                        match result {
                            Ok(s) => self.tree.replace(id, s),
                            Err(_) => {
                                let n = &mut self.tree.nodes[id];
                                n.loading = false;
                                n.complete = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Drill into `id` (folders only; a file is selected), loading deeper levels when
    /// needed.
    pub fn enter(&mut self, id: usize) {
        let n = &self.tree.nodes[id];
        if !n.is_dir || n.other {
            self.selected = Some(id);
            return;
        }
        // Measure deeper when this level (or a folder right below it) was cut off by the
        // scan depth, so the view always shows real contents.
        let shallow =
            !n.complete || n.children.iter().any(|&c| self.tree.nodes[c].is_dir && !self.tree.nodes[c].complete);
        if shallow && !n.loading {
            let path = n.path.clone();
            self.tree.nodes[id].loading = true;
            scan::scan_subtree(path, self.tx.clone(), self.sub_ctl.clone(), self.wake.clone());
        }
        self.current = id;
        self.selected = None;
    }

    /// Show level `id` (a breadcrumb) without measuring.
    pub fn go(&mut self, id: usize) {
        if id < self.tree.nodes.len() {
            self.current = id;
            self.selected = None;
        }
    }

    /// One level up; returns whether there was one. The folder left is selected.
    pub fn up(&mut self) -> bool {
        let Some(u) = self.tree.nodes[self.current].parent else { return false };
        let from = self.current;
        self.current = u;
        self.selected = Some(from);
        true
    }

    /// Root → current level (the breadcrumb).
    pub fn crumbs(&self) -> Vec<usize> {
        self.tree.chain(self.current)
    }

    /// What a click on tile `id` acts on: a folder itself, else the top-level child of the
    /// current view that contains it.
    pub fn click_target(&self, id: usize) -> usize {
        let chain = self.tree.chain(id);
        let pos = chain.iter().position(|&c| c == self.current).unwrap_or(0);
        if self.tree.nodes[id].is_dir { id } else { chain.get(pos + 1).copied().unwrap_or(id) }
    }

    /// A click on tile `id`: drill into the folder it belongs to, else select it.
    pub fn click(&mut self, id: usize) {
        let target = self.click_target(id);
        let n = &self.tree.nodes[target];
        if n.is_dir && !n.other {
            self.enter(target);
        } else {
            self.selected = Some(id);
        }
    }

    /// A click on side-list row `id`: a folder opens, anything else is selected.
    pub fn side_click(&mut self, id: usize) {
        let n = &self.tree.nodes[id];
        if n.is_dir && !n.other {
            self.enter(id);
        } else {
            self.selected = Some(id);
        }
    }

    /// `id` opens on a double click (a file, not the folded node).
    pub fn launchable(&self, id: usize) -> bool {
        let n = &self.tree.nodes[id];
        !n.is_dir && !n.other
    }

    /// The side list's rows for its tab.
    pub fn side_items(&self) -> Vec<usize> {
        match self.side {
            SideTab::Contents => self.tree.nodes[self.current].children.clone(),
            SideTab::Largest => self.tree.largest_files(self.current, LARGEST),
        }
    }

    /// The side list's note when it is empty.
    pub fn side_empty(&self) -> &'static str {
        if self.tree.nodes[self.current].loading {
            tr!("tools-treemap-measuring")
        } else {
            tr!("tools-treemap-no-files")
        }
    }

    /// The map's note when the current level has no tiles.
    pub fn map_empty(&self) -> Option<&'static str> {
        let n = &self.tree.nodes[self.current];
        if !n.children.is_empty() {
            return None;
        }
        Some(if n.loading || self.scanning.is_some() {
            tr!("tools-treemap-measuring")
        } else if n.is_dir {
            tr!("tools-treemap-empty-folder")
        } else {
            tr!("tools-treemap-single-file")
        })
    }

    /// The menu of tile `id` (render [`TreemapItem::Trash`] after a separator).
    pub fn menu_items(&self, id: usize) -> Vec<TreemapItem> {
        let n = &self.tree.nodes[id];
        if n.other {
            return vec![TreemapItem::ManySmall];
        }
        let mut v = if n.is_dir { vec![TreemapItem::ZoomIn, TreemapItem::OpenInPane] } else { vec![TreemapItem::Open] };
        v.extend([
            TreemapItem::Reveal,
            TreemapItem::RevealInSystem,
            TreemapItem::CopyPath,
            TreemapItem::AddToShelf,
            TreemapItem::Trash,
        ]);
        v
    }

    /// Whether menu entry `item` of tile `id` can run.
    pub fn item_enabled(&self, id: usize, item: TreemapItem) -> bool {
        match item {
            TreemapItem::ManySmall => false,
            TreemapItem::Trash => id != 0,
            _ => true,
        }
    }

    /// Run menu entry `item` on tile `id`. Returns whether the result shows in a pane
    /// (Open in pane, Reveal): a full-screen frontend steps aside.
    pub fn apply_item(&mut self, core: &mut AppCore, id: usize, item: TreemapItem) -> bool {
        if !self.item_enabled(id, item) {
            return false;
        }
        let path = self.tree.nodes[id].path.clone();
        match item {
            TreemapItem::ManySmall => {}
            TreemapItem::ZoomIn => self.enter(id),
            TreemapItem::OpenInPane => {
                let pane = core.ws.pane().id;
                core.cmds.push(Cmd::Navigate { pane, location: Location::Dir(path), select: None });
                return true;
            }
            TreemapItem::Open => crate::ops::launch(core, &path),
            TreemapItem::Reveal => {
                core.cmds.push(Cmd::RevealPath(path));
                return true;
            }
            TreemapItem::RevealInSystem => core.cmds.push(Cmd::RevealInSystem(path)),
            TreemapItem::CopyPath => core.cmds.push(Cmd::CopyText(show_path(&path))),
            TreemapItem::AddToShelf => queue(ToolCmd::ShelfAdd(vec![path])),
            TreemapItem::Trash => self.ask_trash(id),
        }
        false
    }

    /// Ask to move `id` to the trash (not the root).
    pub fn ask_trash(&mut self, id: usize) {
        if id != 0 && id < self.tree.nodes.len() {
            self.confirm_delete = Some(id);
        }
    }

    /// The trash confirmation's question.
    pub fn confirm_text(&self, style: SizeStyle) -> Option<String> {
        let n = &self.tree.nodes[self.confirm_delete?];
        Some(tr!("tools-treemap-confirm-trash", name = n.label(), size = format_size(n.bytes, style)))
    }

    /// The confirmed trash: queues `FileOpCmd::Delete { Trash }` (the core's delete prompt,
    /// the jobs engine, undo); the node leaves the tree.
    pub fn confirm_trash(&mut self, core: &mut AppCore) {
        if let Some(c) = self.take_trash() {
            core.cmds.push(c);
        }
    }

    /// [`Self::confirm_trash`]'s command, the node already out of the tree (`None` when
    /// nothing was asked).
    pub fn take_trash(&mut self) -> Option<Cmd> {
        let id = self.confirm_delete.take()?;
        let path = self.tree.nodes[id].path.clone();
        if self.tree.is_under(self.current, id) {
            self.current = self.tree.nodes[id].parent.unwrap_or(0);
        }
        self.tree.remove(id);
        self.selected = None;
        Some(Cmd::FileOp(FileOpCmd::Delete { paths: vec![path], mode: DeleteMode::Trash }))
    }

    /// The totals of the current level ("12.4 GiB · 48 210 files").
    pub fn totals(&self, style: SizeStyle) -> String {
        let n = &self.tree.nodes[self.current];
        tr!("tools-treemap-totals", size = format_size(n.bytes, style), count = n.files)
    }

    /// The scan line.
    pub fn status(&self, style: SizeStyle) -> Status {
        if let Some(s) = &self.scanning {
            let cur = if s.current.as_os_str().is_empty() {
                tr!("tools-treemap-listing").to_string()
            } else {
                soyuz_core::entry::display_name(&s.current)
            };
            let text = tr!(
                "tools-treemap-scanning",
                done = s.done.min(s.total),
                total = s.total,
                size = format_size(self.tree.nodes[0].bytes, style),
                current = cur
            );
            Status::Scanning { done: s.done, total: s.total, text }
        } else if let Some(e) = &self.failed {
            Status::Failed(tr!("tools-treemap-failed", error = e))
        } else if let Some(secs) = self.finished_in {
            let secs = soyuz_core::locale::format_decimal(f64::from(secs), 1);
            Status::Done(if self.errors > 0 {
                tr!("tools-treemap-scanned-errors", secs = secs, errors = self.errors)
            } else {
                tr!("tools-treemap-scanned", secs = secs)
            })
        } else {
            Status::Idle
        }
    }

    /// The share line of `id` ("12.5% of parent · 3.1% of view").
    pub fn share_text(&self, id: usize) -> String {
        let n = &self.tree.nodes[id];
        let parent = n.parent.map(|p| self.tree.nodes[p].bytes).unwrap_or(0);
        let view = self.tree.nodes[self.current].bytes;
        tr!("tools-treemap-share", parent = percent(share(n.bytes, parent)), view = percent(share(n.bytes, view)))
    }

    /// The bucket of node `id`.
    pub fn bucket(&self, id: usize) -> Bucket {
        Bucket::of(self.tree.nodes[id].class)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(m: &mut TreemapModel) {
        let t = Instant::now();
        while m.is_busy() && t.elapsed().as_secs() < 20 {
            m.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        m.poll();
    }

    #[test]
    fn buckets_follow_the_gui_legend() {
        assert_eq!(Bucket::of(FileClass::Pdf), Bucket::Docs);
        assert_eq!(Bucket::of(FileClass::Sheet), Bucket::Docs);
        assert_eq!(Bucket::of(FileClass::Slides), Bucket::Docs);
        assert_eq!(Bucket::of(FileClass::Text), Bucket::Docs);
        assert_eq!(Bucket::of(FileClass::Doc), Bucket::Docs);
        assert_eq!(Bucket::of(FileClass::Font), Bucket::Other);
        assert_eq!(Bucket::of(FileClass::Link), Bucket::Other);
        assert_eq!(Bucket::of(FileClass::Exe), Bucket::Exe);
        assert_eq!(Bucket::LEGEND.len(), 9);
        assert!(Bucket::LEGEND.iter().all(|b| !b.label().is_empty()));
    }

    #[test]
    fn scan_drill_up_and_trash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("a.bin"), vec![0u8; 3000]).unwrap();
        std::fs::create_dir_all(root.join("b").join("c")).unwrap();
        std::fs::write(root.join("b").join("x.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(root.join("b").join("c").join("y.bin"), vec![0u8; 1000]).unwrap();
        let mut m = TreemapModel::new(root.clone(), crate::host::no_wake());
        wait(&mut m);
        assert!(!m.is_scanning());
        assert!(matches!(m.status(SizeStyle::Binary), Status::Done(_)));
        assert_eq!(m.tree.nodes[0].bytes, 5000);
        assert_eq!(m.tree.nodes[0].files, 3);
        let names: Vec<&str> = m.side_items().iter().map(|&i| m.tree.nodes[i].label()).collect();
        assert_eq!(names, ["a.bin", "b"], "children by size");
        let b = m.tree.find(&root.join("b")).unwrap();
        let y = m.tree.find(&root.join("b").join("c").join("y.bin")).unwrap();
        // A click deep inside b drills into b (the top-level child that holds it).
        assert_eq!(m.click_target(y), b);
        m.click(y);
        assert_eq!(m.current, b);
        wait(&mut m);
        assert_eq!(m.crumbs(), vec![0, m.tree.find(&root.join("b")).unwrap()]);
        assert!(m.up());
        assert_eq!(m.current, 0);
        assert!(!m.up(), "the root has no parent");
        // A file is selected, never launched, by a click.
        let a = m.tree.find(&root.join("a.bin")).unwrap();
        m.click(a);
        assert_eq!((m.current, m.selected), (0, Some(a)));
        assert_eq!(m.menu_items(a)[0], TreemapItem::Open);
        assert!(!m.item_enabled(0, TreemapItem::Trash), "never the root");
        // The trash: a FileOp (never run here) and the node leaves the tree.
        m.ask_trash(0);
        assert_eq!(m.confirm_delete, None, "never the root");
        m.ask_trash(a);
        assert!(m.confirm_text(SizeStyle::Binary).is_some_and(|t| t.contains("a.bin")));
        let c = m.take_trash();
        assert!(matches!(
            c,
            Some(Cmd::FileOp(FileOpCmd::Delete { paths, mode: DeleteMode::Trash })) if paths == vec![root.join("a.bin")]
        ));
        assert!(root.join("a.bin").exists(), "nothing ran");
        assert_eq!(m.tree.nodes[0].bytes, 2000);
        assert!(m.tree.find(&root.join("a.bin")).is_none());
        assert!(m.take_trash().is_none(), "asked once, taken once");
    }

    #[test]
    fn diff_marks_tiles_and_totals() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let write = |p: &Path, len: usize, newer: bool| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![1u8; len]).unwrap();
            if !newer {
                std::fs::File::options().write(true).open(p).unwrap().set_modified(old).unwrap();
            }
        };
        write(&a.join("same.bin"), 1000, false);
        write(&b.join("same.bin"), 1000, false);
        write(&a.join("sub").join("edited.bin"), 2000, true);
        write(&b.join("sub").join("edited.bin"), 1500, false);
        write(&a.join("new").join("x.bin"), 500, false);
        write(&b.join("gone.bin"), 10, false);
        let mut m = TreemapModel::new(a.clone(), crate::host::no_wake()).with_diff(b.clone(), false);
        assert_eq!(m.diff_other(), Some(b.as_path()));
        wait(&mut m);
        let id = |m: &TreemapModel, p: &Path| m.tree.find(p).unwrap();
        assert_eq!(m.mark(id(&m, &a.join("same.bin"))), Some(Mark::Same));
        assert_eq!(m.mark(id(&m, &a.join("sub"))), Some(Mark::Changed));
        assert_eq!(m.mark(id(&m, &a.join("sub").join("edited.bin"))), Some(Mark::NewerHere));
        assert_eq!(m.mark(id(&m, &a.join("new"))), Some(Mark::OnlyHere));
        let totals = m.diff_totals();
        assert_eq!(totals, vec![(Mark::OnlyHere, 500), (Mark::NewerHere, 2000), (Mark::Same, 1000)]);
        let status = m.diff.as_ref().unwrap().status();
        assert!(status.contains('3') && status.contains('1'), "two here, one only there: {status}");
        // Back to the kind colours, and the compare runs again with a rescan.
        m.toggle_diff();
        assert_eq!(m.mark(0), None);
        assert!(m.diff_totals().is_empty());
        m.toggle_diff();
        m.rescan();
        wait(&mut m);
        assert_eq!(m.mark(0), Some(Mark::Changed));
    }
}
