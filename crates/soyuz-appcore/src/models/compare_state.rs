//! Two-way sync with real conflict handling (frontend-agnostic; moved from the GUI's
//! `ui/tools/sync_state.rs`).
//!
//! A plain compare can't tell "deleted on the left" from "new on the right", or "edited
//! on both sides" from "edited on one". So a two-way profile remembers what both trees
//! looked like after the last successful sync: a small JSON state file per folder pair
//! (`sync_state/<hash>.json` next to the config) mapping each relative path to its
//! `(size, mtime, kind)` on the left and on the right.
//!
//! [`decide`] is the three-way decision table (base, left now, right now):
//!
//! | base      | left      | right     | decision                                         |
//! |-----------|-----------|-----------|--------------------------------------------------|
//! | —         | ✓         | —         | copy → (new on the left)                          |
//! | —         | —         | ✓         | copy ← (new on the right)                         |
//! | —         | ✓         | ✓         | nothing if equal, else conflict *both created*    |
//! | ✓         | —         | —         | nothing (deleted on both sides)                   |
//! | ✓         | =         | —         | delete left (deleted on the right)                |
//! | ✓         | changed   | —         | conflict *changed left, deleted right*            |
//! | ✓         | —         | =         | delete right (deleted on the left)                |
//! | ✓         | —         | changed   | conflict *deleted left, changed right*            |
//! | ✓         | =         | =         | nothing                                          |
//! | ✓         | changed   | =         | copy →                                            |
//! | ✓         | =         | changed   | copy ←                                            |
//! | ✓         | changed   | changed   | nothing if now equal, else conflict *both changed*|
//!
//! Without any state file (the first run) a path on both sides that differs goes to the
//! newer side when the times are clearly apart, else it is a *both created* conflict.
//!
//! Folders: a new folder is copied whole when everything under it is new; a folder
//! deleted on one side is deleted on the other only when nothing under it changed
//! (otherwise its items are decided one by one and the folder stays).
//!
//! After the sync job finishes, [`next_state`] records every path that is now equal on
//! both sides; skipped and failed paths keep their previous base so the same question
//! comes back next time instead of silently resolving itself.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use soyuz_core::{Control, EntryKind, FsResult, OpControl};
use soyuz_fs::{WalkItem, WalkOptions};
use soyuz_jobs::SyncAction;

/// Times closer than this are equal (FAT has 2 s resolution), like `compare_dirs`.
pub const TOLERANCE_MS: i64 = 2000;

/// What one side of a path looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dir: bool,
    #[serde(default)]
    pub size: u64,
    /// Modification time, milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
}

impl FileState {
    pub fn file(size: u64, mtime: Option<i64>) -> Self {
        FileState { dir: false, size, mtime }
    }

    pub fn folder() -> Self {
        FileState { dir: true, size: 0, mtime: None }
    }

    pub fn from_meta(dir: bool, size: u64, modified: Option<SystemTime>) -> Self {
        if dir { FileState::folder() } else { FileState::file(size, modified.map(ms)) }
    }

    pub fn modified(&self) -> Option<SystemTime> {
        let m = self.mtime?;
        if m >= 0 {
            Some(UNIX_EPOCH + Duration::from_millis(m as u64))
        } else {
            UNIX_EPOCH.checked_sub(Duration::from_millis(m.unsigned_abs()))
        }
    }

    /// Changed compared to the base (kind, size or time; folders never change).
    pub fn changed_since(&self, base: &FileState, size_only: bool) -> bool {
        if self.dir != base.dir {
            return true;
        }
        !self.dir && (self.size != base.size || (!size_only && !times_equal(self.mtime, base.mtime)))
    }

    /// The two sides hold the same thing (same kind; files: same size and time).
    pub fn same_as(&self, o: &FileState, size_only: bool) -> bool {
        !self.changed_since(o, size_only)
    }
}

fn ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis().min(i64::MAX as u128) as i64,
        Err(e) => -(e.duration().as_millis().min(i64::MAX as u128) as i64),
    }
}

fn times_equal(a: Option<i64>, b: Option<i64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= TOLERANCE_MS,
        (None, None) => true,
        _ => false,
    }
}

/// Clearly newer (by more than the tolerance)?
fn newer(a: Option<i64>, b: Option<i64>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a - b > TOLERANCE_MS)
}

/// A tree scan: relative path → state.
pub type Tree = BTreeMap<PathBuf, FileState>;

/// Base of one path: both sides as they were after the last sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasePair {
    pub l: FileState,
    pub r: FileState,
}

/// What we know about a path's past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base<'a> {
    /// No state file at all (first two-way run of this pair).
    Unknown,
    /// A state file exists but did not have this path.
    Absent,
    Known(&'a BasePair),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Changed on both sides since the last sync.
    BothChanged,
    /// Created on both sides (or no shared history) with different contents.
    BothCreated,
    /// Changed on the left, deleted on the right.
    ChangedLeftDeletedRight,
    /// Deleted on the left, changed on the right.
    DeletedLeftChangedRight,
}

impl ConflictKind {
    pub fn label(self) -> &'static str {
        match self {
            ConflictKind::BothChanged => tr!("tools-conflict-both-changed"),
            ConflictKind::BothCreated => tr!("tools-conflict-both-created"),
            ConflictKind::ChangedLeftDeletedRight => tr!("tools-conflict-changed-left-deleted-right"),
            ConflictKind::DeletedLeftChangedRight => tr!("tools-conflict-deleted-left-changed-right"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decision {
    Nothing,
    CopyLR,
    CopyRL,
    DeleteL,
    DeleteR,
    Conflict(ConflictKind),
}

/// The three-way decision table (see the module docs).
pub fn decide(base: Base<'_>, l: Option<&FileState>, r: Option<&FileState>, size_only: bool) -> Decision {
    use Decision as D;
    match (base, l, r) {
        (_, None, None) => D::Nothing,
        (Base::Unknown | Base::Absent, Some(_), None) => D::CopyLR,
        (Base::Unknown | Base::Absent, None, Some(_)) => D::CopyRL,
        (Base::Unknown, Some(l), Some(r)) => {
            if l.same_as(r, size_only) {
                D::Nothing
            } else if l.dir == r.dir && !size_only && newer(l.mtime, r.mtime) {
                D::CopyLR
            } else if l.dir == r.dir && !size_only && newer(r.mtime, l.mtime) {
                D::CopyRL
            } else {
                D::Conflict(ConflictKind::BothCreated)
            }
        }
        (Base::Absent, Some(l), Some(r)) => {
            if l.same_as(r, size_only) {
                D::Nothing
            } else {
                D::Conflict(ConflictKind::BothCreated)
            }
        }
        (Base::Known(b), Some(l), None) => {
            if l.changed_since(&b.l, size_only) {
                D::Conflict(ConflictKind::ChangedLeftDeletedRight)
            } else {
                D::DeleteL
            }
        }
        (Base::Known(b), None, Some(r)) => {
            if r.changed_since(&b.r, size_only) {
                D::Conflict(ConflictKind::DeletedLeftChangedRight)
            } else {
                D::DeleteR
            }
        }
        (Base::Known(b), Some(l), Some(r)) => {
            match (l.changed_since(&b.l, size_only), r.changed_since(&b.r, size_only)) {
                (false, false) => D::Nothing,
                (true, false) => D::CopyLR,
                (false, true) => D::CopyRL,
                (true, true) if l.same_as(r, size_only) => D::Nothing,
                (true, true) => D::Conflict(ConflictKind::BothChanged),
            }
        }
    }
}

/// One path that needs something done (or decided).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub rel: PathBuf,
    pub left: Option<FileState>,
    pub right: Option<FileState>,
    pub decision: Decision,
}

/// Result of [`plan`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TwoWayPlan {
    /// Paths to act on or decide, sorted.
    pub items: Vec<Item>,
    /// Paths that are not acted on now but must keep their base (a folder whose deletion
    /// waits for its changed contents to be decided).
    pub keep_base: Vec<PathBuf>,
    /// Paths identical on both sides.
    pub same: u64,
}

/// Decide every path of the two trees against the saved state and collapse folders.
pub fn plan(state: Option<&SyncState>, left: &Tree, right: &Tree, size_only: bool) -> TwoWayPlan {
    let keys: BTreeSet<&PathBuf> = left.keys().chain(right.keys()).collect();
    let mut decided: Vec<(PathBuf, Option<FileState>, Option<FileState>, Decision)> = Vec::with_capacity(keys.len());
    for rel in keys {
        let base = match state {
            None => Base::Unknown,
            Some(s) => s.get(rel).map_or(Base::Absent, Base::Known),
        };
        let (l, r) = (left.get(rel), right.get(rel));
        decided.push((rel.clone(), l.copied(), r.copied(), decide(base, l, r, size_only)));
    }
    let mut out = TwoWayPlan::default();
    // `decided` is sorted by path, so a folder's descendants follow it directly.
    let mut i = 0;
    while i < decided.len() {
        let (rel, l, r, d) = decided[i].clone();
        let is_dir = l.or(r).is_some_and(|s| s.dir);
        let end = if is_dir { subtree_end(&decided, i) } else { i + 1 };
        let kids = &decided[i + 1..end];
        match d {
            Decision::CopyLR | Decision::CopyRL | Decision::DeleteL | Decision::DeleteR if is_dir => {
                if kids.iter().all(|k| k.3 == d) {
                    // The whole subtree goes the same way: one action on the folder.
                    out.items.push(Item { rel, left: l, right: r, decision: d });
                    i = end;
                    continue;
                }
                if matches!(d, Decision::DeleteL | Decision::DeleteR) {
                    out.keep_base.push(rel);
                }
                // Copies create parent folders themselves; decide the children one by one.
                i += 1;
                continue;
            }
            Decision::Conflict(_) if l.zip(r).is_some_and(|(a, b)| a.dir != b.dir) => {
                // File on one side, folder on the other: the choice applies to the whole path.
                out.items.push(Item { rel, left: l, right: r, decision: d });
                i = end;
                continue;
            }
            Decision::Nothing => {
                if l.is_some() && r.is_some() && !is_dir {
                    out.same += 1;
                }
            }
            _ => out.items.push(Item { rel, left: l, right: r, decision: d }),
        }
        i += 1;
    }
    out
}

/// Index one past the last descendant of the folder at `i`.
fn subtree_end(v: &[(PathBuf, Option<FileState>, Option<FileState>, Decision)], i: usize) -> usize {
    let root = &v[i].0;
    let mut j = i + 1;
    while j < v.len() && v[j].0.starts_with(root) {
        j += 1;
    }
    j
}

/// The user's answer to a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Make the right side match the left.
    Left,
    /// Make the left side match the right.
    Right,
    /// Keep both: the left version keeps the name on both sides, the right version is kept
    /// on both sides under a conflict name.
    KeepBoth,
    Skip,
}

impl Resolution {
    pub fn label(self) -> &'static str {
        match self {
            Resolution::Left => tr!("tools-resolution-left"),
            Resolution::Right => tr!("tools-resolution-right"),
            Resolution::KeepBoth => tr!("tools-resolution-both"),
            Resolution::Skip => tr!("common-skip"),
        }
    }
}

/// Sync steps for a resolved conflict. `alt` is the conflict name for the right version.
pub fn resolve(item: &Item, res: Resolution, alt: &Path) -> Vec<SyncAction> {
    let rel = item.rel.clone();
    match (res, item.left.is_some(), item.right.is_some()) {
        (Resolution::Skip, _, _) | (_, false, false) => Vec::new(),
        (Resolution::Left, true, _) => vec![SyncAction::CopyLR(rel)],
        (Resolution::Left, false, true) => vec![SyncAction::DeleteR(rel)],
        (Resolution::Right, _, true) => vec![SyncAction::CopyRL(rel)],
        (Resolution::Right, true, false) => vec![SyncAction::DeleteL(rel)],
        (Resolution::KeepBoth, true, true) => vec![
            SyncAction::CopyAs { src_left: false, rel: rel.clone(), dst_left: false, to: alt.to_path_buf() },
            SyncAction::CopyAs { src_left: false, rel: rel.clone(), dst_left: true, to: alt.to_path_buf() },
            SyncAction::CopyLR(rel),
        ],
        // Keep both when one side is gone = keep the one that exists.
        (Resolution::KeepBoth, true, false) => vec![SyncAction::CopyLR(rel)],
        (Resolution::KeepBoth, false, true) => vec![SyncAction::CopyRL(rel)],
    }
}

/// `dir/name (conflict <tag>).ext`, numbered (` 2`, ` 3`, …) while `taken` says the name
/// exists.
pub fn conflict_name(rel: &Path, tag: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let parent = rel.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = rel.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = rel.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut n = 1;
    loop {
        let num = if n == 1 { String::new() } else { format!(" {n}") };
        let cand = parent.join(format!("{stem} (conflict {tag}{num}){ext}"));
        if !taken(&cand) || n > 999 {
            return cand;
        }
        n += 1;
    }
}

// ---------------------------------------------------------------------------------------
// State file.

pub const STATE_VERSION: u32 = 1;

/// Saved base of one folder pair.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncState {
    #[serde(default)]
    pub version: u32,
    pub left: PathBuf,
    pub right: PathBuf,
    /// When the state was recorded.
    #[serde(default, with = "soyuz_core::time::serde_rfc3339::option")]
    pub synced: Option<SystemTime>,
    /// Relative path (`/`-separated) → both sides after the last sync.
    #[serde(default)]
    pub entries: BTreeMap<String, BasePair>,
}

fn key(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

impl SyncState {
    pub fn get(&self, rel: &Path) -> Option<&BasePair> {
        self.entries.get(&key(rel))
    }
}

/// The state after a sync: every path now equal on both sides is recorded (files by size:
/// their contents were just synced, and copies may not keep times); paths in `keep`
/// (skipped, failed, waiting) keep their old base; everything else is dropped.
pub fn next_state(
    old: Option<&SyncState>,
    left_root: &Path,
    right_root: &Path,
    left: &Tree,
    right: &Tree,
    keep: &HashSet<PathBuf>,
    now: SystemTime,
) -> SyncState {
    let mut entries = BTreeMap::new();
    let keys: BTreeSet<&PathBuf> = left.keys().chain(right.keys()).collect();
    for rel in keys {
        let kept =
            keep.contains(rel) || keep.iter().any(|k| rel.starts_with(k) && rel != k && is_dir_in(left, right, k));
        if kept {
            if let Some(b) = old.and_then(|o| o.get(rel)) {
                entries.insert(key(rel), *b);
            }
            continue;
        }
        if let (Some(l), Some(r)) = (left.get(rel), right.get(rel))
            && l.dir == r.dir
            && (l.dir || l.size == r.size)
        {
            entries.insert(key(rel), BasePair { l: *l, r: *r });
        }
    }
    // Kept paths gone from both sides still keep their base (e.g. a skipped deletion).
    if let Some(o) = old {
        for k in keep {
            if let Some(b) = o.get(k) {
                entries.entry(key(k)).or_insert(*b);
            }
        }
    }
    SyncState {
        version: STATE_VERSION,
        left: left_root.to_path_buf(),
        right: right_root.to_path_buf(),
        synced: Some(now),
        entries,
    }
}

/// Is `k` a folder on either side? (Its subtree is kept with it.)
fn is_dir_in(left: &Tree, right: &Tree, k: &Path) -> bool {
    left.get(k).or_else(|| right.get(k)).is_some_and(|s| s.dir)
}

/// FNV-1a of the pair (case-folded: Windows paths are case-insensitive).
fn pair_hash(left: &Path, right: &Path) -> u64 {
    let s = format!("{}\n{}", left.to_string_lossy(), right.to_string_lossy());
    let s = if cfg!(windows) { s.to_lowercase() } else { s };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Where the state of a folder pair lives (inside `dir`, normally the config dir).
pub fn state_path_in(dir: &Path, left: &Path, right: &Path) -> PathBuf {
    dir.join("sync_state").join(format!("{:016x}.json", pair_hash(left, right)))
}

pub fn state_path(left: &Path, right: &Path) -> PathBuf {
    state_path_in(&soyuz_core::config::config_dir(), left, right)
}

/// Load the state of a pair (worker thread). Missing → `None`; a broken file is renamed
/// to `.bad` and treated as missing (the next run is a "first run").
pub fn load_state(path: &Path) -> Option<SyncState> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice::<SyncState>(&bytes) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("{}: unreadable sync state ({e}); starting over", path.display());
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            None
        }
    }
}

/// Write the state atomically (worker thread).
pub fn save_state(path: &Path, s: &SyncState) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(s).map_err(|e| e.to_string())?;
    soyuz_fs::write_atomic(path, &bytes).map_err(|e| e.to_string())
}

/// Scan a tree (worker thread). Our own partial-copy temp files are skipped.
pub fn scan(root: &Path, ctl: &OpControl) -> FsResult<Tree> {
    let mut map = Tree::new();
    soyuz_fs::walk(root, &WalkOptions::default(), ctl, &mut |item| {
        if let WalkItem::Entry { entry, depth } = item
            && depth > 0
            && let Ok(rel) = entry.path.strip_prefix(root)
            && !entry.name.contains(soyuz_fs::copy::PART_MARKER)
        {
            let dir = matches!(entry.kind, EntryKind::Dir);
            map.insert(rel.to_path_buf(), FileState::from_meta(dir, entry.size, entry.modified));
        }
        if ctl.is_cancelled() { Control::Stop } else { Control::Continue }
    })?;
    ctl.checkpoint()?;
    Ok(map)
}

/// Rescan both sides after a sync job and save the new state (worker thread).
pub fn record_after_sync(left: &Path, right: &Path, keep: &HashSet<PathBuf>) -> Result<SyncState, String> {
    record_after_sync_in(&soyuz_core::config::config_dir(), left, right, keep)
}

/// [`record_after_sync`] with the state kept inside `dir` (normally the config dir).
pub fn record_after_sync_in(
    dir: &Path,
    left: &Path,
    right: &Path,
    keep: &HashSet<PathBuf>,
) -> Result<SyncState, String> {
    let ctl = OpControl::new();
    let l = scan(left, &ctl).map_err(|e| e.to_string())?;
    let r = scan(right, &ctl).map_err(|e| e.to_string())?;
    let path = state_path_in(dir, left, right);
    let old = load_state(&path);
    let st = next_state(old.as_ref(), left, right, &l, &r, keep, SystemTime::now());
    save_state(&path, &st)?;
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Decision as D;

    fn f(size: u64, t: i64) -> FileState {
        FileState::file(size, Some(t * 1000))
    }

    fn pair(l: FileState, r: FileState) -> BasePair {
        BasePair { l, r }
    }

    #[test]
    fn three_way_decision_table() {
        let b = pair(f(10, 100), f(10, 100));
        let k = Base::Known(&b);
        let same = f(10, 100);
        let edited = f(12, 200);
        let edited2 = f(14, 300);
        let cases: Vec<(Base<'_>, Option<FileState>, Option<FileState>, Decision)> = vec![
            // No history for the path.
            (Base::Absent, Some(same), None, D::CopyLR),
            (Base::Absent, None, Some(same), D::CopyRL),
            (Base::Absent, Some(same), Some(same), D::Nothing),
            (Base::Absent, Some(edited), Some(same), D::Conflict(ConflictKind::BothCreated)),
            (Base::Absent, None, None, D::Nothing),
            // Known base.
            (k, None, None, D::Nothing),
            (k, Some(same), None, D::DeleteL),
            (k, Some(edited), None, D::Conflict(ConflictKind::ChangedLeftDeletedRight)),
            (k, None, Some(same), D::DeleteR),
            (k, None, Some(edited), D::Conflict(ConflictKind::DeletedLeftChangedRight)),
            (k, Some(same), Some(same), D::Nothing),
            (k, Some(edited), Some(same), D::CopyLR),
            (k, Some(same), Some(edited), D::CopyRL),
            (k, Some(edited), Some(edited), D::Nothing),
            (k, Some(edited), Some(edited2), D::Conflict(ConflictKind::BothChanged)),
            // Time within the 2 s tolerance is not a change.
            (k, Some(FileState::file(10, Some(101_500))), Some(same), D::Nothing),
            // A file replaced by a folder is a change.
            (k, Some(FileState::folder()), None, D::Conflict(ConflictKind::ChangedLeftDeletedRight)),
            // First run (no state file): newer side wins, close times conflict.
            (Base::Unknown, Some(f(10, 500)), Some(f(11, 100)), D::CopyLR),
            (Base::Unknown, Some(f(10, 100)), Some(f(11, 500)), D::CopyRL),
            (Base::Unknown, Some(f(10, 100)), Some(f(11, 101)), D::Conflict(ConflictKind::BothCreated)),
            (Base::Unknown, Some(FileState::folder()), Some(f(1, 1)), D::Conflict(ConflictKind::BothCreated)),
            (Base::Unknown, Some(FileState::folder()), Some(FileState::folder()), D::Nothing),
            (Base::Unknown, Some(same), None, D::CopyLR),
        ];
        for (i, (base, l, r, want)) in cases.into_iter().enumerate() {
            assert_eq!(decide(base, l.as_ref(), r.as_ref(), false), want, "case {i}: {base:?} {l:?} {r:?}");
        }
        // Size only: a touched file with the same size is unchanged.
        assert_eq!(decide(k, Some(&f(10, 999)), Some(&same), true), D::Nothing);
        assert_eq!(decide(k, Some(&f(10, 999)), Some(&same), false), D::CopyLR);
    }

    fn tree(items: &[(&str, FileState)]) -> Tree {
        items.iter().map(|(p, s)| (PathBuf::from(p), *s)).collect()
    }

    fn state(items: &[(&str, FileState)]) -> SyncState {
        SyncState {
            version: STATE_VERSION,
            entries: items.iter().map(|(p, s)| (p.to_string(), pair(*s, *s))).collect(),
            ..SyncState::default()
        }
    }

    #[test]
    fn folders_collapse_or_split() {
        let d = FileState::folder();
        // New folder on the left with new contents: copied as one item.
        let st = state(&[("keep.txt", f(1, 1))]);
        let l = tree(&[("keep.txt", f(1, 1)), ("new", d), ("new/a", f(2, 2)), ("new/b", d), ("new/b/c", f(3, 3))]);
        let r = tree(&[("keep.txt", f(1, 1))]);
        let p = plan(Some(&st), &l, &r, false);
        assert_eq!(p.items.len(), 1);
        assert_eq!((p.items[0].rel.as_path(), p.items[0].decision), (Path::new("new"), D::CopyLR));
        assert_eq!(p.same, 1);
        // Folder deleted on the right, untouched on the left: one delete.
        let st = state(&[("old", d), ("old/a", f(2, 2))]);
        let l = tree(&[("old", d), ("old/a", f(2, 2))]);
        let p = plan(Some(&st), &l, &Tree::new(), false);
        assert_eq!(
            p.items.iter().map(|i| (i.rel.clone(), i.decision)).collect::<Vec<_>>(),
            [(PathBuf::from("old"), D::DeleteL)]
        );
        // …but a file edited inside it meanwhile is a conflict and the folder stays.
        let l = tree(&[("old", d), ("old/a", f(9, 9)), ("old/z", f(1, 1))]);
        let st = state(&[("old", d), ("old/a", f(2, 2)), ("old/z", f(1, 1))]);
        let p = plan(Some(&st), &l, &Tree::new(), false);
        let got: Vec<_> = p.items.iter().map(|i| (i.rel.to_string_lossy().replace('\\', "/"), i.decision)).collect();
        assert_eq!(
            got,
            [
                ("old/a".to_string(), D::Conflict(ConflictKind::ChangedLeftDeletedRight)),
                ("old/z".to_string(), D::DeleteL),
            ]
        );
        assert_eq!(p.keep_base, [PathBuf::from("old")]);
        // File vs folder clash: one conflict for the whole path.
        let l = tree(&[("x", d), ("x/in", f(1, 1))]);
        let r = tree(&[("x", f(5, 5))]);
        let p = plan(None, &l, &r, false);
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].decision, D::Conflict(ConflictKind::BothCreated));
    }

    #[test]
    fn resolutions_become_sync_actions() {
        let item = |l: Option<FileState>, r: Option<FileState>| Item {
            rel: "a/doc.txt".into(),
            left: l,
            right: r,
            decision: D::Conflict(ConflictKind::BothChanged),
        };
        let both = item(Some(f(1, 1)), Some(f(2, 2)));
        let alt = Path::new("a/doc (conflict right).txt");
        assert_eq!(resolve(&both, Resolution::Left, alt), [SyncAction::CopyLR("a/doc.txt".into())]);
        assert_eq!(resolve(&both, Resolution::Right, alt), [SyncAction::CopyRL("a/doc.txt".into())]);
        assert!(resolve(&both, Resolution::Skip, alt).is_empty());
        let kb = resolve(&both, Resolution::KeepBoth, alt);
        assert_eq!(kb.len(), 3);
        assert!(
            matches!(&kb[0], SyncAction::CopyAs { src_left: false, dst_left: false, to, .. } if to == alt),
            "right version saved first"
        );
        assert_eq!(kb[2], SyncAction::CopyLR("a/doc.txt".into()));
        let left_only = item(Some(f(1, 1)), None);
        assert_eq!(resolve(&left_only, Resolution::Right, alt), [SyncAction::DeleteL("a/doc.txt".into())]);
        assert_eq!(resolve(&left_only, Resolution::KeepBoth, alt), [SyncAction::CopyLR("a/doc.txt".into())]);
        let right_only = item(None, Some(f(1, 1)));
        assert_eq!(resolve(&right_only, Resolution::Left, alt), [SyncAction::DeleteR("a/doc.txt".into())]);
    }

    #[test]
    fn conflict_names_are_unique() {
        let taken = |p: &Path| p == Path::new("d/r (conflict right 2026-10-08).txt");
        assert_eq!(
            conflict_name(Path::new("d/r.txt"), "right 2026-10-08", taken),
            PathBuf::from("d/r (conflict right 2026-10-08 2).txt")
        );
        assert_eq!(conflict_name(Path::new("Makefile"), "left", |_| false), PathBuf::from("Makefile (conflict left)"));
    }

    #[test]
    fn next_state_records_equal_paths_and_keeps_skipped() {
        let d = FileState::folder();
        let old = state(&[("a", f(1, 1)), ("skipped", f(2, 2)), ("gone", f(3, 3))]);
        let l = tree(&[("a", f(1, 1)), ("dir", d), ("dir/b", f(4, 40)), ("skipped", f(5, 5)), ("diff", f(6, 6))]);
        // `dir/b` was copied without keeping its time: same size counts as synced.
        let r = tree(&[("a", f(1, 1)), ("dir", d), ("dir/b", f(4, 99)), ("skipped", f(7, 7)), ("diff", f(8, 8))]);
        let keep: HashSet<PathBuf> = [PathBuf::from("skipped"), PathBuf::from("gone")].into();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let s = next_state(Some(&old), Path::new("/L"), Path::new("/R"), &l, &r, &keep, now);
        let keys: Vec<&str> = s.entries.keys().map(String::as_str).collect();
        assert_eq!(keys, ["a", "dir", "dir/b", "gone", "skipped"], "diff (unequal, not kept) is dropped");
        assert_eq!(s.entries["skipped"], old.entries["skipped"], "skipped keeps its old base");
        assert_eq!(s.entries["dir/b"].r.mtime, Some(99_000));
        assert_eq!(s.synced, Some(now));
    }

    #[test]
    fn state_file_round_trip_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = state_path_in(dir.path(), Path::new(r"C:\A"), Path::new(r"D:\B"));
        assert!(p.starts_with(dir.path().join("sync_state")));
        if cfg!(windows) {
            assert_eq!(p, state_path_in(dir.path(), Path::new(r"c:\a"), Path::new(r"d:\b")), "case-insensitive");
        }
        assert_ne!(p, state_path_in(dir.path(), Path::new(r"D:\B"), Path::new(r"C:\A")), "order matters");
        assert_eq!(load_state(&p), None);
        let mut s = state(&[("x/y.txt", f(1, 2)), ("x", FileState::folder())]);
        s.left = r"C:\A".into();
        s.synced = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        save_state(&p, &s).unwrap();
        assert_eq!(load_state(&p), Some(s));
        std::fs::write(&p, b"{not json").unwrap();
        assert_eq!(load_state(&p), None);
        assert!(p.with_extension("json.bad").exists(), "broken state is set aside");
    }

    #[test]
    fn scan_and_record_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let (l, r) = (dir.path().join("L"), dir.path().join("R"));
        std::fs::create_dir_all(l.join("sub")).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(l.join("sub/a.txt"), b"hello").unwrap();
        std::fs::write(r.join("b.txt"), b"x").unwrap();
        let ctl = OpControl::new();
        let lt = scan(&l, &ctl).unwrap();
        assert!(lt[Path::new("sub")].dir);
        assert_eq!(lt[Path::new("sub/a.txt")].size, 5);
        let rt = scan(&r, &ctl).unwrap();
        let p = plan(None, &lt, &rt, false);
        let got: Vec<_> = p.items.iter().map(|i| (i.rel.clone(), i.decision)).collect();
        assert_eq!(got, [(PathBuf::from("b.txt"), D::CopyRL), (PathBuf::from("sub"), D::CopyLR)]);
    }
}
