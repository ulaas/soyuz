//! The view of the folder compare and sync screen, a full-screen overlay (Phase T5): drawn into
//! `Regions::area` after Quick Look (`HitTarget::Overlay` over everything first). The GUI's
//! compare window (`ui/tools/compare.rs`) in cells, over the core's `CompareModel`:
//!
//! - the title row (the folders compared, `×` closes);
//! - the setup: the left and right folder fields, the method and the sync mode, then a bar
//!   with Compare (Cancel while scanning), Swap and Profiles, or the folder completions
//!   while a folder field has the keyboard;
//! - the body: a note (nothing compared yet, in sync, failed), the progress of a scan / a
//!   sync job, the review table (category chips with counts, the name filter, one row per
//!   difference: left side, category bar, name with its tag, action, right side; the plan
//!   summary and Review / Conflicts below), the plan (its actions and Start sync) or the
//!   profiles page (the profiles with their schedule and next run; under them, from
//!   [`PROFILE_DETAIL_MIN_ROWS`] rows, the selected profile's schedule, last run and run
//!   reports; New, Edit, Rename, Delete, Save setup, Run now, Dry run / Cancel);
//! - a key-hints row on screens at least [`HINTS_MIN_ROWS`] high.
//!
//! [`prepare`] pumps the model (`crate::input::compare::pump`) and builds [`CompareVm`]
//! into the input state; [`geometry`] is the layout both the view and the input use.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::compare::{
    Category, CompareModel, Method, PlanStats, Preset, ReviewStep, RowAction, Side, Stage, two_way_reason,
};
use soyuz_appcore::models::compare_profiles::{self, RunStatus};
use soyuz_appcore::tr;
use soyuz_core::{SizeStyle, format_size, format_time};
use soyuz_jobs::SyncAction;

use crate::input::InputState;
use crate::input::compare::{Page, State, Zone, button, field};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// The hints row shows from this height.
pub const HINTS_MIN_ROWS: u16 = 16;
/// The side columns show from this width.
pub const META_MIN_COLS: u16 = 70;
/// Width of the action column.
pub const ACTION_COLS: u16 = 5;
/// The profiles page shows the selected profile's details from this many body rows.
pub const PROFILE_DETAIL_MIN_ROWS: u16 = 8;
/// Rows of the details at most.
pub const PROFILE_DETAIL_MAX_ROWS: u16 = 10;

/// The profiles page's body: the list and the selected profile's details (empty when the
/// body is short).
pub fn profile_split(rows: Rect) -> (Rect, Rect) {
    if rows.height < PROFILE_DETAIL_MIN_ROWS {
        return (rows, Rect::default());
    }
    let h = (rows.height / 2).min(PROFILE_DETAIL_MAX_ROWS);
    let list = Rect::new(rows.x, rows.y, rows.width, rows.height - h);
    (list, Rect::new(rows.x, list.bottom(), rows.width, h))
}

/// The screen's layout (one-row parts; empty when they do not fit).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    pub header: Rect,
    pub left: Rect,
    pub right: Rect,
    pub method: Rect,
    pub preset: Rect,
    /// Buttons, or the folder completions.
    pub bar: Rect,
    /// Table: the chips; plan / profiles: the title.
    pub chips: Rect,
    /// Table: the column titles; plan / profiles: a note.
    pub cols: Rect,
    /// The rows (table, plan, profiles) or the note / progress.
    pub rows: Rect,
    pub footer: Rect,
    pub hints: Rect,
    /// Width of each side column of the table (0: hidden).
    pub meta_w: u16,
    /// The action column of the table.
    pub act_x: u16,
    pub act_w: u16,
}

fn take(area: Rect, y: &mut u16, end: u16) -> Rect {
    if *y < end {
        let r = Rect::new(area.x, *y, area.width, 1);
        *y += 1;
        r
    } else {
        Rect::default()
    }
}

/// The layout of the screen in `area`.
pub fn geometry(area: Rect) -> Geometry {
    let mut g = Geometry::default();
    if area.is_empty() {
        return g;
    }
    // One blank column on each side when there is room.
    let inner = if area.width > 8 { Rect::new(area.x + 1, area.y, area.width - 2, area.height) } else { area };
    let mut y = inner.y;
    let mut end = inner.bottom();
    g.header = take(inner, &mut y, end);
    g.left = take(inner, &mut y, end);
    g.right = take(inner, &mut y, end);
    g.method = take(inner, &mut y, end);
    g.preset = take(inner, &mut y, end);
    g.bar = take(inner, &mut y, end);
    if area.height >= HINTS_MIN_ROWS && end > y {
        end -= 1;
        g.hints = Rect::new(inner.x, end, inner.width, 1);
    }
    if end > y {
        end -= 1;
        g.footer = Rect::new(inner.x, end, inner.width, 1);
    }
    g.chips = take(inner, &mut y, end);
    g.cols = take(inner, &mut y, end);
    if end > y {
        g.rows = Rect::new(inner.x, y, inner.width, end - y);
    }
    let w = inner.width;
    g.act_w = ACTION_COLS.min(w);
    g.meta_w = if w >= META_MIN_COLS { ((w - g.act_w) / 5).min(26) } else { 0 };
    let right = if g.meta_w > 0 { g.meta_w + 1 } else { 0 };
    g.act_x = inner.x + w.saturating_sub(right + g.act_w);
    g
}

/// A one-line text field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextVm {
    pub label: String,
    /// The part shown (scrolled to the cursor).
    pub shown: String,
    /// Shown while the field is empty.
    pub placeholder: String,
    /// The cursor's column in `shown` while the field has the keyboard.
    pub cursor: Option<u16>,
}

/// A row of options (method, sync mode).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChoiceVm {
    pub label: String,
    pub options: Vec<String>,
    pub selected: Option<usize>,
    pub focused: bool,
}

/// A category chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipVm {
    pub cat: Category,
    pub label: String,
    pub count: usize,
    pub on: bool,
}

/// One difference of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowVm {
    /// Index into the visible rows (the click target).
    pub index: usize,
    pub rel: String,
    pub dir: bool,
    pub cat: Category,
    pub left: String,
    pub right: String,
    pub action: RowAction,
    pub tag: String,
    pub tag_role: Role,
    pub cursor: bool,
}

/// The review table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableVm {
    pub chips: Vec<ChipVm>,
    pub identical: String,
    pub filter: TextVm,
    /// The first-run note of a two-way sync.
    pub note: Option<String>,
    pub cols: [String; 3],
    /// The rows in view.
    pub rows: Vec<RowVm>,
    /// Visible rows in all (after the chips and the filter).
    pub total: usize,
    /// The rows have the keyboard.
    pub focused: bool,
    pub stats: Vec<(String, Role)>,
    /// The conflicts button's label (two-way conflicts exist), and whether some are open.
    pub conflicts: Option<(String, bool)>,
    pub can_review: bool,
}

/// One action of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRowVm {
    pub glyph: &'static str,
    pub verb: String,
    pub rel: String,
    pub role: Role,
}

/// The plan preview.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanVm {
    pub title: String,
    pub stats: Vec<(String, Role)>,
    pub notes: Vec<(String, Role)>,
    /// The actions in view.
    pub rows: Vec<PlanRowVm>,
    pub total: usize,
    pub run_label: String,
    /// The plan deletes something (the run button is drawn as a warning).
    pub danger: bool,
}

/// One saved sync profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileVm {
    pub name: String,
    pub paths: String,
    /// "Next run" or what it is doing now.
    pub status: String,
    /// It is running now (planning, syncing, recording).
    pub running: bool,
    pub cursor: bool,
    /// Index (the click target).
    pub index: usize,
}

/// The selected profile's details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileDetailVm {
    /// "Daily at 18:00 · Next: today 18:00".
    pub schedule: String,
    /// What it is doing, its last run, or "Never run".
    pub last: (String, Role),
    pub reports_title: String,
    /// The run reports, newest first (or "Loading…" / "No runs yet").
    pub reports: Vec<(String, Role)>,
}

/// The profiles page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfilesVm {
    pub title: String,
    pub note: String,
    pub rows: Vec<ProfileVm>,
    pub empty: Option<(String, String)>,
    pub detail: Option<ProfileDetailVm>,
    /// The selected profile is running (Cancel instead of Run now / Dry run).
    pub running: bool,
}

/// The body under the setup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BodyVm {
    #[default]
    Nothing,
    /// A centred note (title, text, the title's role).
    Note(String, String, Role),
    /// A running scan / sync job / state recording (text, done, total).
    Busy(String, u64, u64),
    Table(TableVm),
    Plan(PlanVm),
    Profiles(ProfilesVm),
}

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareVm {
    pub visible: bool,
    pub title: String,
    /// The folders the rows belong to.
    pub sub: String,
    pub left: TextVm,
    pub right: TextVm,
    pub method: ChoiceVm,
    pub preset: ChoiceVm,
    pub scanning: bool,
    /// Folder completions of the focused folder field, and the highlighted one.
    pub completions: Vec<String>,
    pub pick: usize,
    pub page: Page,
    pub body: BodyVm,
    pub hints: String,
}

impl CompareVm {
    pub fn from_core(_core: &AppCore, input: &InputState, _regions: &Regions) -> CompareVm {
        if input.compare.open { input.compare.vm.clone() } else { CompareVm::default() }
    }
}

/// It is drawn this frame (the screen is open). The input state knows; the core does not.
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// A category's colour role (the GUI's `CategoryColor`).
pub fn category_role(c: Category) -> Role {
    match c {
        Category::OnlyLeft => Role::Accent,
        Category::NewerLeft => Role::Success,
        Category::Differ => Role::Danger,
        Category::NewerRight => Role::Warning,
        Category::OnlyRight => Role::Video,
    }
}

/// The glyph of a row action (ASCII look-alikes come from `borders::asciify`).
pub fn action_glyph(a: RowAction) -> &'static str {
    match a {
        RowAction::CopyLeft => "←",
        RowAction::CopyRight => "→",
        RowAction::Skip => "·",
        RowAction::Delete => "✕",
        RowAction::KeepBoth => "«»",
    }
}

/// One side of a row: size and date, "Folder · date", or `missing` when absent.
pub fn meta(core: &AppCore, s: Option<&Side>, missing: &str) -> String {
    let cfg = &core.config.general;
    let date = |s: &Side| s.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default();
    match s {
        None => missing.to_string(),
        Some(m) if m.dir => tr!("tools-compare-meta-folder", modified = date(m)),
        Some(m) => format!("{} · {}", format_size(m.size, cfg.size_style), date(m)),
    }
}

fn stats_spans(s: &PlanStats) -> Vec<(String, Role)> {
    let role = |n: usize, on: Role| if n > 0 { on } else { Role::Muted };
    let del = s.delete_left + s.delete_right;
    let mut v = vec![
        (tr!("tools-plan-to-right", count = s.to_right), role(s.to_right, Role::Accent)),
        (tr!("tools-plan-to-left", count = s.to_left), role(s.to_left, Role::Accent)),
        (tr!("tools-plan-to-delete", count = del), role(del, Role::Danger)),
    ];
    if s.bytes > 0 {
        v.push((tr!("tools-plan-to-copy", size = format_size(s.bytes, SizeStyle::Binary)), Role::Text));
    }
    v
}

fn text_vm(
    label: &str,
    placeholder: &str,
    text: &str,
    edit: &mut crate::widgets::LineEdit,
    w: u16,
    focused: bool,
) -> TextVm {
    let (shown, col) = edit.view(text, w.max(1));
    TextVm { label: label.into(), shown, placeholder: placeholder.into(), cursor: focused.then_some(col) }
}

/// Width of the setup labels in `w` columns.
fn label_w(vm: &CompareVm, w: u16) -> u16 {
    let l = [&vm.left.label, &vm.right.label, &vm.method.label, &vm.preset.label]
        .iter()
        .map(|s| text::width(s))
        .max()
        .unwrap_or(0);
    (l as u16 + 1).min(w / 3)
}

/// Step 1 of a frame (`view::prepare`): pump the model, then build the view model.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    let st = &mut input.compare;
    if !st.open {
        return;
    }
    st.geo = geometry(regions.area);
    crate::input::compare::pump(core, st);
    let Some(model) = st.model.clone() else { return };
    let m = model.borrow();
    st.vm = build(core, st, &m);
}

fn build(core: &AppCore, st: &mut State, m: &CompareModel) -> CompareVm {
    let g = st.geo;
    let mut vm = CompareVm {
        visible: true,
        title: tr!("tools-compare-title").into(),
        sub: m
            .compared()
            .map(|(l, r)| format!("{}  ←→  {}", soyuz_core::entry::display_name(l), soyuz_core::entry::display_name(r)))
            .unwrap_or_default(),
        scanning: m.is_scanning(),
        page: st.page,
        ..CompareVm::default()
    };
    vm.method = ChoiceVm {
        label: tr!("tools-compare-by").into(),
        options: Method::ALL.iter().map(|x| x.label().to_string()).collect(),
        selected: Method::ALL.iter().position(|x| *x == m.method),
        focused: st.zone == Zone::Method && st.page == Page::Compare,
    };
    vm.preset = ChoiceVm {
        label: tr!("tools-compare-sync-mode").into(),
        options: Preset::ALL.iter().map(|x| x.label().to_string()).collect(),
        selected: Preset::ALL.iter().position(|x| *x == m.preset),
        focused: st.zone == Zone::Preset && st.page == Page::Compare,
    };
    vm.left.label = tr!("tools-compare-left-hint").into();
    vm.right.label = tr!("tools-compare-right-hint").into();
    let lw = label_w(&vm, g.left.width);
    let fw = g.left.width.saturating_sub(lw);
    let on = |z: Zone| st.zone == z && st.page == Page::Compare;
    vm.left = text_vm(&vm.left.label, tr!("tools-compare-left-hint"), &m.left, &mut st.left_edit, fw, on(Zone::Left));
    vm.right =
        text_vm(&vm.right.label, tr!("tools-compare-right-hint"), &m.right, &mut st.right_edit, fw, on(Zone::Right));
    vm.completions = st.names.clone();
    vm.pick = st.pick;
    vm.hints = hints(st, m);
    vm.body = if st.page == Page::Profiles { profiles_body(core, st) } else { body(core, st, m) };
    vm
}

fn hints(st: &State, m: &CompareModel) -> String {
    if st.page == Page::Profiles {
        return tr!("tui-cmpprof-hints-page").into();
    }
    if m.is_scanning() {
        return tr!("tui-compare-hints-scanning").into();
    }
    if matches!(m.stage(), Stage::Review) && m.step == ReviewStep::Plan {
        return tr!("tui-compare-hints-plan").into();
    }
    if st.zone == Zone::Rows {
        return tr!("tui-compare-hints-rows").into();
    }
    tr!("tui-compare-hints-setup").into()
}

fn body(core: &AppCore, st: &mut State, m: &CompareModel) -> BodyVm {
    match m.stage() {
        Stage::Setup if m.rows().is_empty() => {
            return BodyVm::Note(
                tr!("tools-compare-empty").into(),
                tr!("tools-compare-empty-body").into(),
                Role::Title,
            );
        }
        Stage::Scanning { started, .. } => {
            let secs = soyuz_core::locale::format_decimal(started.elapsed().as_secs_f64(), 1);
            return BodyVm::Busy(tr!("tools-compare-running", secs = secs), 0, 0);
        }
        Stage::Submitted(job) => {
            let (done, total) =
                core.fileops.jobs.snapshot(*job).map_or((0, 0), |s| (s.progress.items_done, s.progress.items_total));
            return BodyVm::Busy(tr!("tools-compare-syncing").into(), done, total);
        }
        Stage::Recording(_) => return BodyVm::Busy(tr!("tools-compare-recording").into(), 0, 0),
        Stage::Failed(e) => return BodyVm::Note(tr!("tools-compare-failed").into(), e.clone(), Role::Danger),
        _ => {}
    }
    if m.step == ReviewStep::Plan {
        return plan_body(st, m);
    }
    if m.rows().is_empty() {
        return BodyVm::Note(
            tr!("tools-compare-in-sync").into(),
            tr!("tools-compare-in-sync-body").into(),
            Role::Success,
        );
    }
    BodyVm::Table(table_body(core, st, m))
}

fn table_body(core: &AppCore, st: &mut State, m: &CompareModel) -> TableVm {
    let g = st.geo;
    let counts = m.counts();
    let chips = Category::ALL
        .iter()
        .enumerate()
        .map(|(i, c)| ChipVm { cat: *c, label: c.label().into(), count: counts[i], on: m.show[i] })
        .collect();
    let vis = m.visible_rows();
    let height = usize::from(g.rows.height).max(1);
    let mut list = crate::widgets::SelectList { cursor: m.cursor, top: st.top };
    list.clamp(vis.len(), height);
    list.show_cursor(height);
    st.top = list.top;
    let two_way = m.two_way().is_some();
    let focused = st.zone == Zone::Rows;
    let rows = list
        .visible(vis.len(), usize::from(g.rows.height))
        .map(|k| {
            let r = &m.rows()[vis[k]];
            let (tag, tag_role) = match r.conflict {
                Some(_) if r.action == RowAction::Skip => (tr!("tools-git-conflict").to_string(), Role::Danger),
                Some(_) => (tr!("tools-compare-resolved").to_string(), Role::Success),
                None if two_way => (two_way_reason(r).to_string(), category_role(r.cat)),
                None => (r.cat.label().to_string(), category_role(r.cat)),
            };
            RowVm {
                index: k,
                rel: r.rel.to_string_lossy().into_owned(),
                dir: r.is_dir(),
                cat: r.cat,
                left: meta(core, r.left.as_ref(), "—"),
                right: meta(core, r.right.as_ref(), "—"),
                action: r.action,
                tag,
                tag_role,
                cursor: k == list.cursor,
            }
        })
        .collect();
    let (conflicts, open) = m.conflict_counts();
    let conflicts = (conflicts > 0).then(|| {
        let label = if open > 0 {
            tr!("tools-compare-resolve-n", count = open)
        } else {
            tr!("tools-compare-conflicts-button").to_string()
        };
        (label, open > 0)
    });
    let filter_w = filter_width(g.chips.width);
    let stats = m.stats();
    TableVm {
        chips,
        identical: tr!("tools-compare-identical", count = m.same()),
        filter: text_vm(
            "/",
            tr!("tools-compare-filter-hint"),
            &m.search,
            &mut st.filter_edit,
            filter_w.saturating_sub(2),
            st.zone == Zone::Filter,
        ),
        note: (m.first_run() && !m.rows().is_empty()).then(|| tr!("tools-compare-first-run").to_string()),
        cols: [
            tr!("tools-compare-col-left").into(),
            tr!("tools-compare-col-name").into(),
            tr!("tools-compare-col-right").into(),
        ],
        rows,
        total: vis.len(),
        focused,
        can_review: stats.total() > 0,
        stats: stats_spans(&stats),
        conflicts,
    }
}

/// Width of the filter field in the chips row.
fn filter_width(w: u16) -> u16 {
    (w / 4).clamp(0, 28)
}

fn plan_body(st: &mut State, m: &CompareModel) -> BodyVm {
    let Some(plan) = m.plan() else { return BodyVm::Nothing };
    let stats = m.stats();
    let (_, open) = m.conflict_counts();
    let mut notes = Vec::new();
    if stats.deletes() {
        notes.push((tr!("tools-compare-plan-trash-note").to_string(), Role::Warning));
    }
    if open > 0 {
        notes.push((tr!("tools-compare-plan-open-conflicts", count = open), Role::Warning));
    }
    let height = usize::from(st.geo.rows.height);
    let total = plan.actions.len();
    st.plan_top = st.plan_top.min(total.saturating_sub(height.max(1)));
    let rows = plan
        .actions
        .iter()
        .skip(st.plan_top)
        .take(height)
        .map(|a| {
            let (glyph, verb, rel, role) = match a {
                SyncAction::CopyLR(x) => ("→", tr!("tools-compare-copy-right"), x, Role::Accent),
                SyncAction::CopyRL(x) => ("←", tr!("tools-compare-copy-left"), x, Role::Accent),
                SyncAction::DeleteL(x) => ("✕", tr!("tools-compare-delete-left"), x, Role::Danger),
                SyncAction::DeleteR(x) => ("✕", tr!("tools-compare-delete-right"), x, Role::Danger),
                SyncAction::CopyAs { dst_left, to, .. } => {
                    let verb = if *dst_left {
                        tr!("tools-compare-conflict-copy-left")
                    } else {
                        tr!("tools-compare-conflict-copy-right")
                    };
                    ("«»", verb, to, Role::Warning)
                }
            };
            PlanRowVm { glyph, verb: verb.into(), rel: rel.to_string_lossy().into_owned(), role }
        })
        .collect();
    BodyVm::Plan(PlanVm {
        title: tr!("tools-compare-plan-title").into(),
        stats: stats_spans(&stats),
        notes,
        rows,
        total,
        run_label: tr!("tools-compare-start-sync", count = total),
        danger: stats.deletes(),
    })
}

/// A run status's colour role (the GUI's `status_color`).
pub fn status_role(s: RunStatus) -> Role {
    match s {
        RunStatus::InSync | RunStatus::Applied => Role::Success,
        RunStatus::DryRun => Role::Accent,
        RunStatus::Partial => Role::Warning,
        RunStatus::Failed => Role::Danger,
        RunStatus::Cancelled => Role::Muted,
    }
}

fn profile_detail(core: &AppCore, st: &State, enabled: bool) -> Option<ProfileDetailVm> {
    let p = st.profile_list.get(st.profiles.cursor)?;
    let schedule = tr!(
        "tui-cmpprof-schedule-line",
        schedule = p.schedule.label(),
        next = compare_profiles::next_label(p, enabled)
    );
    let last = match (compare_profiles::activity(&p.name), compare_profiles::last_report(&p.name)) {
        (Some(what), _) => (what.to_string(), Role::Accent),
        (None, Some(r)) => {
            let when = soyuz_core::time::format_time_at(
                r.finished.unwrap_or(r.started),
                &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Relative),
                std::time::SystemTime::now(),
            );
            let text = tr!("tools-profiles-last-run", when = when, trigger = r.trigger.label(), summary = r.summary());
            (format!("{} · {text}", r.status.label()), status_role(r.status))
        }
        (None, None) => (tr!("tools-profiles-never-run").to_string(), Role::Muted),
    };
    let date = &core.config.general.date_format;
    let reports = match st.reports.as_ref().filter(|(n, _)| *n == p.name) {
        None => vec![(tr!("common-loading").to_string(), Role::Muted)],
        Some((_, v)) if v.is_empty() => vec![(tr!("tools-profiles-no-runs").to_string(), Role::Muted)],
        Some((_, v)) => v
            .iter()
            .map(|r| {
                let line = tr!(
                    "tui-cmpprof-report-line",
                    when = format_time(r.started, date),
                    status = r.status.label(),
                    trigger = r.trigger.label(),
                    summary = r.summary()
                );
                (line, status_role(r.status))
            })
            .collect(),
    };
    Some(ProfileDetailVm { schedule, last, reports_title: tr!("tools-profiles-reports").into(), reports })
}

fn profiles_body(core: &AppCore, st: &State) -> BodyVm {
    let enabled = core.config.sync.scheduler;
    let (list_r, detail_r) = profile_split(st.geo.rows);
    let height = usize::from(list_r.height);
    let list = &st.profile_list;
    let rows = st
        .profiles
        .visible(list.len(), height)
        .map(|i| {
            let p = &list[i];
            let activity = compare_profiles::activity(&p.name);
            ProfileVm {
                name: p.name.clone(),
                paths: tr!(
                    "tools-profiles-card-paths",
                    left = compare_profiles::short_path(&p.left),
                    right = compare_profiles::short_path(&p.right),
                    mode = p.preset.label(),
                    method = p.method.label()
                ),
                status: activity.map(str::to_string).unwrap_or_else(|| {
                    format!("{} · {}", p.schedule.label(), compare_profiles::next_label(p, enabled))
                }),
                running: activity.is_some(),
                cursor: i == st.profiles.cursor,
                index: i,
            }
        })
        .collect();
    BodyVm::Profiles(ProfilesVm {
        title: format!("{} · {}", tr!("tools-profiles-title"), tr!("tools-profiles-count", count = list.len())),
        note: if enabled { tr!("tools-profiles-scheduler-on") } else { tr!("tools-profiles-scheduler-off") }.into(),
        rows,
        empty: list
            .is_empty()
            .then(|| (tr!("tools-profiles-empty").to_string(), tr!("tools-profiles-empty-body").to_string())),
        detail: if detail_r.is_empty() { None } else { profile_detail(core, st, enabled) },
        running: list.get(st.profiles.cursor).is_some_and(|p| compare_profiles::is_running(&p.name)),
    })
}

// ---------------------------------------------------------------------------------------
// Drawing.

/// Write `s` at `(x, y)`, at most up to `end` (cut with "…"); returns the next column.
fn put(buf: &mut Buffer, x: u16, y: u16, end: u16, s: &str, style: Style) -> u16 {
    if x >= end || s.is_empty() {
        return x;
    }
    let max = usize::from(end - x);
    let s = text::truncate_end(s, max);
    let (nx, _) = buf.set_stringn(x, y, &s, max, style);
    nx.min(end)
}

/// Write `s` as a click target; returns the next column.
#[allow(clippy::too_many_arguments)]
fn put_hit(buf: &mut Buffer, x: u16, y: u16, end: u16, s: &str, style: Style, hits: &mut Hits, t: HitTarget) -> u16 {
    let nx = put(buf, x, y, end, s, style);
    if nx > x {
        hits.push(Rect::new(x, y, nx - x, 1), t);
    }
    nx
}

/// A `[label]` button.
#[allow(clippy::too_many_arguments)]
fn btn(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    end: u16,
    label: &str,
    role: Role,
    theme: &TuiTheme,
    hits: &mut Hits,
    id: u8,
) -> u16 {
    let nx = put_hit(buf, x, y, end, &format!("[{label}]"), theme.style(role), hits, HitTarget::CompareButton(id));
    nx.saturating_add(1).min(end)
}

/// Buttons drawn from the right edge of `r` (rightmost last in `items`).
fn right_buttons(buf: &mut Buffer, r: Rect, items: &[(String, Role, u8)], theme: &TuiTheme, hits: &mut Hits) -> u16 {
    let w: u16 = items.iter().map(|(l, ..)| text::width(l) as u16 + 3).sum();
    let mut x = r.right().saturating_sub(w).max(r.x);
    let start = x;
    for (label, role, id) in items {
        x = btn(buf, x, r.y, r.right(), label, *role, theme, hits, *id);
    }
    start
}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_compare(f: &mut Frame<'_>, area: Rect, m: &CompareVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    hits.push(area, HitTarget::Overlay);
    f.render_widget(Clear, area);
    f.buffer_mut().set_style(area, theme.base().patch(theme.style(Role::Text)));
    let g = geometry(area);
    let mut cursor = None;
    {
        let buf = f.buffer_mut();
        header(buf, g.header, m, theme, hits);
        let lw = label_w(m, g.left.width);
        cursor = cursor.or(text_row(buf, g.left, lw, &m.left, theme, hits, HitTarget::CompareField(field::LEFT)));
        cursor = cursor.or(text_row(buf, g.right, lw, &m.right, theme, hits, HitTarget::CompareField(field::RIGHT)));
        choice_row(buf, g.method, lw, &m.method, theme, hits, field::METHOD, field::METHOD_OPTION);
        choice_row(buf, g.preset, lw, &m.preset, theme, hits, field::PRESET, field::PRESET_OPTION);
        bar(buf, g.bar, m, theme, hits);
        match &m.body {
            BodyVm::Nothing => {}
            BodyVm::Note(t, b, role) => note(buf, g, t, b, *role, theme),
            BodyVm::Busy(t, done, total) => busy(buf, g, t, *done, *total, theme),
            BodyVm::Table(t) => cursor = table(buf, &g, t, theme, hits).or(cursor),
            BodyVm::Plan(p) => plan(buf, &g, p, theme, hits),
            BodyVm::Profiles(p) => profiles(buf, &g, p, theme, hits),
        }
        if !g.hints.is_empty() {
            put(buf, g.hints.x, g.hints.y, g.hints.right(), &m.hints, theme.style(Role::KeyHint));
        }
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

fn header(buf: &mut Buffer, r: Rect, m: &CompareVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let close_w = 3.min(r.width);
    let end = r.right() - close_w;
    let x = put(buf, r.x, r.y, end, &m.title, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    if !m.sub.is_empty() {
        put(buf, x.saturating_add(2).min(end), r.y, end, &m.sub, theme.style(Role::Muted));
    }
    put_hit(buf, end, r.y, r.right(), " × ", theme.style(Role::Muted), hits, HitTarget::CompareButton(button::CLOSE));
}

/// A labelled text field; returns the cursor position while it has the keyboard.
fn text_row(
    buf: &mut Buffer,
    r: Rect,
    lw: u16,
    t: &TextVm,
    theme: &TuiTheme,
    hits: &mut Hits,
    target: HitTarget,
) -> Option<(u16, u16)> {
    if r.is_empty() {
        return None;
    }
    hits.push(r, target);
    let label_role = if t.cursor.is_some() { Role::Accent } else { Role::Muted };
    put(buf, r.x, r.y, r.x + lw.min(r.width), &t.label, theme.style(label_role));
    field_at(buf, Rect::new(r.x + lw.min(r.width), r.y, r.width.saturating_sub(lw), 1), t, theme)
}

/// The field itself in `r`.
fn field_at(buf: &mut Buffer, r: Rect, t: &TextVm, theme: &TuiTheme) -> Option<(u16, u16)> {
    if r.is_empty() {
        return None;
    }
    buf.set_style(r, theme.style(Role::Input));
    if t.shown.is_empty() && t.cursor.is_none() {
        put(buf, r.x, r.y, r.right(), &t.placeholder, theme.style(Role::Input).patch(theme.style(Role::Faint)));
    } else {
        put(buf, r.x, r.y, r.right(), &t.shown, theme.style(Role::Input));
    }
    t.cursor.map(|c| (r.x + c.min(r.width - 1), r.y))
}

#[allow(clippy::too_many_arguments)]
fn choice_row(buf: &mut Buffer, r: Rect, lw: u16, c: &ChoiceVm, theme: &TuiTheme, hits: &mut Hits, id: u8, first: u8) {
    if r.is_empty() {
        return;
    }
    hits.push(r, HitTarget::CompareField(id));
    let label_role = if c.focused { Role::Accent } else { Role::Muted };
    let mut x = put(buf, r.x, r.y, r.x + lw.min(r.width), &c.label, theme.style(label_role));
    x = x.max(r.x + lw.min(r.width));
    for (i, o) in c.options.iter().enumerate() {
        if x >= r.right() {
            break;
        }
        let on = c.selected == Some(i);
        let item = format!("({}) {o}", if on { "•" } else { " " });
        let style = match (on, c.focused) {
            (true, true) => theme.style(Role::PaletteSelected),
            (true, false) => theme.style(Role::Text).add_modifier(Modifier::BOLD),
            _ => theme.style(Role::Muted),
        };
        let t = HitTarget::CompareField(first.saturating_add(i as u8));
        x = put_hit(buf, x, r.y, r.right(), &item, style, hits, t).saturating_add(2).min(r.right());
    }
}

fn bar(buf: &mut Buffer, r: Rect, m: &CompareVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    if !m.completions.is_empty() {
        let mut x = put(buf, r.x, r.y, r.right(), "Tab ›", theme.style(Role::KeyHint));
        for (i, n) in m.completions.iter().enumerate() {
            x = x.saturating_add(1).min(r.right());
            let role = if i == m.pick { Role::PaletteSelected } else { Role::Folder };
            let t = HitTarget::CompareField(field::COMPLETION.saturating_add(i as u8));
            x = put_hit(buf, x, r.y, r.right(), n, theme.style(role), hits, t);
        }
        return;
    }
    let mut x = r.x;
    let (label, id) = if m.scanning {
        (format!("Esc {}", tr!("common-cancel")), button::COMPARE)
    } else {
        (format!("F5 {}", tr!("tools-compare-start")), button::COMPARE)
    };
    x = btn(buf, x, r.y, r.right(), &label, Role::ButtonFocused, theme, hits, id);
    x = btn(
        buf,
        x,
        r.y,
        r.right(),
        &format!("F6 {}", tr!("tools-compare-swap")),
        Role::Button,
        theme,
        hits,
        button::SWAP,
    );
    let pr = if m.page == Page::Profiles { Role::ButtonFocused } else { Role::Button };
    btn(buf, x, r.y, r.right(), &format!("F8 {}", tr!("tools-profiles")), pr, theme, hits, button::PROFILES);
}

/// A note centred in the body.
fn note(buf: &mut Buffer, g: Geometry, title: &str, body: &str, role: Role, theme: &TuiTheme) {
    let r = body_rect(&g);
    if r.is_empty() {
        return;
    }
    let mut lines = vec![(title.to_string(), theme.style(role).add_modifier(Modifier::BOLD))];
    lines.extend(text::wrap(body, usize::from(r.width)).into_iter().map(|l| (l, theme.style(Role::Muted))));
    let top = r.y + r.height.saturating_sub(lines.len() as u16) / 2;
    for ((s, style), y) in lines.iter().zip(top..r.bottom()) {
        let w = (text::width(s) as u16).min(r.width);
        put(buf, r.x + (r.width - w) / 2, y, r.right(), s, *style);
    }
}

/// The whole body (chips row to footer).
fn body_rect(g: &Geometry) -> Rect {
    let rs = [g.chips, g.cols, g.rows, g.footer];
    rs.iter().filter(|r| !r.is_empty()).fold(Rect::default(), |a, r| if a.is_empty() { *r } else { a.union(*r) })
}

fn busy(buf: &mut Buffer, g: Geometry, t: &str, done: u64, total: u64, theme: &TuiTheme) {
    let r = body_rect(&g);
    if r.is_empty() {
        return;
    }
    let y = r.y + r.height.saturating_sub(2) / 2;
    put(buf, r.x, y, r.right(), t, theme.style(Role::Muted));
    if total > 0 && y + 1 < r.bottom() {
        let label = tr!("tui-compare-progress", done = done, total = total);
        let lw = text::width(&label) as u16 + 1;
        let w = r.width.saturating_sub(lw);
        let fill = (u64::from(w) * done.min(total) / total) as u16;
        buf.set_style(Rect::new(r.x, y + 1, fill, 1), theme.style(Role::Progress));
        buf.set_style(Rect::new(r.x + fill, y + 1, w - fill, 1), theme.style(Role::ProgressTrack));
        put(buf, r.x + w + 1.min(lw), y + 1, r.right(), &label, theme.style(Role::Muted));
    }
}

/// Spans separated by " · ".
fn spans(buf: &mut Buffer, x: u16, y: u16, end: u16, items: &[(String, Role)], theme: &TuiTheme) -> u16 {
    let mut x = x;
    for (i, (s, role)) in items.iter().enumerate() {
        if i > 0 {
            x = put(buf, x, y, end, " · ", theme.style(Role::Faint));
        }
        x = put(buf, x, y, end, s, theme.style(*role));
    }
    x
}

fn table(buf: &mut Buffer, g: &Geometry, t: &TableVm, theme: &TuiTheme, hits: &mut Hits) -> Option<(u16, u16)> {
    let mut cursor = None;
    // Chips, identical count, filter.
    let r = g.chips;
    if !r.is_empty() {
        let fw = filter_width(r.width);
        let end = r.right().saturating_sub(fw + u16::from(fw > 0));
        let mut x = r.x;
        for (i, c) in t.chips.iter().enumerate() {
            let s = format!("{} {} {} {}", i + 1, if c.on { "[x]" } else { "[ ]" }, c.label, c.count);
            let style = if c.on { theme.style(category_role(c.cat)) } else { theme.style(Role::Muted) };
            x = put_hit(buf, x, r.y, end, &s, style, hits, HitTarget::CompareChip(i as u8)).saturating_add(2).min(end);
        }
        put(buf, x, r.y, end, &t.identical, theme.style(Role::Muted));
        if fw > 2 {
            let fr = Rect::new(r.right() - fw, r.y, fw, 1);
            hits.push(fr, HitTarget::CompareField(field::FILTER));
            put(buf, fr.x, fr.y, fr.right(), "/ ", theme.style(Role::KeyHint));
            cursor = field_at(buf, Rect::new(fr.x + 2, fr.y, fw - 2, 1), &t.filter, theme);
        }
    }
    // Column titles, or the first-run note.
    let r = g.cols;
    if !r.is_empty() {
        let muted = theme.style(Role::Muted);
        if let Some(n) = &t.note {
            put(buf, r.x, r.y, r.right(), n, muted);
        } else {
            let name_x = if g.meta_w > 0 { r.x + g.meta_w + 1 } else { r.x };
            if g.meta_w > 0 {
                put(buf, r.x, r.y, r.x + g.meta_w, &t.cols[0], muted);
                let rw = text::width(&t.cols[2]) as u16;
                put(buf, r.right().saturating_sub(rw), r.y, r.right(), &t.cols[2], muted);
            }
            put(buf, name_x + 2, r.y, g.act_x, &t.cols[1], muted);
        }
    }
    // Rows.
    let r = g.rows;
    for (row, y) in t.rows.iter().zip(r.y..r.bottom()) {
        draw_row(buf, g, Rect::new(r.x, y, r.width, 1), row, t.focused, theme, hits);
    }
    // Plan summary and buttons.
    let r = g.footer;
    if !r.is_empty() {
        let mut items = Vec::new();
        if let Some((label, open)) = &t.conflicts {
            items.push((format!("C {label}"), if *open { Role::Warning } else { Role::Button }, button::CONFLICTS));
        }
        let review_role = if t.can_review { Role::ButtonFocused } else { Role::Disabled };
        items.push((format!("Enter {}", tr!("tools-compare-review")), review_role, button::REVIEW));
        let start = right_buttons(buf, r, &items, theme, hits);
        spans(buf, r.x, r.y, start.saturating_sub(1), &t.stats, theme);
    }
    cursor
}

#[allow(clippy::too_many_arguments)]
fn draw_row(buf: &mut Buffer, g: &Geometry, r: Rect, row: &RowVm, focused: bool, theme: &TuiTheme, hits: &mut Hits) {
    hits.push(r, HitTarget::CompareRow(row.index));
    let base = if row.cursor {
        theme.style(if focused { Role::Cursor } else { Role::CursorInactive })
    } else {
        Style::default()
    };
    buf.set_style(r, base);
    let st = |role: Role| theme.style(role).patch(base);
    let mut x = r.x;
    if g.meta_w > 0 {
        let lead = matches!(row.cat, Category::OnlyLeft | Category::NewerLeft);
        put(buf, x, r.y, x + g.meta_w, &row.left, st(if lead { Role::Text } else { Role::Muted }));
        let trail = matches!(row.cat, Category::OnlyRight | Category::NewerRight);
        let w = (text::width(&row.right) as u16).min(g.meta_w);
        put(buf, r.right() - w, r.y, r.right(), &row.right, st(if trail { Role::Text } else { Role::Muted }));
        x += g.meta_w + 1;
    }
    let name_end = g.act_x.saturating_sub(1).max(x);
    x = put(buf, x, r.y, name_end, "│ ", st(category_role(row.cat)));
    let name = if row.dir { format!("{}/", row.rel) } else { row.rel.clone() };
    let role = match (row.dir, row.action) {
        (true, _) => Role::Folder,
        (_, RowAction::Skip) => Role::Muted,
        _ => Role::Text,
    };
    let tag_w = text::width(&row.tag) as u16 + 2;
    let room = name_end.saturating_sub(x);
    let name_room = if room > tag_w + 8 { room - tag_w } else { room };
    let nx = put(buf, x, r.y, x + name_room, &name, st(role));
    if name_room < room {
        put(buf, nx + 2, r.y, name_end, &row.tag, st(row.tag_role));
    }
    // Action, centred in its column.
    let g_ = action_glyph(row.action);
    let aw = g.act_w.min(r.right().saturating_sub(g.act_x));
    if aw > 0 {
        let gw = text::width(g_) as u16;
        let ax = g.act_x + aw.saturating_sub(gw) / 2;
        let role = match row.action {
            RowAction::Skip => Role::Muted,
            RowAction::Delete => Role::Danger,
            RowAction::KeepBoth => Role::Warning,
            _ => Role::Accent,
        };
        put(buf, ax, r.y, g.act_x + aw, g_, st(role).add_modifier(Modifier::BOLD));
    }
}

fn plan(buf: &mut Buffer, g: &Geometry, p: &PlanVm, theme: &TuiTheme, hits: &mut Hits) {
    let r = g.chips;
    if !r.is_empty() {
        let x = put(buf, r.x, r.y, r.right(), &p.title, theme.style(Role::Title).add_modifier(Modifier::BOLD));
        spans(buf, x.saturating_add(2).min(r.right()), r.y, r.right(), &p.stats, theme);
    }
    let r = g.cols;
    if !r.is_empty() {
        spans(buf, r.x, r.y, r.right(), &p.notes, theme);
    }
    let r = g.rows;
    let verb_w = p.rows.iter().map(|a| text::width(&a.verb)).max().unwrap_or(0) as u16 + 2;
    for (a, y) in p.rows.iter().zip(r.y..r.bottom()) {
        let x = put(buf, r.x, y, r.right(), a.glyph, theme.style(a.role));
        let x = put(buf, x + 1, y, r.right(), &a.verb, theme.style(Role::Muted)).max(r.x + 3 + verb_w);
        put(buf, x.min(r.right()), y, r.right(), &a.rel, theme.style(Role::Text));
    }
    let r = g.footer;
    if !r.is_empty() {
        let back = format!("Backspace {}", tr!("tools-back"));
        let x = btn(buf, r.x, r.y, r.right(), &back, Role::Button, theme, hits, button::BACK);
        let role = match (p.total > 0, p.danger) {
            (false, _) => Role::Disabled,
            (true, true) => Role::Warning,
            (true, false) => Role::ButtonFocused,
        };
        let items = [(format!("Enter {}", p.run_label), role, button::RUN)];
        let w = text::width(&items[0].0) as u16 + 3;
        if r.right().saturating_sub(w) >= x {
            right_buttons(buf, r, &items, theme, hits);
        }
    }
}

fn profiles(buf: &mut Buffer, g: &Geometry, p: &ProfilesVm, theme: &TuiTheme, hits: &mut Hits) {
    let r = g.chips;
    if !r.is_empty() {
        put(buf, r.x, r.y, r.right(), &p.title, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    }
    let r = g.cols;
    if !r.is_empty() {
        put(buf, r.x, r.y, r.right(), &p.note, theme.style(Role::Muted));
    }
    if let Some((t, b)) = &p.empty {
        let mut g2 = *g;
        g2.chips = Rect::default();
        g2.cols = Rect::default();
        g2.footer = Rect::default();
        note(buf, g2, t, b, Role::Title, theme);
    }
    let (r, detail_r) = profile_split(g.rows);
    let name_w = p.rows.iter().map(|x| text::width(&x.name)).max().unwrap_or(0).min(usize::from(r.width / 3)) as u16;
    for (row, y) in p.rows.iter().zip(r.y..r.bottom()) {
        let line = Rect::new(r.x, y, r.width, 1);
        hits.push(line, HitTarget::CompareProfile(row.index));
        let base = if row.cursor { theme.style(Role::Cursor) } else { Style::default() };
        buf.set_style(line, base);
        let st = |role: Role| theme.style(role).patch(base);
        let sw = (text::width(&row.status) as u16).min(r.width / 3);
        let end = r.right().saturating_sub(sw + 1);
        put(buf, r.x, y, end, &row.name, st(Role::Text).add_modifier(Modifier::BOLD));
        put(buf, (r.x + name_w + 2).min(end), y, end, &row.paths, st(Role::Muted));
        let role = if row.running { Role::Accent } else { Role::Text };
        put(buf, r.right() - sw, y, r.right(), &row.status, st(role));
    }
    if let Some(d) = &p.detail {
        profile_detail_rows(buf, detail_r, d, theme);
    }
    let r = g.footer;
    if !r.is_empty() {
        let back = format!("Esc {}", tr!("tools-back"));
        let mut x = btn(buf, r.x, r.y, r.right(), &back, Role::Button, theme, hits, button::BACK);
        let mut items: Vec<(String, Role, u8)> = Vec::new();
        if p.empty.is_none() {
            items.push((format!("Enter {}", tr!("tui-compare-profile-load")), Role::ButtonFocused, button::LOAD));
            if p.running {
                items.push((format!("X {}", tr!("common-cancel")), Role::Warning, button::CANCEL_RUN));
            } else {
                items.push((format!("R {}", tr!("tools-profiles-run-now")), Role::Button, button::RUN_PROFILE));
                items.push((format!("D {}", tr!("tools-status-dry-run")), Role::Button, button::DRY_RUN));
            }
            items.push((format!("E {}", tr!("tui-cmpprof-edit")), Role::Button, button::EDIT));
            items.push((format!("F2 {}", tr!("common-rename")), Role::Button, button::RENAME));
            items.push((format!("Del {}", tr!("common-delete")), Role::Button, button::DELETE));
        }
        items.push((format!("N {}", tr!("tui-cmpprof-new")), Role::Button, button::NEW));
        items.push((format!("S {}", tr!("tui-cmpprof-save-setup")), Role::Button, button::SAVE_SETUP));
        for (label, role, id) in &items {
            // Whole buttons only: one that does not fit is left out with the rest.
            if x.saturating_add(text::width(label) as u16 + 2) > r.right() {
                break;
            }
            x = btn(buf, x, r.y, r.right(), label, *role, theme, hits, *id);
        }
    }
}

/// The selected profile's schedule, last run and run reports in `r`.
fn profile_detail_rows(buf: &mut Buffer, r: Rect, d: &ProfileDetailVm, theme: &TuiTheme) {
    if r.is_empty() {
        return;
    }
    let mut lines: Vec<(&str, Style)> = vec![
        (&d.schedule, theme.style(Role::Text)),
        (&d.last.0, theme.style(d.last.1)),
        (&d.reports_title, theme.style(Role::Title).add_modifier(Modifier::BOLD)),
    ];
    lines.extend(d.reports.iter().map(|(s, role)| (s.as_str(), theme.style(*role))));
    // A faint rule above the details.
    let rule = "─".repeat(usize::from(r.width));
    put(buf, r.x, r.y, r.right(), &rule, theme.style(Role::Faint));
    for ((s, style), y) in lines.iter().zip(r.y + 1..r.bottom()) {
        put(buf, r.x, y, r.right(), s, *style);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn vm() -> CompareVm {
        let rows = (0..30)
            .map(|i| RowVm {
                index: i,
                rel: format!("dir/file{i}.txt"),
                dir: false,
                cat: Category::ALL[i % 5],
                left: "4 KiB · today".into(),
                right: "—".into(),
                action: RowAction::CHOICES[i % 4],
                tag: "Only left".into(),
                tag_role: Role::Accent,
                cursor: i == 2,
            })
            .collect();
        CompareVm {
            visible: true,
            title: "Compare & sync".into(),
            sub: "a ←→ b".into(),
            left: TextVm { label: "Left folder".into(), shown: "/tmp/a".into(), cursor: Some(6), ..TextVm::default() },
            right: TextVm { label: "Right folder".into(), shown: "/tmp/b".into(), ..TextVm::default() },
            method: ChoiceVm {
                label: "By".into(),
                options: vec!["Date".into(), "Size".into()],
                selected: Some(0),
                focused: false,
            },
            preset: ChoiceVm { label: "Mode".into(), options: vec!["Mirror".into()], selected: Some(0), focused: true },
            body: BodyVm::Table(TableVm {
                chips: Category::ALL
                    .iter()
                    .map(|c| ChipVm { cat: *c, label: c.label().into(), count: 3, on: true })
                    .collect(),
                rows,
                total: 30,
                focused: true,
                stats: vec![("3 → right".into(), Role::Accent)],
                conflicts: Some(("Resolve 2".into(), true)),
                can_review: true,
                ..TableVm::default()
            }),
            hints: "hints".into(),
            ..CompareVm::default()
        }
    }

    fn draw(m: &CompareVm, w: u16, h: u16) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_compare(f, f.area(), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        (lines, hits)
    }

    #[test]
    fn screen_rows_and_targets() {
        let (lines, hits) = draw(&vm(), 100, 30);
        assert!(lines[0].contains("Compare & sync"), "{lines:?}");
        assert!(lines[1].contains("/tmp/a"));
        let g = geometry(Rect::new(0, 0, 100, 30));
        assert_eq!(hits.at(g.left.x + 20, g.left.y), Some(HitTarget::CompareField(field::LEFT)));
        assert_eq!(hits.at(g.rows.x + 30, g.rows.y), Some(HitTarget::CompareRow(0)));
        assert_eq!(hits.at(g.chips.x, g.chips.y), Some(HitTarget::CompareChip(0)));
        assert_eq!(hits.at(98, 0), Some(HitTarget::CompareButton(button::CLOSE)));
        assert!(lines[usize::from(g.rows.y)].contains("file0.txt"));
        assert!(lines[usize::from(g.footer.y)].contains("Review"));
        assert!(g.act_x > g.rows.x && g.act_x + g.act_w <= g.rows.right());
        // Clicks never fall through.
        assert_eq!(hits.at(50, 29), Some(HitTarget::Overlay));
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let m = vm();
        let mut plan = vm();
        plan.body = BodyVm::Plan(PlanVm {
            title: "Plan".into(),
            rows: vec![PlanRowVm { glyph: "→", verb: "Copy".into(), rel: "a".into(), role: Role::Accent }],
            total: 1,
            run_label: "Start sync (1)".into(),
            ..PlanVm::default()
        });
        let mut prof = vm();
        prof.body = BodyVm::Profiles(ProfilesVm {
            title: "Profiles".into(),
            rows: vec![ProfileVm {
                name: "p".into(),
                paths: "a ←→ b".into(),
                status: "Manual".into(),
                running: false,
                cursor: true,
                index: 0,
            }],
            ..ProfilesVm::default()
        });
        let mut busy = vm();
        busy.body = BodyVm::Busy("Syncing".into(), 3, 10);
        let mut note = vm();
        note.body = BodyVm::Note("Empty".into(), "Pick two folders".into(), Role::Title);
        note.completions = vec!["sub".into(), "other".into()];
        for m in [&m, &plan, &prof, &busy, &note] {
            for w in 0..24 {
                for h in 0..20 {
                    draw(m, w, h);
                }
            }
            draw(m, 200, 3);
            draw(m, 3, 60);
        }
    }

    #[test]
    fn geometry_parts_do_not_overlap() {
        for h in 0..40 {
            let g = geometry(Rect::new(0, 0, 80, h));
            let parts =
                [g.header, g.left, g.right, g.method, g.preset, g.bar, g.chips, g.cols, g.rows, g.footer, g.hints];
            for (i, a) in parts.iter().enumerate() {
                assert!(a.bottom() <= h.max(a.y), "part {i} out of the area at {h}");
                for b in &parts[i + 1..] {
                    assert!(a.is_empty() || b.is_empty() || !a.intersects(*b), "overlap at height {h}");
                }
            }
        }
    }
}
