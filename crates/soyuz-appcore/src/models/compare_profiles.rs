//! Sync profiles and their scheduler (frontend-agnostic; moved from the GUI's
//! `ui/tools/compare.rs` and `ui/tools/sync_sched.rs`).
//!
//! Profiles (`sync_profiles.toml` next to the config) carry a [`Schedule`]: manual, every N
//! minutes, daily at a time, when a file changes (recursive watcher on both folders), or
//! once when the app starts. While Soyuz is open the scheduler ([`tick`], once per frame
//! on the UI thread, no I/O) starts due runs: a worker plans the sync
//! ([`compare::compute`](super::compare::compute)), the plan becomes a normal `Sync` job in
//! the activity panel, and when it ends a [`RunReport`] (what was / would be changed,
//! conflicts, status) is saved under `sync_reports/<profile>/`. A dry run (or
//! `sync.scheduled_dry_run`) only writes the report. There is no OS scheduler or service:
//! nothing runs while the app is closed; a daily run that was missed is caught up at the
//! next start.
//!
//! Unattended two-way runs never guess: conflicts are skipped (their base is kept) and
//! listed in the report with a toast pointing at the profiles window.
//!
//! Profile editing for every frontend: [`profiles`], [`profile`], [`create_profile`],
//! [`update_profile`], [`rename_profile`], [`delete_profile`], [`set_schedule`] (validated,
//! saved on the shared store worker), [`ScheduleKind`] with the editor's ranges, and
//! [`next_run`] / [`next_of`] / [`next_label`] for the "next run" column.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use crossbeam_channel::{Receiver, bounded, unbounded};
use serde::{Deserialize, Serialize};
use soyuz_core::OpControl;
use soyuz_core::time::serde_rfc3339;
use soyuz_jobs::{JobId, JobKind, JobState, SyncAction, SyncPlan};

use super::compare::{self, Method, Preset};
use super::compare_state::{self as sync_state, ConflictKind};
use crate::core::AppCore;
use crate::host::Host;
use crate::models::toasts::Toast;

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
    /// When the scheduler runs this profile on its own.
    #[serde(default)]
    pub schedule: Schedule,
}

impl SyncProfile {
    /// A manual profile (method "date + size").
    pub fn new(name: impl Into<String>, left: PathBuf, right: PathBuf, preset: Preset) -> Self {
        SyncProfile { name: name.into(), left, right, method: Method::default(), preset, schedule: Schedule::Manual }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct ProfileFile {
    #[serde(default)]
    profiles: Vec<SyncProfile>,
}

/// `sync_profiles.toml` inside `dir` (normally the config dir).
pub fn profiles_path_in(dir: &Path) -> PathBuf {
    dir.join("sync_profiles.toml")
}

/// Where this thread's profiles live (see [`set_store_dir`]).
pub fn profiles_path() -> PathBuf {
    profiles_path_in(&store_dir())
}

/// Load the profiles saved inside `dir` (small file; worker or when a window opens).
/// A broken file is backed up and protected from being overwritten (`store_file`).
pub fn load_profiles_in(dir: &Path) -> Vec<SyncProfile> {
    crate::services::store_file::load_toml::<ProfileFile>(&profiles_path_in(dir)).value().profiles
}

/// Load this thread's saved profiles.
pub fn load_profiles() -> Vec<SyncProfile> {
    load_profiles_in(&store_dir())
}

/// Save `profiles` inside `dir`, in order on the shared store worker (flushed on exit).
pub fn save_profiles_in(dir: &Path, profiles: &[SyncProfile]) {
    match toml::to_string_pretty(&ProfileFile { profiles: profiles.to_vec() }) {
        Ok(text) => crate::services::store_file::save_async(profiles_path_in(dir), text.into_bytes()),
        Err(e) => log::warn!("cannot save sync profiles: {e}"),
    }
}

/// Why a profile edit was refused (frontends word it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// The name is empty.
    EmptyName,
    /// Another profile has this name.
    Duplicate(String),
    /// No profile has this name.
    NotFound(String),
    /// A folder is empty.
    MissingPath,
    /// Both folders are the same.
    SamePaths,
    /// The profile is running (rename / delete after it ends, or cancel it).
    Running(String),
}

/// Check a profile before it is stored: trimmed non-empty name not used by another
/// profile (`except` = the profile being edited), two different folders. Returns the
/// profile with its name and folders trimmed and its schedule clamped.
pub fn validate(profiles: &[SyncProfile], p: SyncProfile, except: Option<&str>) -> Result<SyncProfile, ProfileError> {
    let name = p.name.trim().to_string();
    if name.is_empty() {
        return Err(ProfileError::EmptyName);
    }
    if profiles.iter().any(|o| o.name == name && Some(o.name.as_str()) != except) {
        return Err(ProfileError::Duplicate(name));
    }
    let trim = |p: &Path| PathBuf::from(p.to_string_lossy().trim());
    let (left, right) = (trim(&p.left), trim(&p.right));
    if left.as_os_str().is_empty() || right.as_os_str().is_empty() {
        return Err(ProfileError::MissingPath);
    }
    if left == right {
        return Err(ProfileError::SamePaths);
    }
    Ok(SyncProfile { name, left, right, schedule: p.schedule.clamped(), ..p })
}

// ---------------------------------------------------------------------------------------
// Schedules.

/// When a profile runs on its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    #[default]
    Manual,
    /// Every `minutes` (counted from the last run).
    Interval { minutes: u32 },
    /// Every day at `hour:minute` local time (a missed run is caught up at start).
    Daily { hour: u8, minute: u8 },
    /// When something changes in either folder (debounced, `sync.change_debounce_secs`).
    OnChange,
    /// Once, shortly after the app starts.
    OnStart,
}

/// Interval schedules: minutes between runs.
pub const INTERVAL_MINUTES: RangeInclusive<u32> = 1..=1440;
/// Daily schedules: hour of the day.
pub const DAILY_HOURS: RangeInclusive<u8> = 0..=23;
/// Daily schedules: minute of the hour.
pub const DAILY_MINUTES: RangeInclusive<u8> = 0..=59;

impl Schedule {
    pub fn label(&self) -> String {
        match self {
            Schedule::Manual => tr!("tools-sched-manual").into(),
            Schedule::Interval { minutes } if *minutes % 60 == 0 && *minutes >= 60 => {
                tr!("tools-sched-every-hours", count = minutes / 60)
            }
            Schedule::Interval { minutes } => tr!("tools-sched-every-minutes", count = *minutes),
            Schedule::Daily { hour, minute } => tr!("tools-sched-daily", time = format!("{hour:02}:{minute:02}")),
            Schedule::OnChange => tr!("tools-sched-on-change").into(),
            Schedule::OnStart => tr!("tools-sched-on-start").into(),
        }
    }

    pub fn kind(&self) -> ScheduleKind {
        match self {
            Schedule::Manual => ScheduleKind::Manual,
            Schedule::Interval { .. } => ScheduleKind::Interval,
            Schedule::Daily { .. } => ScheduleKind::Daily,
            Schedule::OnChange => ScheduleKind::OnChange,
            Schedule::OnStart => ScheduleKind::OnStart,
        }
    }

    /// The numbers kept inside the editor's ranges.
    pub fn clamped(self) -> Schedule {
        match self {
            Schedule::Interval { minutes } => {
                Schedule::Interval { minutes: minutes.clamp(*INTERVAL_MINUTES.start(), *INTERVAL_MINUTES.end()) }
            }
            Schedule::Daily { hour, minute } => Schedule::Daily {
                hour: hour.clamp(*DAILY_HOURS.start(), *DAILY_HOURS.end()),
                minute: minute.clamp(*DAILY_MINUTES.start(), *DAILY_MINUTES.end()),
            },
            other => other,
        }
    }
}

/// The kinds of [`Schedule`] (the editor's segmented control).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScheduleKind {
    Manual,
    Interval,
    Daily,
    OnChange,
    OnStart,
}

impl ScheduleKind {
    pub const ALL: [ScheduleKind; 5] = [
        ScheduleKind::Manual,
        ScheduleKind::Interval,
        ScheduleKind::Daily,
        ScheduleKind::OnChange,
        ScheduleKind::OnStart,
    ];

    /// Short label of the kind.
    pub fn label(self) -> &'static str {
        match self {
            ScheduleKind::Manual => tr!("tools-sched-manual"),
            ScheduleKind::Interval => tr!("tools-sched-kind-interval"),
            ScheduleKind::Daily => tr!("tools-sched-kind-daily"),
            ScheduleKind::OnChange => tr!("tools-sched-kind-change"),
            ScheduleKind::OnStart => tr!("tools-sched-kind-start"),
        }
    }

    /// The schedule a kind starts with when picked (hourly, daily at 18:00).
    pub fn default_schedule(self) -> Schedule {
        match self {
            ScheduleKind::Manual => Schedule::Manual,
            ScheduleKind::Interval => Schedule::Interval { minutes: 60 },
            ScheduleKind::Daily => Schedule::Daily { hour: 18, minute: 0 },
            ScheduleKind::OnChange => Schedule::OnChange,
            ScheduleKind::OnStart => Schedule::OnStart,
        }
    }

    /// Explanation shown under the editor for this kind.
    pub fn note(self) -> Option<&'static str> {
        match self {
            ScheduleKind::Daily => Some(tr!("tools-sched-daily-note")),
            ScheduleKind::OnChange => Some(tr!("tools-sched-change-note")),
            _ => None,
        }
    }
}

/// Delay of "on start" runs after the session started (the UI settles first).
pub const START_DELAY: Duration = Duration::from_secs(5);

/// Local time of `date` at `hour:minute` (DST gaps move forward an hour).
fn at<Tz: TimeZone>(tz: &Tz, date: NaiveDate, hour: u8, minute: u8) -> SystemTime {
    let naive = date.and_hms_opt(u32::from(hour.min(23)), u32::from(minute.min(59)), 0).unwrap_or_default();
    match tz.from_local_datetime(&naive).earliest() {
        Some(t) => SystemTime::from(t.with_timezone(&chrono::Utc)),
        None => {
            let later = naive + chrono::Duration::hours(1);
            tz.from_local_datetime(&later)
                .earliest()
                .map(|t| SystemTime::from(t.with_timezone(&chrono::Utc)))
                .unwrap_or_else(|| SystemTime::from(chrono::Utc.from_utc_datetime(&naive)))
        }
    }
}

/// When `s` should run next, given the last run, the session start and now (in `tz`).
/// `None` = not time-driven (manual, on change, or "on start" already done). A time at or
/// before `now` means "due now".
pub fn next_run<Tz: TimeZone>(
    s: &Schedule,
    last: Option<SystemTime>,
    session_start: SystemTime,
    now: SystemTime,
    tz: &Tz,
) -> Option<SystemTime> {
    match s {
        Schedule::Manual | Schedule::OnChange => None,
        Schedule::OnStart => last.is_none_or(|l| l < session_start).then_some(session_start + START_DELAY),
        Schedule::Interval { minutes } => {
            let every = Duration::from_secs(u64::from((*minutes).max(1)) * 60);
            Some(match last {
                Some(l) => l + every,
                None => session_start + START_DELAY,
            })
        }
        Schedule::Daily { hour, minute } => {
            let today = DateTime::<chrono::Utc>::from(now).with_timezone(tz).date_naive();
            let today_at = at(tz, today, *hour, *minute);
            let prev = if today_at <= now { today_at } else { at(tz, today.pred_opt()?, *hour, *minute) };
            let next = if today_at > now { today_at } else { at(tz, today.succ_opt()?, *hour, *minute) };
            match last {
                // Missed while the app was closed (or busy): catch up now.
                Some(l) if l < prev => Some(prev),
                _ => Some(next),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Reports.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Manual,
    Schedule,
    Change,
    Start,
}

impl Trigger {
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Manual => tr!("tools-trigger-manual"),
            Trigger::Schedule => tr!("tools-trigger-schedule"),
            Trigger::Change => tr!("tools-trigger-change"),
            Trigger::Start => tr!("tools-trigger-start"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Nothing to do.
    InSync,
    /// Dry run: the report lists what would change.
    DryRun,
    Applied,
    /// Applied with failed or skipped items.
    Partial,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            RunStatus::InSync => tr!("tools-status-in-sync"),
            RunStatus::DryRun => tr!("tools-status-dry-run"),
            RunStatus::Applied => tr!("tools-status-applied"),
            RunStatus::Partial => tr!("tools-status-partial"),
            RunStatus::Failed => tr!("tools-status-failed"),
            RunStatus::Cancelled => tr!("tools-cancelled"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportOp {
    CopyToRight,
    CopyToLeft,
    DeleteLeft,
    DeleteRight,
    /// "Keep both": the right version kept under another name.
    KeepCopy,
}

impl ReportOp {
    pub fn label(self) -> &'static str {
        match self {
            ReportOp::CopyToRight => tr!("tools-compare-copy-right"),
            ReportOp::CopyToLeft => tr!("tools-compare-copy-left"),
            ReportOp::DeleteLeft => tr!("tools-compare-delete-left"),
            ReportOp::DeleteRight => tr!("tools-compare-delete-right"),
            ReportOp::KeepCopy => tr!("tools-report-keep-copy"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAction {
    pub op: ReportOp,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportConflict {
    pub path: String,
    pub kind: ConflictKind,
}

/// Most actions kept in one report (the counts stay exact).
pub const REPORT_MAX_ACTIONS: usize = 5000;

/// One run (or dry run) of a profile, saved as JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunReport {
    pub profile: String,
    pub left: PathBuf,
    pub right: PathBuf,
    #[serde(with = "serde_rfc3339")]
    pub started: SystemTime,
    #[serde(default, with = "serde_rfc3339::option", skip_serializing_if = "Option::is_none")]
    pub finished: Option<SystemTime>,
    pub trigger: Trigger,
    pub dry_run: bool,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// Planned actions (capped at [`REPORT_MAX_ACTIONS`]).
    #[serde(default)]
    pub actions: Vec<ReportAction>,
    #[serde(default)]
    pub total_actions: usize,
    #[serde(default)]
    pub conflicts: Vec<ReportConflict>,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub failed: u64,
}

impl RunReport {
    /// One-line summary for the profiles list.
    pub fn summary(&self) -> String {
        let n = |op: ReportOp| self.actions.iter().filter(|a| a.op == op).count();
        let copies = n(ReportOp::CopyToRight) + n(ReportOp::CopyToLeft) + n(ReportOp::KeepCopy);
        let deletes = n(ReportOp::DeleteLeft) + n(ReportOp::DeleteRight);
        let mut parts = Vec::new();
        match self.status {
            RunStatus::InSync => parts.push(tr!("tools-report-nothing").to_string()),
            RunStatus::Failed | RunStatus::Cancelled if !self.message.is_empty() => parts.push(self.message.clone()),
            _ => {
                if copies > 0 {
                    parts.push(if self.dry_run {
                        tr!("tools-report-would-copy", count = copies)
                    } else {
                        tr!("tools-report-copy", count = copies)
                    });
                }
                if deletes > 0 {
                    parts.push(if self.dry_run {
                        tr!("tools-report-would-delete", count = deletes)
                    } else {
                        tr!("tools-report-delete", count = deletes)
                    });
                }
                if self.total_actions > self.actions.len() {
                    parts.push(tr!("tools-report-actions-total", count = self.total_actions));
                }
            }
        }
        if self.failed > 0 {
            parts.push(tr!("tools-report-failed", count = self.failed));
        }
        if !self.conflicts.is_empty() {
            parts.push(tr!("tools-report-conflicts", count = self.conflicts.len()));
        }
        parts.join(" · ")
    }
}

/// Report lines for a plan.
pub fn report_actions(plan: &SyncPlan) -> Vec<ReportAction> {
    let s = |p: &Path| p.to_string_lossy().replace('\\', "/");
    plan.actions
        .iter()
        .take(REPORT_MAX_ACTIONS)
        .map(|a| match a {
            SyncAction::CopyLR(p) => ReportAction { op: ReportOp::CopyToRight, path: s(p) },
            SyncAction::CopyRL(p) => ReportAction { op: ReportOp::CopyToLeft, path: s(p) },
            SyncAction::DeleteL(p) => ReportAction { op: ReportOp::DeleteLeft, path: s(p) },
            SyncAction::DeleteR(p) => ReportAction { op: ReportOp::DeleteRight, path: s(p) },
            SyncAction::CopyAs { to, .. } => ReportAction { op: ReportOp::KeepCopy, path: s(to) },
        })
        .collect()
}

fn sanitize(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(40).collect();
    if s.is_empty() { "profile".into() } else { s }
}

/// Folder of a profile's reports inside `base` (normally the config dir).
pub fn reports_dir_in(base: &Path, profile: &str) -> PathBuf {
    let mut h: u32 = 0x811c_9dc5;
    for b in profile.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    base.join("sync_reports").join(format!("{}-{h:08x}", sanitize(profile)))
}

/// Folder of a profile's reports for this thread's store (see [`set_store_dir`]).
pub fn reports_dir(profile: &str) -> PathBuf {
    reports_dir_in(&store_dir(), profile)
}

/// Save a report and keep only the newest `keep` (worker thread).
pub fn save_report_in(dir: &Path, r: &RunReport, keep: usize) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stamp = DateTime::<Local>::from(r.started).format("%Y%m%d-%H%M%S%.3f").to_string().replace('.', "-");
    let path = dir.join(format!("{stamp}.json"));
    let bytes = serde_json::to_vec_pretty(r).map_err(|e| e.to_string())?;
    soyuz_fs::write_atomic(&path, &bytes).map_err(|e| e.to_string())?;
    let mut files = report_files(dir);
    while files.len() > keep.max(1) {
        let old = files.remove(0);
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

/// Report files, oldest first (names sort by time).
fn report_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Reports in `dir`, newest first (worker thread).
pub fn load_reports_in(dir: &Path, max: usize) -> Vec<RunReport> {
    report_files(dir)
        .iter()
        .rev()
        .take(max)
        .filter_map(|p| std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()))
        .collect()
}

/// Load the newest `max` reports of a profile on a worker; the receiver gets them once
/// (the frontend's wake-up redraws).
pub fn load_reports_async(profile: &str, max: usize) -> Receiver<Vec<RunReport>> {
    let (tx, rx) = bounded(1);
    let dir = reports_dir(profile);
    let wake = super::tools::wake();
    let _ = std::thread::Builder::new().name("soyuz-sync-reports".into()).spawn(move || {
        let _ = tx.send(load_reports_in(&dir, max));
        wake();
    });
    rx
}

// ---------------------------------------------------------------------------------------
// Runtime (UI thread).

/// Planned sync of one run (worker → UI).
#[derive(Debug)]
struct Planned {
    plan: SyncPlan,
    report: RunReport,
    /// Two-way: paths that keep their base after the job.
    keep: Option<HashSet<PathBuf>>,
}

enum Phase {
    Planning(Receiver<Result<Planned, String>>, Arc<OpControl>),
    Syncing { job: JobId, planned: Box<Planned> },
    Recording(Receiver<Result<(), String>>, Box<RunReport>),
}

struct Run {
    phase: Phase,
    trigger: Trigger,
    dry: bool,
}

struct WatchSet {
    _watcher: soyuz_fs::Watcher,
    rx: Receiver<soyuz_fs::WatchEvent>,
    /// Watch id → profile name.
    ids: HashMap<soyuz_fs::WatchId, String>,
    /// Watched (profile, roots) to know when to rebuild.
    key: Vec<(String, PathBuf, PathBuf)>,
}

/// Profiles + their newest report, loaded in the background at start.
type Loaded = (Vec<SyncProfile>, HashMap<String, RunReport>);

#[derive(Default)]
pub struct Scheduler {
    /// Where profiles, reports and two-way states live (`None` = the config dir).
    dir: Option<PathBuf>,
    started: Option<SystemTime>,
    profiles: Option<Vec<SyncProfile>>,
    loading: Option<Receiver<Loaded>>,
    last: HashMap<String, RunReport>,
    runs: HashMap<String, Run>,
    queued: Vec<(String, Trigger, bool)>,
    watch: Option<WatchSet>,
    change_due: HashMap<String, Instant>,
    /// Changes before this instant are our own sync's echo.
    quiet_until: HashMap<String, Instant>,
    watch_failed: bool,
}

impl Scheduler {
    fn dir(&self) -> PathBuf {
        self.dir.clone().unwrap_or_else(soyuz_core::config::config_dir)
    }
}

thread_local! {
    static SCHED: RefCell<Scheduler> = RefCell::new(Scheduler::default());
}

/// Run `f` with the scheduler (UI thread).
pub fn with<R>(f: impl FnOnce(&mut Scheduler) -> R) -> R {
    SCHED.with_borrow_mut(f)
}

/// Keep this thread's profiles, reports and two-way states inside `dir` instead of the
/// config dir (tests, sandboxes). Forgets the loaded profiles so they are read from there.
pub fn set_store_dir(dir: PathBuf) {
    with(|s| {
        *s = Scheduler { dir: Some(dir), ..Scheduler::default() };
    });
}

/// Where this thread's profiles, reports and two-way states live.
pub fn store_dir() -> PathBuf {
    with(|s| s.dir())
}

/// Saved profiles, sorted by name (loads synchronously if the background load hasn't
/// landed yet — a small file, read once).
pub fn profiles() -> Vec<SyncProfile> {
    with(|s| {
        if s.profiles.is_none() {
            s.profiles = Some(load_profiles_in(&s.dir()));
        }
        s.profiles.clone().unwrap_or_default()
    })
}

/// One saved profile.
pub fn profile(name: &str) -> Option<SyncProfile> {
    profiles().into_iter().find(|p| p.name == name)
}

/// Replace and save the profiles (compare window, profiles window).
pub fn set_profiles(mut v: Vec<SyncProfile>) {
    v.sort_by_key(|a| a.name.to_lowercase());
    with(|s| {
        save_profiles_in(&s.dir(), &v);
        let names: HashSet<&str> = v.iter().map(|p| p.name.as_str()).collect();
        s.last.retain(|k, _| names.contains(k.as_str()));
        s.profiles = Some(v);
    });
}

/// Is the profile planning, syncing or recording right now?
pub fn is_running(name: &str) -> bool {
    with(|s| s.runs.contains_key(name))
}

/// Add a new profile (validated, see [`validate`]) and save.
pub fn create_profile(p: SyncProfile) -> Result<(), ProfileError> {
    let mut all = profiles();
    let p = validate(&all, p, None)?;
    all.push(p);
    set_profiles(all);
    Ok(())
}

/// Replace the profile `name` with `p` (folders, method, mode, schedule; a different
/// `p.name` renames it, see [`rename_profile`]) and save.
pub fn update_profile(name: &str, p: SyncProfile) -> Result<(), ProfileError> {
    let mut all = profiles();
    let Some(i) = all.iter().position(|o| o.name == name) else { return Err(ProfileError::NotFound(name.into())) };
    let p = validate(&all, p, Some(name))?;
    let renamed = p.name != name;
    if renamed && is_running(name) {
        return Err(ProfileError::Running(name.into()));
    }
    let new_name = p.name.clone();
    all[i] = p;
    if renamed {
        move_history(name, &new_name);
    }
    set_profiles(all);
    Ok(())
}

/// Rename a profile (its last report and saved reports move along) and save.
pub fn rename_profile(old: &str, new: &str) -> Result<(), ProfileError> {
    let Some(mut p) = profile(old) else { return Err(ProfileError::NotFound(old.into())) };
    p.name = new.to_string();
    update_profile(old, p)
}

/// Delete a profile and save (its reports stay on disk). Refused while it runs.
pub fn delete_profile(name: &str) -> Result<(), ProfileError> {
    if is_running(name) {
        return Err(ProfileError::Running(name.into()));
    }
    let mut all = profiles();
    let before = all.len();
    all.retain(|p| p.name != name);
    if all.len() == before {
        return Err(ProfileError::NotFound(name.into()));
    }
    set_profiles(all);
    with(|s| {
        s.change_due.remove(name);
        s.quiet_until.remove(name);
        s.queued.retain(|(n, ..)| n != name);
    });
    Ok(())
}

/// Change a profile's schedule (clamped to the editor's ranges) and save.
pub fn set_schedule(name: &str, schedule: Schedule) -> Result<(), ProfileError> {
    let Some(mut p) = profile(name) else { return Err(ProfileError::NotFound(name.into())) };
    p.schedule = schedule;
    update_profile(name, p)
}

/// A renamed profile keeps its history: the in-memory last report now, the report folder
/// on a worker.
fn move_history(old: &str, new: &str) {
    let dir = with(|s| {
        if let Some(mut r) = s.last.remove(old) {
            r.profile = new.to_string();
            s.last.insert(new.to_string(), r);
        }
        s.change_due.remove(old);
        s.quiet_until.remove(old);
        for q in &mut s.queued {
            if q.0 == old {
                q.0 = new.to_string();
            }
        }
        s.dir()
    });
    let (from, to) = (reports_dir_in(&dir, old), reports_dir_in(&dir, new));
    let _ = std::thread::Builder::new().name("soyuz-sync-rename".into()).spawn(move || {
        if from.is_dir() && !to.exists() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::rename(&from, &to) {
                log::warn!("cannot move sync reports {} → {}: {e}", from.display(), to.display());
            }
        }
    });
}

/// Start a run of `name` next frame (`dry`: only write the report).
pub fn run_now(name: &str, dry: bool) {
    with(|s| s.queued.push((name.to_string(), Trigger::Manual, dry)));
    super::tools::wake()();
}

/// Cancel the planning or the sync job of a running profile.
pub fn cancel(core: &AppCore, name: &str) {
    with(|s| {
        if let Some(run) = s.runs.get(name) {
            match &run.phase {
                Phase::Planning(_, ctl) => ctl.cancel(),
                Phase::Syncing { job, .. } => core.fileops.jobs.engine().cancel(*job),
                Phase::Recording(..) => {}
            }
        }
    });
}

/// What a profile is doing right now, for the list.
pub fn activity(name: &str) -> Option<&'static str> {
    with(|s| {
        s.runs.get(name).map(|r| match r.phase {
            Phase::Planning(..) => tr!("tools-activity-planning"),
            Phase::Syncing { .. } => tr!("tools-activity-syncing"),
            Phase::Recording(..) => tr!("tools-activity-recording"),
        })
    })
}

pub fn last_report(name: &str) -> Option<RunReport> {
    with(|s| s.last.get(name).cloned())
}

/// Next time-driven run of a profile (see [`next_run`]).
pub fn next_of(p: &SyncProfile) -> Option<SystemTime> {
    with(|s| {
        let started = s.started.unwrap_or_else(SystemTime::now);
        let last = s.last.get(&p.name).map(|r| r.started);
        next_run(&p.schedule, last, started, SystemTime::now(), &Local)
    })
}

/// Once per frame (UI thread, no blocking I/O): loads the profiles, keeps the watchers,
/// starts due runs, submits planned syncs as jobs and records their reports. Every
/// frontend that wants scheduled profiles calls it.
pub fn tick(core: &mut AppCore, host: &dyn Host) {
    let cfg = core.config.sync.clone();
    let mut toasts: Vec<Toast> = Vec::new();
    let mut submit: Vec<(String, Trigger, Box<Planned>)> = Vec::new();
    let mut wake_in: Option<Duration> = None;
    with(|s| {
        let now_wall = SystemTime::now();
        let started = *s.started.get_or_insert(now_wall);
        let dir = s.dir();
        // Background load of profiles + their newest reports.
        if s.profiles.is_none() && s.loading.is_none() {
            let (tx, rx) = bounded(1);
            let wake = super::tools::wake();
            let d = dir.clone();
            let spawned = std::thread::Builder::new().name("soyuz-sync-load".into()).spawn(move || {
                let profiles = load_profiles_in(&d);
                let last = profiles
                    .iter()
                    .filter_map(|p| {
                        load_reports_in(&reports_dir_in(&d, &p.name), 1).into_iter().next().map(|r| (p.name.clone(), r))
                    })
                    .collect();
                let _ = tx.send((profiles, last));
                wake();
            });
            if spawned.is_ok() {
                s.loading = Some(rx);
            } else {
                s.profiles = Some(Vec::new());
            }
        }
        if let Some(rx) = &s.loading
            && let Ok((profiles, last)) = rx.try_recv()
        {
            if s.profiles.is_none() {
                s.profiles = Some(profiles);
            }
            for (k, v) in last {
                s.last.entry(k).or_insert(v);
            }
            s.loading = None;
        }
        if s.loading.is_some() {
            return;
        }
        let profiles = s.profiles.clone().unwrap_or_default();
        // Watchers for "on change" profiles.
        sync_watchers(s, &profiles, cfg.scheduler, &mut toasts);
        let debounce = Duration::from_secs(cfg.change_debounce_secs.max(1));
        if let Some(w) = &s.watch {
            let now = Instant::now();
            for ev in w.rx.try_iter() {
                if let Some(name) = w.ids.get(&ev.id) {
                    let quiet = s.quiet_until.get(name).is_some_and(|q| now < *q);
                    if !quiet && !s.runs.contains_key(name) {
                        s.change_due.insert(name.clone(), now + debounce);
                    }
                }
            }
        }
        // Due runs.
        let mut start: Vec<(String, Trigger, bool)> = std::mem::take(&mut s.queued);
        if cfg.scheduler {
            for p in &profiles {
                if s.runs.contains_key(&p.name) || start.iter().any(|(n, ..)| *n == p.name) {
                    continue;
                }
                let last = s.last.get(&p.name).map(|r| r.started);
                if let Some(at) = next_run(&p.schedule, last, started, now_wall, &Local) {
                    match at.duration_since(now_wall) {
                        Err(_) | Ok(Duration::ZERO) => {
                            let trig = if p.schedule == Schedule::OnStart { Trigger::Start } else { Trigger::Schedule };
                            start.push((p.name.clone(), trig, cfg.scheduled_dry_run));
                        }
                        Ok(d) => wake_in = Some(wake_in.map_or(d, |w| w.min(d))),
                    }
                }
                if let Some(due) = s.change_due.get(&p.name).copied() {
                    let now = Instant::now();
                    if now >= due {
                        s.change_due.remove(&p.name);
                        start.push((p.name.clone(), Trigger::Change, cfg.scheduled_dry_run));
                    } else {
                        let d = due - now;
                        wake_in = Some(wake_in.map_or(d, |w| w.min(d)));
                    }
                }
            }
        }
        for (name, trigger, dry) in start {
            if s.runs.contains_key(&name) {
                continue;
            }
            let Some(p) = profiles.iter().find(|p| p.name == name).cloned() else { continue };
            let ctl = OpControl::new();
            let (tx, rx) = bounded(1);
            let c2 = ctl.clone();
            let wake = super::tools::wake();
            let d = dir.clone();
            let spawned = std::thread::Builder::new().name("soyuz-sync-plan".into()).spawn(move || {
                let out = plan_profile_in(&d, &p, trigger, dry, &c2);
                let _ = tx.send(out);
                wake();
            });
            if spawned.is_ok() {
                s.runs.insert(name, Run { phase: Phase::Planning(rx, ctl), trigger, dry });
            }
        }
        // Advance running profiles.
        let names: Vec<String> = s.runs.keys().cloned().collect();
        for name in names {
            let Some(run) = s.runs.remove(&name) else { continue };
            let (trigger, dry) = (run.trigger, run.dry);
            let back = |phase| Run { phase, trigger, dry };
            match run.phase {
                Phase::Planning(rx, ctl) => match rx.try_recv() {
                    Ok(Ok(planned)) => {
                        let nothing = planned.plan.actions.is_empty();
                        if planned.report.dry_run || nothing {
                            let mut rep = planned.report;
                            rep.status = match (nothing, rep.dry_run, rep.conflicts.is_empty()) {
                                (true, _, true) => RunStatus::InSync,
                                (_, true, _) => RunStatus::DryRun,
                                _ => RunStatus::Partial,
                            };
                            finish(s, rep, cfg.reports_kept, &mut toasts);
                        } else {
                            submit.push((name, trigger, Box::new(planned)));
                        }
                    }
                    Ok(Err(e)) => {
                        let p = profiles.iter().find(|p| p.name == name);
                        let mut rep = blank_report(&name, p, trigger, dry);
                        rep.status = if ctl.is_cancelled() { RunStatus::Cancelled } else { RunStatus::Failed };
                        rep.message = e;
                        finish(s, rep, cfg.reports_kept, &mut toasts);
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        s.runs.insert(name, back(Phase::Planning(rx, ctl)));
                    }
                    Err(_) => {}
                },
                Phase::Syncing { job, planned } => {
                    let snap = core.fileops.jobs.snapshot(job);
                    let state = snap.map(|x| x.state.clone());
                    match state {
                        Some(st) if !st.is_terminal() => {
                            s.runs.insert(name, back(Phase::Syncing { job, planned }));
                        }
                        st => {
                            let Planned { mut report, keep, plan } = *planned;
                            report.finished = Some(SystemTime::now());
                            let mut failed_paths = Vec::new();
                            match st {
                                Some(JobState::Done(sum)) => {
                                    report.failed = sum.items_failed;
                                    report.bytes = sum.bytes;
                                    report.status = if sum.items_failed > 0 || !report.conflicts.is_empty() {
                                        RunStatus::Partial
                                    } else {
                                        RunStatus::Applied
                                    };
                                    failed_paths = sum.errors.iter().map(|(p, _)| p.clone()).collect();
                                }
                                Some(JobState::Failed(e)) => {
                                    report.status = RunStatus::Failed;
                                    report.message = e.to_string();
                                }
                                _ => report.status = RunStatus::Cancelled,
                            }
                            let applied = matches!(report.status, RunStatus::Applied | RunStatus::Partial);
                            match keep {
                                Some(mut keep) if applied => {
                                    for f in failed_paths {
                                        if let Ok(rel) =
                                            f.strip_prefix(&plan.left).or_else(|_| f.strip_prefix(&plan.right))
                                        {
                                            keep.insert(rel.to_path_buf());
                                        }
                                    }
                                    let (tx, rx) = bounded(1);
                                    let (l, r) = (plan.left.clone(), plan.right.clone());
                                    let wake = super::tools::wake();
                                    let d = dir.clone();
                                    let spawned =
                                        std::thread::Builder::new().name("soyuz-sync-state".into()).spawn(move || {
                                            let res = sync_state::record_after_sync_in(&d, &l, &r, &keep);
                                            let _ = tx.send(res.map(|_| ()));
                                            wake();
                                        });
                                    if spawned.is_ok() {
                                        s.runs.insert(name, back(Phase::Recording(rx, Box::new(report))));
                                    } else {
                                        finish(s, report, cfg.reports_kept, &mut toasts);
                                    }
                                }
                                _ => finish(s, report, cfg.reports_kept, &mut toasts),
                            }
                        }
                    }
                }
                Phase::Recording(rx, report) => match rx.try_recv() {
                    Ok(res) => {
                        let mut report = *report;
                        if let Err(e) = res {
                            report.message = tr!("tools-report-state-not-saved", error = e);
                            report.status = RunStatus::Partial;
                        }
                        finish(s, report, cfg.reports_kept, &mut toasts);
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        s.runs.insert(name, back(Phase::Recording(rx, report)));
                    }
                    Err(_) => {
                        let report = *report;
                        finish(s, report, cfg.reports_kept, &mut toasts);
                    }
                },
            }
        }
        if !s.runs.is_empty() {
            wake_in = Some(wake_in.map_or(Duration::from_millis(250), |w| w.min(Duration::from_millis(250))));
        }
        let quiet_after = debounce;
        s.quiet_until.retain(|_, q| Instant::now() < *q + quiet_after);
    });
    // Submit jobs (needs the core; outside the scheduler borrow).
    for (name, trigger, planned) in submit {
        let label = tr!("tools-sched-job-label", name = &name);
        let mut spec = core.fileops.jobs.spec(JobKind::Sync { plan: planned.plan.clone() });
        spec.options.label = Some(label);
        let job = crate::models::archive::submit_job(core, spec);
        with(|s| {
            s.runs.insert(name, Run { phase: Phase::Syncing { job, planned }, trigger, dry: false });
        });
    }
    for t in toasts {
        core.ui.toasts.push(t);
    }
    if let Some(d) = wake_in {
        host.redraw_after(d.min(Duration::from_secs(60)));
    }
}

fn blank_report(name: &str, p: Option<&SyncProfile>, trigger: Trigger, dry_run: bool) -> RunReport {
    RunReport {
        profile: name.to_string(),
        left: p.map(|p| p.left.clone()).unwrap_or_default(),
        right: p.map(|p| p.right.clone()).unwrap_or_default(),
        started: SystemTime::now(),
        finished: None,
        trigger,
        dry_run,
        status: RunStatus::Failed,
        message: String::new(),
        actions: Vec::new(),
        total_actions: 0,
        conflicts: Vec::new(),
        bytes: 0,
        failed: 0,
    }
}

/// Record the finished report (in memory now, on disk via a worker) and notify.
fn finish(s: &mut Scheduler, mut rep: RunReport, keep: u32, toasts: &mut Vec<Toast>) {
    rep.finished.get_or_insert_with(SystemTime::now);
    let name = rep.profile.clone();
    s.quiet_until.insert(name.clone(), Instant::now() + Duration::from_secs(3));
    s.change_due.remove(&name);
    let unattended = rep.trigger != Trigger::Manual;
    let conflicts = rep.conflicts.len();
    let toast = match rep.status {
        RunStatus::Failed => {
            Some(Toast::error(tr!("tools-sched-toast-failed", name = &name)).body(rep.message.clone()))
        }
        _ if conflicts > 0 => Some(
            Toast::warning(tr!("tools-sched-toast-conflicts", name = &name, count = conflicts))
                .body(tr!("tools-sched-toast-conflicts-body"))
                .action(tr!("tools-sched-open-profiles"), soyuz_core::config::Action::SyncProfiles),
        ),
        RunStatus::DryRun if !unattended => {
            Some(Toast::info(tr!("tools-sched-toast-dry-run", name = &name)).body(rep.summary()))
        }
        RunStatus::InSync if !unattended => Some(Toast::success(tr!("tools-sched-toast-in-sync", name = &name))),
        _ => None,
    };
    if let Some(t) = toast {
        toasts.push(t);
    }
    let dir = reports_dir_in(&s.dir(), &name);
    let saved = rep.clone();
    let _ = std::thread::Builder::new().name("soyuz-sync-report".into()).spawn(move || {
        if let Err(e) = save_report_in(&dir, &saved, keep as usize) {
            log::warn!("cannot save sync report for {}: {e}", saved.profile);
        }
    });
    s.last.insert(name, rep);
}

/// Plan one run (worker thread); two-way states live inside `dir`.
fn plan_profile_in(
    dir: &Path,
    p: &SyncProfile,
    trigger: Trigger,
    dry: bool,
    ctl: &OpControl,
) -> Result<Planned, String> {
    let mut report = blank_report(&p.name, Some(p), trigger, dry);
    for (side, dir) in [("left", &p.left), ("right", &p.right)] {
        if !soyuz_fs::exists_nofollow(dir) {
            return Err(tr!("tools-sched-err-unavailable", side = side, path = dir.display().to_string()));
        }
    }
    let computed = compare::compute_in(dir, &p.left, &p.right, p.method, p.preset, ctl)?;
    // Unattended: conflicts are never guessed (skipped, kept at their base).
    let rows = computed.rows;
    report.conflicts = rows
        .iter()
        .filter_map(|r| {
            r.conflict.map(|k| ReportConflict { path: r.rel.to_string_lossy().replace('\\', "/"), kind: k })
        })
        .collect();
    let tag = computed.two_way.as_ref().map_or("right".to_string(), |t| t.tag.clone());
    let plan = compare::build_plan(&p.left, &p.right, &rows, &tag);
    report.actions = report_actions(&plan);
    report.total_actions = plan.actions.len();
    report.bytes = compare::plan_stats(&rows).bytes;
    let keep = computed.two_way.as_ref().map(|ctx| compare::keep_after_sync(&rows, ctx, &p.left, &p.right, &[]));
    Ok(Planned { plan, report, keep })
}

/// Keep one recursive watcher per side of every "on change" profile.
fn sync_watchers(s: &mut Scheduler, profiles: &[SyncProfile], enabled: bool, toasts: &mut Vec<Toast>) {
    let key: Vec<(String, PathBuf, PathBuf)> = if enabled {
        profiles
            .iter()
            .filter(|p| p.schedule == Schedule::OnChange)
            .map(|p| (p.name.clone(), p.left.clone(), p.right.clone()))
            .collect()
    } else {
        Vec::new()
    };
    if s.watch.as_ref().map_or(key.is_empty(), |w| w.key == key) {
        return;
    }
    s.watch = None;
    if key.is_empty() {
        return;
    }
    let (tx, rx) = unbounded();
    let wake = super::tools::wake();
    let mut watcher = match soyuz_fs::Watcher::new(tx, Duration::from_millis(500), wake) {
        Ok(w) => w,
        Err(e) => {
            if !s.watch_failed {
                toasts.push(Toast::warning(tr!("tools-sched-watch-failed")).body(e.to_string()));
                s.watch_failed = true;
            }
            return;
        }
    };
    let mut ids = HashMap::new();
    for (name, l, r) in &key {
        for dir in [l, r] {
            match watcher.watch(dir, true) {
                Ok(id) => {
                    ids.insert(id, name.clone());
                }
                Err(e) => log::warn!("sync profile {name}: cannot watch {}: {e}", dir.display()),
            }
        }
    }
    s.watch = Some(WatchSet { _watcher: watcher, rx, ids, key });
}

/// Relative "next run" text for the list.
pub fn next_label(p: &SyncProfile, enabled: bool) -> String {
    if !enabled && p.schedule != Schedule::Manual {
        return tr!("tools-next-scheduler-off").into();
    }
    match p.schedule {
        Schedule::Manual => tr!("tools-next-manual").into(),
        Schedule::OnChange => tr!("tools-next-on-change").into(),
        _ => match next_of(p) {
            None => tr!("tools-next-ran").into(),
            Some(t) => match t.duration_since(SystemTime::now()) {
                Err(_) => tr!("tools-next-now").into(),
                Ok(d) if d.as_secs() < 90 => tr!("tools-next-minute").into(),
                Ok(d) if d.as_secs() < 3600 => tr!("tools-next-in-minutes", count = d.as_secs().div_ceil(60)),
                Ok(_) => {
                    let lt = DateTime::<Local>::from(t);
                    let today = Local::now().date_naive();
                    let time = lt.format("%H:%M").to_string();
                    if lt.date_naive() == today {
                        tr!("tools-next-today", time = time)
                    } else if Some(lt.date_naive()) == today.succ_opt() {
                        tr!("tools-next-tomorrow", time = time)
                    } else {
                        let months = soyuz_core::locale::month_abbr(soyuz_core::locale::current());
                        let month = months[lt.month0() as usize % 12];
                        tr!("tools-next-date", month = month, day = lt.day().to_string())
                    }
                }
            },
        },
    }
}

/// `C:\…\parent\name` for long paths (the full path goes in a tooltip / detail line).
pub fn short_path(p: &Path) -> String {
    let full = p.display().to_string();
    let comps: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let n = comps.len();
    if full.chars().count() <= 48 || n <= 3 {
        return full;
    }
    let sep = std::path::MAIN_SEPARATOR;
    let root = comps[0].trim_end_matches(['\\', '/']);
    format!("{root}{sep}…{sep}{}{sep}{}", comps[n - 2], comps[n - 1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn tz() -> FixedOffset {
        FixedOffset::east_opt(3 * 3600).unwrap()
    }

    /// `2026-10-08 hh:mm` at +03:00.
    fn local(day: u32, h: u32, m: u32) -> SystemTime {
        SystemTime::from(tz().with_ymd_and_hms(2026, 10, day, h, m, 0).unwrap())
    }

    #[test]
    fn next_run_for_each_schedule() {
        let start = local(8, 9, 0);
        let now = local(8, 10, 0);
        // Manual / on change: not time driven.
        assert_eq!(next_run(&Schedule::Manual, None, start, now, &tz()), None);
        assert_eq!(next_run(&Schedule::OnChange, None, start, now, &tz()), None);
        // On start: once per session.
        assert_eq!(next_run(&Schedule::OnStart, None, start, now, &tz()), Some(start + START_DELAY));
        assert_eq!(next_run(&Schedule::OnStart, Some(local(7, 23, 0)), start, now, &tz()), Some(start + START_DELAY));
        assert_eq!(next_run(&Schedule::OnStart, Some(local(8, 9, 1)), start, now, &tz()), None);
        // Interval: from the last run; never run → soon after start.
        let every = Schedule::Interval { minutes: 30 };
        assert_eq!(next_run(&every, Some(local(8, 9, 45)), start, now, &tz()), Some(local(8, 10, 15)));
        assert_eq!(next_run(&every, None, start, now, &tz()), Some(start + START_DELAY));
        assert!(next_run(&every, Some(local(8, 9, 0)), start, now, &tz()).unwrap() <= now, "overdue = due now");
        assert_eq!(
            next_run(&Schedule::Interval { minutes: 0 }, Some(now), start, now, &tz()),
            Some(now + Duration::from_secs(60)),
            "at least a minute"
        );
    }

    #[test]
    fn daily_schedule_next_and_catch_up() {
        let daily = Schedule::Daily { hour: 18, minute: 30 };
        let start = local(8, 9, 0);
        // Later today.
        assert_eq!(next_run(&daily, Some(local(7, 18, 31)), start, local(8, 10, 0), &tz()), Some(local(8, 18, 30)));
        // Already ran today → tomorrow.
        assert_eq!(next_run(&daily, Some(local(8, 18, 30)), start, local(8, 20, 0), &tz()), Some(local(9, 18, 30)));
        // Missed yesterday's run while closed → due now (yesterday's slot).
        let t = next_run(&daily, Some(local(6, 18, 30)), start, local(8, 10, 0), &tz()).unwrap();
        assert_eq!(t, local(7, 18, 30));
        assert!(t <= local(8, 10, 0));
        // Never ran: wait for the next slot (no surprise run when the profile is created).
        assert_eq!(next_run(&daily, None, start, local(8, 19, 0), &tz()), Some(local(9, 18, 30)));
        // Labels.
        assert_eq!(daily.label(), "Daily at 18:30");
        assert_eq!(Schedule::Interval { minutes: 120 }.label(), "Every 2 hours");
        assert_eq!(Schedule::Interval { minutes: 15 }.label(), "Every 15 min");
    }

    #[test]
    fn schedule_kinds_defaults_and_clamping() {
        for k in ScheduleKind::ALL {
            assert_eq!(k.default_schedule().kind(), k);
            assert!(!k.label().is_empty());
        }
        assert_eq!(ScheduleKind::Interval.default_schedule(), Schedule::Interval { minutes: 60 });
        assert_eq!(ScheduleKind::Daily.default_schedule(), Schedule::Daily { hour: 18, minute: 0 });
        assert!(ScheduleKind::Daily.note().is_some() && ScheduleKind::OnChange.note().is_some());
        assert!(ScheduleKind::Manual.note().is_none());
        assert_eq!(Schedule::Interval { minutes: 0 }.clamped(), Schedule::Interval { minutes: 1 });
        assert_eq!(Schedule::Interval { minutes: 99_999 }.clamped(), Schedule::Interval { minutes: 1440 });
        assert_eq!(Schedule::Daily { hour: 30, minute: 75 }.clamped(), Schedule::Daily { hour: 23, minute: 59 });
        assert_eq!(Schedule::OnStart.clamped(), Schedule::OnStart);
    }

    #[test]
    fn schedule_serializes_tagged() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            s: Schedule,
        }
        for s in [
            Schedule::Manual,
            Schedule::Interval { minutes: 15 },
            Schedule::Daily { hour: 7, minute: 5 },
            Schedule::OnChange,
            Schedule::OnStart,
        ] {
            let text = toml::to_string(&W { s: s.clone() }).unwrap();
            assert_eq!(toml::from_str::<W>(&text).unwrap().s, s, "{text}");
        }
        let text = toml::to_string(&W { s: Schedule::Daily { hour: 7, minute: 5 } }).unwrap();
        assert!(text.contains("kind = \"daily\""), "{text}");
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
                schedule: Schedule::Daily { hour: 7, minute: 30 },
            }],
        };
        let text = toml::to_string_pretty(&f).unwrap();
        let back: ProfileFile = toml::from_str(&text).unwrap();
        assert_eq!(back.profiles, f.profiles);
        assert!(text.contains("mirror_right"));
        // Profiles saved before schedules existed still load (manual).
        let old: ProfileFile = toml::from_str("[[profiles]]\nname = \"x\"\nleft = \"/a\"\nright = \"/b\"\n").unwrap();
        assert_eq!(old.profiles[0].schedule, Schedule::Manual);
    }

    #[test]
    fn validation_rules() {
        let a = SyncProfile::new("A", "/a".into(), "/b".into(), Preset::Update);
        let all = vec![a.clone()];
        let p = |name: &str, l: &str, r: &str| SyncProfile::new(name, l.into(), r.into(), Preset::Update);
        assert_eq!(validate(&all, p("  ", "/x", "/y"), None), Err(ProfileError::EmptyName));
        assert_eq!(validate(&all, p("A", "/x", "/y"), None), Err(ProfileError::Duplicate("A".into())));
        assert!(validate(&all, p("A", "/x", "/y"), Some("A")).is_ok(), "editing itself");
        assert_eq!(validate(&all, p("B", "", "/y"), None), Err(ProfileError::MissingPath));
        assert_eq!(validate(&all, p("B", "/y", "/y"), None), Err(ProfileError::SamePaths));
        let ok =
            validate(&all, SyncProfile { schedule: Schedule::Interval { minutes: 0 }, ..p(" B ", "/x", "/y") }, None)
                .unwrap();
        assert_eq!((ok.name.as_str(), ok.schedule), ("B", Schedule::Interval { minutes: 1 }));
    }

    #[test]
    fn profile_crud_round_trip_on_a_temp_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        set_store_dir(dir.path().to_path_buf());
        assert_eq!(store_dir(), dir.path());
        assert!(profiles().is_empty());
        let p = |name: &str| SyncProfile::new(name, dir.path().join("L"), dir.path().join("R"), Preset::MirrorRight);
        create_profile(p("Photos")).unwrap();
        create_profile(p("archive")).unwrap();
        assert_eq!(create_profile(p("Photos")), Err(ProfileError::Duplicate("Photos".into())));
        assert_eq!(profiles().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["archive", "Photos"], "sorted");
        // Edit: schedule, method; rename moves the last report along.
        set_schedule("Photos", Schedule::Daily { hour: 7, minute: 30 }).unwrap();
        let mut edited = profile("Photos").unwrap();
        edited.method = Method::Content;
        update_profile("Photos", edited).unwrap();
        with(|s| {
            s.last.insert("Photos".into(), blank_report("Photos", None, Trigger::Manual, true));
        });
        rename_profile("Photos", "Pictures").unwrap();
        assert!(profile("Photos").is_none());
        assert_eq!(last_report("Pictures").map(|r| r.profile), Some("Pictures".into()));
        assert_eq!(rename_profile("Pictures", "archive"), Err(ProfileError::Duplicate("archive".into())));
        assert_eq!(rename_profile("nope", "x"), Err(ProfileError::NotFound("nope".into())));
        delete_profile("archive").unwrap();
        assert_eq!(delete_profile("archive"), Err(ProfileError::NotFound("archive".into())));
        // On disk (after the store worker wrote it).
        assert!(crate::services::store_file::flush(Duration::from_secs(10)));
        let back = load_profiles_in(dir.path());
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "Pictures");
        assert_eq!(back[0].method, Method::Content);
        assert_eq!(back[0].preset, Preset::MirrorRight);
        assert_eq!(back[0].schedule, Schedule::Daily { hour: 7, minute: 30 });
        assert!(profiles_path().starts_with(dir.path()));
        // A fresh scheduler on this thread reads them back.
        set_store_dir(dir.path().to_path_buf());
        assert_eq!(profiles(), back);
        // The next run of a daily profile is in the next day (or later today).
        let next = next_of(&back[0]).unwrap();
        assert!(next > SystemTime::now() && next <= SystemTime::now() + Duration::from_secs(25 * 3600));
        assert!(!next_label(&back[0], true).is_empty());
        assert_eq!(next_label(&back[0], false), tr!("tools-next-scheduler-off"));
    }

    #[test]
    fn reports_persist_prune_and_summarize() {
        let dir = tempfile::tempdir().unwrap();
        let rdir = reports_dir_in(dir.path(), "Photos → NAS");
        assert!(rdir.starts_with(dir.path().join("sync_reports")));
        assert_ne!(rdir, reports_dir_in(dir.path(), "Photos -> NAS"), "names hash apart");
        let plan = SyncPlan {
            left: "/L".into(),
            right: "/R".into(),
            actions: vec![
                SyncAction::CopyLR("a.txt".into()),
                SyncAction::CopyRL("b/c.txt".into()),
                SyncAction::DeleteR("old".into()),
            ],
        };
        let mut r = blank_report("Photos → NAS", None, Trigger::Schedule, true);
        r.actions = report_actions(&plan);
        r.total_actions = 3;
        r.status = RunStatus::DryRun;
        r.conflicts = vec![ReportConflict { path: "x".into(), kind: ConflictKind::BothChanged }];
        assert_eq!(r.summary(), "would copy 2 · would delete 1 · 1 conflict skipped");
        for i in 0..5u64 {
            let mut x = r.clone();
            x.started = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i * 60);
            save_report_in(&rdir, &x, 3).unwrap();
        }
        let back = load_reports_in(&rdir, 10);
        assert_eq!(back.len(), 3, "pruned to the newest 3");
        assert!(back[0].started > back[1].started, "newest first");
        assert_eq!(back[0].actions, r.actions);
        assert_eq!(back[0].conflicts, r.conflicts);
    }

    #[test]
    fn dry_run_plan_of_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (l, r) = (dir.path().join("L"), dir.path().join("R"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(l.join("only-left.txt"), b"1").unwrap();
        let p = SyncProfile {
            name: "t".into(),
            left: l.clone(),
            right: r.clone(),
            method: Method::DateSize,
            preset: Preset::Update,
            schedule: Schedule::Manual,
        };
        let cfg = dir.path().join("cfg");
        let planned = plan_profile_in(&cfg, &p, Trigger::Manual, true, &OpControl::new()).unwrap();
        assert_eq!(planned.plan.actions, vec![SyncAction::CopyLR("only-left.txt".into())]);
        assert_eq!(planned.report.actions[0].op, ReportOp::CopyToRight);
        assert!(planned.keep.is_none());
        let missing = SyncProfile { left: dir.path().join("nope"), ..p };
        assert!(
            plan_profile_in(&cfg, &missing, Trigger::Manual, true, &OpControl::new())
                .unwrap_err()
                .contains("left folder")
        );
    }

    #[test]
    fn long_paths_are_shortened() {
        assert_eq!(short_path(Path::new(r"C:\a\b")), r"C:\a\b");
        let long = Path::new(r"C:\Users\someone\AppData\Local\Temp\a-very-long-folder-name\project\L");
        if cfg!(windows) {
            assert_eq!(short_path(long), r"C:\…\project\L");
        }
    }
}
