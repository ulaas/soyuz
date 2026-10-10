//! The app-owned [`JobEngine`]: construction from `[jobs]`, the per-frame
//! drain of engine events, snapshot polling only while something is active (no idle
//! repaint spin), a per-job metadata side table used for titles / completion handling,
//! incremental log fetching and shutdown on exit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use soyuz_archive::ExtractLimits;
use soyuz_core::config::{Config, PreserveItem, ReflinkPref};
use soyuz_core::{HashAlgo, Location};
use soyuz_fs::{CopyOptions, Preserve, RealProbe, Reflink};
use soyuz_jobs::{
    EngineConfig, HistoryEntry, HistoryId, JobEngine, JobEvent, JobId, JobKind, JobKindBrief, JobOptions, JobSnapshot,
    JobSpec, LogLine, QueueMode, UndoRecord,
};

use crate::state::pane::PaneId;

/// What the user asked for (drives titles, icons and the completion toast).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Duplicate,
    Trash,
    Delete,
    Wipe,
    Rename,
    NewFolder,
    NewFile,
    Link,
    /// Attributes / timestamps / permissions (Properties, bulk attributes).
    SetMeta,
    Restore,
    EmptyTrash,
    Undo,
    Redo,
    Retry,
    Other,
}

impl OpKind {
    pub fn from_brief(b: JobKindBrief) -> OpKind {
        match b {
            JobKindBrief::Copy => OpKind::Copy,
            JobKindBrief::Move => OpKind::Move,
            JobKindBrief::Delete => OpKind::Delete,
            JobKindBrief::Trash => OpKind::Trash,
            JobKindBrief::Rename => OpKind::Rename,
            JobKindBrief::Create => OpKind::NewFolder,
            JobKindBrief::RestoreTrash => OpKind::Restore,
            JobKindBrief::EmptyTrash => OpKind::EmptyTrash,
            JobKindBrief::Undo => OpKind::Undo,
            JobKindBrief::SetMeta => OpKind::SetMeta,
            _ => OpKind::Other,
        }
    }

    /// Whether a finished job of this kind can be undone from the completion toast.
    pub fn undoable(self) -> bool {
        matches!(
            self,
            OpKind::Copy
                | OpKind::Move
                | OpKind::Duplicate
                | OpKind::Trash
                | OpKind::Rename
                | OpKind::NewFolder
                | OpKind::NewFile
                | OpKind::Link
                | OpKind::SetMeta
                | OpKind::Redo
        )
    }
}

/// Follow-up work once a job finishes successfully.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AfterJob {
    #[default]
    None,
    /// Select the created item in `pane` and start inline rename on it.
    SelectAndRename { pane: PaneId, path: PathBuf },
    /// Put the cursor on `name` in `pane` (rename keeps the cursor on the item).
    Select { pane: PaneId, name: String },
    /// Clear the internal clipboard (a cut was pasted).
    ClearClipboard,
}

/// Who started a job other than the user directly (a badge on its activity card).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobOrigin {
    /// A rule in `[[automation.rules]]`.
    Automation,
    /// A script action (`scripts/*.rhai`).
    Script,
    /// `soyuz cp|mv|rm|…` from a terminal.
    Cli,
}

/// App-side facts about a submitted job.
#[derive(Clone, Debug)]
pub struct JobMeta {
    pub op: OpKind,
    /// Number of top-level items the user acted on.
    pub items: usize,
    /// First item (for "“report.pdf”" titles when `items == 1`).
    pub first: Option<PathBuf>,
    pub dest: Option<PathBuf>,
    pub submitted: Instant,
    pub after: AfterJob,
    /// Show a completion toast (off for tiny interactive ops like rename / new folder).
    pub toast: bool,
    /// Started by the in-app assistant for this task (`ai::TaskId`); tags the card.
    pub ai_task: Option<u64>,
    /// Started by this MCP client (tags the card with its name).
    pub mcp_client: Option<String>,
    /// Started by an automation rule, a script or the CLI.
    pub origin: Option<JobOrigin>,
}

impl JobMeta {
    pub fn new(op: OpKind, items: &[PathBuf], dest: Option<PathBuf>) -> Self {
        JobMeta {
            op,
            items: items.len(),
            first: items.first().cloned(),
            dest,
            submitted: Instant::now(),
            after: AfterJob::None,
            toast: true,
            ai_task: None,
            mcp_client: None,
            origin: None,
        }
    }

    /// Mark the job as started by the assistant (`task` = `ai::TaskId`).
    pub fn ai(mut self, task: u64) -> Self {
        self.ai_task = Some(task);
        self
    }

    /// Mark the job as started by an external MCP client.
    pub fn mcp(mut self, client: &str) -> Self {
        self.mcp_client = Some(client.to_string());
        self
    }

    /// Mark the job as started by an automation rule, a script or the CLI.
    pub fn origin(mut self, origin: JobOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    pub fn after(mut self, after: AfterJob) -> Self {
        self.after = after;
        self
    }

    pub fn quiet(mut self) -> Self {
        self.toast = false;
        self
    }
}

pub struct JobsService {
    engine: Arc<JobEngine>,
    /// Latest snapshots, ordered by job id.
    snapshots: Vec<JobSnapshot>,
    meta: HashMap<JobId, JobMeta>,
    history: Vec<HistoryEntry>,
    history_dirty: bool,
    logs: HashMap<JobId, Vec<LogLine>>,
    /// Copy options derived from `[jobs]` (merged into every copy/move spec).
    copy: CopyOptions,
    /// `[archive]` safety settings (merged into every extract spec).
    extract_limits: ExtractLimits,
    preserve_special_bits: bool,
    /// Debug hook: `SOYUZ_THROTTLE_MBPS` caps copy throughput (to watch progress / pause).
    throttle_bps: Option<u64>,
    /// Undo state of every history entry seen so far (`None` until the first refresh,
    /// so entries loaded from disk at startup are not replayed).
    seen: Option<HashMap<HistoryId, bool>>,
    /// Renames / moves completed (or undone) since the last [`Self::take_moves`].
    moves: Vec<(PathBuf, PathBuf)>,
    /// Paths changed by finished jobs, for relisting the panes that show them.
    affected: AffectedPaths,
}

/// Collects the paths finished jobs changed: the touched roots of jobs submitted through
/// the app (sources, destinations) once they finish, and the paths of new / undone /
/// redone history entries (covers undo, redo and retry jobs the engine creates itself).
#[derive(Default)]
pub struct AffectedPaths {
    /// Touched roots of submitted jobs, until they finish.
    touched: HashMap<JobId, Vec<PathBuf>>,
    pending: Vec<PathBuf>,
}

impl AffectedPaths {
    pub fn submitted(&mut self, id: JobId, kind: &JobKind) {
        self.touched.insert(id, kind.touched_roots());
    }

    pub fn event(&mut self, e: &JobEvent) {
        match e {
            JobEvent::Finished(id, _) => {
                if let Some(t) = self.touched.remove(id) {
                    self.pending.extend(t);
                }
            }
            JobEvent::Removed(id) => {
                self.touched.remove(id);
            }
            _ => {}
        }
    }

    /// A history entry was added, undone or redone.
    pub fn history(&mut self, h: &HistoryEntry) {
        self.pending.extend(soyuz_jobs::undo::touched_roots(&h.records));
        self.pending.extend(h.job.touched_roots());
    }

    pub fn take(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.pending)
    }
}

impl JobsService {
    pub fn new(ctx: &egui::Context, config: &Config) -> Self {
        let c = ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || c.request_repaint());
        let engine = JobEngine::new(engine_config(config), Arc::new(RealProbe), wake);
        let throttle_bps = std::env::var("SOYUZ_THROTTLE_MBPS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v > 0)
            .map(|mb| mb * 1024 * 1024);
        let mut s = JobsService {
            engine,
            snapshots: Vec::new(),
            meta: HashMap::new(),
            history: Vec::new(),
            history_dirty: true,
            logs: HashMap::new(),
            copy: copy_options(config),
            extract_limits: extract_limits(config),
            preserve_special_bits: config.archive.preserve_special_bits,
            throttle_bps,
            seen: None,
            moves: Vec::new(),
            affected: AffectedPaths::default(),
        };
        s.refresh_history();
        s
    }

    /// Re-read `[jobs]` copy options and `[archive]` safety settings (config reload).
    pub fn reconfigure(&mut self, config: &Config) {
        self.copy = copy_options(config);
        self.extract_limits = extract_limits(config);
        self.preserve_special_bits = config.archive.preserve_special_bits;
    }

    pub fn engine(&self) -> &JobEngine {
        &self.engine
    }

    /// A shared handle to the engine (worker threads that wait on jobs: the MCP bridge, the assistant).
    pub fn engine_arc(&self) -> Arc<JobEngine> {
        self.engine.clone()
    }

    /// Default options for a new spec (copy settings from the config, journaling on).
    pub fn spec(&self, kind: JobKind) -> JobSpec {
        let mut spec = JobSpec::new(kind);
        spec.options.copy = self.copy.clone();
        spec.options.copy.throttle_bps = self.throttle_bps;
        spec
    }

    pub fn submit(&mut self, mut spec: JobSpec, meta: JobMeta) -> JobId {
        if matches!(spec.kind, JobKind::Extract { .. }) {
            spec.options.extract_limits = self.extract_limits.clone();
            spec.options.preserve_special_bits = self.preserve_special_bits;
        }
        let kind = spec.kind.clone();
        let id = self.engine.submit(spec);
        self.affected.submitted(id, &kind);
        self.meta.insert(id, meta);
        self.refresh_snapshots();
        id
    }

    /// Register metadata for a job the engine created itself (undo / redo / retry).
    pub fn track(&mut self, id: JobId, meta: JobMeta) {
        self.meta.insert(id, meta);
        self.refresh_snapshots();
    }

    pub fn meta(&self, id: JobId) -> Option<&JobMeta> {
        self.meta.get(&id)
    }

    pub fn snapshots(&self) -> &[JobSnapshot] {
        &self.snapshots
    }

    pub fn snapshot(&self, id: JobId) -> Option<&JobSnapshot> {
        self.snapshots.iter().find(|s| s.id == id)
    }

    /// Drain engine events and refresh snapshots when anything changed or a job is
    /// running. Repaints are requested at 10 Hz only while a job is actually working.
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<JobEvent> {
        let events: Vec<JobEvent> = self.engine.events().try_iter().collect();
        for e in &events {
            match e {
                JobEvent::HistoryChanged => self.history_dirty = true,
                JobEvent::Removed(id) => {
                    self.logs.remove(id);
                    self.meta.remove(id);
                }
                _ => {}
            }
            self.affected.event(e);
        }
        let working = self.snapshots.iter().any(|s| s.state.is_active());
        if !events.is_empty() || working {
            self.refresh_snapshots();
        }
        if self.snapshots.iter().any(|s| s.state.is_active()) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.history_dirty {
            self.refresh_history();
        }
        events
    }

    pub fn refresh_snapshots(&mut self) {
        let mut s = self.engine.snapshots();
        s.sort_by_key(|s| s.id);
        self.snapshots = s;
    }

    fn refresh_history(&mut self) {
        self.history = self.engine.history();
        self.history_dirty = false;
        let first = self.seen.is_none();
        let seen = self.seen.get_or_insert_with(HashMap::new);
        for h in &self.history {
            let before = seen.insert(h.id, h.undone);
            if first || before == Some(h.undone) {
                continue;
            }
            // New entry (done) or a redo → forward; an undo → backward.
            let forward = !h.undone;
            if before.is_none() && h.undone {
                continue;
            }
            self.moves.extend(path_moves(&h.records, forward));
            // Undo / redo / retry jobs are created by the engine: their paths come from here.
            self.affected.history(h);
        }
    }

    /// Renames / moves finished since the last call (`from → to`, undo already reversed).
    /// The app re-keys path-based state (tags) with them.
    pub fn take_moves(&mut self) -> Vec<(PathBuf, PathBuf)> {
        std::mem::take(&mut self.moves)
    }

    /// Paths changed by jobs that finished since the last call (sources, destinations,
    /// created / copied / trashed paths). The app relists panes showing them.
    pub fn take_affected(&mut self) -> Vec<PathBuf> {
        self.affected.take()
    }

    /// Oldest first.
    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// Log lines of a job (fetched incrementally).
    pub fn log(&mut self, id: JobId) -> &[LogLine] {
        let have = self.logs.get(&id).map_or(0, Vec::len);
        let want = self.snapshot(id).map_or(have, |s| s.log_len);
        if want > have {
            let fresh = self.engine.log(id, have);
            self.logs.entry(id).or_default().extend(fresh);
        }
        self.logs.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Jobs that are not finished.
    pub fn pending(&self) -> impl Iterator<Item = &JobSnapshot> {
        self.snapshots.iter().filter(|s| !s.state.is_terminal())
    }

    pub fn shutdown(&self) {
        self.engine.shutdown(Duration::from_secs(2));
    }
}

impl Drop for JobsService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `from → to` pairs of the renames / moves in `records` (reversed for an undo).
pub fn path_moves(records: &[UndoRecord], forward: bool) -> Vec<(PathBuf, PathBuf)> {
    let pairs = records.iter().filter_map(|r| match r {
        UndoRecord::Moved { from, to } | UndoRecord::Renamed { from, to } => Some((from.clone(), to.clone())),
        _ => None,
    });
    if forward { pairs.collect() } else { pairs.rev().map(|(a, b)| (b, a)).collect() }
}

/// `[jobs]` → engine configuration (history next to the config file).
pub fn engine_config(config: &Config) -> EngineConfig {
    let j = &config.jobs;
    EngineConfig {
        max_parallel: j.max_parallel.max(1),
        default_queue: if j.per_device_serial { QueueMode::PerDevice } else { QueueMode::Parallel },
        reset_apply_all_between_jobs: j.reset_apply_all_between_jobs,
        history_limit: j.history_limit.max(1),
        history_path: Some(soyuz_core::config::config_dir().join("history.jsonl")),
        journal_path: crate::services::crash::journal_path(),
        ..EngineConfig::default()
    }
}

/// `[jobs]` → copy options.
pub fn copy_options(config: &Config) -> CopyOptions {
    let j = &config.jobs;
    let mut preserve = Preserve::empty();
    for p in &j.preserve {
        preserve |= match p {
            PreserveItem::Times => Preserve::TIMES,
            PreserveItem::Attrs => Preserve::ATTRS,
            PreserveItem::Xattr => Preserve::XATTR,
            PreserveItem::Acl => Preserve::ACL,
            PreserveItem::Owner => Preserve::OWNER,
            PreserveItem::Sparse => Preserve::SPARSE,
        };
    }
    CopyOptions {
        preserve,
        reflink: match j.reflink {
            ReflinkPref::Auto => Reflink::Auto,
            ReflinkPref::Never => Reflink::Never,
            ReflinkPref::Always => Reflink::Always,
        },
        unbuffered_threshold: j.unbuffered_threshold_mib.max(1) * 1024 * 1024,
        chunk: (j.copy_buffer_mib.clamp(1, 256) * 1024 * 1024) as usize,
        verify: j.verify_after_copy.then_some(HashAlgo::Blake3),
        ..CopyOptions::default()
    }
}

/// `[archive]` → decompression-bomb limits for extract jobs and "open entry".
pub fn extract_limits(config: &Config) -> ExtractLimits {
    const MIB: u64 = 1024 * 1024;
    let a = &config.archive;
    ExtractLimits {
        max_ratio: a.bomb_ratio,
        min_total_bytes: a.bomb_floor_mib.saturating_mul(MIB),
        max_entries: a.max_entries,
        max_entry_bytes: a.max_entry_mib.saturating_mul(MIB),
    }
}

/// Default `JobOptions` with journaling (exposed for tests of option building).
pub fn journaled() -> JobOptions {
    JobOptions { journal: true, ..JobOptions::default() }
}

/// Directory a location refers to on disk, if any.
pub fn location_dir(loc: &Location) -> Option<PathBuf> {
    match loc {
        Location::Dir(p) => Some(p.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_config_from_jobs_section() {
        let mut c = Config::default();
        c.jobs.max_parallel = 0;
        c.jobs.per_device_serial = false;
        c.jobs.history_limit = 42;
        let e = engine_config(&c);
        assert_eq!(e.max_parallel, 1, "never zero");
        assert_eq!(e.default_queue, QueueMode::Parallel);
        assert_eq!(e.history_limit, 42);
        assert!(e.history_path.unwrap().ends_with("history.jsonl"));
    }

    #[test]
    fn copy_options_from_jobs_section() {
        let mut c = Config::default();
        c.jobs.preserve = vec![PreserveItem::Times, PreserveItem::Sparse];
        c.jobs.verify_after_copy = true;
        c.jobs.copy_buffer_mib = 4;
        let o = copy_options(&c);
        assert_eq!(o.preserve, Preserve::TIMES | Preserve::SPARSE);
        assert_eq!(o.verify, Some(HashAlgo::Blake3));
        assert_eq!(o.chunk, 4 * 1024 * 1024);
        assert!(journaled().journal);
    }

    #[test]
    fn extract_limits_from_archive_section() {
        let d = extract_limits(&Config::default());
        assert_eq!(d, ExtractLimits::default(), "config defaults match the archive crate's");
        let mut c = Config::default();
        c.archive.bomb_ratio = 0;
        c.archive.max_entry_mib = 2;
        let l = extract_limits(&c);
        assert_eq!(l.total_cap(1 << 20), None);
        assert_eq!(l.entry_cap(), Some(2 * 1024 * 1024));
    }

    #[test]
    fn moves_from_records() {
        let r = vec![
            UndoRecord::Renamed { from: "/a/x".into(), to: "/a/y".into() },
            UndoRecord::Created("/a/n".into()),
            UndoRecord::Moved { from: "/a/z".into(), to: "/b/z".into() },
        ];
        let fwd = path_moves(&r, true);
        assert_eq!(fwd, vec![("/a/x".into(), "/a/y".into()), ("/a/z".into(), "/b/z".into())]);
        let back = path_moves(&r, false);
        assert_eq!(back, vec![("/b/z".into(), "/a/z".into()), ("/a/y".into(), "/a/x".into())]);
    }

    #[test]
    fn op_kinds() {
        assert_eq!(OpKind::from_brief(JobKindBrief::Trash), OpKind::Trash);
        assert!(OpKind::Copy.undoable());
        assert!(!OpKind::Delete.undoable());
        assert!(!OpKind::Undo.undoable());
    }
}
