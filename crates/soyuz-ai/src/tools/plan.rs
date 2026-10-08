//! Dry-run plans for write tools: the planned items, conflicts and sizes, plus the
//! `JobSpec` (or app action) that runs after approval. Planning only reads metadata.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};
use soyuz_core::{Attributes, ConflictResolution, Control, EntryKind, OpControl};
use soyuz_fs::{AttrSet, TimesSet, WalkItem, WalkOptions};
use soyuz_jobs::{CreateKind, DeleteMode, JobKind, JobSpec, SyncAction, SyncPlan};

use super::args::*;
use super::scope::Scope;
use super::{Risk, ToolError};

/// Upper bound on entries walked while sizing one plan.
pub const MAX_PLAN_WALK: u64 = 2_000_000;
/// Items included in the model-facing plan summary.
pub const MODEL_ITEMS: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanOp {
    Copy,
    Move,
    Rename,
    Create,
    Mkdir,
    SetMeta,
    Extract,
    Compress,
    Restore,
    Trash,
    Delete,
    EmptyTrash,
    Tag,
    SyncCopy,
    SyncDelete,
    Undo,
    RunCommand,
}

/// A problem with one planned item.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlanConflict {
    /// The destination exists; `resolution` is what the job will do.
    Exists {
        kind: String,
        size: u64,
        resolution: String,
    },
    /// The source does not exist.
    Missing,
    /// A folder would be copied / moved into itself.
    IntoSelf,
    /// Source and destination are the same item (no-op).
    SameItem,
    /// Two items would get the same name.
    DuplicateTarget,
    InvalidName {
        reason: String,
    },
}

impl PlanConflict {
    /// Conflicts that make the plan not executable.
    pub fn is_blocking(&self) -> bool {
        !matches!(self, PlanConflict::Exists { resolution, .. } if resolution != "blocked")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlanItem {
    pub op: PlanOp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst: Option<PathBuf>,
    /// `file` / `dir` / `link` / …
    pub kind: String,
    /// File bytes in the item's tree.
    pub bytes: u64,
    /// Entries in the item's tree (1 for a file).
    pub entries: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<PlanConflict>,
}

impl PlanItem {
    fn new(op: PlanOp, src: Option<PathBuf>, dst: Option<PathBuf>) -> Self {
        PlanItem { op, src, dst, kind: String::new(), bytes: 0, entries: 0, conflict: None }
    }
}

/// What runs after approval.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlanAction {
    Job(JobSpec),
    /// Handled by the app (tags, undo, custom commands).
    App {
        op: String,
        args: Value,
    },
    /// Nothing to do.
    None,
}

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub tool: String,
    pub summary: String,
    pub risk: Risk,
    pub items: Vec<PlanItem>,
    pub total_bytes: u64,
    pub total_entries: u64,
    pub conflicts: usize,
    /// No blocking conflicts and something to do.
    pub executable: bool,
    pub warnings: Vec<String>,
    pub action: PlanAction,
}

impl Plan {
    fn finish(tool: &str, risk: Risk, summary: String, items: Vec<PlanItem>, action: PlanAction) -> Plan {
        let total_bytes = items.iter().map(|i| i.bytes).sum();
        let total_entries = items.iter().map(|i| i.entries).sum();
        let conflicts = items.iter().filter(|i| i.conflict.is_some()).count();
        let blocking = items.iter().any(|i| i.conflict.as_ref().is_some_and(PlanConflict::is_blocking));
        let executable = !blocking && !matches!(action, PlanAction::None);
        Plan {
            tool: tool.to_string(),
            summary,
            risk,
            items,
            total_bytes,
            total_entries,
            conflicts,
            executable,
            warnings: Vec::new(),
            action,
        }
    }

    /// The job, when the plan runs one.
    pub fn job(&self) -> Option<&JobSpec> {
        match &self.action {
            PlanAction::Job(j) => Some(j),
            _ => None,
        }
    }

    /// Compact JSON for the model: summary, totals and the first [`MODEL_ITEMS`] items
    /// (the job spec itself stays with the host).
    pub fn to_model_value(&self) -> Value {
        let items: Vec<Value> =
            self.items.iter().take(MODEL_ITEMS).map(|i| serde_json::to_value(i).unwrap_or(Value::Null)).collect();
        json!({
            "tool": self.tool,
            "summary": self.summary,
            "risk": self.risk,
            "executable": self.executable,
            "total_bytes": self.total_bytes,
            "total_entries": self.total_entries,
            "conflicts": self.conflicts,
            "warnings": self.warnings,
            "items": items,
            "items_truncated": self.items.len() > MODEL_ITEMS,
        })
    }
}

fn kind_name(k: &EntryKind) -> &'static str {
    match k {
        EntryKind::File => "file",
        EntryKind::Dir => "dir",
        EntryKind::Symlink { .. } => "link",
        EntryKind::Junction => "junction",
        _ => "other",
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// Sizes a tree and refuses it when it contains a blocked (sensitive) path.
pub fn tree_totals(scope: &Scope, root: &Path, ctl: &OpControl) -> Result<(u64, u64), ToolError> {
    let mut bytes = 0u64;
    let mut entries = 0u64;
    let mut blocked: Option<PathBuf> = None;
    let opts = WalkOptions::default();
    let r = soyuz_fs::walk(root, &opts, ctl, &mut |item| {
        if ctl.is_cancelled() {
            return Control::Stop;
        }
        if let WalkItem::Entry { entry, .. } = item {
            if scope.is_blocked(&entry.path) {
                blocked = Some(entry.path.clone());
                return Control::Stop;
            }
            entries += 1;
            if matches!(entry.kind, EntryKind::File) {
                bytes += entry.size;
            }
            if entries >= MAX_PLAN_WALK {
                return Control::Stop;
            }
        }
        Control::Continue
    });
    if ctl.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    if let Some(p) = blocked {
        return Err(ToolError::Scope(super::ScopeError::Sensitive(p)));
    }
    r.map_err(|e| ToolError::Failed(e.to_string()))?;
    Ok((bytes, entries))
}

fn conflict_label(c: ConflictArg) -> &'static str {
    match c {
        ConflictArg::Ask => "ask",
        ConflictArg::Skip => "skip",
        ConflictArg::KeepBoth => "keep_both",
        ConflictArg::Overwrite => "overwrite",
        ConflictArg::OverwriteIfNewer => "overwrite_if_newer",
    }
}

fn resolution(c: ConflictArg) -> Option<ConflictResolution> {
    match c {
        ConflictArg::Ask => None,
        ConflictArg::Skip => Some(ConflictResolution::Skip),
        ConflictArg::KeepBoth => Some(ConflictResolution::KeepBoth),
        ConflictArg::Overwrite => Some(ConflictResolution::Overwrite),
        ConflictArg::OverwriteIfNewer => Some(ConflictResolution::OverwriteIfNewer),
    }
}

fn labelled(kind: JobKind, label: String) -> JobSpec {
    let mut spec = JobSpec::new(kind);
    spec.options.label = Some(label);
    spec
}

fn exists_conflict(dst: &Path, resolution: &str) -> Option<PlanConflict> {
    soyuz_fs::stat(dst, false).ok().map(|e| PlanConflict::Exists {
        kind: kind_name(&e.kind).into(),
        size: e.size,
        resolution: resolution.into(),
    })
}

/// Copy / move plan.
pub fn transfer(
    scope: &Scope,
    is_move: bool,
    sources: &[PathBuf],
    dest_dir: &Path,
    conflict: ConflictArg,
    ctl: &OpControl,
) -> Result<Plan, ToolError> {
    let (tool, op) = if is_move { ("move", PlanOp::Move) } else { ("copy", PlanOp::Copy) };
    if sources.is_empty() {
        return Err(ToolError::InvalidArgs(vec!["no sources".into()]));
    }
    let mut warnings = Vec::new();
    match soyuz_fs::stat(dest_dir, true) {
        Ok(e) if e.is_dir_like() => {}
        Ok(_) => return Err(ToolError::Failed(format!("destination is not a folder: {}", dest_dir.display()))),
        Err(_) => warnings.push(format!("destination folder does not exist yet: {}", dest_dir.display())),
    }
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for src in sources {
        let name = src.file_name().map(PathBuf::from).unwrap_or_default();
        let dst = dest_dir.join(&name);
        let mut item = PlanItem::new(op, Some(src.clone()), Some(dst.clone()));
        match soyuz_fs::stat(src, false) {
            Err(_) => item.conflict = Some(PlanConflict::Missing),
            Ok(e) => {
                item.kind = kind_name(&e.kind).into();
                let (bytes, entries) =
                    if e.is_dir_like() && !matches!(e.kind, EntryKind::Junction | EntryKind::Symlink { .. }) {
                        tree_totals(scope, src, ctl)?
                    } else {
                        (e.size, 1)
                    };
                item.bytes = bytes;
                item.entries = entries;
                if !seen.insert(name.to_string_lossy().to_lowercase()) {
                    item.conflict = Some(PlanConflict::DuplicateTarget);
                } else if soyuz_fs::path_overlaps(src, &dst) && src != &dst {
                    item.conflict = Some(PlanConflict::IntoSelf);
                } else if src == &dst || soyuz_fs::normalize(src) == soyuz_fs::normalize(&dst) {
                    item.conflict = if is_move {
                        Some(PlanConflict::SameItem)
                    } else {
                        // Copy into its own folder = duplicate with an auto name.
                        Some(PlanConflict::Exists {
                            kind: item.kind.clone(),
                            size: e.size,
                            resolution: "keep_both".into(),
                        })
                    };
                } else {
                    item.conflict = exists_conflict(&dst, conflict_label(conflict));
                }
            }
        }
        items.push(item);
    }
    let n = items.len();
    let kind = if is_move {
        JobKind::Move { sources: sources.to_vec(), dest_dir: dest_dir.to_path_buf(), rename_to: None }
    } else {
        JobKind::Copy { sources: sources.to_vec(), dest_dir: dest_dir.to_path_buf(), rename_to: None }
    };
    let verb = if is_move { "Move" } else { "Copy" };
    let summary = format!("{verb} {} into {}", plural(n, "item"), dest_dir.display());
    let mut spec = labelled(kind, format!("AI: {summary}"));
    let same_dir_copy = !is_move && items.iter().all(|i| i.src.as_ref().and_then(|s| s.parent()) == Some(dest_dir));
    spec.options.conflict = if same_dir_copy { Some(ConflictResolution::KeepBoth) } else { resolution(conflict) };
    let mut plan = Plan::finish(tool, Risk::Write, summary, items, PlanAction::Job(spec));
    plan.warnings = warnings;
    Ok(plan)
}

fn check_new_name(name: &str) -> Option<PlanConflict> {
    if name.contains(['/', '\\']) {
        return Some(PlanConflict::InvalidName { reason: "names cannot contain path separators".into() });
    }
    soyuz_core::validate_name(name).err().map(|e| PlanConflict::InvalidName { reason: e.to_string() })
}

/// Rename plan from `(path, new_name)` pairs.
pub fn rename(tool: &str, pairs: &[(PathBuf, String)]) -> Result<Plan, ToolError> {
    if pairs.is_empty() {
        return Err(ToolError::InvalidArgs(vec!["nothing to rename".into()]));
    }
    let sources: HashSet<String> = pairs.iter().map(|(p, _)| key(p)).collect();
    let mut targets: HashMap<String, usize> = HashMap::new();
    let mut items = Vec::new();
    let mut job_pairs = Vec::new();
    for (path, new_name) in pairs {
        let Some(parent) = path.parent() else {
            return Err(ToolError::InvalidArgs(vec![format!("cannot rename {}", path.display())]));
        };
        let dst = parent.join(new_name);
        let mut item = PlanItem::new(PlanOp::Rename, Some(path.clone()), Some(dst.clone()));
        match soyuz_fs::stat(path, false) {
            Err(_) => item.conflict = Some(PlanConflict::Missing),
            Ok(e) => {
                item.kind = kind_name(&e.kind).into();
                item.bytes = if matches!(e.kind, EntryKind::File) { e.size } else { 0 };
                item.entries = 1;
                let n = targets.entry(key(&dst)).or_insert(0);
                *n += 1;
                item.conflict = if let Some(c) = check_new_name(new_name) {
                    Some(c)
                } else if path.file_name().map(|f| f.to_string_lossy() == new_name.as_str()).unwrap_or(false) {
                    Some(PlanConflict::SameItem)
                } else if *n > 1 {
                    Some(PlanConflict::DuplicateTarget)
                } else if soyuz_fs::exists_nofollow(&dst)
                    && !sources.contains(&key(&dst))
                    && !soyuz_fs::is_case_variant(path, &dst)
                {
                    // Renaming never overwrites; a target freed by another rename in this batch is fine.
                    exists_conflict(&dst, "blocked")
                } else {
                    None
                };
            }
        }
        if item.conflict.is_none() {
            job_pairs.push((path.clone(), dst));
        }
        items.push(item);
    }
    let n = job_pairs.len();
    let summary = if pairs.len() == 1 {
        format!("Rename {} to {}", pairs[0].0.display(), pairs[0].1)
    } else {
        format!("Rename {}", plural(n, "item"))
    };
    let action = if job_pairs.is_empty() {
        PlanAction::None
    } else {
        PlanAction::Job(labelled(JobKind::Rename { pairs: job_pairs }, format!("AI: {summary}")))
    };
    Ok(Plan::finish(tool, Risk::Write, summary, items, action))
}

fn key(p: &Path) -> String {
    let s = soyuz_fs::normalize(p).to_string_lossy().to_string();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

/// Apply a [`RenameRule`] to a file name.
pub fn apply_rule(name: &str, rule: &RenameRule) -> Result<String, ToolError> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if rule.keep_extension && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut s = stem.to_string();
    if !rule.find.is_empty() {
        s = if rule.regex {
            let re = regex_lite(&rule.find)?;
            re.replace_all(&s, rule.replace.as_str()).into_owned()
        } else {
            s.replace(&rule.find, &rule.replace)
        };
    }
    Ok(format!("{}{s}{}{ext}", rule.prefix, rule.suffix))
}

fn regex_lite(pat: &str) -> Result<regex::Regex, ToolError> {
    regex::RegexBuilder::new(pat)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| ToolError::InvalidArgs(vec![format!("bad regex: {e}")]))
}

/// Batch rename (pairs or paths + rule).
pub fn batch_rename(a: &BatchRenameArgs) -> Result<Plan, ToolError> {
    let mut pairs: Vec<(PathBuf, String)> = a.pairs.iter().map(|p| (p.path.clone(), p.new_name.clone())).collect();
    if !a.paths.is_empty() {
        let rule = a.rule.as_ref().ok_or_else(|| ToolError::InvalidArgs(vec!["`paths` needs a `rule`".into()]))?;
        for p in &a.paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            pairs.push((p.clone(), apply_rule(&name, rule)?));
        }
    }
    rename("batch_rename", &pairs)
}

/// mkdir / create_file plan.
pub fn create(path: &Path, dir: bool) -> Result<Plan, ToolError> {
    let (tool, op) = if dir { ("mkdir", PlanOp::Mkdir) } else { ("create_file", PlanOp::Create) };
    let mut item = PlanItem::new(op, None, Some(path.to_path_buf()));
    item.kind = if dir { "dir" } else { "file" }.into();
    item.entries = 1;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    item.conflict = check_new_name(&name).or_else(|| exists_conflict(path, "blocked"));
    let kind = if dir { CreateKind::Dir } else { CreateKind::File { template: None } };
    let summary = format!("Create {} {}", if dir { "folder" } else { "file" }, path.display());
    let spec = labelled(JobKind::Create { path: path.to_path_buf(), kind }, format!("AI: {summary}"));
    Ok(Plan::finish(tool, Risk::Write, summary, vec![item], PlanAction::Job(spec)))
}

fn simple_items(
    op: PlanOp,
    paths: &[PathBuf],
    scope: &Scope,
    ctl: &OpControl,
    deep: bool,
) -> Result<Vec<PlanItem>, ToolError> {
    let mut items = Vec::new();
    for p in paths {
        let mut item = PlanItem::new(op, Some(p.clone()), None);
        match soyuz_fs::stat(p, false) {
            Err(_) => item.conflict = Some(PlanConflict::Missing),
            Ok(e) => {
                item.kind = kind_name(&e.kind).into();
                (item.bytes, item.entries) =
                    if deep && matches!(e.kind, EntryKind::Dir) { tree_totals(scope, p, ctl)? } else { (e.size, 1) };
            }
        }
        items.push(item);
    }
    Ok(items)
}

/// trash / delete_permanent plan.
pub fn delete(scope: &Scope, paths: &[PathBuf], permanent: bool, ctl: &OpControl) -> Result<Plan, ToolError> {
    if paths.is_empty() {
        return Err(ToolError::InvalidArgs(vec!["no paths".into()]));
    }
    let (tool, op, mode, verb) = if permanent {
        ("delete_permanent", PlanOp::Delete, DeleteMode::Permanent, "Permanently delete")
    } else {
        ("trash", PlanOp::Trash, DeleteMode::Trash, "Move to trash")
    };
    let items = simple_items(op, paths, scope, ctl, true)?;
    let summary = format!("{verb} {}", plural(items.len(), "item"));
    let spec = labelled(JobKind::Delete { paths: paths.to_vec(), mode }, format!("AI: {summary}"));
    let mut plan = Plan::finish(tool, Risk::Destructive, summary, items, PlanAction::Job(spec));
    if permanent {
        plan.warnings.push("Permanent deletion cannot be undone.".into());
    }
    Ok(plan)
}

fn parse_time(s: &str) -> Result<std::time::SystemTime, ToolError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(std::time::SystemTime::from)
        .map_err(|e| ToolError::InvalidArgs(vec![format!("bad date-time “{s}”: {e}")]))
}

/// set_attributes plan.
pub fn set_attributes(scope: &Scope, a: &SetAttributesArgs, ctl: &OpControl) -> Result<Plan, ToolError> {
    let mut set = Attributes::empty();
    let mut clear = Attributes::empty();
    for (flag, v) in [(Attributes::READONLY, a.readonly), (Attributes::HIDDEN, a.hidden)] {
        match v {
            Some(true) => set |= flag,
            Some(false) => clear |= flag,
            None => {}
        }
    }
    let attrs = (!(set | clear).is_empty()).then_some(AttrSet { set, clear });
    let times = a.modified.as_deref().map(parse_time).transpose()?.map(|m| TimesSet {
        modified: Some(m),
        created: None,
        accessed: None,
    });
    if attrs.is_none() && times.is_none() && a.mode.is_none() {
        return Err(ToolError::InvalidArgs(vec!["nothing to change".into()]));
    }
    let items = simple_items(PlanOp::SetMeta, &a.paths, scope, ctl, false)?;
    let summary = format!("Change attributes of {}", plural(items.len(), "item"));
    let spec =
        labelled(JobKind::SetMeta { paths: a.paths.clone(), times, attrs, mode: a.mode }, format!("AI: {summary}"));
    Ok(Plan::finish("set_attributes", Risk::Write, summary, items, PlanAction::Job(spec)))
}

/// extract plan: lists the archive and flags top-level names that already exist.
pub fn extract(a: &ExtractArgs) -> Result<Plan, ToolError> {
    let mut reader = soyuz_archive::open(&a.archive, None).map_err(|e| ToolError::Failed(e.to_string()))?;
    let entries = reader.entries().map_err(|e| ToolError::Failed(e.to_string()))?;
    let mut tops: Vec<(String, bool, u64, u64)> = Vec::new();
    for e in entries {
        let top = e.path.split('/').next().unwrap_or_default().to_string();
        if top.is_empty() {
            continue;
        }
        match tops.iter_mut().find(|t| t.0 == top) {
            Some(t) => {
                t.2 += e.size;
                t.3 += 1;
                t.1 = true;
            }
            None => tops.push((top.clone(), e.is_dir || e.path.contains('/'), e.size, 1)),
        }
    }
    let items: Vec<PlanItem> = tops
        .into_iter()
        .map(|(name, dir, bytes, n)| {
            let dst = a.dest_dir.join(&name);
            let mut it = PlanItem::new(PlanOp::Extract, None, Some(dst.clone()));
            it.kind = if dir { "dir" } else { "file" }.into();
            it.bytes = bytes;
            it.entries = n;
            it.conflict = exists_conflict(&dst, "ask");
            it
        })
        .collect();
    let summary = format!("Extract {} into {}", a.archive.display(), a.dest_dir.display());
    let spec = labelled(
        JobKind::Extract {
            archive: a.archive.clone(),
            entries: None,
            dest_dir: a.dest_dir.clone(),
            password: None,
            strip_components: 0,
        },
        format!("AI: {summary}"),
    );
    Ok(Plan::finish("extract", Risk::Write, summary, items, PlanAction::Job(spec)))
}

fn archive_format(f: ArchiveFormatArg) -> soyuz_archive::ArchiveFormat {
    use soyuz_archive::ArchiveFormat as F;
    match f {
        ArchiveFormatArg::Zip => F::Zip,
        ArchiveFormatArg::Tar => F::Tar,
        ArchiveFormatArg::TarGz => F::TarGz,
        ArchiveFormatArg::TarBz2 => F::TarBz2,
        ArchiveFormatArg::TarXz => F::TarXz,
        ArchiveFormatArg::TarZst => F::TarZst,
        ArchiveFormatArg::SevenZ => F::SevenZ,
    }
}

/// compress plan.
pub fn compress(scope: &Scope, a: &CompressArgs, ctl: &OpControl) -> Result<Plan, ToolError> {
    let mut items = simple_items(PlanOp::Compress, &a.sources, scope, ctl, true)?;
    let mut out = PlanItem::new(PlanOp::Create, None, Some(a.dest.clone()));
    out.kind = "file".into();
    out.conflict = exists_conflict(&a.dest, "blocked");
    items.push(out);
    let summary = format!("Compress {} into {}", plural(a.sources.len(), "item"), a.dest.display());
    let spec = labelled(
        JobKind::Compress {
            sources: a.sources.clone(),
            dest: a.dest.clone(),
            format: archive_format(a.format),
            level: a.level.unwrap_or(6).min(9),
            password: None,
        },
        format!("AI: {summary}"),
    );
    Ok(Plan::finish("compress", Risk::Write, summary, items, PlanAction::Job(spec)))
}

/// restore_from_trash plan.
pub fn restore(scope: &Scope, a: &RestoreArgs) -> Result<Plan, ToolError> {
    let all = soyuz_fs::trash::list().map_err(|e| ToolError::Failed(e.to_string()))?;
    let mut items = Vec::new();
    let mut chosen = Vec::new();
    for id in &a.ids {
        let Some(t) = all.iter().find(|t| &t.id == id) else {
            let mut it = PlanItem::new(PlanOp::Restore, None, None);
            it.kind = format!("unknown trash id {id}");
            it.conflict = Some(PlanConflict::Missing);
            items.push(it);
            continue;
        };
        let dst = match &a.to {
            Some(dir) => dir.join(t.original.file_name().unwrap_or_default()),
            None => t.original.clone(),
        };
        if a.to.is_none() {
            scope.check_write(&dst)?;
        }
        let mut it = PlanItem::new(PlanOp::Restore, None, Some(dst.clone()));
        it.kind = kind_name(&t.kind).into();
        it.bytes = t.size;
        it.entries = 1;
        it.conflict = exists_conflict(&dst, "ask");
        items.push(it);
        chosen.push(t.clone());
    }
    let summary = format!("Restore {} from the trash", plural(chosen.len(), "item"));
    let action = if chosen.is_empty() {
        PlanAction::None
    } else {
        PlanAction::Job(labelled(JobKind::RestoreTrash { items: chosen, to: a.to.clone() }, format!("AI: {summary}")))
    };
    Ok(Plan::finish("restore_from_trash", Risk::Write, summary, items, action))
}

/// empty_trash plan.
pub fn empty_trash() -> Result<Plan, ToolError> {
    let all = soyuz_fs::trash::list().map_err(|e| ToolError::Failed(e.to_string()))?;
    let items: Vec<PlanItem> = all
        .iter()
        .map(|t| {
            let mut it = PlanItem::new(PlanOp::EmptyTrash, Some(t.original.clone()), None);
            it.kind = kind_name(&t.kind).into();
            it.bytes = t.size;
            it.entries = 1;
            it
        })
        .collect();
    let summary = format!("Empty the trash ({})", plural(items.len(), "item"));
    let action = if items.is_empty() {
        PlanAction::None
    } else {
        PlanAction::Job(labelled(JobKind::EmptyTrash, "AI: empty trash".into()))
    };
    let mut plan = Plan::finish("empty_trash", Risk::Destructive, summary, items, action);
    plan.warnings.push("Emptying the trash cannot be undone.".into());
    Ok(plan)
}

/// sync_apply plan from a folder comparison.
pub fn sync(scope: &Scope, a: &SyncArgs, ctl: &OpControl) -> Result<Plan, ToolError> {
    let cmp = soyuz_search::compare_dirs(&a.left, &a.right, a.by_content, ctl)
        .map_err(|e| ToolError::Failed(e.to_string()))?;
    let mut actions = Vec::new();
    match a.direction {
        SyncDirection::LeftToRight => {
            actions.extend(
                cmp.only_left.iter().chain(&cmp.newer_left).chain(&cmp.differ).cloned().map(SyncAction::CopyLR),
            );
            actions.extend(cmp.newer_right.iter().cloned().map(SyncAction::CopyLR));
            if a.delete_extra {
                actions.extend(cmp.only_right.iter().cloned().map(SyncAction::DeleteR));
            }
        }
        SyncDirection::RightToLeft => {
            actions.extend(
                cmp.only_right.iter().chain(&cmp.newer_right).chain(&cmp.differ).cloned().map(SyncAction::CopyRL),
            );
            actions.extend(cmp.newer_left.iter().cloned().map(SyncAction::CopyRL));
            if a.delete_extra {
                actions.extend(cmp.only_left.iter().cloned().map(SyncAction::DeleteL));
            }
        }
        SyncDirection::Both => {
            actions.extend(cmp.only_left.iter().chain(&cmp.newer_left).cloned().map(SyncAction::CopyLR));
            actions.extend(cmp.only_right.iter().chain(&cmp.newer_right).cloned().map(SyncAction::CopyRL));
        }
    }
    let mut items = Vec::new();
    for act in &actions {
        let (op, src, dst) = match act {
            SyncAction::CopyLR(r) => (PlanOp::SyncCopy, Some(a.left.join(r)), Some(a.right.join(r))),
            SyncAction::CopyRL(r) => (PlanOp::SyncCopy, Some(a.right.join(r)), Some(a.left.join(r))),
            SyncAction::DeleteL(r) => (PlanOp::SyncDelete, Some(a.left.join(r)), None),
            SyncAction::DeleteR(r) => (PlanOp::SyncDelete, Some(a.right.join(r)), None),
        };
        for p in src.iter().chain(dst.iter()) {
            if scope.is_blocked(p) {
                return Err(ToolError::Scope(super::ScopeError::Sensitive(p.clone())));
            }
        }
        let mut it = PlanItem::new(op, src.clone(), dst);
        if let Some(e) = src.as_deref().and_then(|s| soyuz_fs::stat(s, false).ok()) {
            it.kind = kind_name(&e.kind).into();
            it.bytes = if matches!(e.kind, EntryKind::File) { e.size } else { 0 };
        }
        it.entries = 1;
        items.push(it);
    }
    let deletes = items.iter().filter(|i| i.op == PlanOp::SyncDelete).count();
    let summary = format!(
        "Sync {} ⇄ {}: {} to copy, {} to delete",
        a.left.display(),
        a.right.display(),
        items.len() - deletes,
        deletes
    );
    let action = if actions.is_empty() {
        PlanAction::None
    } else {
        PlanAction::Job(labelled(
            JobKind::Sync { plan: SyncPlan { left: a.left.clone(), right: a.right.clone(), actions } },
            "AI: sync folders".into(),
        ))
    };
    Ok(Plan::finish("sync_apply", Risk::Destructive, summary, items, action))
}

/// Plans executed by the app rather than the jobs engine (tags, undo, custom commands).
pub fn app_action(tool: &str, risk: Risk, summary: String, op: PlanOp, paths: &[PathBuf], args: Value) -> Plan {
    let items = if paths.is_empty() {
        vec![PlanItem::new(op, None, None)]
    } else {
        paths.iter().map(|p| PlanItem { entries: 1, ..PlanItem::new(op, Some(p.clone()), None) }).collect()
    };
    Plan::finish(tool, risk, summary, items, PlanAction::App { op: tool.to_string(), args })
}
