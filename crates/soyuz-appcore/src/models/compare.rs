//! Folder compare + sync (frontend-agnostic; moved from the GUI's `ui/tools/compare.rs`).
//!
//! `soyuz_search::compare_dirs` runs on a worker; the result becomes a table of rows
//! (only-left, only-right, newer-left, newer-right, differ) with a per-row action (copy →,
//! copy ←, delete, skip). Presets fill the actions (two-way update, mirror left → right,
//! mirror right → left); the plan is previewed and then submitted as one `JobKind::Sync`
//! job. Profiles (left, right, method, preset, schedule) live in
//! [`compare_profiles`](super::compare_profiles).
//!
//! The "Two-way sync" mode plans with the three-way state of
//! [`compare_state`](super::compare_state) instead of a plain compare, so deletions and
//! edits on both sides are told apart; conflicts get their own step (keep left / keep
//! right / keep both / skip, apply to all), and after the job the new state is recorded.
//!
//! [`CompareModel`] is the compare screen's state machine (setup → scanning → review →
//! conflicts / plan → submitted → recording → scanning again); frontends draw it and feed
//! it input. The scan and the state recording run on workers, never on the UI thread.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use serde::{Deserialize, Serialize};
use soyuz_core::OpControl;
use soyuz_jobs::{JobId, JobKind, SyncAction, SyncPlan};

use super::compare_profiles::{self, SyncProfile};
use super::compare_state::{self as sync_state, ConflictKind, Resolution};
use crate::core::AppCore;
use crate::host::Wake;
use crate::models::toasts::Toast;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    OnlyLeft,
    NewerLeft,
    Differ,
    NewerRight,
    OnlyRight,
}

impl Category {
    pub const ALL: [Category; 5] =
        [Category::OnlyLeft, Category::NewerLeft, Category::Differ, Category::NewerRight, Category::OnlyRight];

    pub fn label(self) -> &'static str {
        match self {
            Category::OnlyLeft => tr!("tools-compare-only-left"),
            Category::NewerLeft => tr!("tools-compare-newer-left"),
            Category::Differ => tr!("tools-compare-different"),
            Category::NewerRight => tr!("tools-compare-newer-right"),
            Category::OnlyRight => tr!("tools-compare-only-right"),
        }
    }

    /// Position in [`Category::ALL`] (filter chips, counts).
    pub fn index(self) -> usize {
        Category::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RowAction {
    #[default]
    Skip,
    CopyRight,
    CopyLeft,
    Delete,
    /// Two-way conflicts: keep both versions (the right one under a conflict name).
    KeepBoth,
}

impl RowAction {
    /// The per-row choices in display order (← · → ✕).
    pub const CHOICES: [RowAction; 4] = [RowAction::CopyLeft, RowAction::Skip, RowAction::CopyRight, RowAction::Delete];

    pub fn label(self) -> &'static str {
        match self {
            RowAction::Skip => tr!("common-skip"),
            RowAction::CopyRight => tr!("tools-compare-copy-right"),
            RowAction::CopyLeft => tr!("tools-compare-copy-left"),
            RowAction::Delete => tr!("common-delete"),
            RowAction::KeepBoth => tr!("tools-resolution-both"),
        }
    }
}

/// Is `action` meaningful for `cat`?
pub fn allowed(cat: Category, action: RowAction) -> bool {
    !matches!(
        (cat, action),
        (Category::OnlyLeft, RowAction::CopyLeft)
            | (Category::OnlyRight, RowAction::CopyRight)
            | (Category::OnlyLeft | Category::OnlyRight, RowAction::KeepBoth)
    )
}

/// Row action for a conflict resolution.
pub fn resolution_action(row: &Row, res: Resolution) -> RowAction {
    match res {
        Resolution::Left if row.left.is_some() => RowAction::CopyRight,
        Resolution::Right if row.right.is_some() => RowAction::CopyLeft,
        Resolution::Left | Resolution::Right => RowAction::Delete,
        Resolution::KeepBoth if row.left.is_some() && row.right.is_some() => RowAction::KeepBoth,
        Resolution::KeepBoth if row.left.is_some() => RowAction::CopyRight,
        Resolution::KeepBoth => RowAction::CopyLeft,
        Resolution::Skip => RowAction::Skip,
    }
}

/// The resolution a row's action stands for (conflict rows).
pub fn row_resolution(row: &Row) -> Resolution {
    match row.action {
        RowAction::Skip => Resolution::Skip,
        RowAction::KeepBoth => Resolution::KeepBoth,
        RowAction::CopyRight => Resolution::Left,
        RowAction::CopyLeft => Resolution::Right,
        // Delete = take the side where the item is missing.
        RowAction::Delete if row.left.is_none() => Resolution::Left,
        RowAction::Delete => Resolution::Right,
    }
}

/// The resolution choices of a conflict, in display order.
pub const RESOLUTIONS: [Resolution; 4] = [Resolution::Left, Resolution::Right, Resolution::KeepBoth, Resolution::Skip];

/// Can `res` be chosen for `row`? ("Keep both" needs the item on both sides.)
pub fn resolution_allowed(row: &Row, res: Resolution) -> bool {
    res != Resolution::KeepBoth || (row.left.is_some() && row.right.is_some())
}

/// Tooltip / help line of a resolution choice for `row`.
pub fn resolution_tip(row: &Row, res: Resolution) -> &'static str {
    match res {
        Resolution::Left if row.left.is_none() => tr!("tools-compare-res-delete-right"),
        Resolution::Right if row.right.is_none() => tr!("tools-compare-res-delete-left"),
        Resolution::Left => tr!("tools-compare-res-left"),
        Resolution::Right => tr!("tools-compare-res-right"),
        Resolution::KeepBoth => tr!("tools-compare-res-both"),
        Resolution::Skip => tr!("tools-compare-res-skip"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Copy missing and newer items both ways; conflicts are skipped.
    #[default]
    Update,
    /// Make the right side identical to the left (deletes extra items on the right).
    MirrorRight,
    /// Make the left side identical to the right.
    MirrorLeft,
    /// Three-way sync against the state of the last sync (deletions propagate,
    /// conflicts are asked).
    TwoWay,
    /// Leave actions as set by hand.
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::TwoWay, Preset::Update, Preset::MirrorRight, Preset::MirrorLeft];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Update => tr!("tools-preset-update"),
            Preset::MirrorRight => tr!("tools-preset-mirror-right"),
            Preset::MirrorLeft => tr!("tools-preset-mirror-left"),
            Preset::TwoWay => tr!("tools-preset-two-way"),
            Preset::Custom => tr!("tools-split-custom"),
        }
    }

    /// Default action of a row in this preset.
    pub fn action(self, cat: Category) -> RowAction {
        use Category as C;
        use RowAction as A;
        match self {
            Preset::Update => match cat {
                C::OnlyLeft | C::NewerLeft => A::CopyRight,
                C::OnlyRight | C::NewerRight => A::CopyLeft,
                C::Differ => A::Skip,
            },
            Preset::MirrorRight => match cat {
                C::OnlyRight => A::Delete,
                _ => A::CopyRight,
            },
            Preset::MirrorLeft => match cat {
                C::OnlyLeft => A::Delete,
                _ => A::CopyLeft,
            },
            // Two-way rows get their actions from the decision table.
            Preset::TwoWay | Preset::Custom => A::Skip,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Size + modification time (fast).
    #[default]
    DateSize,
    /// Size only (ignore timestamps).
    Size,
    /// Byte-by-byte contents.
    Content,
}

impl Method {
    pub const ALL: [Method; 3] = [Method::DateSize, Method::Size, Method::Content];

    pub fn label(self) -> &'static str {
        match self {
            Method::DateSize => tr!("tools-method-date-size"),
            Method::Size => tr!("tools-method-size"),
            Method::Content => tr!("tools-method-content"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub dir: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub rel: PathBuf,
    pub cat: Category,
    pub left: Option<Side>,
    pub right: Option<Side>,
    pub action: RowAction,
    /// Two-way sync: the path changed on both sides (needs a choice; skipped until made).
    pub conflict: Option<ConflictKind>,
}

impl Row {
    /// A folder on either side.
    pub fn is_dir(&self) -> bool {
        self.left.as_ref().or(self.right.as_ref()).is_some_and(|m| m.dir)
    }

    /// An unresolved two-way conflict.
    pub fn open_conflict(&self) -> bool {
        self.conflict.is_some() && self.action == RowAction::Skip
    }
}

/// Turn the rows' actions into a sync plan: copies first, then deletions. Actions that
/// make no sense for a row (copy from the side where it is missing) are ignored. "Keep
/// both" saves the right version under a conflict name tagged `tag` on both sides.
pub fn build_plan(left: &Path, right: &Path, rows: &[Row], tag: &str) -> SyncPlan {
    let mut copies = Vec::new();
    let mut deletes = Vec::new();
    let names: HashSet<&Path> = rows.iter().map(|r| r.rel.as_path()).collect();
    let mut alts: HashSet<PathBuf> = HashSet::new();
    for r in rows {
        if !allowed(r.cat, r.action) {
            continue;
        }
        match r.action {
            RowAction::Skip => {}
            RowAction::KeepBoth => {
                let alt = sync_state::conflict_name(&r.rel, tag, |p| names.contains(p) || alts.contains(p));
                let item = sync_state::Item {
                    rel: r.rel.clone(),
                    left: Some(sync_state::FileState::folder()),
                    right: Some(sync_state::FileState::folder()),
                    decision: sync_state::Decision::Conflict(ConflictKind::BothChanged),
                };
                copies.extend(sync_state::resolve(&item, Resolution::KeepBoth, &alt));
                alts.insert(alt);
            }
            RowAction::CopyRight => copies.push(SyncAction::CopyLR(r.rel.clone())),
            RowAction::CopyLeft => copies.push(SyncAction::CopyRL(r.rel.clone())),
            RowAction::Delete => match r.cat {
                Category::OnlyLeft => deletes.push(SyncAction::DeleteL(r.rel.clone())),
                Category::OnlyRight => deletes.push(SyncAction::DeleteR(r.rel.clone())),
                _ => {
                    deletes.push(SyncAction::DeleteL(r.rel.clone()));
                    deletes.push(SyncAction::DeleteR(r.rel.clone()));
                }
            },
        }
    }
    copies.extend(deletes);
    SyncPlan { left: left.to_path_buf(), right: right.to_path_buf(), actions: copies }
}

/// Counts for the plan summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanStats {
    pub to_right: usize,
    pub to_left: usize,
    pub delete_left: usize,
    pub delete_right: usize,
    pub bytes: u64,
}

impl PlanStats {
    /// Every planned copy and deletion.
    pub fn total(&self) -> usize {
        self.to_left + self.to_right + self.delete_left + self.delete_right
    }

    /// Something gets deleted (the run button turns into a danger button).
    pub fn deletes(&self) -> bool {
        self.delete_left + self.delete_right > 0
    }
}

pub fn plan_stats(rows: &[Row]) -> PlanStats {
    let mut s = PlanStats::default();
    for r in rows.iter().filter(|r| allowed(r.cat, r.action)) {
        match r.action {
            RowAction::CopyRight => {
                s.to_right += 1;
                s.bytes += r.left.as_ref().filter(|m| !m.dir).map_or(0, |m| m.size);
            }
            RowAction::CopyLeft => {
                s.to_left += 1;
                s.bytes += r.right.as_ref().filter(|m| !m.dir).map_or(0, |m| m.size);
            }
            RowAction::Delete => {
                if r.left.is_some() {
                    s.delete_left += 1;
                }
                if r.right.is_some() {
                    s.delete_right += 1;
                }
            }
            RowAction::KeepBoth => {
                s.to_right += 1;
                s.to_left += 1;
                s.bytes += [&r.left, &r.right]
                    .iter()
                    .filter_map(|m| m.as_ref())
                    .filter(|m| !m.dir)
                    .map(|m| m.size)
                    .sum::<u64>();
            }
            RowAction::Skip => {}
        }
    }
    s
}

/// Build rows from a compare result (metadata from `stat`).
pub fn rows_from(
    res: &soyuz_search::CompareResult,
    method: Method,
    stat: impl Fn(&Path, bool) -> Option<Side>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let groups: [(&Vec<PathBuf>, Category); 5] = [
        (&res.only_left, Category::OnlyLeft),
        (&res.newer_left, Category::NewerLeft),
        (&res.differ, Category::Differ),
        (&res.newer_right, Category::NewerRight),
        (&res.only_right, Category::OnlyRight),
    ];
    for (list, cat) in groups {
        for rel in list {
            let left = (cat != Category::OnlyRight).then(|| stat(rel, true)).flatten();
            let right = (cat != Category::OnlyLeft).then(|| stat(rel, false)).flatten();
            // "Size only": equal sizes count as identical even when the dates differ.
            if method == Method::Size
                && matches!(cat, Category::NewerLeft | Category::NewerRight)
                && let (Some(l), Some(r)) = (&left, &right)
                && !l.dir
                && !r.dir
                && l.size == r.size
            {
                continue;
            }
            rows.push(Row { rel: rel.clone(), cat, left, right, action: RowAction::Skip, conflict: None });
        }
    }
    rows.sort_by(|a, b| a.rel.cmp(&b.rel));
    rows
}

/// Rows of a two-way plan: actions from the decision table, conflicts skipped until the
/// user chooses.
pub fn rows_from_two_way(plan: &sync_state::TwoWayPlan) -> Vec<Row> {
    use sync_state::Decision as D;
    let side = |s: &Option<sync_state::FileState>| s.map(|f| Side { size: f.size, modified: f.modified(), dir: f.dir });
    plan.items
        .iter()
        .filter(|i| i.decision != D::Nothing)
        .map(|i| {
            let cat = match (&i.left, &i.right) {
                (Some(_), None) => Category::OnlyLeft,
                (None, Some(_)) => Category::OnlyRight,
                (Some(l), Some(r)) => match (l.mtime, r.mtime) {
                    (Some(a), Some(b)) if a - b > sync_state::TOLERANCE_MS => Category::NewerLeft,
                    (Some(a), Some(b)) if b - a > sync_state::TOLERANCE_MS => Category::NewerRight,
                    _ => Category::Differ,
                },
                (None, None) => Category::Differ,
            };
            let (action, conflict) = match i.decision {
                D::CopyLR => (RowAction::CopyRight, None),
                D::CopyRL => (RowAction::CopyLeft, None),
                D::DeleteL | D::DeleteR => (RowAction::Delete, None),
                D::Conflict(k) => (RowAction::Skip, Some(k)),
                D::Nothing => (RowAction::Skip, None),
            };
            Row { rel: i.rel.clone(), cat, left: side(&i.left), right: side(&i.right), action, conflict }
        })
        .collect()
}

/// Why a two-way row has its action (the table's tag).
pub fn two_way_reason(row: &Row) -> &'static str {
    match (row.left.is_some(), row.right.is_some(), row.action) {
        (true, false, RowAction::Delete) => tr!("tools-reason-deleted-right"),
        (false, true, RowAction::Delete) => tr!("tools-reason-deleted-left"),
        (true, false, _) => tr!("tools-reason-new-left"),
        (false, true, _) => tr!("tools-reason-new-right"),
        (_, _, RowAction::CopyRight) => tr!("tools-reason-changed-left"),
        (_, _, RowAction::CopyLeft) => tr!("tools-reason-changed-right"),
        _ => row.cat.label(),
    }
}

/// Extra results of a two-way compare (used after the sync to record the new state).
#[derive(Clone, Debug, Default)]
pub struct TwoWayCtx {
    /// Folders waiting for their contents to be decided (keep their base).
    pub keep_base: Vec<PathBuf>,
    /// No state file existed (first two-way run of this pair).
    pub first_run: bool,
    /// Conflict-name tag for "keep both" copies, e.g. `right 2026-10-08 1430`.
    pub tag: String,
}

/// Result of [`compute`].
pub struct Computed {
    pub rows: Vec<Row>,
    pub same: u64,
    pub two_way: Option<TwoWayCtx>,
}

/// Compare two folders for `preset` (blocking; worker thread). Two-way plans against the
/// saved state; the other presets use `compare_dirs` and fill the preset's actions.
pub fn compute(l: &Path, r: &Path, method: Method, preset: Preset, ctl: &OpControl) -> Result<Computed, String> {
    compute_in(&soyuz_core::config::config_dir(), l, r, method, preset, ctl)
}

/// [`compute`] with the two-way state kept inside `state_dir` (normally the config dir).
pub fn compute_in(
    state_dir: &Path,
    l: &Path,
    r: &Path,
    method: Method,
    preset: Preset,
    ctl: &OpControl,
) -> Result<Computed, String> {
    if preset == Preset::TwoWay {
        let lt = sync_state::scan(l, ctl).map_err(|e| e.to_string())?;
        let rt = sync_state::scan(r, ctl).map_err(|e| e.to_string())?;
        let state_path = sync_state::state_path_in(state_dir, l, r);
        let state = sync_state::load_state(&state_path);
        let plan = sync_state::plan(state.as_ref(), &lt, &rt, method == Method::Size);
        let tag = format!("right {}", chrono::Local::now().format("%Y-%m-%d %H%M"));
        let ctx = TwoWayCtx { keep_base: plan.keep_base.clone(), first_run: state.is_none(), tag };
        let rows = rows_from_two_way(&plan);
        if rows.is_empty() {
            // Already in sync: remember this as the base, or later deletions would look
            // like new files on the other side.
            let keep: HashSet<PathBuf> = plan.keep_base.iter().cloned().collect();
            let next = sync_state::next_state(state.as_ref(), l, r, &lt, &rt, &keep, SystemTime::now());
            if state.as_ref().map(|s| &s.entries) != Some(&next.entries) {
                sync_state::save_state(&state_path, &next)?;
            }
        }
        return Ok(Computed { rows, same: plan.same, two_way: Some(ctx) });
    }
    let res = soyuz_search::compare_dirs(l, r, method == Method::Content, ctl).map_err(|e| e.to_string())?;
    let stat = |rel: &Path, left: bool| {
        let p = if left { l.join(rel) } else { r.join(rel) };
        soyuz_fs::stat(&p, false).ok().map(|e| Side { size: e.size, modified: e.modified, dir: e.is_dir_like() })
    };
    let mut rows = rows_from(&res, method, stat);
    if preset != Preset::Custom {
        for row in &mut rows {
            row.action = preset.action(row.cat);
        }
    }
    Ok(Computed { rows, same: res.same, two_way: None })
}

/// Paths to keep at their old base after a two-way sync: everything not acted on
/// (skipped rows, unresolved conflicts), folders waiting on their contents and the paths
/// that failed (`failed` are absolute).
pub fn keep_after_sync(
    rows: &[Row],
    ctx: &TwoWayCtx,
    left: &Path,
    right: &Path,
    failed: &[PathBuf],
) -> HashSet<PathBuf> {
    let mut keep: HashSet<PathBuf> = rows
        .iter()
        .filter(|r| r.action == RowAction::Skip || !allowed(r.cat, r.action))
        .map(|r| r.rel.clone())
        .collect();
    keep.extend(ctx.keep_base.iter().cloned());
    for f in failed {
        if let Ok(rel) = f.strip_prefix(left).or_else(|_| f.strip_prefix(right)) {
            keep.insert(rel.to_path_buf());
        }
    }
    keep
}

// ---------------------------------------------------------------------------------------
// The compare screen's state machine.

/// Where the compare is (the rows of the last result stay in the model meanwhile).
pub enum Stage {
    /// Nothing running: before the first scan, or after a cancelled one.
    Setup,
    /// The compare worker is running.
    Scanning { rx: Receiver<Result<Computed, String>>, ctl: Arc<OpControl>, started: Instant },
    /// A result is in: the rows can be reviewed ([`ReviewStep`]).
    Review,
    /// The compare failed (or the paths are not usable).
    Failed(String),
    /// A sync job is running; the folders are compared again when it ends.
    Submitted(JobId),
    /// Two-way: the new sync state is being recorded after the job.
    Recording(Receiver<Result<(), String>>),
}

/// Which part of the review is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReviewStep {
    /// The rows with their actions.
    #[default]
    Table,
    /// The plan preview (the list of sync actions) with the run button.
    Plan,
    /// Two-way conflicts: one choice per conflict, or apply to all.
    Conflicts,
}

/// What a two-way sync job needs once it is done (to record the new state).
struct PendingRecord {
    left: PathBuf,
    right: PathBuf,
    rows: Vec<Row>,
    ctx: TwoWayCtx,
}

/// Folder compare + sync state for one compare screen / window.
pub struct CompareModel {
    /// Left folder as typed.
    pub left: String,
    /// Right folder as typed.
    pub right: String,
    pub method: Method,
    pub preset: Preset,
    /// The review part shown (meaningful while [`Stage::Review`] or a stale result shows).
    pub step: ReviewStep,
    /// Category filter, indexed like [`Category::ALL`].
    pub show: [bool; 5],
    /// Name filter (case-insensitive substring of the relative path).
    pub search: String,
    /// Keyboard cursor: an index into [`CompareModel::visible_rows`] (frontends clamp it).
    pub cursor: usize,
    /// "Save as profile" name.
    pub profile_name: String,
    stage: Stage,
    rows: Vec<Row>,
    same: u64,
    profiles: Vec<SyncProfile>,
    /// Paths the current rows were computed for.
    compared: Option<(PathBuf, PathBuf)>,
    two_way: Option<TwoWayCtx>,
    pending_record: Option<PendingRecord>,
    /// Where two-way states live (the config dir unless a test says otherwise).
    state_dir: PathBuf,
    wake: Wake,
}

/// Path text for the folder fields (no verbatim prefix).
fn show_path(p: &Path) -> String {
    soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned()
}

impl CompareModel {
    /// A compare of `left` with `right`; starts at once when `right` is given. `wake`
    /// redraws the frontend when a worker finishes (`models::tools::wake()`).
    pub fn new(left: PathBuf, right: Option<PathBuf>, profiles: Vec<SyncProfile>, wake: Wake) -> Self {
        Self::new_in(compare_profiles::store_dir(), left, right, profiles, wake)
    }

    /// [`CompareModel::new`] keeping two-way states inside `state_dir` (`new` uses
    /// [`compare_profiles::store_dir`], the config dir unless a test moved it).
    pub fn new_in(
        state_dir: PathBuf,
        left: PathBuf,
        right: Option<PathBuf>,
        profiles: Vec<SyncProfile>,
        wake: Wake,
    ) -> Self {
        let mut m = CompareModel {
            left: show_path(&left),
            right: right.as_deref().map(show_path).unwrap_or_default(),
            method: Method::DateSize,
            preset: Preset::Update,
            step: ReviewStep::Table,
            show: [true; 5],
            search: String::new(),
            cursor: 0,
            profile_name: String::new(),
            stage: Stage::Setup,
            rows: Vec::new(),
            same: 0,
            profiles,
            compared: None,
            two_way: None,
            pending_record: None,
            state_dir,
            wake,
        };
        if right.is_some() {
            m.start();
        }
        m
    }

    /// Open with a saved profile loaded and compared (profiles window, conflict toasts).
    pub fn with_profile(pr: &SyncProfile, profiles: Vec<SyncProfile>, wake: Wake) -> Self {
        let mut m = CompareModel::new(pr.left.clone(), None, profiles, wake);
        m.load_profile(pr.clone());
        m
    }

    /// Take a profile's folders, method and mode, and compare.
    pub fn load_profile(&mut self, pr: SyncProfile) {
        self.left = show_path(&pr.left);
        self.right = show_path(&pr.right);
        self.method = pr.method;
        self.preset = pr.preset;
        self.profile_name = pr.name;
        self.start();
    }

    // ---- Reading.

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Identical paths of the last result.
    pub fn same(&self) -> u64 {
        self.same
    }

    /// Two-way extras of the last result (`None` for the other modes).
    pub fn two_way(&self) -> Option<&TwoWayCtx> {
        self.two_way.as_ref()
    }

    /// No two-way state existed for this pair (the rows are a first-run guess).
    pub fn first_run(&self) -> bool {
        self.two_way.as_ref().is_some_and(|t| t.first_run)
    }

    /// The folders the rows are relative to.
    pub fn compared(&self) -> Option<&(PathBuf, PathBuf)> {
        self.compared.as_ref()
    }

    pub fn profiles(&self) -> &[SyncProfile] {
        &self.profiles
    }

    pub fn is_scanning(&self) -> bool {
        matches!(self.stage, Stage::Scanning { .. })
    }

    /// Time since the running scan started.
    pub fn scan_elapsed(&self) -> Option<Duration> {
        match &self.stage {
            Stage::Scanning { started, .. } => Some(started.elapsed()),
            _ => None,
        }
    }

    /// The error of a failed compare.
    pub fn error(&self) -> Option<&str> {
        match &self.stage {
            Stage::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// The submitted sync job.
    pub fn job(&self) -> Option<JobId> {
        match self.stage {
            Stage::Submitted(j) => Some(j),
            _ => None,
        }
    }

    /// Does the row pass the category chips and the name filter?
    pub fn visible(&self, r: &Row) -> bool {
        if !self.show[r.cat.index()] {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        q.is_empty() || r.rel.to_string_lossy().to_lowercase().contains(&q)
    }

    /// Indexes of the visible rows.
    pub fn visible_rows(&self) -> Vec<usize> {
        (0..self.rows.len()).filter(|&i| self.visible(&self.rows[i])).collect()
    }

    /// Rows per category (indexed like [`Category::ALL`]).
    pub fn counts(&self) -> [usize; 5] {
        let mut counts = [0usize; 5];
        for r in &self.rows {
            counts[r.cat.index()] += 1;
        }
        counts
    }

    pub fn stats(&self) -> PlanStats {
        plan_stats(&self.rows)
    }

    /// Indexes of the two-way conflict rows.
    pub fn conflict_rows(&self) -> Vec<usize> {
        (0..self.rows.len()).filter(|&i| self.rows[i].conflict.is_some()).collect()
    }

    /// (conflicts, still undecided).
    pub fn conflict_counts(&self) -> (usize, usize) {
        let total = self.rows.iter().filter(|r| r.conflict.is_some()).count();
        let open = self.rows.iter().filter(|r| r.open_conflict()).count();
        (total, open)
    }

    /// Conflict-name tag for "keep both" copies.
    pub fn tag(&self) -> String {
        self.two_way.as_ref().map_or("right", |t| t.tag.as_str()).to_string()
    }

    /// The sync plan of the current rows (`None` before the first result).
    pub fn plan(&self) -> Option<SyncPlan> {
        let (l, r) = self.compared.as_ref()?;
        Some(build_plan(l, r, &self.rows, &self.tag()))
    }

    // ---- Scanning.

    /// Compare (again): cancels a running scan; fails on empty or equal paths.
    pub fn start(&mut self) {
        if let Stage::Scanning { ctl, .. } = &self.stage {
            ctl.cancel();
        }
        let (l, r) = (PathBuf::from(self.left.trim()), PathBuf::from(self.right.trim()));
        if l.as_os_str().is_empty() || r.as_os_str().is_empty() {
            self.stage = Stage::Failed(tr!("tools-compare-err-choose").into());
            return;
        }
        if l == r {
            self.stage = Stage::Failed(tr!("tools-compare-err-same").into());
            return;
        }
        let ctl = OpControl::new();
        let (tx, rx) = bounded(1);
        let c2 = ctl.clone();
        let (method, preset) = (self.method, self.preset);
        let wake = self.wake.clone();
        let (l2, r2, dir) = (l.clone(), r.clone(), self.state_dir.clone());
        let spawned = std::thread::Builder::new().name("soyuz-compare".into()).spawn(move || {
            let out = compute_in(&dir, &l2, &r2, method, preset, &c2);
            if !c2.is_cancelled() {
                let _ = tx.send(out);
            }
            wake();
        });
        if spawned.is_ok() {
            self.compared = Some((l, r));
            self.stage = Stage::Scanning { rx, ctl, started: Instant::now() };
            self.step = ReviewStep::Table;
        }
    }

    /// Cancel a running scan (its result is dropped) and go back to setup.
    pub fn cancel(&mut self) {
        if let Stage::Scanning { ctl, .. } = &self.stage {
            ctl.cancel();
            self.stage = Stage::Setup;
        }
    }

    /// The screen closes: stop a running scan.
    pub fn close(&mut self) {
        if let Stage::Scanning { ctl, .. } = &self.stage {
            ctl.cancel();
        }
    }

    /// Take the scan result when it is in (no job / recording handling; see [`Self::poll`]).
    /// Returns `true` when the stage changed.
    pub fn poll_scan(&mut self) -> bool {
        let Stage::Scanning { rx, .. } = &self.stage else { return false };
        match rx.try_recv() {
            Ok(Ok(c)) => {
                self.rows = c.rows;
                self.same = c.same;
                self.two_way = c.two_way;
                self.stage = Stage::Review;
                self.cursor = 0;
            }
            Ok(Err(e)) => self.stage = Stage::Failed(e),
            Err(TryRecvError::Disconnected) => self.stage = Stage::Setup,
            Err(TryRecvError::Empty) => return false,
        }
        true
    }

    /// Once per frame: the scan result, the end of the sync job (then record the two-way
    /// state and compare again) and the end of the recording. Returns `true` when the
    /// stage changed.
    pub fn poll(&mut self, core: &mut AppCore) -> bool {
        let mut changed = self.poll_scan();
        if let Stage::Submitted(job) = self.stage {
            let snap = core.fileops.jobs.snapshot(job);
            if snap.is_none_or(|s| s.state.is_terminal()) {
                let failed = snap.and_then(|s| match &s.state {
                    soyuz_jobs::JobState::Done(sum) => Some(sum.errors.iter().map(|(p, _)| p.clone()).collect()),
                    _ => None,
                });
                match (self.pending_record.take(), failed) {
                    // Record the new two-way state, then compare again.
                    (Some(rec), Some(failed)) => self.record_after_sync(rec, failed),
                    _ => self.start(),
                }
                changed = true;
            }
        }
        if let Stage::Recording(rx) = &self.stage {
            match rx.try_recv() {
                Ok(res) => {
                    if let Err(e) = res {
                        core.ui.toasts.push(Toast::warning(tr!("tools-compare-state-save-failed")).body(e));
                    }
                    self.start();
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    self.start();
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        changed
    }

    /// Rescan both sides and save the two-way state on a worker.
    fn record_after_sync(&mut self, rec: PendingRecord, failed: Vec<PathBuf>) {
        let keep = keep_after_sync(&rec.rows, &rec.ctx, &rec.left, &rec.right, &failed);
        let (tx, rx) = bounded(1);
        let wake = self.wake.clone();
        let dir = self.state_dir.clone();
        let spawned = std::thread::Builder::new().name("soyuz-sync-state".into()).spawn(move || {
            let _ = tx.send(sync_state::record_after_sync_in(&dir, &rec.left, &rec.right, &keep).map(|_| ()));
            wake();
        });
        self.stage = if spawned.is_ok() { Stage::Recording(rx) } else { Stage::Setup };
    }

    // ---- Setup edits.

    /// Swap the folders (compares again when a result is shown).
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.left, &mut self.right);
        if matches!(self.stage, Stage::Review) {
            self.start();
        }
    }

    /// Change the compare method (compares again unless nothing ran yet).
    pub fn set_method(&mut self, m: Method) {
        if self.method == m {
            return;
        }
        self.method = m;
        if !matches!(self.stage, Stage::Setup) {
            self.start();
        }
    }

    /// Change the sync mode: two-way plans differently, so switching in or out compares
    /// again; the other modes refill the actions.
    pub fn set_preset(&mut self, pr: Preset) {
        if self.preset == pr {
            return;
        }
        let replan = self.preset == Preset::TwoWay
            || pr == Preset::TwoWay
            || (self.preset == Preset::Custom && self.two_way.is_some());
        self.preset = pr;
        if replan && !matches!(self.stage, Stage::Setup) {
            self.start();
        } else {
            self.apply_preset();
        }
    }

    /// Fill every row's action from the current preset (not for two-way / custom).
    pub fn apply_preset(&mut self) {
        if matches!(self.preset, Preset::Custom | Preset::TwoWay) {
            return;
        }
        for r in &mut self.rows {
            r.action = self.preset.action(r.cat);
        }
    }

    /// Show or hide a category.
    pub fn toggle_filter(&mut self, cat: Category) {
        let i = cat.index();
        self.show[i] = !self.show[i];
    }

    // ---- Review edits.

    /// Set one row's action (ignored when it makes no sense for the row). A hand edit
    /// turns the mode into "custom" (two-way keeps its mode).
    pub fn set_action(&mut self, i: usize, a: RowAction) -> bool {
        let Some(r) = self.rows.get_mut(i) else { return false };
        if !allowed(r.cat, a) {
            return false;
        }
        r.action = a;
        if self.two_way.is_none() {
            self.preset = Preset::Custom;
        }
        true
    }

    /// Set the action of several rows (the bulk buttons over the visible rows).
    pub fn set_actions(&mut self, rows: &[usize], a: RowAction) {
        for &i in rows {
            if let Some(r) = self.rows.get_mut(i)
                && allowed(r.cat, a)
            {
                r.action = a;
            }
        }
        if self.two_way.is_none() {
            self.preset = Preset::Custom;
        }
    }

    /// Next allowed action of a row in [`RowAction::CHOICES`] order (keyboard cycling).
    pub fn cycle_action(&mut self, i: usize) -> bool {
        let Some(r) = self.rows.get(i) else { return false };
        let at = RowAction::CHOICES.iter().position(|a| *a == r.action);
        let n = RowAction::CHOICES.len();
        let start = at.map_or(0, |k| k + 1);
        let next = (0..n).map(|k| RowAction::CHOICES[(start + k) % n]).find(|a| allowed(r.cat, *a));
        next.is_some_and(|a| self.set_action(i, a))
    }

    /// Answer one two-way conflict.
    pub fn resolve(&mut self, i: usize, res: Resolution) -> bool {
        let Some(r) = self.rows.get(i) else { return false };
        if r.conflict.is_none() || !resolution_allowed(r, res) {
            return false;
        }
        self.rows[i].action = resolution_action(&self.rows[i], res);
        true
    }

    /// Answer every conflict at once; `None` keeps the newer side.
    pub fn resolve_all(&mut self, choice: Option<Resolution>) {
        for i in self.conflict_rows() {
            let row = &self.rows[i];
            let res = choice.unwrap_or_else(|| {
                match (row.left.as_ref().and_then(|s| s.modified), row.right.as_ref().and_then(|s| s.modified)) {
                    (Some(a), Some(b)) if b > a => Resolution::Right,
                    (None, Some(_)) => Resolution::Right,
                    _ => Resolution::Left,
                }
            });
            self.rows[i].action = resolution_action(&self.rows[i], res);
        }
    }

    // ---- Running.

    /// Submit the current plan as one `JobKind::Sync` job (`None`: nothing to do).
    pub fn submit(&mut self, core: &mut AppCore) -> Option<JobId> {
        let plan = self.plan().filter(|p| !p.actions.is_empty())?;
        Some(self.submit_plan(core, plan))
    }

    /// Submit `plan` (built from these rows) as one `JobKind::Sync` job through the jobs
    /// engine; the folders are compared again once it ends.
    pub fn submit_plan(&mut self, core: &mut AppCore, plan: SyncPlan) -> JobId {
        let label = tr!(
            "tools-compare-job-label",
            left = soyuz_core::entry::display_name(&plan.left),
            right = soyuz_core::entry::display_name(&plan.right)
        );
        self.pending_record = self.two_way.clone().map(|ctx| PendingRecord {
            left: plan.left.clone(),
            right: plan.right.clone(),
            rows: self.rows.clone(),
            ctx,
        });
        let mut spec = core.fileops.jobs.spec(JobKind::Sync { plan });
        spec.options.label = Some(label);
        let job = crate::models::archive::submit_job(core, spec);
        // The folders change: compare again once the job is done.
        self.rows.clear();
        self.stage = Stage::Submitted(job);
        self.step = ReviewStep::Table;
        job
    }

    // ---- Profiles.

    /// Can the current setup be saved as a profile?
    pub fn can_save_profile(&self) -> bool {
        !self.profile_name.trim().is_empty() && !self.left.trim().is_empty() && !self.right.trim().is_empty()
    }

    /// The current setup as a profile named [`Self::profile_name`] (keeps the schedule of
    /// a profile with that name; a hand-edited table saves as its base mode).
    pub fn current_profile(&self) -> Option<SyncProfile> {
        if !self.can_save_profile() {
            return None;
        }
        let name = self.profile_name.trim().to_string();
        let schedule = self.profiles.iter().find(|p| p.name == name).map(|p| p.schedule.clone());
        let preset = match self.preset {
            Preset::Custom if self.two_way.is_some() => Preset::TwoWay,
            Preset::Custom => Preset::Update,
            other => other,
        };
        Some(SyncProfile {
            name,
            left: PathBuf::from(self.left.trim()),
            right: PathBuf::from(self.right.trim()),
            method: self.method,
            preset,
            schedule: schedule.unwrap_or_default(),
        })
    }

    /// Save the current setup as a profile (replacing one with the same name) and store
    /// the profiles.
    pub fn save_profile(&mut self) -> bool {
        let Some(prof) = self.current_profile() else { return false };
        self.profiles.retain(|p| p.name != prof.name);
        self.profiles.push(prof);
        self.profiles.sort_by_key(|a| a.name.to_lowercase());
        compare_profiles::set_profiles(self.profiles.clone());
        true
    }

    /// Load the `i`-th profile and compare.
    pub fn load_profile_at(&mut self, i: usize) {
        if let Some(pr) = self.profiles.get(i).cloned() {
            self.load_profile(pr);
        }
    }

    /// Delete the `i`-th profile and store the profiles.
    pub fn delete_profile_at(&mut self, i: usize) {
        if i < self.profiles.len() {
            self.profiles.remove(i);
            compare_profiles::set_profiles(self.profiles.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(rel: &str, cat: Category, action: RowAction) -> Row {
        let side = Some(Side { size: 10, modified: None, dir: false });
        Row {
            rel: rel.into(),
            cat,
            left: (cat != Category::OnlyRight).then(|| side.clone()).flatten(),
            right: (cat != Category::OnlyLeft).then(|| side.clone()).flatten(),
            action,
            conflict: None,
        }
    }

    #[test]
    fn row_actions_map_to_sync_plan() {
        let rows = vec![
            row("a.txt", Category::OnlyLeft, RowAction::CopyRight),
            row("b.txt", Category::OnlyRight, RowAction::CopyLeft),
            row("c.txt", Category::NewerLeft, RowAction::CopyRight),
            row("d.txt", Category::NewerRight, RowAction::CopyLeft),
            row("e.txt", Category::OnlyRight, RowAction::Delete),
            row("f.txt", Category::OnlyLeft, RowAction::Delete),
            row("g.txt", Category::Differ, RowAction::Delete),
            row("h.txt", Category::Differ, RowAction::Skip),
            // Invalid: nothing to copy from the right.
            row("i.txt", Category::OnlyLeft, RowAction::CopyLeft),
        ];
        let plan = build_plan(Path::new("/L"), Path::new("/R"), &rows, "right");
        assert_eq!(plan.left, PathBuf::from("/L"));
        assert_eq!(plan.right, PathBuf::from("/R"));
        use SyncAction::*;
        assert_eq!(
            plan.actions,
            vec![
                CopyLR("a.txt".into()),
                CopyRL("b.txt".into()),
                CopyLR("c.txt".into()),
                CopyRL("d.txt".into()),
                DeleteR("e.txt".into()),
                DeleteL("f.txt".into()),
                DeleteL("g.txt".into()),
                DeleteR("g.txt".into()),
            ],
            "copies first, then deletions; invalid and skipped rows dropped"
        );
        let s = plan_stats(&rows);
        assert_eq!((s.to_right, s.to_left, s.delete_left, s.delete_right), (2, 2, 2, 2));
        assert_eq!(s.bytes, 40);
        assert_eq!(s.total(), 8);
        assert!(s.deletes());
    }

    #[test]
    fn presets_fill_actions() {
        use Category as C;
        use RowAction as A;
        assert_eq!(Preset::Update.action(C::OnlyLeft), A::CopyRight);
        assert_eq!(Preset::Update.action(C::OnlyRight), A::CopyLeft);
        assert_eq!(Preset::Update.action(C::Differ), A::Skip);
        assert_eq!(Preset::MirrorRight.action(C::OnlyRight), A::Delete);
        assert_eq!(Preset::MirrorRight.action(C::NewerRight), A::CopyRight, "mirror overwrites newer targets");
        assert_eq!(Preset::MirrorLeft.action(C::OnlyLeft), A::Delete);
        assert_eq!(Preset::MirrorLeft.action(C::Differ), A::CopyLeft);
        for p in Preset::ALL {
            for c in Category::ALL {
                assert!(allowed(c, p.action(c)), "{p:?} gives a valid action for {c:?}");
            }
        }
        // Mirror plan for a typical result.
        let mut rows = vec![
            row("new", C::OnlyLeft, A::Skip),
            row("old", C::OnlyRight, A::Skip),
            row("chg", C::NewerRight, A::Skip),
        ];
        for r in &mut rows {
            r.action = Preset::MirrorRight.action(r.cat);
        }
        let plan = build_plan(Path::new("/a"), Path::new("/b"), &rows, "right");
        assert_eq!(
            plan.actions,
            vec![SyncAction::CopyLR("new".into()), SyncAction::CopyLR("chg".into()), SyncAction::DeleteR("old".into())]
        );
    }

    #[test]
    fn rows_from_compare_result_and_size_method() {
        let res = soyuz_search::CompareResult {
            only_left: vec!["x".into()],
            only_right: vec!["y".into()],
            newer_left: vec!["same_size".into(), "bigger".into()],
            newer_right: vec![],
            differ: vec!["d".into()],
            same: 3,
        };
        let stat = |rel: &Path, left: bool| {
            let size = if rel == Path::new("bigger") && left { 20 } else { 10 };
            Some(Side { size, modified: None, dir: false })
        };
        let rows = rows_from(&res, Method::DateSize, stat);
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().find(|r| r.rel == Path::new("x")).unwrap().right.is_none());
        let rows = rows_from(&res, Method::Size, stat);
        assert_eq!(rows.len(), 4, "equal sizes are identical when comparing by size only");
        assert!(rows.iter().all(|r| r.rel != Path::new("same_size")));
    }

    #[test]
    fn two_way_rows_conflicts_and_keep_both_plan() {
        use sync_state::{Decision as D, FileState, Item, TwoWayPlan};
        let f = |s| Some(FileState::file(s, Some(1_000_000)));
        let plan = TwoWayPlan {
            items: vec![
                Item { rel: "new.txt".into(), left: f(1), right: None, decision: D::CopyLR },
                Item { rel: "gone.txt".into(), left: None, right: f(2), decision: D::DeleteR },
                Item {
                    rel: "both.txt".into(),
                    left: f(3),
                    right: f(4),
                    decision: D::Conflict(ConflictKind::BothChanged),
                },
            ],
            keep_base: vec!["waiting".into()],
            same: 0,
        };
        let mut rows = rows_from_two_way(&plan);
        assert_eq!(rows[0].action, RowAction::CopyRight);
        assert_eq!((rows[1].cat, rows[1].action), (Category::OnlyRight, RowAction::Delete));
        assert_eq!((rows[2].action, rows[2].conflict), (RowAction::Skip, Some(ConflictKind::BothChanged)));
        assert_eq!(row_resolution(&rows[2]), Resolution::Skip);
        assert_eq!(two_way_reason(&rows[0]), "New left");
        assert_eq!(two_way_reason(&rows[1]), "Deleted left");
        // Choices map back and forth.
        for res in RESOLUTIONS {
            rows[2].action = resolution_action(&rows[2], res);
            assert_eq!(row_resolution(&rows[2]), res);
        }
        rows[2].action = resolution_action(&rows[2], Resolution::KeepBoth);
        let p = build_plan(Path::new("/L"), Path::new("/R"), &rows, "right 2026-10-08 1200");
        let alt = PathBuf::from("both (conflict right 2026-10-08 1200).txt");
        assert_eq!(
            p.actions,
            vec![
                SyncAction::CopyLR("new.txt".into()),
                SyncAction::CopyAs { src_left: false, rel: "both.txt".into(), dst_left: false, to: alt.clone() },
                SyncAction::CopyAs { src_left: false, rel: "both.txt".into(), dst_left: true, to: alt },
                SyncAction::CopyLR("both.txt".into()),
                SyncAction::DeleteR("gone.txt".into()),
            ]
        );
        // Skipped conflicts, waiting folders and failures keep their base.
        rows[2].action = RowAction::Skip;
        let ctx = TwoWayCtx { keep_base: vec!["waiting".into()], ..TwoWayCtx::default() };
        let keep = keep_after_sync(&rows, &ctx, Path::new("/L"), Path::new("/R"), &[PathBuf::from("/L/new.txt")]);
        let mut keep: Vec<_> = keep.into_iter().collect();
        keep.sort();
        assert_eq!(keep, [PathBuf::from("both.txt"), "new.txt".into(), "waiting".into()]);
    }

    // ---- CompareModel.

    /// Two temp trees: `L` and `R` plus a state dir, all inside one temp dir.
    struct Trees {
        _tmp: tempfile::TempDir,
        l: PathBuf,
        r: PathBuf,
        state: PathBuf,
    }

    fn trees() -> Trees {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r, state) = (tmp.path().join("L"), tmp.path().join("R"), tmp.path().join("cfg"));
        std::fs::create_dir_all(l.join("sub")).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(l.join("only-left.txt"), b"left").unwrap();
        std::fs::write(l.join("sub/deep.txt"), b"deep").unwrap();
        std::fs::write(r.join("only-right.txt"), b"right").unwrap();
        std::fs::write(l.join("same.txt"), b"same").unwrap();
        std::fs::write(r.join("same.txt"), b"same").unwrap();
        let t = SystemTime::now() - Duration::from_secs(3600);
        for p in [l.join("same.txt"), r.join("same.txt")] {
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        Trees { _tmp: tmp, l, r, state }
    }

    /// Poll until the scan is done (or fail after a while).
    fn wait(m: &mut CompareModel) {
        let start = Instant::now();
        while m.is_scanning() {
            m.poll_scan();
            assert!(start.elapsed() < Duration::from_secs(30), "the compare never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn model(t: &Trees) -> CompareModel {
        CompareModel::new_in(t.state.clone(), t.l.clone(), Some(t.r.clone()), Vec::new(), crate::host::no_wake())
    }

    #[test]
    fn model_scan_review_and_plan() {
        let t = trees();
        let mut m = model(&t);
        assert!(m.is_scanning(), "a right folder starts the compare at once");
        wait(&mut m);
        assert!(matches!(m.stage(), Stage::Review), "{:?}", m.error());
        assert_eq!(m.same(), 1);
        let names: Vec<_> = m.rows().iter().map(|r| (r.rel.to_string_lossy().replace('\\', "/"), r.cat)).collect();
        assert_eq!(
            names,
            [
                ("only-left.txt".to_string(), Category::OnlyLeft),
                ("only-right.txt".to_string(), Category::OnlyRight),
                ("sub".to_string(), Category::OnlyLeft),
            ]
        );
        // Update: copy both ways.
        let s = m.stats();
        assert_eq!((s.to_right, s.to_left, s.delete_left, s.delete_right), (2, 1, 0, 0));
        // Filters.
        assert_eq!(m.counts(), [2, 0, 0, 0, 1]);
        m.toggle_filter(Category::OnlyLeft);
        assert_eq!(m.visible_rows(), [1]);
        m.toggle_filter(Category::OnlyLeft);
        m.search = "SUB".into();
        assert_eq!(m.visible_rows(), [2]);
        m.search.clear();
        // Mirror right → left deletes the right-only file; switching modes refills.
        m.set_preset(Preset::MirrorRight);
        assert!(matches!(m.stage(), Stage::Review), "non two-way modes refill without a rescan");
        let plan = m.plan().unwrap();
        assert_eq!(
            plan.actions,
            vec![
                SyncAction::CopyLR("only-left.txt".into()),
                SyncAction::CopyLR("sub".into()),
                SyncAction::DeleteR("only-right.txt".into())
            ]
        );
        // A hand edit makes the mode custom; impossible actions are refused.
        assert!(!m.set_action(0, RowAction::CopyLeft), "nothing on the right to copy from");
        assert!(m.set_action(2, RowAction::Skip));
        assert_eq!(m.preset, Preset::Custom);
        assert!(m.cycle_action(2), "skip → copy right");
        assert_eq!(m.rows()[2].action, RowAction::CopyRight);
        assert!(m.cycle_action(0), "copy right → delete (copy left is not allowed)");
        assert_eq!(m.rows()[0].action, RowAction::Delete);
        // Bulk over the visible rows.
        let vis = m.visible_rows();
        m.set_actions(&vis, RowAction::Skip);
        assert!(m.plan().unwrap().actions.is_empty());
        // Swapping compares again with the sides exchanged.
        m.swap();
        assert!(m.is_scanning());
        wait(&mut m);
        assert_eq!(m.rows()[0].cat, Category::OnlyRight, "only-left.txt is on the right now");
    }

    #[test]
    fn model_rejects_bad_paths_and_cancels_through_op_control() {
        let t = trees();
        let mut m = CompareModel::new_in(t.state.clone(), t.l.clone(), None, Vec::new(), crate::host::no_wake());
        assert!(matches!(m.stage(), Stage::Setup));
        m.start();
        assert_eq!(m.error(), Some(tr!("tools-compare-err-choose")));
        m.right = m.left.clone();
        m.start();
        assert_eq!(m.error(), Some(tr!("tools-compare-err-same")));
        // Cancel a running scan: back to setup, and its result never lands.
        m.right = t.r.display().to_string();
        m.start();
        let Stage::Scanning { ctl, .. } = m.stage() else { panic!("not scanning") };
        let ctl = ctl.clone();
        m.cancel();
        assert!(ctl.is_cancelled());
        assert!(matches!(m.stage(), Stage::Setup));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!m.poll_scan(), "a cancelled scan is not polled");
        assert!(m.rows().is_empty());
        // The worker itself stops on a cancelled control.
        let ctl = OpControl::new();
        ctl.cancel();
        assert!(compute_in(&t.state, &t.l, &t.r, Method::DateSize, Preset::Update, &ctl).is_err());
        assert!(compute_in(&t.state, &t.l, &t.r, Method::DateSize, Preset::TwoWay, &ctl).is_err());
        // Changing the method before any scan does not start one.
        let mut m = CompareModel::new_in(t.state.clone(), t.l.clone(), None, Vec::new(), crate::host::no_wake());
        m.set_method(Method::Content);
        assert!(matches!(m.stage(), Stage::Setup));
    }

    #[test]
    fn model_two_way_conflicts() {
        let t = trees();
        // A file on both sides with different contents and times far apart on the first
        // run goes to the newer side; close times are a "both created" conflict.
        std::fs::write(t.l.join("both.txt"), b"left version").unwrap();
        std::fs::write(t.r.join("both.txt"), b"right!").unwrap();
        let mut m = model(&t);
        m.close();
        m.preset = Preset::TwoWay;
        m.start();
        wait(&mut m);
        assert!(m.first_run());
        let (total, open) = m.conflict_counts();
        assert_eq!((total, open), (1, 1), "{:?}", m.rows());
        let ci = m.conflict_rows()[0];
        assert_eq!(m.rows()[ci].rel, PathBuf::from("both.txt"));
        assert!(m.rows()[ci].open_conflict());
        // The conflicts step: one answer, then apply to all.
        m.step = ReviewStep::Conflicts;
        assert!(m.resolve(ci, Resolution::KeepBoth));
        assert_eq!(m.rows()[ci].action, RowAction::KeepBoth);
        assert_eq!(m.conflict_counts(), (1, 0));
        let plain = (0..m.rows().len()).find(|&i| m.rows()[i].conflict.is_none()).unwrap();
        assert!(!m.resolve(plain, Resolution::Left), "only conflicts take answers");
        m.resolve_all(Some(Resolution::Left));
        assert_eq!(row_resolution(&m.rows()[ci]), Resolution::Left);
        m.resolve_all(Some(Resolution::KeepBoth));
        let plan = m.plan().unwrap();
        let tag = m.tag();
        assert!(tag.starts_with("right "), "{tag}");
        let alt = PathBuf::from(format!("both (conflict {tag}).txt"));
        assert!(plan.actions.contains(&SyncAction::CopyAs {
            src_left: false,
            rel: "both.txt".into(),
            dst_left: true,
            to: alt
        }));
        assert!(plan.actions.contains(&SyncAction::CopyLR("only-left.txt".into())));
        assert!(plan.actions.contains(&SyncAction::CopyRL("only-right.txt".into())));
        // Hand edits keep the two-way mode; switching out of it compares again.
        assert!(m.set_action(ci, RowAction::Skip));
        assert_eq!(m.preset, Preset::TwoWay);
        m.set_preset(Preset::Update);
        assert!(m.is_scanning());
        wait(&mut m);
        assert!(m.two_way().is_none());
        assert!(m.conflict_rows().is_empty());
    }

    #[test]
    fn model_profile_from_setup() {
        let t = trees();
        let mut m = model(&t);
        m.close();
        assert!(m.current_profile().is_none(), "needs a name");
        m.profile_name = "  Backup ".into();
        m.method = Method::Content;
        m.preset = Preset::Custom;
        let p = m.current_profile().unwrap();
        assert_eq!(p.name, "Backup");
        assert_eq!((p.method, p.preset), (Method::Content, Preset::Update), "custom saves as its base mode");
        assert_eq!(p.left, t.l);
    }
}
