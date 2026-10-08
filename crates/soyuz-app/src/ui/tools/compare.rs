//! Folder compare + sync (PLAN M12). `soyuz_search::compare_dirs` runs on a worker; the
//! result becomes a table of rows (only-left, only-right, newer-left, newer-right,
//! differ) with a per-row action (copy →, copy ←, delete, skip). Presets fill the
//! actions (two-way update, mirror left → right, mirror right → left); the plan is
//! previewed and then submitted as one `JobKind::Sync` job. Profiles (left, right,
//! method, preset, schedule) are saved in `sync_profiles.toml` next to the config and
//! owned by the scheduler (`sync_sched`).
//!
//! F8: the "Two-way sync" mode plans with the three-way state of `sync_state` instead of
//! a plain compare, so deletions and edits on both sides are told apart; conflicts get
//! their own stage (keep left / keep right / keep both / skip, apply to all), and after
//! the job the new state is recorded.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crossbeam_channel::{Receiver, bounded};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use soyuz_core::{OpControl, SizeStyle, format_size, format_time};
use soyuz_jobs::{JobKind, SyncAction, SyncPlan};

use super::sync_state::{self, ConflictKind, Resolution};
use super::{button, filter_chip, progress_bar, text_field, tool_window};
use crate::app::SoyuzApp;
use crate::theme::{Palette, icons, icons::ph, palette};
use crate::ui::widgets::{menu_item, menu_separator, trunc_galley};

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
            Category::OnlyLeft => "Only left",
            Category::NewerLeft => "Newer left",
            Category::Differ => "Different",
            Category::NewerRight => "Newer right",
            Category::OnlyRight => "Only right",
        }
    }

    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Category::OnlyLeft => p.accent,
            Category::NewerLeft => p.success,
            Category::Differ => p.danger,
            Category::NewerRight => p.warning,
            Category::OnlyRight => p.glyph_video,
        }
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
    /// F8: three-way sync against the state of the last sync (deletions propagate,
    /// conflicts are asked).
    TwoWay,
    /// Leave actions as set by hand.
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::TwoWay, Preset::Update, Preset::MirrorRight, Preset::MirrorLeft];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Update => "Two-way update",
            Preset::MirrorRight => "Mirror left → right",
            Preset::MirrorLeft => "Mirror right → left",
            Preset::TwoWay => "Two-way sync",
            Preset::Custom => "Custom",
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
    pub fn label(self) -> &'static str {
        match self {
            Method::DateSize => "Date & size",
            Method::Size => "Size",
            Method::Content => "Content",
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

/// Turn the rows' actions into a sync plan: copies first, then deletions. Actions that
/// make no sense for a row (copy from the side where it is missing) are ignored. "Keep
/// both" saves the right version under a conflict name tagged `tag` on both sides.
pub fn build_plan(left: &Path, right: &Path, rows: &[Row], tag: &str) -> SyncPlan {
    let mut copies = Vec::new();
    let mut deletes = Vec::new();
    let names: std::collections::HashSet<&Path> = rows.iter().map(|r| r.rel.as_path()).collect();
    let mut alts: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
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
        (true, false, RowAction::Delete) => "Deleted right",
        (false, true, RowAction::Delete) => "Deleted left",
        (true, false, _) => "New left",
        (false, true, _) => "New right",
        (_, _, RowAction::CopyRight) => "Changed left",
        (_, _, RowAction::CopyLeft) => "Changed right",
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
    if preset == Preset::TwoWay {
        let lt = sync_state::scan(l, ctl).map_err(|e| e.to_string())?;
        let rt = sync_state::scan(r, ctl).map_err(|e| e.to_string())?;
        let state = sync_state::load_state(&sync_state::state_path(l, r));
        let plan = sync_state::plan(state.as_ref(), &lt, &rt, method == Method::Size);
        let tag = format!("right {}", chrono::Local::now().format("%Y-%m-%d %H%M"));
        let ctx = TwoWayCtx { keep_base: plan.keep_base.clone(), first_run: state.is_none(), tag };
        let rows = rows_from_two_way(&plan);
        if rows.is_empty() {
            // Already in sync: remember this as the base, or later deletions would look
            // like new files on the other side.
            let keep: std::collections::HashSet<PathBuf> = plan.keep_base.iter().cloned().collect();
            let next = sync_state::next_state(state.as_ref(), l, r, &lt, &rt, &keep, std::time::SystemTime::now());
            if state.as_ref().map(|s| &s.entries) != Some(&next.entries) {
                sync_state::save_state(&sync_state::state_path(l, r), &next)?;
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
) -> std::collections::HashSet<PathBuf> {
    let mut keep: std::collections::HashSet<PathBuf> = rows
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
// Profiles.

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncProfile {
    pub name: String,
    pub left: PathBuf,
    pub right: PathBuf,
    #[serde(default)]
    pub method: Method,
    #[serde(default)]
    pub preset: Preset,
    /// F8: when the scheduler runs this profile on its own.
    #[serde(default)]
    pub schedule: super::sync_sched::Schedule,
}

#[derive(Default, Serialize, Deserialize)]
struct ProfileFile {
    #[serde(default)]
    profiles: Vec<SyncProfile>,
}

fn profiles_path() -> PathBuf {
    soyuz_core::config::config_dir().join("sync_profiles.toml")
}

/// Load saved profiles (small file; read when the window opens).
/// A broken file is backed up and protected from being overwritten (`store_file`).
pub fn load_profiles() -> Vec<SyncProfile> {
    crate::services::store_file::load_toml::<ProfileFile>(&profiles_path()).value().profiles
}

/// Saved in order on the shared store worker (flushed by `App::save`).
pub(crate) fn save_profiles(profiles: &[SyncProfile]) {
    match toml::to_string_pretty(&ProfileFile { profiles: profiles.to_vec() }) {
        Ok(text) => crate::services::store_file::save_async(profiles_path(), text.into_bytes()),
        Err(e) => log::warn!("cannot save sync profiles: {e}"),
    }
}

// ---------------------------------------------------------------------------------------
// Window.

enum State {
    Idle,
    Running {
        rx: Receiver<Result<Computed, String>>,
        ctl: Arc<OpControl>,
        started: Instant,
    },
    Ready,
    Failed(String),
    /// A sync job is running; compare again when it finishes.
    Syncing(soyuz_jobs::JobId),
    /// Two-way: recording the new sync state after the job.
    Recording(Receiver<Result<(), String>>),
}

#[derive(PartialEq, Eq)]
enum Stage {
    Table,
    Preview,
    Conflicts,
}

/// What a two-way sync job needs once it is done (to record the new state).
struct PendingRecord {
    left: PathBuf,
    right: PathBuf,
    rows: Vec<Row>,
    ctx: TwoWayCtx,
}

pub struct CompareWindow {
    pub open: bool,
    left: String,
    right: String,
    method: Method,
    preset: Preset,
    state: State,
    rows: Vec<Row>,
    same: u64,
    show: [bool; 5],
    search: String,
    stage: Stage,
    profiles: Vec<SyncProfile>,
    profile_name: String,
    /// Paths the current rows were computed for.
    compared: Option<(PathBuf, PathBuf)>,
    hovered: Option<usize>,
    two_way: Option<TwoWayCtx>,
    pending_record: Option<PendingRecord>,
}

impl CompareWindow {
    pub fn new(left: PathBuf, right: Option<PathBuf>, profiles: Vec<SyncProfile>) -> Self {
        let mut w = CompareWindow {
            open: true,
            left: super::tags_ui::show_path(&left),
            right: right.as_deref().map(super::tags_ui::show_path).unwrap_or_default(),
            method: Method::DateSize,
            preset: Preset::Update,
            state: State::Idle,
            rows: Vec::new(),
            same: 0,
            show: [true; 5],
            search: String::new(),
            stage: Stage::Table,
            profiles,
            profile_name: String::new(),
            compared: None,
            hovered: None,
            two_way: None,
            pending_record: None,
        };
        if right.is_some() {
            w.start();
        }
        w
    }

    /// Open with a saved profile loaded and compared (profiles window, conflict toasts).
    pub fn with_profile(pr: &SyncProfile, profiles: Vec<SyncProfile>) -> Self {
        let mut w = CompareWindow::new(pr.left.clone(), None, profiles);
        w.load_profile(pr.clone());
        w
    }

    fn load_profile(&mut self, pr: SyncProfile) {
        self.left = super::tags_ui::show_path(&pr.left);
        self.right = super::tags_ui::show_path(&pr.right);
        self.method = pr.method;
        self.preset = pr.preset;
        self.profile_name = pr.name;
        self.start();
    }

    fn start(&mut self) {
        if let State::Running { ctl, .. } = &self.state {
            ctl.cancel();
        }
        let (l, r) = (PathBuf::from(self.left.trim()), PathBuf::from(self.right.trim()));
        if l.as_os_str().is_empty() || r.as_os_str().is_empty() {
            self.state = State::Failed("Choose two folders to compare.".into());
            return;
        }
        if l == r {
            self.state = State::Failed("Both sides are the same folder.".into());
            return;
        }
        let ctl = OpControl::new();
        let (tx, rx) = bounded(1);
        let c2 = ctl.clone();
        let (method, preset) = (self.method, self.preset);
        let wake = super::wake();
        let (l2, r2) = (l.clone(), r.clone());
        let spawned = std::thread::Builder::new().name("soyuz-compare".into()).spawn(move || {
            let out = compute(&l2, &r2, method, preset, &c2);
            if !c2.is_cancelled() {
                let _ = tx.send(out);
            }
            wake();
        });
        if spawned.is_ok() {
            self.compared = Some((l, r));
            self.state = State::Running { rx, ctl, started: Instant::now() };
            self.stage = Stage::Table;
        }
    }

    fn poll(&mut self) {
        let State::Running { rx, .. } = &self.state else { return };
        match rx.try_recv() {
            Ok(Ok(c)) => {
                self.rows = c.rows;
                self.same = c.same;
                self.two_way = c.two_way;
                self.state = State::Ready;
            }
            Ok(Err(e)) => self.state = State::Failed(e),
            Err(crossbeam_channel::TryRecvError::Disconnected) => self.state = State::Idle,
            Err(_) => {}
        }
    }

    fn apply_preset(&mut self) {
        if matches!(self.preset, Preset::Custom | Preset::TwoWay) {
            return;
        }
        for r in &mut self.rows {
            r.action = self.preset.action(r.cat);
        }
    }

    fn visible(&self, r: &Row) -> bool {
        let ci = Category::ALL.iter().position(|c| *c == r.cat).unwrap_or(0);
        if !self.show[ci] {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        q.is_empty() || r.rel.to_string_lossy().to_lowercase().contains(&q)
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        self.poll();
        if let State::Syncing(job) = self.state {
            let snap = app.fileops.jobs.snapshot(job);
            if snap.as_ref().is_none_or(|s| s.state.is_terminal()) {
                let failed = snap.as_ref().and_then(|s| match &s.state {
                    soyuz_jobs::JobState::Done(sum) => Some(sum.errors.iter().map(|(p, _)| p.clone()).collect()),
                    _ => None,
                });
                match (self.pending_record.take(), failed) {
                    // Record the new two-way state, then compare again.
                    (Some(rec), Some(failed)) => self.record(rec, failed),
                    _ => self.start(),
                }
            }
        }
        if let State::Recording(rx) = &self.state {
            match rx.try_recv() {
                Ok(res) => {
                    if let Err(e) = res {
                        app.ui.toasts.push(crate::ui::toasts::Toast::warning("Couldn't save the sync state").body(e));
                    }
                    self.start();
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.start(),
                Err(_) => {}
            }
        }
        let sub = match &self.compared {
            Some((l, r)) => {
                format!("{}  ↔  {}", soyuz_core::entry::display_name(l), soyuz_core::entry::display_name(r))
            }
            None => String::new(),
        };
        let mut submit = None;
        let keep = tool_window(ctx, "compare", "Compare & sync", &sub, ph::GIT_DIFF, vec2(1040.0, 660.0), |ui| {
            submit = self.body(ui, app);
        });
        if let Some(plan) = submit {
            self.run(app, plan);
        }
        if !keep {
            if let State::Running { ctl, .. } = &self.state {
                ctl.cancel();
            }
            self.open = false;
        }
    }

    /// Rescan both sides and save the two-way state on a worker.
    fn record(&mut self, rec: PendingRecord, failed: Vec<PathBuf>) {
        let keep = keep_after_sync(&rec.rows, &rec.ctx, &rec.left, &rec.right, &failed);
        let (tx, rx) = bounded(1);
        let wake = super::wake();
        let spawned = std::thread::Builder::new().name("soyuz-sync-state".into()).spawn(move || {
            let _ = tx.send(sync_state::record_after_sync(&rec.left, &rec.right, &keep).map(|_| ()));
            wake();
        });
        self.state = if spawned.is_ok() { State::Recording(rx) } else { State::Idle };
    }

    fn run(&mut self, app: &mut SoyuzApp, plan: SyncPlan) {
        let label = format!(
            "Sync {} ↔ {}",
            soyuz_core::entry::display_name(&plan.left),
            soyuz_core::entry::display_name(&plan.right)
        );
        self.pending_record = self.two_way.clone().map(|ctx| PendingRecord {
            left: plan.left.clone(),
            right: plan.right.clone(),
            rows: self.rows.clone(),
            ctx,
        });
        let mut spec = app.fileops.jobs.spec(JobKind::Sync { plan });
        spec.options.label = Some(label);
        let job = crate::ui::archive::submit_job(app, spec);
        // The folders change: compare again once the job is done.
        self.rows.clear();
        self.state = State::Syncing(job);
        self.stage = Stage::Table;
    }

    fn body(&mut self, ui: &mut Ui, app: &mut SoyuzApp) -> Option<SyncPlan> {
        let p = palette(ui.ctx());
        // Paths.
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 44.0) / 2.0;
            ui.label(icons::icon(ph::FOLDER_SIMPLE, 14.0).color(Category::OnlyLeft.color(&p)));
            text_field(ui, &mut self.left, "Left folder", w - 22.0);
            if crate::ui::widgets::icon_button(ui, ph::ARROWS_LEFT_RIGHT, "Swap sides", true).clicked() {
                std::mem::swap(&mut self.left, &mut self.right);
                if matches!(self.state, State::Ready) {
                    self.start();
                }
            }
            text_field(ui, &mut self.right, "Right folder", w - 22.0);
            ui.label(icons::icon(ph::FOLDER_SIMPLE, 14.0).color(Category::OnlyRight.color(&p)));
        });
        ui.add_space(4.0);
        // Method, preset, compare, profiles.
        ui.horizontal(|ui| {
            ui.label(RichText::new("Compare by").font(FontId::proportional(12.0)).color(p.text_tertiary));
            for m in [Method::DateSize, Method::Size, Method::Content] {
                let mut on = self.method == m;
                if super::toggle_chip(ui, m.label(), &mut on, p.accent).clicked() && self.method != m {
                    self.method = m;
                    if !matches!(self.state, State::Idle) {
                        self.start();
                    }
                }
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Sync mode").font(FontId::proportional(12.0)).color(p.text_tertiary));
            for pr in Preset::ALL {
                let mut on = self.preset == pr;
                let mut chip = super::toggle_chip(ui, pr.label(), &mut on, p.accent);
                if pr == Preset::TwoWay {
                    chip = chip.on_hover_text(
                        "Remembers the last sync, so deletions and edits on both sides are told apart. \
                         Changes on both sides are asked as conflicts.",
                    );
                }
                if chip.clicked() && self.preset != pr {
                    // Two-way plans differently: compare again when switching in or out.
                    let replan = self.preset == Preset::TwoWay
                        || pr == Preset::TwoWay
                        || (self.preset == Preset::Custom && self.two_way.is_some());
                    self.preset = pr;
                    if replan && !matches!(self.state, State::Idle) {
                        self.start();
                    } else {
                        self.apply_preset();
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let running = matches!(self.state, State::Running { .. });
                if running {
                    if button(ui, Some(ph::STOP), "Cancel", 0, true).clicked()
                        && let State::Running { ctl, .. } = &self.state
                    {
                        ctl.cancel();
                        self.state = State::Idle;
                    }
                } else if button(ui, Some(ph::ARROW_CLOCKWISE), "Compare", 1, true).clicked() {
                    self.start();
                }
                self.profiles_menu(ui);
            });
        });
        ui.add_space(6.0);
        match &self.state {
            State::Idle if self.rows.is_empty() => {
                empty_note(
                    ui,
                    ph::GIT_DIFF,
                    "Pick two folders and press Compare",
                    "Differences show up here, ready to sync.",
                );
                return None;
            }
            State::Running { started, .. } => {
                progress_bar(ui, ui.available_width(), None);
                ui.label(
                    RichText::new(format!("Comparing… {:.1} s", started.elapsed().as_secs_f32()))
                        .font(FontId::proportional(12.0))
                        .color(p.text_secondary),
                );
                return None;
            }
            State::Syncing(job) => {
                let snap = app.fileops.jobs.snapshot(*job);
                let frac = snap.as_ref().and_then(|s| {
                    (s.progress.items_total > 0).then(|| s.progress.items_done as f32 / s.progress.items_total as f32)
                });
                progress_bar(ui, ui.available_width(), frac);
                ui.label(
                    RichText::new("Syncing… the folders are compared again when the job finishes.")
                        .font(FontId::proportional(12.0))
                        .color(p.text_secondary),
                );
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                return None;
            }
            State::Recording(_) => {
                progress_bar(ui, ui.available_width(), None);
                ui.label(
                    RichText::new("Recording the sync state…").font(FontId::proportional(12.0)).color(p.text_secondary),
                );
                return None;
            }
            State::Failed(e) => {
                let e = e.clone();
                empty_note(ui, ph::WARNING_CIRCLE, "Can't compare these folders", &e);
                return None;
            }
            _ => {}
        }
        if self.stage == Stage::Preview {
            return self.preview(ui);
        }
        if self.stage == Stage::Conflicts {
            self.conflicts(ui, app);
            return None;
        }
        // Filters.
        let mut counts = [0usize; 5];
        for r in &self.rows {
            counts[Category::ALL.iter().position(|c| *c == r.cat).unwrap_or(0)] += 1;
        }
        ui.horizontal(|ui| {
            for (i, c) in Category::ALL.iter().enumerate() {
                filter_chip(ui, c.label(), counts[i], c.color(&p), &mut self.show[i]);
            }
            ui.label(
                RichText::new(format!("· {} identical", crate::ui::statusbar::group_thousands(self.same as usize)))
                    .font(FontId::proportional(12.0))
                    .color(p.text_tertiary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                text_field(ui, &mut self.search, "Filter names", 180.0);
            });
        });
        ui.add_space(4.0);
        if self.two_way.as_ref().is_some_and(|t| t.first_run) && !self.rows.is_empty() {
            ui.label(
                RichText::new(
                    "First two-way sync of these folders: newer files win, unclear ones are conflicts. \
                     From now on deletions and edits on both sides are tracked.",
                )
                .font(FontId::proportional(11.5))
                .color(p.text_tertiary),
            );
        }
        if self.rows.is_empty() {
            empty_note(ui, ph::CHECK_CIRCLE, "The folders are in sync", "No differences were found.");
            return None;
        }
        let vis: Vec<usize> = (0..self.rows.len()).filter(|&i| self.visible(&self.rows[i])).collect();
        let footer_h = 40.0;
        let table_h = (ui.available_height() - footer_h).max(80.0);
        let width = ui.available_width();
        self.table(ui, app, &vis, width, table_h);
        // Footer: plan summary + preview.
        let stats = plan_stats(&self.rows);
        let total = stats.to_left + stats.to_right + stats.delete_left + stats.delete_right;
        let conflicts = self.rows.iter().filter(|r| r.conflict.is_some()).count();
        let open = self.rows.iter().filter(|r| r.conflict.is_some() && r.action == RowAction::Skip).count();
        let mut go = false;
        let mut resolve = false;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            plan_line(ui, &p, &stats);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                go = button(ui, Some(ph::ARROW_RIGHT), "Review sync…", 1, total > 0).clicked();
                if conflicts > 0 {
                    let label = if open > 0 {
                        format!("Resolve {open} conflict{}…", if open == 1 { "" } else { "s" })
                    } else {
                        "Conflicts…".to_string()
                    };
                    let kind = if open > 0 { 2 } else { 0 };
                    resolve = button(ui, Some(ph::GIT_MERGE), &label, kind, true).clicked();
                }
            });
        });
        if go {
            self.stage = Stage::Preview;
        }
        if resolve {
            self.stage = Stage::Conflicts;
        }
        None
    }

    fn profiles_menu(&mut self, ui: &mut Ui) {
        let resp = button(ui, Some(ph::FLOPPY_DISK), "Profiles", 0, true);
        egui::Popup::menu(&resp).width(260.0).show(|ui| {
            ui.set_width(260.0);
            if self.profiles.is_empty() {
                menu_item(ui, Some(ph::INFO), "No saved profiles", None, false);
            }
            let mut load = None;
            let mut delete = None;
            for (i, pr) in self.profiles.iter().enumerate() {
                ui.horizontal(|ui| {
                    let r = menu_item(ui, Some(ph::FOLDER_SIMPLE_STAR), &pr.name, None, true).on_hover_text(format!(
                        "{}\n↔ {}\n{} · {}",
                        pr.left.display(),
                        pr.right.display(),
                        pr.method.label(),
                        pr.preset.label()
                    ));
                    if r.clicked() {
                        load = Some(i);
                    }
                    if r.secondary_clicked() {
                        delete = Some(i);
                    }
                });
            }
            if let Some(i) = load {
                let pr = self.profiles[i].clone();
                self.load_profile(pr);
                ui.close();
            }
            if let Some(i) = delete {
                self.profiles.remove(i);
                super::sync_sched::set_profiles(self.profiles.clone());
            }
            menu_separator(ui);
            ui.label(
                RichText::new("Save current as")
                    .font(FontId::proportional(11.0))
                    .color(palette(ui.ctx()).text_tertiary),
            );
            ui.horizontal(|ui| {
                text_field(ui, &mut self.profile_name, "Profile name", 170.0);
                let ok = !self.profile_name.trim().is_empty()
                    && !self.left.trim().is_empty()
                    && !self.right.trim().is_empty();
                if button(ui, None, "Save", 1, ok).clicked() {
                    let name = self.profile_name.trim().to_string();
                    let schedule = self.profiles.iter().find(|p| p.name == name).map(|p| p.schedule.clone());
                    // A hand-edited table saves as its base mode.
                    let preset = match self.preset {
                        Preset::Custom if self.two_way.is_some() => Preset::TwoWay,
                        Preset::Custom => Preset::Update,
                        other => other,
                    };
                    let prof = SyncProfile {
                        name,
                        left: PathBuf::from(self.left.trim()),
                        right: PathBuf::from(self.right.trim()),
                        method: self.method,
                        preset,
                        schedule: schedule.unwrap_or_default(),
                    };
                    self.profiles.retain(|p| p.name != prof.name);
                    self.profiles.push(prof);
                    self.profiles.sort_by_key(|a| a.name.to_lowercase());
                    super::sync_sched::set_profiles(self.profiles.clone());
                    ui.close();
                }
            });
            ui.label(
                RichText::new("Right-click a profile to delete it")
                    .font(FontId::proportional(10.5))
                    .color(palette(ui.ctx()).text_tertiary),
            );
            menu_separator(ui);
            if menu_item(ui, Some(ph::CALENDAR_CHECK), "Schedules & reports…", None, true).clicked() {
                super::queue(super::ToolCmd::SyncProfiles);
                ui.close();
            }
        });
    }

    fn table(&mut self, ui: &mut Ui, app: &SoyuzApp, vis: &[usize], width: f32, height: f32) {
        let p = palette(ui.ctx());
        let row_h = 28.0;
        let meta_w = 190.0;
        let act_w = 132.0;
        let name_w = (width - 2.0 * meta_w - act_w).max(160.0);
        // Header with bulk actions.
        let (hr, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
        let painter = ui.painter();
        let hfont = FontId::proportional(11.0);
        painter.text(
            pos2(hr.left() + 10.0, hr.center().y),
            Align2::LEFT_CENTER,
            "LEFT",
            hfont.clone(),
            p.text_tertiary,
        );
        painter.text(
            pos2(hr.left() + meta_w + 10.0, hr.center().y),
            Align2::LEFT_CENTER,
            "NAME",
            hfont.clone(),
            p.text_tertiary,
        );
        painter.text(pos2(hr.right() - 10.0, hr.center().y), Align2::RIGHT_CENTER, "RIGHT", hfont, p.text_tertiary);
        let ar = Rect::from_min_size(pos2(hr.left() + meta_w + name_w, hr.top() + 1.0), vec2(act_w, 24.0));
        if let Some(a) = action_buttons(ui, ar, None, None, "all visible rows") {
            for &i in vis {
                let r = &mut self.rows[i];
                if allowed(r.cat, a) {
                    r.action = a;
                }
            }
            if self.two_way.is_none() {
                self.preset = Preset::Custom;
            }
        }
        ui.painter().line_segment(
            [pos2(hr.left(), hr.bottom()), pos2(hr.right(), hr.bottom())],
            Stroke::new(1.0, p.border_subtle),
        );
        let cfg = &app.config.general;
        let two_way = self.two_way.is_some();
        let mut changed = false;
        let mut hovered = None;
        ScrollArea::vertical().id_salt("compare.rows").max_height(height - 28.0).auto_shrink([false, false]).show_rows(
            ui,
            row_h,
            vis.len(),
            |ui, range| {
                for k in range {
                    let i = vis[k];
                    let (r, resp) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                    let row = &self.rows[i];
                    let painter = ui.painter();
                    if resp.hovered() {
                        hovered = Some(i);
                        painter.rect_filled(r, CornerRadius::same(6), p.bg_surface2);
                    }
                    let color = row.cat.color(&p);
                    // Left meta.
                    let meta = |s: &Option<Side>| -> String {
                        match s {
                            Some(m) if m.dir => {
                                format!(
                                    "Folder  ·  {}",
                                    m.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default()
                                )
                            }
                            Some(m) => format!(
                                "{}  ·  {}",
                                format_size(m.size, cfg.size_style),
                                m.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default()
                            ),
                            None => "—".into(),
                        }
                    };
                    let lc = if matches!(row.cat, Category::NewerLeft | Category::OnlyLeft) {
                        p.text_primary
                    } else {
                        p.text_secondary
                    };
                    let rc = if matches!(row.cat, Category::NewerRight | Category::OnlyRight) {
                        p.text_primary
                    } else {
                        p.text_secondary
                    };
                    let g = trunc_galley(painter, &meta(&row.left), FontId::proportional(11.5), lc, meta_w - 16.0);
                    painter.galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, lc);
                    let g = trunc_galley(painter, &meta(&row.right), FontId::proportional(11.5), rc, meta_w - 16.0);
                    painter.galley(pos2(r.right() - 10.0 - g.size().x, r.center().y - g.size().y / 2.0), g, rc);
                    // Name with category bar.
                    let nx = r.left() + meta_w;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(nx, r.top() + 6.0), vec2(3.0, row_h - 12.0)),
                        CornerRadius::same(1),
                        color,
                    );
                    let is_dir = row.left.as_ref().or(row.right.as_ref()).is_some_and(|m| m.dir);
                    let glyph = if is_dir { ph::FOLDER_SIMPLE } else { ph::FILE };
                    painter.text(
                        pos2(nx + 18.0, r.center().y),
                        Align2::CENTER_CENTER,
                        glyph,
                        icons::font(14.0),
                        if is_dir { p.glyph_folder } else { p.text_secondary },
                    );
                    let name = row.rel.to_string_lossy();
                    let skipped = row.action == RowAction::Skip;
                    let ncolor = if skipped { p.text_secondary } else { p.text_primary };
                    let g = trunc_galley(painter, &name, FontId::proportional(12.5), ncolor, name_w - 40.0 - 80.0);
                    let gw = g.size().x;
                    painter.galley(pos2(nx + 30.0, r.center().y - g.size().y / 2.0), g, ncolor);
                    let (tag_text, tag_color) = match row.conflict {
                        Some(_) if row.action == RowAction::Skip => ("Conflict", p.danger),
                        Some(_) => ("Resolved", p.success),
                        None if two_way => (two_way_reason(row), color),
                        None => (row.cat.label(), color),
                    };
                    let tag = trunc_galley(painter, tag_text, FontId::proportional(10.5), tag_color, 80.0);
                    let tag_rect =
                        Rect::from_min_size(pos2(nx + 36.0 + gw, r.center().y - tag.size().y / 2.0), tag.size());
                    painter.galley(tag_rect.min, tag, tag_color);
                    let conflict = row.conflict;
                    if let Some(k) = conflict {
                        ui.interact(tag_rect, ui.id().with(("compare.conflict", i)), Sense::hover())
                            .on_hover_text(k.label());
                    }
                    // Actions.
                    let ar = Rect::from_min_size(pos2(nx + name_w, r.top() + 2.0), vec2(act_w, row_h - 4.0));
                    let cat = row.cat;
                    let current = row.action;
                    if let Some(a) = action_buttons(ui, ar, Some(cat), Some(current), "") {
                        self.rows[i].action = a;
                        changed = true;
                    }
                }
            },
        );
        self.hovered = hovered;
        if changed && self.two_way.is_none() {
            self.preset = Preset::Custom;
        }
    }

    fn preview(&mut self, ui: &mut Ui) -> Option<SyncPlan> {
        let p = palette(ui.ctx());
        let (Some((l, r)), stats) = (self.compared.clone(), plan_stats(&self.rows)) else { return None };
        let tag = self.two_way.as_ref().map_or("right", |t| t.tag.as_str()).to_string();
        let plan = build_plan(&l, &r, &self.rows, &tag);
        let open = self.rows.iter().filter(|r| r.conflict.is_some() && r.action == RowAction::Skip).count();
        ui.label(
            RichText::new("Review the sync plan").font(crate::ui::views::flow::semibold(14.0)).color(p.text_primary),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| plan_line(ui, &p, &stats));
        if stats.delete_left + stats.delete_right > 0 {
            ui.label(
                RichText::new("Deleted items go to the trash when the volume supports it.")
                    .font(FontId::proportional(12.0))
                    .color(p.warning),
            );
        }
        if open > 0 {
            ui.label(
                RichText::new(format!(
                    "{open} unresolved conflict{} will be skipped and asked again next time.",
                    if open == 1 { "" } else { "s" }
                ))
                .font(FontId::proportional(12.0))
                .color(p.warning),
            );
        }
        ui.add_space(6.0);
        let h = (ui.available_height() - 44.0).max(80.0);
        ScrollArea::vertical().id_salt("compare.plan").max_height(h).auto_shrink([false, false]).show_rows(
            ui,
            24.0,
            plan.actions.len(),
            |ui, range| {
                for a in &plan.actions[range] {
                    let (glyph, verb, rel, color) = match a {
                        SyncAction::CopyLR(x) => (ph::ARROW_RIGHT, "Copy to right", x, p.accent),
                        SyncAction::CopyRL(x) => (ph::ARROW_LEFT, "Copy to left", x, p.accent),
                        SyncAction::DeleteL(x) => (ph::TRASH, "Delete on left", x, p.danger),
                        SyncAction::DeleteR(x) => (ph::TRASH, "Delete on right", x, p.danger),
                        SyncAction::CopyAs { dst_left, to, .. } => {
                            let verb = if *dst_left { "Conflict copy ←" } else { "Conflict copy →" };
                            (ph::COPY, verb, to, p.warning)
                        }
                    };
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                    let painter = ui.painter();
                    painter.text(
                        pos2(row.left() + 12.0, row.center().y),
                        Align2::CENTER_CENTER,
                        glyph,
                        icons::font(13.0),
                        color,
                    );
                    painter.text(
                        pos2(row.left() + 28.0, row.center().y),
                        Align2::LEFT_CENTER,
                        verb,
                        FontId::proportional(12.0),
                        p.text_secondary,
                    );
                    let g = trunc_galley(
                        painter,
                        &rel.to_string_lossy(),
                        FontId::proportional(12.5),
                        p.text_primary,
                        row.width() - 160.0,
                    );
                    painter.galley(pos2(row.left() + 140.0, row.center().y - g.size().y / 2.0), g, p.text_primary);
                }
            },
        );
        let mut out = None;
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if button(ui, Some(ph::ARROW_LEFT), "Back", 0, true).clicked() {
                self.stage = Stage::Table;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let danger = stats.delete_left + stats.delete_right > 0;
                let label = format!("Start sync ({})", plan.actions.len());
                if button(ui, Some(ph::PLAY), &label, if danger { 2 } else { 1 }, !plan.actions.is_empty()).clicked() {
                    out = Some(plan.clone());
                }
            });
        });
        out
    }
}

impl CompareWindow {
    /// Conflict stage (two-way): per item keep left / right / both / skip, plus apply to all.
    fn conflicts(&mut self, ui: &mut Ui, app: &SoyuzApp) {
        let p = palette(ui.ctx());
        let cfg = &app.config.general;
        let idx: Vec<usize> = (0..self.rows.len()).filter(|&i| self.rows[i].conflict.is_some()).collect();
        let open = idx.iter().filter(|&&i| self.rows[i].action == RowAction::Skip).count();
        ui.label(RichText::new("Resolve conflicts").font(crate::ui::views::flow::semibold(14.0)).color(p.text_primary));
        ui.label(
            RichText::new(format!(
                "{} item{} changed on both sides since the last sync. Choose which version wins; \
                 skipped items are asked again next time.",
                idx.len(),
                if idx.len() == 1 { "" } else { "s" }
            ))
            .font(FontId::proportional(12.0))
            .color(p.text_secondary),
        );
        ui.add_space(4.0);
        // Apply to all.
        let mut all: Option<Option<Resolution>> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Apply to all").font(FontId::proportional(12.0)).color(p.text_tertiary));
            for r in [Resolution::Left, Resolution::Right, Resolution::KeepBoth, Resolution::Skip] {
                if button(ui, None, r.label(), 0, true).clicked() {
                    all = Some(Some(r));
                }
            }
            if button(ui, Some(ph::CLOCK), "Keep newer", 0, true)
                .on_hover_text("For each item, the side modified last wins")
                .clicked()
            {
                all = Some(None);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (t, c) = if open == 0 {
                    ("All decided".to_string(), p.success)
                } else {
                    (format!("{open} open"), p.warning)
                };
                ui.label(RichText::new(t).font(FontId::proportional(12.0)).color(c));
            });
        });
        if let Some(choice) = all {
            for &i in &idx {
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
        ui.add_space(6.0);
        let row_h = 52.0;
        let h = (ui.available_height() - 44.0).max(80.0);
        let meta = |s: &Option<Side>| -> String {
            match s {
                None => "Deleted".into(),
                Some(m) if m.dir => "Folder".into(),
                Some(m) => format!(
                    "{}  ·  {}",
                    format_size(m.size, cfg.size_style),
                    m.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default()
                ),
            }
        };
        let mut set: Option<(usize, Resolution)> = None;
        ScrollArea::vertical().id_salt("compare.conflicts").max_height(h).auto_shrink([false, false]).show_rows(
            ui,
            row_h,
            idx.len(),
            |ui, range| {
                for k in range {
                    let i = idx[k];
                    let row = &self.rows[i];
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface2);
                    }
                    let painter = ui.painter();
                    let current = row_resolution(row);
                    let decided = current != Resolution::Skip;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(r.left() + 2.0, r.top() + 8.0), vec2(3.0, row_h - 16.0)),
                        CornerRadius::same(1),
                        if decided { p.success } else { p.danger },
                    );
                    let choices_w = 330.0;
                    let text_w = (r.width() - choices_w - 24.0).max(120.0);
                    let name = row.rel.to_string_lossy();
                    let g =
                        trunc_galley(painter, &name, crate::ui::views::flow::semibold(12.5), p.text_primary, text_w);
                    let gw = g.size().x;
                    painter.galley(pos2(r.left() + 14.0, r.top() + 7.0), g, p.text_primary);
                    if let Some(kind) = row.conflict {
                        let t = trunc_galley(painter, kind.label(), FontId::proportional(10.5), p.danger, 200.0);
                        painter.galley(pos2(r.left() + 22.0 + gw, r.top() + 9.0), t, p.danger);
                    }
                    let lm = format!("Left: {}", meta(&row.left));
                    let rm = format!("Right: {}", meta(&row.right));
                    let half = (text_w / 2.0 - 6.0).max(60.0);
                    let lg = trunc_galley(painter, &lm, FontId::proportional(11.5), p.text_secondary, half);
                    painter.galley(pos2(r.left() + 14.0, r.top() + 28.0), lg, p.text_secondary);
                    let rg = trunc_galley(painter, &rm, FontId::proportional(11.5), p.text_secondary, half);
                    painter.galley(pos2(r.left() + 14.0 + half + 12.0, r.top() + 28.0), rg, p.text_secondary);
                    // Choices.
                    let mut x = r.right() - choices_w;
                    let both = row.left.is_some() && row.right.is_some();
                    for res in [Resolution::Left, Resolution::Right, Resolution::KeepBoth, Resolution::Skip] {
                        let w = 78.0;
                        let cr = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(w, 26.0));
                        x += w + 4.0;
                        let ok = res != Resolution::KeepBoth || both;
                        let id = ui.id().with(("compare.res", i, res as u8));
                        let resp = ui.interact(cr, id, if ok { Sense::click() } else { Sense::hover() });
                        let on = current == res;
                        let accent = match res {
                            Resolution::Skip => p.text_secondary,
                            Resolution::KeepBoth => p.warning,
                            _ => p.accent,
                        };
                        let fill = if on {
                            accent.gamma_multiply(0.22)
                        } else if resp.hovered() && ok {
                            p.bg_surface3
                        } else {
                            p.bg_surface2
                        };
                        ui.painter().rect_filled(cr, CornerRadius::same(6), fill);
                        let fg = if !ok {
                            p.text_tertiary.gamma_multiply(0.4)
                        } else if on {
                            accent
                        } else {
                            p.text_secondary
                        };
                        ui.painter().text(
                            cr.center(),
                            Align2::CENTER_CENTER,
                            res.label(),
                            FontId::proportional(11.5),
                            fg,
                        );
                        let tip = match res {
                            Resolution::Left if row.left.is_none() => "Delete it on the right too",
                            Resolution::Right if row.right.is_none() => "Delete it on the left too",
                            Resolution::Left => "Copy the left version over the right one",
                            Resolution::Right => "Copy the right version over the left one",
                            Resolution::KeepBoth => {
                                "Left keeps the name; the right version is kept as a conflict copy on both sides"
                            }
                            Resolution::Skip => "Decide later",
                        };
                        if ok && resp.on_hover_text(tip).clicked() {
                            set = Some((i, res));
                        }
                    }
                }
            },
        );
        if let Some((i, res)) = set {
            self.rows[i].action = resolution_action(&self.rows[i], res);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if button(ui, Some(ph::ARROW_LEFT), "Back", 0, true).clicked() {
                self.stage = Stage::Table;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, Some(ph::ARROW_RIGHT), "Review sync…", 1, true).clicked() {
                    self.stage = Stage::Preview;
                }
            });
        });
    }
}

/// Four action buttons (← · → ✕) in `rect`; returns the clicked action.
fn action_buttons(
    ui: &mut Ui,
    rect: Rect,
    cat: Option<Category>,
    current: Option<RowAction>,
    tip: &str,
) -> Option<RowAction> {
    let p = palette(ui.ctx());
    let items = [
        (RowAction::CopyLeft, ph::ARROW_LEFT, "Copy to left"),
        (RowAction::Skip, ph::MINUS, "Skip"),
        (RowAction::CopyRight, ph::ARROW_RIGHT, "Copy to right"),
        (RowAction::Delete, ph::TRASH, "Delete"),
    ];
    let w = rect.width() / items.len() as f32;
    let mut out = None;
    if current.is_some() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.bg_surface2.gamma_multiply(0.6));
    }
    for (k, (a, glyph, label)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + k as f32 * w, rect.top()), vec2(w, rect.height())).shrink(1.0);
        let ok = cat.is_none_or(|c| allowed(c, *a));
        let id = ui.id().with(("compare.act", rect.min.x as i32, rect.min.y as i32, k));
        let resp = ui.interact(r, id, if ok { Sense::click() } else { Sense::hover() });
        let on = current == Some(*a);
        let color = match a {
            RowAction::Delete => p.danger,
            RowAction::Skip => p.text_secondary,
            _ => p.accent,
        };
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(5), color.gamma_multiply(0.22));
        } else if resp.hovered() && ok {
            ui.painter().rect_filled(r, CornerRadius::same(5), p.bg_surface3);
        }
        let fg = if !ok {
            p.text_tertiary.gamma_multiply(0.35)
        } else if on {
            color
        } else {
            p.text_tertiary
        };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *glyph, icons::font(13.0), fg);
        let hint = if tip.is_empty() { label.to_string() } else { format!("{label} ({tip})") };
        if ok && resp.on_hover_text(hint).clicked() {
            out = Some(*a);
        }
    }
    out
}

fn plan_line(ui: &mut Ui, p: &Palette, s: &PlanStats) {
    let small = |t: String, c: Color32| RichText::new(t).font(FontId::proportional(12.0)).color(c);
    ui.label(small(format!("{} → right", s.to_right), if s.to_right > 0 { p.accent } else { p.text_tertiary }));
    ui.label(small("·".into(), p.text_tertiary));
    ui.label(small(format!("{} ← left", s.to_left), if s.to_left > 0 { p.accent } else { p.text_tertiary }));
    ui.label(small("·".into(), p.text_tertiary));
    let del = s.delete_left + s.delete_right;
    ui.label(small(format!("{del} to delete"), if del > 0 { p.danger } else { p.text_tertiary }));
    if s.bytes > 0 {
        ui.label(small("·".into(), p.text_tertiary));
        ui.label(small(format!("{} to copy", format_size(s.bytes, SizeStyle::Binary)), p.text_secondary));
    }
}

/// Centered glyph + title + subtitle filling the remaining space.
pub fn empty_note(ui: &mut Ui, glyph: &str, title: &str, sub: &str) {
    let p = palette(ui.ctx());
    let (r, _) = ui.allocate_exact_size(ui.available_size().max(vec2(10.0, 120.0)), Sense::hover());
    let c = r.center();
    ui.painter().text(c - vec2(0.0, 28.0), Align2::CENTER_CENTER, glyph, icons::font(34.0), p.text_tertiary);
    ui.painter().text(
        c + vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        title,
        crate::ui::views::flow::semibold(14.0),
        p.text_primary,
    );
    let g = ui.painter().layout(sub.to_string(), FontId::proportional(12.0), p.text_secondary, r.width() * 0.7);
    ui.painter().galley(pos2(c.x - g.size().x / 2.0, c.y + 22.0), g, p.text_secondary);
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
    fn profiles_serialize() {
        let f = ProfileFile {
            profiles: vec![SyncProfile {
                name: "Backup".into(),
                left: "/src".into(),
                right: "/dst".into(),
                method: Method::Content,
                preset: Preset::MirrorRight,
                schedule: super::super::sync_sched::Schedule::Daily { hour: 7, minute: 30 },
            }],
        };
        let text = toml::to_string_pretty(&f).unwrap();
        let back: ProfileFile = toml::from_str(&text).unwrap();
        assert_eq!(back.profiles, f.profiles);
        assert!(text.contains("mirror_right"));
        // Profiles saved before schedules existed still load (manual).
        let old: ProfileFile = toml::from_str("[[profiles]]\nname = \"x\"\nleft = \"/a\"\nright = \"/b\"\n").unwrap();
        assert_eq!(old.profiles[0].schedule, super::super::sync_sched::Schedule::Manual);
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
        for res in [Resolution::Left, Resolution::Right, Resolution::KeepBoth, Resolution::Skip] {
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
}
