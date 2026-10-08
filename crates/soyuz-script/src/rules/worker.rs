//! The automation runtime: one thread that owns a folder watcher for the rules'
//! folders, debounces events with the [`Scheduler`], re-checks items when they are due
//! and runs each item's steps as jobs on the shared [`JobEngine`] (one after another,
//! so a move is finished before the rename that follows it). Events for the UI (jobs
//! to tag in the activity panel, run-log lines, tags to apply) come back on a channel.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender, select, unbounded};
use soyuz_core::config::{Automation, RuleAction};
use soyuz_core::{ConflictResolution, EntryInfo};
use soyuz_fs::{CopyOptions, WatchEvent, WatchId, WatchKind, Watcher};
use soyuz_jobs::{JobEngine, JobId, JobKind, JobSpec, JobState};

use super::{CompiledRule, Scheduler, Step, plan_step};
use crate::ScriptSet;

/// Severity of a run-log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    /// A job ran (or a tag / script step finished).
    Done,
    /// Dry run: what would have happened.
    DryRun,
    /// A step failed; the item's remaining steps were skipped.
    Error,
    /// Informational (watching started, …).
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub when: SystemTime,
    pub path: PathBuf,
    pub kind: LogKind,
    pub message: String,
}

/// Messages to the worker.
pub enum Cmd {
    Configure {
        automation: Automation,
        copy: CopyOptions,
    },
    Scripts(Arc<ScriptSet>),
    /// Act on everything the rule (by name) matches now.
    RunNow(String),
}

/// Messages from the worker.
#[derive(Clone, Debug)]
pub enum Event {
    /// A job was submitted for `rule`.
    Submitted {
        job: JobId,
        rule: String,
        items: Vec<PathBuf>,
    },
    Log {
        rule: String,
        entry: LogEntry,
    },
    /// Add `tag` to `paths` (tags live on the UI thread).
    Tag {
        paths: Vec<PathBuf>,
        tag: String,
    },
    Toast(String),
    /// Per rule: watching, or why not.
    Status(Vec<(String, Result<(), String>)>),
}

/// Handle to the running worker; dropping it stops the thread.
pub struct Handle {
    tx: Sender<Cmd>,
    pub events: Receiver<Event>,
}

impl Handle {
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

/// Start the worker. `wake` is called after events are sent (repaint the UI).
pub fn spawn(engine: Arc<JobEngine>, wake: Arc<dyn Fn() + Send + Sync>, debounce: Duration) -> Handle {
    let (tx, rx) = unbounded::<Cmd>();
    let (etx, erx) = unbounded::<Event>();
    let _ = std::thread::Builder::new().name("soyuz-automation".into()).spawn(move || {
        let mut w = Worker::new(engine, etx, wake, debounce);
        w.run(rx);
    });
    Handle { tx, events: erx }
}

/// What one item still has to do.
#[derive(Debug)]
struct Pipeline {
    rule: usize,
    rule_name: String,
    dry: bool,
    current: PathBuf,
    modified: Option<SystemTime>,
    actions: VecDeque<RuleAction>,
    queue: VecDeque<Step>,
    waiting: Option<(JobId, PathBuf, String)>,
    done: Vec<String>,
}

pub(crate) struct Worker {
    engine: Arc<JobEngine>,
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    watcher: Option<Watcher>,
    watch_rx: Receiver<WatchEvent>,
    watches: HashMap<WatchId, Vec<usize>>,
    rules: Vec<CompiledRule>,
    enabled: bool,
    dry_run: bool,
    copy: CopyOptions,
    scripts: Option<Arc<ScriptSet>>,
    scheduler: Scheduler,
    pipelines: Vec<Pipeline>,
    /// `(name, folder)` of rules whose existing items were already handled.
    seen: HashSet<(String, PathBuf)>,
}

impl Worker {
    pub(crate) fn new(
        engine: Arc<JobEngine>,
        events: Sender<Event>,
        wake: Arc<dyn Fn() + Send + Sync>,
        debounce: Duration,
    ) -> Self {
        let (wtx, wrx) = unbounded();
        let watcher = match Watcher::new(wtx, debounce, Arc::new(|| {})) {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn!("automation: folder watching unavailable: {e}");
                None
            }
        };
        Worker {
            engine,
            events,
            wake,
            watcher,
            watch_rx: wrx,
            watches: HashMap::new(),
            rules: Vec::new(),
            enabled: false,
            dry_run: false,
            copy: CopyOptions::default(),
            scripts: None,
            scheduler: Scheduler::default(),
            pipelines: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn log(&self, rule: &str, path: &Path, kind: LogKind, message: String) {
        self.emit(Event::Log {
            rule: rule.to_string(),
            entry: LogEntry { when: SystemTime::now(), path: path.to_path_buf(), kind, message },
        });
    }

    fn run(&mut self, rx: Receiver<Cmd>) {
        loop {
            let busy = !self.pipelines.is_empty();
            let wait = match self.scheduler.next_due() {
                Some(t) => t.saturating_duration_since(Instant::now()).min(Duration::from_millis(500)),
                None => Duration::from_millis(if busy { 150 } else { 1000 }),
            }
            .min(if busy { Duration::from_millis(150) } else { Duration::from_secs(1) });
            select! {
                recv(rx) -> m => match m {
                    Ok(c) => self.command(c),
                    Err(_) => return,
                },
                recv(self.watch_rx) -> ev => if let Ok(ev) = ev { self.watch_event(ev) },
                default(wait) => {}
            }
            self.tick(Instant::now());
        }
    }

    pub(crate) fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Configure { automation, copy } => self.configure(automation, copy),
            Cmd::Scripts(s) => self.scripts = Some(s),
            Cmd::RunNow(name) => {
                let Some(idx) = self.rules.iter().position(|r| r.rule.name == name) else {
                    self.emit(Event::Toast(format!("Rule “{name}” is not active (check its settings)")));
                    return;
                };
                let n = self.scan(idx, true);
                if n == 0 {
                    self.emit(Event::Toast(format!("“{name}”: nothing matches right now")));
                }
            }
        }
    }

    fn configure(&mut self, a: Automation, copy: CopyOptions) {
        self.copy = copy;
        self.enabled = a.enabled;
        self.dry_run = a.dry_run;
        if let Some(w) = &mut self.watcher {
            for id in self.watches.keys() {
                w.unwatch(*id);
            }
        }
        self.watches.clear();
        self.scheduler.clear();
        self.rules.clear();
        let mut status = Vec::new();
        for r in &a.rules {
            if !r.enabled || !a.enabled {
                continue;
            }
            match CompiledRule::compile(r) {
                Ok(c) => self.rules.push(c),
                Err(e) => status.push((r.name.clone(), Err(e))),
            }
        }
        // Pipelines keep running, but their rule indices may now be stale: re-key by name.
        for p in &mut self.pipelines {
            p.rule = self.rules.iter().position(|r| r.rule.name == p.rule_name).unwrap_or(usize::MAX);
        }
        let mut by_folder: HashMap<(PathBuf, bool), Vec<usize>> = HashMap::new();
        for (i, r) in self.rules.iter().enumerate() {
            by_folder.entry((r.rule.watch.clone(), r.rule.recursive)).or_default().push(i);
        }
        for ((dir, recursive), idx) in by_folder {
            let res = match &mut self.watcher {
                Some(w) => w.watch(&dir, recursive).map_err(|e| e.to_string()),
                None => Err("folder watching is unavailable".to_string()),
            };
            match res {
                Ok(id) => {
                    for i in &idx {
                        status.push((self.rules[*i].rule.name.clone(), Ok(())));
                    }
                    self.watches.insert(id, idx);
                }
                Err(e) => {
                    for i in &idx {
                        status.push((self.rules[*i].rule.name.clone(), Err(e.clone())));
                    }
                }
            }
        }
        for i in 0..self.rules.len() {
            let r = &self.rules[i].rule;
            let key = (r.name.clone(), r.watch.clone());
            if r.apply_to_existing && self.seen.insert(key) {
                self.scan(i, false);
            }
        }
        self.emit(Event::Status(status));
    }

    /// Schedule (or with `now`, start) every current match of rule `idx`.
    fn scan(&mut self, idx: usize, now: bool) -> usize {
        let rule = self.rules[idx].rule.clone();
        let (items, _) = match super::preview(&rule, 10_000) {
            Ok(v) => v,
            Err(e) => {
                self.log(&rule.name, &rule.watch, LogKind::Error, e);
                return 0;
            }
        };
        let n = items.len();
        for it in items {
            if now {
                if let Ok(e) = soyuz_fs::stat(&it.path, false) {
                    self.start(idx, &e);
                }
            } else {
                self.scheduler.changed(idx, &it.path, Duration::from_secs(rule.delay_secs), Instant::now());
            }
        }
        n
    }

    pub(crate) fn watch_event(&mut self, ev: WatchEvent) {
        let Some(idx) = self.watches.get(&ev.id).cloned() else { return };
        let now = Instant::now();
        match ev.kind {
            WatchKind::Changed(paths) => {
                for p in paths {
                    for &i in &idx {
                        let r = &self.rules[i];
                        if r.covers(&p) {
                            self.scheduler.changed(i, &p, Duration::from_secs(r.rule.delay_secs), now);
                        }
                    }
                }
            }
            WatchKind::Rescan => {
                for &i in &idx {
                    self.scan(i, false);
                }
            }
            WatchKind::Lost => {
                for &i in &idx {
                    let r = &self.rules[i].rule;
                    self.log(&r.name, &r.watch, LogKind::Error, "the watched folder is gone".into());
                }
            }
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) {
        if self.enabled {
            for (i, path) in self.scheduler.due(now) {
                if i >= self.rules.len() || self.pipelines.iter().any(|p| p.current == path) {
                    continue;
                }
                // Gone (moved away, a temp file) or no longer matching: nothing to do.
                let Ok(e) = soyuz_fs::stat(&path, false) else { continue };
                if self.rules[i].matches(&e, SystemTime::now()) {
                    self.start(i, &e);
                }
            }
        }
        let mut pipelines = std::mem::take(&mut self.pipelines);
        pipelines.retain_mut(|p| !self.advance(p));
        self.pipelines.extend(pipelines);
    }

    fn start(&mut self, idx: usize, e: &EntryInfo) {
        let r = &self.rules[idx].rule;
        let p = Pipeline {
            rule: idx,
            rule_name: r.name.clone(),
            dry: self.dry_run || r.dry_run,
            current: e.path.clone(),
            modified: e.modified,
            actions: r.actions.iter().cloned().collect(),
            queue: VecDeque::new(),
            waiting: None,
            done: Vec::new(),
        };
        self.scheduler.suppress(&e.path, Instant::now());
        self.pipelines.push(p);
    }

    fn conflict(&self, p: &Pipeline) -> Option<ConflictResolution> {
        self.rules.get(p.rule).and_then(|r| r.conflict()).or(Some(ConflictResolution::KeepBoth))
    }

    /// Run the item's next steps; returns `true` when it is finished.
    fn advance(&mut self, p: &mut Pipeline) -> bool {
        if let Some((job, then, summary)) = p.waiting.take() {
            let Some(snap) = self.engine.snapshot(job) else {
                self.log(&p.rule_name, &p.current, LogKind::Error, format!("{summary}: the job disappeared"));
                return true;
            };
            match &snap.state {
                JobState::Done(sum) if sum.items_failed == 0 => {
                    if sum.items_skipped > 0 {
                        self.log(
                            &p.rule_name,
                            &p.current,
                            LogKind::Info,
                            format!("{summary}: skipped (name conflict)"),
                        );
                        return true;
                    }
                    p.done.push(summary);
                    self.scheduler.suppress(&then, Instant::now());
                    p.current = then;
                }
                JobState::Done(sum) => {
                    let why = sum.errors.first().map(|(_, e)| e.to_string()).unwrap_or_else(|| "failed".into());
                    self.log(&p.rule_name, &p.current, LogKind::Error, format!("{summary}: {why}"));
                    return true;
                }
                JobState::Failed(e) => {
                    self.log(&p.rule_name, &p.current, LogKind::Error, format!("{summary}: {e}"));
                    return true;
                }
                JobState::Cancelled => {
                    self.log(&p.rule_name, &p.current, LogKind::Error, format!("{summary}: cancelled"));
                    return true;
                }
                _ => {
                    p.waiting = Some((job, then, summary));
                    return false;
                }
            }
        }
        loop {
            if let Some(step) = p.queue.pop_front() {
                match step {
                    Step::Job { kind, summary, then } => {
                        if p.dry {
                            p.done.push(format!("Would: {summary}"));
                            p.current = then;
                            continue;
                        }
                        let id = self.submit(p, kind, &summary);
                        p.waiting = Some((id, then, summary));
                        return false;
                    }
                    Step::Tag(tag) => {
                        if p.dry {
                            p.done.push(format!("Would: tag {tag}"));
                        } else {
                            self.emit(Event::Tag { paths: vec![p.current.clone()], tag: tag.clone() });
                            p.done.push(format!("Tag {tag}"));
                        }
                    }
                    Step::Script(name) => {
                        if p.dry {
                            p.done.push(format!("Would: run script “{name}”"));
                            continue;
                        }
                        let Some(scripts) = self.scripts.clone() else {
                            self.log(&p.rule_name, &p.current, LogKind::Error, "scripts are not loaded".into());
                            return true;
                        };
                        match scripts.run_rule(&name, &p.current, &p.rule_name) {
                            Ok(out) => {
                                for j in out.jobs {
                                    p.queue.push_back(Step::Job {
                                        kind: j.kind,
                                        summary: j.summary,
                                        then: p.current.clone(),
                                    });
                                }
                                for op in out.ui {
                                    match op {
                                        crate::UiOp::Tag(paths, tag) => self.emit(Event::Tag { paths, tag }),
                                        crate::UiOp::Toast(m) => self.emit(Event::Toast(m)),
                                        _ => {}
                                    }
                                }
                                p.done.push(format!("Script “{name}”"));
                            }
                            Err(e) => {
                                self.log(&p.rule_name, &p.current, LogKind::Error, format!("script “{name}”: {e}"));
                                return true;
                            }
                        }
                    }
                }
                continue;
            }
            let Some(action) = p.actions.pop_front() else {
                let kind = if p.dry { LogKind::DryRun } else { LogKind::Done };
                let msg = if p.done.is_empty() { "nothing to do".to_string() } else { p.done.join(" → ") };
                self.log(&p.rule_name, &p.current, kind, msg);
                return true;
            };
            if matches!(action, RuleAction::Trash) {
                p.actions.clear();
            }
            let dir_exists = |d: &Path| p.dry || soyuz_fs::stat(d, true).is_ok_and(|e| e.is_dir_like());
            match plan_step(&action, &p.current, p.modified, SystemTime::now(), &dir_exists) {
                Ok(steps) => p.queue.extend(steps),
                Err(e) => {
                    self.log(&p.rule_name, &p.current, LogKind::Error, format!("{}: {e}", action.label()));
                    return true;
                }
            }
        }
    }

    fn submit(&mut self, p: &Pipeline, kind: JobKind, summary: &str) -> JobId {
        let items: Vec<PathBuf> = match &kind {
            JobKind::Copy { sources, .. } | JobKind::Move { sources, .. } => sources.clone(),
            JobKind::Delete { paths, .. } => paths.clone(),
            JobKind::Rename { pairs } => pairs.iter().map(|(a, _)| a.clone()).collect(),
            JobKind::Create { path, .. } => vec![path.clone()],
            _ => Vec::new(),
        };
        let creates_dir = matches!(kind, JobKind::Create { .. });
        let mut spec = JobSpec::new(kind);
        spec.options.copy = self.copy.clone();
        spec.options.conflict = if creates_dir { Some(ConflictResolution::MergeDir) } else { self.conflict(p) };
        spec.options.label = Some(format!("{summary} · {}", p.rule_name));
        let id = self.engine.submit(spec);
        self.emit(Event::Submitted { job: id, rule: p.rule_name.clone(), items });
        id
    }

    #[cfg(test)]
    pub(crate) fn idle(&self) -> bool {
        self.pipelines.is_empty() && self.scheduler.pending_len() == 0
    }

    #[cfg(test)]
    pub(crate) fn schedule_now(&mut self, path: &Path) {
        for i in 0..self.rules.len() {
            if self.rules[i].covers(path) {
                self.scheduler.changed(i, path, Duration::ZERO, Instant::now());
            }
        }
    }
}
