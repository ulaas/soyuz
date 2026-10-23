//! Pure folder-tree model: nodes keyed by path, lazily loaded children, the expanded
//! set, flattening into visible rows (hidden rule + filter), reveal-path computation and
//! keyboard navigation. No I/O and no egui: everything here is unit-tested.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// What a root row stands for (decides its glyph and label).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootKind {
    Home,
    /// Index into the volume list it was built from.
    Volume(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub path: PathBuf,
    pub label: String,
    pub kind: RootKind,
}

/// A listed child folder (built on the worker from a directory entry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildInfo {
    pub name: String,
    pub path: PathBuf,
    pub hidden: bool,
    pub system: bool,
}

/// Subfolder probe: does the folder have any subfolder (any / any non-hidden)?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Probe {
    pub any: bool,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Load {
    #[default]
    Unloaded,
    Loading,
    Loaded,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub hidden: bool,
    pub system: bool,
    pub load: Load,
    /// Child folder paths in display order (valid when `load == Loaded`).
    pub children: Vec<PathBuf>,
    /// Probe result before the children are listed (`None` = unknown).
    pub probe: Option<Probe>,
}

/// Hidden-folder rule (same as the file list's `ViewFilter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Visibility {
    pub show_hidden: bool,
    pub show_system: bool,
}

impl Visibility {
    pub fn shows(self, hidden: bool, system: bool) -> bool {
        if !self.show_hidden && hidden {
            return false;
        }
        !(!self.show_system && system && hidden)
    }
}

/// Row identity: the same folder can appear under two roots (Home and its drive).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowKey {
    pub root: usize,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: RowKey,
    pub depth: usize,
    pub name: String,
    /// Show a chevron.
    pub expandable: bool,
    pub expanded: bool,
    pub loading: bool,
    pub failed: bool,
    pub hidden: bool,
    /// Byte range of the filter match in `name`.
    pub matched: Option<(usize, usize)>,
}

/// Result of flattening: the rows plus folders that must be listed to show them.
#[derive(Clone, Debug, Default)]
pub struct Flat {
    pub rows: Vec<Row>,
    pub to_load: Vec<PathBuf>,
}

#[derive(Default)]
pub struct TreeModel {
    pub roots: Vec<Root>,
    nodes: HashMap<PathBuf, Node>,
    pub expanded: BTreeSet<PathBuf>,
    /// Bumped on every change that affects the flattened rows.
    pub version: u64,
    /// The active folder: it and its ancestors stay visible even when hidden.
    pinned: Option<PathBuf>,
}

/// Case-insensitive file systems (Windows, macOS default) compare names that way.
pub fn case_insensitive() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

fn norm(s: &std::ffi::OsStr, ci: bool) -> String {
    let s = s.to_string_lossy();
    if ci { s.to_lowercase() } else { s.into_owned() }
}

fn comp_key(c: Component<'_>, ci: bool) -> String {
    match c {
        Component::Prefix(p) => norm(p.as_os_str(), true),
        Component::RootDir => "/".into(),
        other => norm(other.as_os_str(), ci),
    }
}

/// Components of `path` below `root` (`None` when `path` is not inside `root`).
pub fn rel_components(path: &Path, root: &Path, ci: bool) -> Option<Vec<OsString>> {
    let mut p = path.components().filter(|c| !matches!(c, Component::CurDir));
    for rc in root.components().filter(|c| !matches!(c, Component::CurDir)) {
        let pc = p.next()?;
        if comp_key(rc, ci) != comp_key(pc, ci) {
            return None;
        }
    }
    Some(p.map(|c| c.as_os_str().to_os_string()).collect())
}

/// Index of the root that contains `path` most specifically (longest root).
pub fn best_root(roots: &[Root], path: &Path, ci: bool) -> Option<usize> {
    roots
        .iter()
        .enumerate()
        .filter_map(|(i, r)| rel_components(path, &r.path, ci).map(|rest| (i, rest.len())))
        .min_by_key(|(_, rest)| *rest)
        .map(|(i, _)| i)
}

/// Case-insensitive natural order, the order folders are listed in.
pub fn name_cmp(a: &str, b: &str) -> Ordering {
    soyuz_core::naming::natural_cmp(a, b, true)
}

/// Does `name` contain `needle` (already folded with `fold_for_match`)? Returns the byte range.
pub fn find_match(name: &str, needle_lower: &str) -> Option<(usize, usize)> {
    if needle_lower.is_empty() {
        return None;
    }
    let lower = soyuz_core::fold_for_match(name);
    // Folding can change byte lengths (rare); only trust ranges when it did not.
    let at = lower.find(needle_lower)?;
    if lower.len() == name.len() { Some((at, at + needle_lower.len())) } else { Some((0, 0)) }
}

/// Where a reveal stands after one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reveal {
    /// Every ancestor is expanded and the target row exists.
    Done(RowKey),
    /// Waiting for these folders to be listed (they were expanded).
    Pending(Vec<PathBuf>),
    /// A component is missing (deleted, or filtered out): reveal the deepest ancestor.
    Partial(RowKey),
    /// No root contains the path.
    Outside,
}

impl TreeModel {
    pub fn node(&self, path: &Path) -> Option<&Node> {
        self.nodes.get(path)
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    /// Replace the roots (volumes changed). Nodes are kept: they are keyed by path.
    /// `true` when the roots changed.
    pub fn set_roots(&mut self, roots: Vec<Root>) -> bool {
        if roots == self.roots {
            return false;
        }
        for r in &roots {
            self.nodes.entry(r.path.clone()).or_insert_with(|| Node { name: r.label.clone(), ..Node::default() });
        }
        self.roots = roots;
        self.bump();
        true
    }

    /// Keep `path` and its ancestors visible (the folder the active pane shows).
    pub fn set_pinned(&mut self, path: Option<PathBuf>) {
        if self.pinned != path {
            self.pinned = path;
            self.bump();
        }
    }

    /// The hidden rule, except for the pinned folder and its ancestors.
    fn shown(&self, path: &Path, n: &Node, vis: Visibility) -> bool {
        vis.shows(n.hidden, n.system) || self.pinned.as_deref().is_some_and(|p| p.starts_with(path))
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub fn expand(&mut self, path: &Path) {
        if self.expanded.insert(path.to_path_buf()) {
            self.bump();
        }
    }

    pub fn collapse(&mut self, path: &Path) {
        if self.expanded.remove(path) {
            self.bump();
        }
    }

    pub fn toggle(&mut self, path: &Path) {
        if self.is_expanded(path) { self.collapse(path) } else { self.expand(path) }
    }

    /// Collapse `path` and everything expanded below it.
    pub fn collapse_all_below(&mut self, path: &Path) {
        let before = self.expanded.len();
        self.expanded.retain(|p| !p.starts_with(path));
        if self.expanded.len() != before {
            self.bump();
        }
    }

    /// A listing of `dir` was requested.
    pub fn mark_loading(&mut self, dir: &Path) {
        let n = self.nodes.entry(dir.to_path_buf()).or_default();
        if n.load != Load::Loading {
            // A relist of a loaded folder keeps showing the old children meanwhile.
            if n.load != Load::Loaded {
                n.load = Load::Loading;
                self.bump();
            }
        }
    }

    /// Whether `dir` needs a listing to show its children.
    pub fn needs_load(&self, dir: &Path) -> bool {
        matches!(self.nodes.get(dir).map(|n| &n.load), None | Some(Load::Unloaded))
    }

    /// Store the listed child folders of `dir`, keeping the state of children that are
    /// still there and dropping the subtrees of children that disappeared.
    pub fn set_children(&mut self, dir: &Path, mut children: Vec<ChildInfo>) {
        children.sort_by(|a, b| name_cmp(&a.name, &b.name));
        let old: Vec<PathBuf> = self.nodes.get(dir).map(|n| n.children.clone()).unwrap_or_default();
        let keep: std::collections::HashSet<&PathBuf> = children.iter().map(|c| &c.path).collect();
        for gone in old.iter().filter(|p| !keep.contains(p)) {
            self.forget(gone);
        }
        for c in &children {
            let n = self.nodes.entry(c.path.clone()).or_default();
            n.name = c.name.clone();
            n.hidden = c.hidden;
            n.system = c.system;
        }
        let n = self.nodes.entry(dir.to_path_buf()).or_default();
        n.children = children.into_iter().map(|c| c.path).collect();
        n.load = Load::Loaded;
        self.bump();
    }

    /// Fix the visible-subfolder probe of a loaded folder from its children.
    fn loaded_probe(&self, n: &Node, vis: Visibility) -> bool {
        n.children.iter().any(|c| self.nodes.get(c).is_some_and(|cn| self.shown(c, cn, vis)))
    }

    pub fn set_failed(&mut self, dir: &Path, err: String) {
        let n = self.nodes.entry(dir.to_path_buf()).or_default();
        n.load = Load::Failed(err);
        self.bump();
    }

    pub fn set_probe(&mut self, path: &Path, probe: Probe) {
        if let Some(n) = self.nodes.get_mut(path)
            && n.probe != Some(probe)
            && n.load != Load::Loaded
        {
            n.probe = Some(probe);
            self.bump();
        }
    }

    /// Let a folder whose listing failed be listed again when shown.
    pub fn retry(&mut self, dir: &Path) {
        if let Some(n) = self.nodes.get_mut(dir)
            && matches!(n.load, Load::Failed(_))
        {
            n.load = Load::Unloaded;
            self.bump();
        }
    }

    fn forget(&mut self, path: &Path) {
        if let Some(n) = self.nodes.remove(path) {
            for c in n.children {
                self.forget(&c);
            }
        }
        self.expanded.remove(path);
    }

    /// Folders whose children are shown: expanded, loaded and reachable from a root
    /// through expanded folders (these are the ones worth watching).
    pub fn open_dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack: Vec<&PathBuf> = self.roots.iter().map(|r| &r.path).rev().collect();
        while let Some(p) = stack.pop() {
            if !self.expanded.contains(p) {
                continue;
            }
            let Some(n) = self.nodes.get(p) else { continue };
            if n.load != Load::Loaded {
                continue;
            }
            out.push(p.clone());
            stack.extend(n.children.iter().rev());
        }
        out
    }

    fn expandable(&self, n: &Node, vis: Visibility) -> bool {
        match &n.load {
            Load::Loaded => self.loaded_probe(n, vis),
            Load::Failed(_) => false,
            _ => n.probe.is_none_or(|p| if vis.show_hidden { p.any } else { p.visible }),
        }
    }

    /// Flatten into rows. With a filter, only folders whose name matches (and their
    /// ancestors) are shown, searched through every loaded folder; otherwise the
    /// expanded folders are walked.
    pub fn flatten(&self, vis: Visibility, filter: &str) -> Flat {
        let needle = soyuz_core::fold_for_match(filter.trim());
        let mut flat = Flat::default();
        for (ri, r) in self.roots.iter().enumerate() {
            if needle.is_empty() {
                self.walk(ri, &r.path, 0, vis, &mut flat, Some(&r.label));
            } else {
                let mut rows = Vec::new();
                self.walk_filtered(ri, &r.path, 0, vis, &needle, &mut rows, Some(&r.label));
                flat.rows.extend(rows);
            }
        }
        flat
    }

    fn row(&self, ri: usize, path: &Path, depth: usize, n: &Node, vis: Visibility, label: Option<&str>) -> Row {
        Row {
            key: RowKey { root: ri, path: path.to_path_buf() },
            depth,
            name: label.map_or_else(|| n.name.clone(), str::to_string),
            expandable: self.expandable(n, vis),
            expanded: self.expanded.contains(path),
            loading: n.load == Load::Loading,
            failed: matches!(n.load, Load::Failed(_)),
            hidden: n.hidden,
            matched: None,
        }
    }

    fn walk(&self, ri: usize, path: &Path, depth: usize, vis: Visibility, flat: &mut Flat, label: Option<&str>) {
        let default = Node::default();
        let n = self.nodes.get(path).unwrap_or(&default);
        let row = self.row(ri, path, depth, n, vis, label);
        let open = row.expanded;
        flat.rows.push(row);
        if !open {
            return;
        }
        match n.load {
            Load::Unloaded => {
                if !flat.to_load.iter().any(|p| p == path) {
                    flat.to_load.push(path.to_path_buf());
                }
            }
            Load::Loaded => {
                for c in &n.children {
                    if let Some(cn) = self.nodes.get(c)
                        && self.shown(c, cn, vis)
                    {
                        self.walk(ri, c, depth + 1, vis, flat, None);
                    }
                }
            }
            _ => {}
        }
    }

    /// Returns whether anything at or below `path` matched.
    #[allow(clippy::too_many_arguments)]
    fn walk_filtered(
        &self,
        ri: usize,
        path: &Path,
        depth: usize,
        vis: Visibility,
        needle: &str,
        out: &mut Vec<Row>,
        label: Option<&str>,
    ) -> bool {
        let default = Node::default();
        let n = self.nodes.get(path).unwrap_or(&default);
        let mut row = self.row(ri, path, depth, n, vis, label);
        row.matched = find_match(&row.name, needle);
        let at = out.len();
        out.push(row);
        let mut any_child = false;
        if n.load == Load::Loaded {
            for c in &n.children {
                if let Some(cn) = self.nodes.get(c)
                    && self.shown(c, cn, vis)
                {
                    any_child |= self.walk_filtered(ri, c, depth + 1, vis, needle, out, None);
                }
            }
        }
        let keep = out[at].matched.is_some() || any_child;
        if !keep {
            out.truncate(at);
        } else if any_child {
            out[at].expanded = true;
        }
        keep
    }

    /// One reveal step towards `target` under root `ri`: expands every ancestor whose
    /// children are known and reports which folders still have to be listed.
    pub fn reveal(&mut self, ri: usize, target: &Path, vis: Visibility, ci: bool) -> Reveal {
        let Some(root) = self.roots.get(ri).map(|r| r.path.clone()) else { return Reveal::Outside };
        let Some(rest) = rel_components(target, &root, ci) else { return Reveal::Outside };
        let mut cur = root;
        for name in rest {
            self.expand(&cur);
            let Some(n) = self.nodes.get(&cur) else {
                return Reveal::Pending(vec![cur]);
            };
            match &n.load {
                Load::Loaded => {}
                Load::Failed(_) => return Reveal::Partial(RowKey { root: ri, path: cur }),
                _ => return Reveal::Pending(vec![cur]),
            }
            let want = norm(&name, ci);
            let found = n.children.iter().find(|c| {
                c.file_name().is_some_and(|f| norm(f, ci) == want)
                    && self.nodes.get(*c).is_some_and(|cn| self.shown(c, cn, vis))
            });
            match found {
                Some(c) => cur = c.clone(),
                None => return Reveal::Partial(RowKey { root: ri, path: cur }),
            }
        }
        Reveal::Done(RowKey { root: ri, path: cur })
    }
}

/// Keyboard movement in the flattened rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Home,
    End,
    PageUp(usize),
    PageDown(usize),
    /// Collapse, or go to the parent row.
    Left,
    /// Expand, or go to the first child.
    Right,
}

/// What a keyboard step does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavOutcome {
    Select(usize),
    Expand(usize),
    Collapse(usize),
    None,
}

pub fn nav(rows: &[Row], cur: Option<usize>, step: Nav) -> NavOutcome {
    if rows.is_empty() {
        return NavOutcome::None;
    }
    let last = rows.len() - 1;
    let Some(i) = cur.filter(|i| *i <= last) else {
        return NavOutcome::Select(if matches!(step, Nav::End | Nav::Up) { last } else { 0 });
    };
    let sel = |j: usize| if j == i { NavOutcome::None } else { NavOutcome::Select(j) };
    match step {
        Nav::Up => sel(i.saturating_sub(1)),
        Nav::Down => sel((i + 1).min(last)),
        Nav::Home => sel(0),
        Nav::End => sel(last),
        Nav::PageUp(n) => sel(i.saturating_sub(n.max(1))),
        Nav::PageDown(n) => sel((i + n.max(1)).min(last)),
        Nav::Left => {
            let r = &rows[i];
            if r.expanded && r.expandable {
                return NavOutcome::Collapse(i);
            }
            match parent_row(rows, i) {
                Some(p) => NavOutcome::Select(p),
                None => NavOutcome::None,
            }
        }
        Nav::Right => {
            let r = &rows[i];
            if !r.expandable {
                return NavOutcome::None;
            }
            if !r.expanded {
                return NavOutcome::Expand(i);
            }
            match rows.get(i + 1) {
                Some(next) if next.depth > r.depth => NavOutcome::Select(i + 1),
                _ => NavOutcome::None,
            }
        }
    }
}

/// The closest row above `i` with a smaller depth.
pub fn parent_row(rows: &[Row], i: usize) -> Option<usize> {
    let d = rows.get(i)?.depth;
    (0..i).rev().find(|&j| rows[j].depth < d)
}

/// Type-ahead: the next row (after `cur`, wrapping) whose name starts with `prefix`.
/// Typing one letter repeatedly cycles through the rows starting with it.
pub fn type_ahead(rows: &[Row], cur: Option<usize>, prefix: &str) -> Option<usize> {
    let p = soyuz_core::fold_for_match(prefix);
    if p.is_empty() || rows.is_empty() {
        return None;
    }
    let mut chars = p.chars();
    let first = chars.next()?;
    let repeated = chars.all(|c| c == first);
    let starts = |j: usize, s: &str| soyuz_core::fold_for_match(&rows[j].name).starts_with(s);
    let n = rows.len();
    let start = cur.unwrap_or(n - 1);
    if p.chars().count() > 1 && !repeated {
        // Longer prefixes keep the current row while it still matches.
        if let Some(c) = cur
            && c < n
            && starts(c, &p)
        {
            return Some(c);
        }
        return (1..=n).map(|k| (start + k) % n).find(|&j| starts(j, &p));
    }
    let one = first.to_string();
    (1..=n).map(|k| (start + k) % n).find(|&j| starts(j, &one))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(dir: &str, name: &str) -> ChildInfo {
        ChildInfo { name: name.into(), path: Path::new(dir).join(name), hidden: false, system: false }
    }

    fn model() -> TreeModel {
        let mut m = TreeModel::default();
        m.set_roots(vec![
            Root { path: "/r".into(), label: "Disk".into(), kind: RootKind::Volume(0) },
            Root { path: "/r/home".into(), label: "Home".into(), kind: RootKind::Home },
        ]);
        m
    }

    const VIS: Visibility = Visibility { show_hidden: false, show_system: false };

    fn names(f: &Flat) -> Vec<String> {
        f.rows.iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.name)).collect()
    }

    #[test]
    fn expand_collapse_and_lazy_loading() {
        let mut m = model();
        let f = m.flatten(VIS, "");
        assert_eq!(names(&f), ["Disk", "Home"]);
        assert!(f.rows[0].expandable, "unknown children: assume expandable");
        assert!(f.to_load.is_empty(), "collapsed roots load nothing");

        m.expand(Path::new("/r"));
        let f = m.flatten(VIS, "");
        assert_eq!(f.to_load, [PathBuf::from("/r")]);
        m.mark_loading(Path::new("/r"));
        assert!(m.flatten(VIS, "").rows[0].loading);
        assert!(m.flatten(VIS, "").to_load.is_empty());

        m.set_children(Path::new("/r"), vec![child("/r", "b"), child("/r", "a10"), child("/r", "a9")]);
        assert_eq!(names(&m.flatten(VIS, "")), ["Disk", "  a9", "  a10", "  b", "Home"], "natural order");

        m.set_children(Path::new("/r/a9"), vec![]);
        let f = m.flatten(VIS, "");
        assert!(!f.rows[1].expandable, "listed empty folder has no chevron");
        m.set_probe(Path::new("/r/b"), Probe { any: false, visible: false });
        assert!(!m.flatten(VIS, "").rows[3].expandable);

        m.collapse(Path::new("/r"));
        assert_eq!(names(&m.flatten(VIS, "")), ["Disk", "Home"]);
        m.toggle(Path::new("/r"));
        assert_eq!(m.flatten(VIS, "").rows.len(), 5, "children remembered");
    }

    #[test]
    fn hidden_folders_follow_the_toggle() {
        let mut m = model();
        m.expand(Path::new("/r"));
        let mut h = child("/r", ".git");
        h.hidden = true;
        let mut sys = child("/r", "$Recycle.Bin");
        sys.hidden = true;
        sys.system = true;
        m.set_children(Path::new("/r"), vec![h, sys, child("/r", "src")]);
        assert_eq!(names(&m.flatten(VIS, "")), ["Disk", "  src", "Home"]);
        let shown = Visibility { show_hidden: true, show_system: false };
        assert_eq!(names(&m.flatten(shown, "")), ["Disk", "  .git", "  src", "Home"]);
        let all = Visibility { show_hidden: true, show_system: true };
        assert_eq!(m.flatten(all, "").rows.len(), 5);

        // The active folder (and its ancestors) stays visible, dimmed, while inside it.
        m.set_pinned(Some("/r/.git/objects".into()));
        let f = m.flatten(VIS, "");
        assert_eq!(names(&f), ["Disk", "  .git", "  src", "Home"]);
        assert!(f.rows[1].hidden);
        m.set_pinned(None);

        // A folder with only hidden subfolders has no chevron unless hidden are shown.
        m.set_probe(Path::new("/r/src"), Probe { any: true, visible: false });
        assert!(!m.flatten(VIS, "").rows[1].expandable);
        assert!(m.flatten(shown, "").rows[2].expandable);
    }

    #[test]
    fn relisting_keeps_state_and_drops_removed_subtrees() {
        let mut m = model();
        m.expand(Path::new("/r"));
        m.set_children(Path::new("/r"), vec![child("/r", "a"), child("/r", "b")]);
        m.expand(Path::new("/r/a"));
        m.set_children(Path::new("/r/a"), vec![child("/r/a", "x")]);
        m.expand(Path::new("/r/b"));
        m.set_children(Path::new("/r/b"), vec![child("/r/b", "y")]);
        m.set_children(Path::new("/r"), vec![child("/r", "a"), child("/r", "c")]);
        assert_eq!(names(&m.flatten(VIS, "")), ["Disk", "  a", "    x", "  c", "Home"]);
        assert!(!m.is_expanded(Path::new("/r/b")), "removed folder forgotten");
        assert!(m.node(Path::new("/r/b/y")).is_none());
        assert_eq!(m.open_dirs(), [PathBuf::from("/r"), PathBuf::from("/r/a")]);
    }

    #[test]
    fn filter_shows_matches_with_their_ancestors() {
        let mut m = model();
        m.set_children(Path::new("/r"), vec![child("/r", "src"), child("/r", "docs")]);
        m.set_children(Path::new("/r/src"), vec![child("/r/src", "Tree"), child("/r/src", "ui")]);
        // Filtering searches loaded folders even when collapsed.
        let f = m.flatten(VIS, "TRE");
        assert_eq!(names(&f), ["Disk", "  src", "    Tree"]);
        assert!(f.rows[0].expanded && f.rows[1].expanded);
        assert_eq!(f.rows[2].matched, Some((0, 3)));
        assert_eq!(f.rows[0].matched, None);
        assert!(m.flatten(VIS, "zzz").rows.is_empty());
        assert_eq!(find_match("Projects", "jec"), Some((3, 6)));
        assert_eq!(find_match("x", ""), None);
    }

    #[test]
    fn reveal_expands_ancestors_step_by_step() {
        let mut m = model();
        let target = Path::new("/r/home/proj/src");
        let ri = best_root(&m.roots, target, false).unwrap();
        assert_eq!(ri, 1, "deepest root wins");
        assert_eq!(m.reveal(ri, target, VIS, false), Reveal::Pending(vec!["/r/home".into()]));
        assert!(m.is_expanded(Path::new("/r/home")));
        m.set_children(Path::new("/r/home"), vec![child("/r/home", "proj")]);
        assert_eq!(m.reveal(ri, target, VIS, false), Reveal::Pending(vec!["/r/home/proj".into()]));
        m.set_children(Path::new("/r/home/proj"), vec![child("/r/home/proj", "src")]);
        let done = m.reveal(ri, target, VIS, false);
        assert_eq!(done, Reveal::Done(RowKey { root: 1, path: target.into() }));
        assert!(!m.is_expanded(target), "the target itself stays collapsed");
        let f = m.flatten(VIS, "");
        assert_eq!(names(&f), ["Disk", "Home", "  proj", "    src"]);

        assert_eq!(
            m.reveal(ri, Path::new("/r/home/gone/x"), VIS, false),
            Reveal::Partial(RowKey { root: 1, path: "/r/home".into() })
        );
        assert_eq!(m.reveal(0, Path::new("/elsewhere"), VIS, false), Reveal::Outside);
        assert_eq!(best_root(&m.roots, Path::new("/elsewhere"), false), None);
    }

    #[test]
    fn reveal_matches_case_insensitively_when_asked() {
        let mut m = model();
        m.set_children(Path::new("/r"), vec![child("/r", "Users")]);
        let t = Path::new("/R/users");
        assert_eq!(rel_components(t, Path::new("/r"), true).map(|v| v.len()), Some(1));
        assert_eq!(rel_components(t, Path::new("/r"), false), None);
        assert_eq!(m.reveal(0, t, VIS, true), Reveal::Done(RowKey { root: 0, path: "/r/Users".into() }));
    }

    fn rows(spec: &[(usize, bool, bool)]) -> Vec<Row> {
        spec.iter()
            .enumerate()
            .map(|(i, &(depth, expandable, expanded))| Row {
                key: RowKey { root: 0, path: PathBuf::from(format!("/{i}")) },
                depth,
                name: format!("n{i}"),
                expandable,
                expanded,
                loading: false,
                failed: false,
                hidden: false,
                matched: None,
            })
            .collect()
    }

    #[test]
    fn keyboard_navigation() {
        // 0 root (open) → 1 child (closed, expandable) → 2 child (leaf); 3 root (closed)
        let r = rows(&[(0, true, true), (1, true, false), (1, false, false), (0, true, false)]);
        assert_eq!(nav(&r, None, Nav::Down), NavOutcome::Select(0));
        assert_eq!(nav(&r, None, Nav::End), NavOutcome::Select(3));
        assert_eq!(nav(&r, Some(0), Nav::Down), NavOutcome::Select(1));
        assert_eq!(nav(&r, Some(0), Nav::Up), NavOutcome::None);
        assert_eq!(nav(&r, Some(3), Nav::Down), NavOutcome::None);
        assert_eq!(nav(&r, Some(1), Nav::PageDown(10)), NavOutcome::Select(3));
        assert_eq!(nav(&r, Some(3), Nav::PageUp(2)), NavOutcome::Select(1));
        assert_eq!(nav(&r, Some(1), Nav::Right), NavOutcome::Expand(1));
        assert_eq!(nav(&r, Some(0), Nav::Right), NavOutcome::Select(1), "open: go to first child");
        assert_eq!(nav(&r, Some(2), Nav::Right), NavOutcome::None, "leaf");
        assert_eq!(nav(&r, Some(0), Nav::Left), NavOutcome::Collapse(0));
        assert_eq!(nav(&r, Some(2), Nav::Left), NavOutcome::Select(0), "closed: go to parent");
        assert_eq!(nav(&r, Some(3), Nav::Left), NavOutcome::None);
        assert_eq!(parent_row(&r, 2), Some(0));
        assert_eq!(nav(&[], None, Nav::Down), NavOutcome::None);
    }

    #[test]
    fn type_ahead_cycles_and_extends() {
        let mut r = rows(&[(0, false, false); 4]);
        for (row, name) in r.iter_mut().zip(["Music", "Movies", "Documents", "music2"]) {
            row.name = name.into();
        }
        assert_eq!(type_ahead(&r, None, "m"), Some(0));
        assert_eq!(type_ahead(&r, Some(0), "m"), Some(1), "same letter again cycles");
        assert_eq!(type_ahead(&r, Some(1), "mm"), Some(3), "repeated letter keeps cycling");
        assert_eq!(type_ahead(&r, Some(3), "m"), Some(0), "wraps");
        assert_eq!(type_ahead(&r, Some(0), "mu"), Some(0), "longer prefix keeps a matching row");
        assert_eq!(type_ahead(&r, Some(1), "mus"), Some(3));
        assert_eq!(type_ahead(&r, Some(0), "DOC"), Some(2));
        assert_eq!(type_ahead(&r, Some(0), "x"), None);
    }
}
