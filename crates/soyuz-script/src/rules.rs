//! Rule-based automations (`[[automation.rules]]`): matching, planning and scheduling.
//!
//! Pure parts (tested with temp dirs): [`CompiledRule`] decides whether an entry
//! matches, [`plan_step`] turns one [`RuleAction`] into a job / tag / script step for
//! the item's current path, [`Scheduler`] debounces watcher events (an item is acted
//! on `delay_secs` after its last change) and suppresses the events our own jobs cause.
//! [`preview`] lists what a rule would do right now. The runtime that drives them from
//! the watcher and runs the jobs is [`worker`].

pub mod worker;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::{Regex, RegexBuilder};
use soyuz_core::config::{AutomationRule, RuleAction, RuleConflict, RuleKind};
use soyuz_core::{ConflictResolution, EntryInfo, EntryKind};
use soyuz_jobs::{DeleteMode, JobKind};

/// Names of partial downloads / temp files; never acted on.
const PARTIAL_EXT: &[&str] = &["crdownload", "part", "partial", "download", "tmp", "opdownload", "!ut"];

/// A rule ready for matching.
#[derive(Clone, Debug)]
pub struct CompiledRule {
    pub rule: AutomationRule,
    globs: Option<GlobSet>,
    regex: Option<Regex>,
}

impl CompiledRule {
    pub fn compile(rule: &AutomationRule) -> Result<Self, String> {
        if rule.watch.as_os_str().is_empty() {
            return Err("choose a folder to watch".into());
        }
        if !rule.watch.is_absolute() {
            return Err(format!("“{}” is not an absolute path", rule.watch.display()));
        }
        let pats: Vec<&str> = rule.glob.split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
        let globs = if pats.is_empty() {
            None
        } else {
            let mut b = GlobSetBuilder::new();
            for p in pats {
                b.add(
                    GlobBuilder::new(p)
                        .case_insensitive(true)
                        .literal_separator(true)
                        .build()
                        .map_err(|e| format!("pattern “{p}”: {e}"))?,
                );
            }
            Some(b.build().map_err(|e| e.to_string())?)
        };
        let regex = if rule.regex.trim().is_empty() {
            None
        } else {
            Some(
                RegexBuilder::new(rule.regex.trim())
                    .case_insensitive(true)
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|e| format!("regular expression: {e}"))?,
            )
        };
        for a in &rule.actions {
            match a {
                RuleAction::Move { to } | RuleAction::Copy { to } if !to.is_absolute() => {
                    return Err(format!("{}: “{}” is not an absolute folder", a.label(), to.display()));
                }
                RuleAction::Rename { pattern } if pattern.trim().is_empty() => {
                    return Err("Rename: the pattern is empty".into());
                }
                RuleAction::Tag { tag } if tag.trim().is_empty() => return Err("Tag: choose a color".into()),
                RuleAction::Script { script } if script.trim().is_empty() => {
                    return Err("Run script: choose a script".into());
                }
                _ => {}
            }
        }
        Ok(CompiledRule { rule: rule.clone(), globs, regex })
    }

    /// Whether `path` is inside the watched folder (a direct child unless recursive).
    pub fn covers(&self, path: &Path) -> bool {
        let Ok(rest) = path.strip_prefix(&self.rule.watch) else { return false };
        let n = rest.components().count();
        n == 1 || (self.rule.recursive && n > 1)
    }

    /// Whether the entry passes the rule's filters at `now`.
    pub fn matches(&self, e: &EntryInfo, now: SystemTime) -> bool {
        let r = &self.rule;
        let is_dir = e.is_dir_like();
        let kind_ok = match r.kind {
            RuleKind::Files => !is_dir,
            RuleKind::Folders => is_dir,
            RuleKind::Any => true,
        };
        if !kind_ok || !self.covers(&e.path) {
            return false;
        }
        if !is_dir
            && let Some(ext) = crate::api::name_ext(&e.name)
            && PARTIAL_EXT.iter().any(|p| p.eq_ignore_ascii_case(ext))
        {
            return false;
        }
        if let Some(g) = &self.globs
            && !g.is_match(&e.name)
        {
            return false;
        }
        if let Some(re) = &self.regex
            && !re.is_match(&e.name)
        {
            return false;
        }
        if !is_dir {
            let kib = e.size / 1024;
            if r.min_size_kib > 0 && kib < r.min_size_kib {
                return false;
            }
            if r.max_size_kib > 0 && kib > r.max_size_kib {
                return false;
            }
        }
        if r.min_age_minutes > 0 || r.max_age_minutes > 0 {
            let Some(m) = e.modified else { return false };
            let age = now.duration_since(m).unwrap_or_default().as_secs() / 60;
            if r.min_age_minutes > 0 && age < r.min_age_minutes {
                return false;
            }
            if r.max_age_minutes > 0 && age > r.max_age_minutes {
                return false;
            }
        }
        true
    }

    /// Pre-answered conflict resolution for the rule's jobs (`None` = ask).
    pub fn conflict(&self) -> Option<ConflictResolution> {
        match self.rule.conflict {
            RuleConflict::KeepBoth => Some(ConflictResolution::KeepBoth),
            RuleConflict::Skip => Some(ConflictResolution::Skip),
            RuleConflict::Overwrite => Some(ConflictResolution::Overwrite),
            RuleConflict::Ask => None,
        }
    }
}

fn local_date(t: SystemTime, fmt: &str) -> String {
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format(fmt).to_string()
}

/// Expand a rename pattern for `path`: `{name}` `{stem}` `{ext}` `{date}` `{time}`
/// `{mdate}` `{parent}`; `{{` / `}}` are literal braces.
pub fn expand_pattern(
    pattern: &str,
    path: &Path,
    modified: Option<SystemTime>,
    now: SystemTime,
) -> Result<String, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = crate::api::name_ext(&name).unwrap_or("").to_string();
    let stem = if ext.is_empty() { name.clone() } else { name[..name.len() - ext.len() - 1].to_string() };
    let parent =
        path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = String::new();
    let mut it = pattern.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                out.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                out.push('}');
            }
            '{' => {
                let mut key = String::new();
                loop {
                    match it.next() {
                        Some('}') => break,
                        Some(k) => key.push(k),
                        None => return Err(format!("unclosed “{{{key}”")),
                    }
                }
                out.push_str(&match key.as_str() {
                    "name" => name.clone(),
                    "stem" => stem.clone(),
                    "ext" => ext.clone(),
                    "parent" => parent.clone(),
                    "date" => local_date(now, "%Y-%m-%d"),
                    "time" => local_date(now, "%H%M%S"),
                    "mdate" => local_date(modified.unwrap_or(now), "%Y-%m-%d"),
                    other => return Err(format!("unknown placeholder “{{{other}}}”")),
                });
            }
            c => out.push(c),
        }
    }
    soyuz_core::validate_name(&out).map_err(|e| format!("“{out}”: {e}"))?;
    Ok(out)
}

/// One planned step for an item.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Run a job; afterwards the item lives at `then`.
    Job {
        kind: JobKind,
        summary: String,
        then: PathBuf,
    },
    Tag(String),
    Script(String),
}

fn quoted(p: &Path) -> String {
    format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(p))
}

/// Plan `action` for the item currently at `current`. `dir_exists` tells whether a
/// target folder exists (a missing one gets a "new folder" job first).
pub fn plan_step(
    action: &RuleAction,
    current: &Path,
    modified: Option<SystemTime>,
    now: SystemTime,
    dir_exists: &dyn Fn(&Path) -> bool,
) -> Result<Vec<Step>, String> {
    let name = current.file_name().ok_or("the item has no name")?.to_owned();
    let mut steps = Vec::new();
    // Missing target folders (and missing parents) are created first, outermost first.
    let ensure = |to: &Path, steps: &mut Vec<Step>| {
        let mut missing = Vec::new();
        let mut d = Some(to);
        while let Some(dir) = d {
            if dir.parent().is_none() || dir_exists(dir) {
                break;
            }
            missing.push(dir.to_path_buf());
            d = dir.parent();
        }
        for dir in missing.into_iter().rev() {
            steps.push(Step::Job {
                summary: format!("New folder {}", quoted(&dir)),
                kind: JobKind::Create { path: dir, kind: soyuz_jobs::CreateKind::Dir },
                then: current.to_path_buf(),
            });
        }
    };
    match action {
        RuleAction::Move { to } => {
            ensure(to, &mut steps);
            steps.push(Step::Job {
                kind: JobKind::Move { sources: vec![current.to_path_buf()], dest_dir: to.clone(), rename_to: None },
                summary: format!("Move {} to {}", quoted(current), quoted(to)),
                then: to.join(&name),
            });
        }
        RuleAction::Copy { to } => {
            ensure(to, &mut steps);
            steps.push(Step::Job {
                kind: JobKind::Copy { sources: vec![current.to_path_buf()], dest_dir: to.clone(), rename_to: None },
                summary: format!("Copy {} to {}", quoted(current), quoted(to)),
                then: current.to_path_buf(),
            });
        }
        RuleAction::Rename { pattern } => {
            let new = expand_pattern(pattern, current, modified, now)?;
            let parent = current.parent().ok_or("the item has no folder")?;
            let to = parent.join(&new);
            if to != current {
                steps.push(Step::Job {
                    kind: JobKind::Rename { pairs: vec![(current.to_path_buf(), to.clone())] },
                    summary: format!("Rename {} to {}", quoted(current), quoted(&to)),
                    then: to,
                });
            }
        }
        RuleAction::Trash => steps.push(Step::Job {
            kind: JobKind::Delete { paths: vec![current.to_path_buf()], mode: DeleteMode::Trash },
            summary: format!("Move {} to trash", quoted(current)),
            then: current.to_path_buf(),
        }),
        RuleAction::Tag { tag } => steps.push(Step::Tag(tag.clone())),
        RuleAction::Script { script } => steps.push(Step::Script(script.clone())),
    }
    Ok(steps)
}

/// Human description of a whole rule for one item (dry runs, the live preview).
pub fn describe(rule: &AutomationRule, e: &EntryInfo, now: SystemTime) -> Vec<String> {
    let mut current = e.path.clone();
    let mut out = Vec::new();
    for a in &rule.actions {
        match plan_step(a, &current, e.modified, now, &|_| true) {
            Ok(steps) => {
                for s in steps {
                    match s {
                        Step::Job { summary, then, .. } => {
                            out.push(summary);
                            current = then;
                        }
                        Step::Tag(t) => out.push(format!("Tag {} {t}", quoted(&current))),
                        Step::Script(sc) => out.push(format!("Run script “{sc}” on {}", quoted(&current))),
                    }
                }
                if matches!(a, RuleAction::Trash) {
                    break;
                }
            }
            Err(err) => {
                out.push(format!("{}: {err}", a.label()));
                break;
            }
        }
    }
    out
}

/// One item a rule would act on now.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewItem {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub steps: Vec<String>,
}

/// Items in the rule's folder that match right now (at most `limit`), with what would
/// happen to each. Runs on a worker thread (lists the folder).
pub fn preview(rule: &AutomationRule, limit: usize) -> Result<(Vec<PreviewItem>, usize), String> {
    let c = CompiledRule::compile(rule)?;
    let now = SystemTime::now();
    let opts = soyuz_fs::ListOptions { include_hidden: true, ..Default::default() };
    let mut items = Vec::new();
    let mut total = 0usize;
    let mut stack = vec![(rule.watch.clone(), 0usize)];
    let start = Instant::now();
    while let Some((dir, depth)) = stack.pop() {
        let entries = soyuz_fs::read_dir(&dir, &opts).map_err(|e| e.to_string())?;
        for e in entries {
            if c.matches(&e, now) {
                total += 1;
                if items.len() < limit {
                    items.push(PreviewItem {
                        path: e.path.clone(),
                        is_dir: e.is_dir_like(),
                        size: e.size,
                        steps: describe(rule, &e, now),
                    });
                }
            }
            if rule.recursive && e.kind == EntryKind::Dir && depth < 16 && start.elapsed() < Duration::from_secs(5) {
                stack.push((e.path.clone(), depth + 1));
            }
        }
    }
    items.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((items, total))
}

/// How long events for a path our own jobs produced are ignored.
pub const SUPPRESS_FOR: Duration = Duration::from_secs(30);

/// Debounce + loop protection for watcher events.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// `(rule, path)` → when it is due.
    pending: HashMap<(usize, PathBuf), Instant>,
    /// Paths the automation itself touched recently.
    suppressed: HashMap<PathBuf, Instant>,
}

impl Scheduler {
    /// A change of `path` relevant to `rule`: (re)start its delay.
    pub fn changed(&mut self, rule: usize, path: &Path, delay: Duration, now: Instant) {
        if self.is_suppressed(path, now) {
            return;
        }
        self.pending.insert((rule, path.to_path_buf()), now + delay);
    }

    /// Items whose delay has passed (removed from the queue).
    pub fn due(&mut self, now: Instant) -> Vec<(usize, PathBuf)> {
        let mut out: Vec<(usize, PathBuf)> =
            self.pending.iter().filter(|(_, at)| **at <= now).map(|(k, _)| k.clone()).collect();
        out.sort();
        for k in &out {
            self.pending.remove(k);
        }
        out
    }

    /// Ignore events for `path` for [`SUPPRESS_FOR`] (output of our own jobs).
    pub fn suppress(&mut self, path: &Path, now: Instant) {
        self.suppressed.insert(path.to_path_buf(), now + SUPPRESS_FOR);
        self.pending.retain(|(_, p), _| p != path);
    }

    pub fn is_suppressed(&mut self, path: &Path, now: Instant) -> bool {
        self.suppressed.retain(|_, until| *until > now);
        self.suppressed.contains_key(path)
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Next deadline (to sleep until).
    pub fn next_due(&self) -> Option<Instant> {
        self.pending.values().min().copied()
    }

    /// Forget everything about rules `>= n` (rules were removed / reordered).
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests;
