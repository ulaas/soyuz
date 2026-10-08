//! [`FsToolHost`]: read tools straight on soyuz-fs / soyuz-search / soyuz-archive, and
//! dry-run plans for write tools. It never changes the disk: `execute` is left to hosts
//! that own a `JobEngine` (the app, the MCP server).

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use soyuz_core::config::AiScope;
use soyuz_core::{EntryInfo, EntryKind, EntryKindSet, HashAlgo, NamePatternSpec, PatternKind};
use soyuz_search::{DupeEvent, SearchEvent, SearchQuery, SizeEvent};

use super::args::*;
use super::plan::{self, Plan, PlanOp};
use super::scope::Scope;
use super::{CallContext, Risk, ToolError, ToolHost, ToolInput};

/// Wall-clock cap for one read tool.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub struct FsToolHost {
    pub scope: Scope,
    /// `read_text` cap in bytes.
    pub max_read_bytes: u64,
    /// Search threads (0 = auto).
    pub search_threads: usize,
}

impl FsToolHost {
    pub fn new(scope: Scope) -> Self {
        FsToolHost { scope, max_read_bytes: 256 * 1024, search_threads: 0 }
    }

    pub fn from_config(cfg: &AiScope) -> Self {
        FsToolHost { max_read_bytes: cfg.max_read_kib.max(1) * 1024, ..Self::new(Scope::from_config(cfg)) }
    }
}

fn time(t: Option<SystemTime>) -> Value {
    t.map(|t| json!(soyuz_core::time::serde_rfc3339::to_string(&t))).unwrap_or(Value::Null)
}

fn kind(k: &EntryKind) -> &'static str {
    match k {
        EntryKind::File => "file",
        EntryKind::Dir => "dir",
        EntryKind::Symlink { .. } => "link",
        EntryKind::Junction => "junction",
        _ => "other",
    }
}

/// Model-facing entry (metadata only).
pub fn entry_json(e: &EntryInfo) -> Value {
    json!({
        "path": e.path,
        "name": e.name,
        "kind": kind(&e.kind),
        "size": e.size,
        "modified": time(e.modified),
        "hidden": e.attrs.contains(soyuz_core::Attributes::HIDDEN),
    })
}

fn failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(e.to_string())
}

fn hash_algo(a: HashAlgoArg) -> HashAlgo {
    match a {
        HashAlgoArg::Blake3 => HashAlgo::Blake3,
        HashAlgoArg::Sha256 => HashAlgo::Sha256,
        HashAlgoArg::Sha1 => HashAlgo::Sha1,
        HashAlgoArg::Md5 => HashAlgo::Md5,
        HashAlgoArg::Crc32 => HashAlgo::Crc32,
    }
}

impl FsToolHost {
    fn list_dir(&self, a: &ListDirArgs) -> Result<Value, ToolError> {
        let opts = soyuz_fs::ListOptions { include_hidden: a.include_hidden, ..Default::default() };
        let mut entries = soyuz_fs::read_dir(&a.path, &opts).map_err(failed)?;
        entries.retain(|e| !self.scope.is_blocked(&e.path));
        entries.sort_by(|x, y| {
            y.is_dir_like().cmp(&x.is_dir_like()).then_with(|| soyuz_core::natural_cmp(&x.name, &y.name, true))
        });
        let limit = a.limit.unwrap_or(500).min(5000) as usize;
        let total = entries.len();
        let list: Vec<Value> = entries.iter().take(limit).map(entry_json).collect();
        Ok(json!({"path": a.path, "total": total, "truncated": total > limit, "entries": list}))
    }

    fn search(&self, a: &SearchArgs, ctx: &CallContext) -> Result<Value, ToolError> {
        let max = a.max_results.unwrap_or(200).clamp(1, 2000) as usize;
        let name = a.name.as_ref().map(|n| {
            let kind = match a.name_match {
                NameMatch::Substring => PatternKind::Substring,
                NameMatch::Glob => PatternKind::Glob,
                NameMatch::Regex => PatternKind::Regex,
                NameMatch::Fuzzy => PatternKind::Fuzzy,
            };
            NamePatternSpec::new(kind, n)
        });
        let parse = |s: &Option<String>| -> Result<Option<SystemTime>, ToolError> {
            s.as_deref()
                .map(|t| {
                    chrono::DateTime::parse_from_rfc3339(t)
                        .map(SystemTime::from)
                        .map_err(|e| ToolError::InvalidArgs(vec![format!("bad date-time “{t}”: {e}")]))
                })
                .transpose()
        };
        let (after, before) = (parse(&a.modified_after)?, parse(&a.modified_before)?);
        let q = SearchQuery {
            roots: a.roots.clone(),
            name,
            content: a.content.as_ref().map(|c| soyuz_search::ContentPatternSpec {
                regex: a.content_regex,
                text: c.clone(),
                case_sensitive: false,
            }),
            size: (a.min_size.is_some() || a.max_size.is_some()).then_some((a.min_size, a.max_size)),
            modified: (after.is_some() || before.is_some()).then_some((after, before)),
            kinds: a.kind.and_then(|k| match k {
                KindFilter::File => Some(EntryKindSet::FILE),
                KindFilter::Dir => Some(EntryKindSet::DIR),
                KindFilter::Any => None,
            }),
            extensions: a.extensions.iter().map(|e| e.trim_start_matches('.').to_string()).collect(),
            max_depth: a.max_depth.map(|d| d as usize),
            include_hidden: a.include_hidden,
            max_results: max * 2,
            ..SearchQuery::default()
        };
        let handle = soyuz_search::start(q, self.search_threads, Arc::new(|| {})).map_err(failed)?;
        let deadline = Instant::now() + READ_TIMEOUT;
        let mut hits = Vec::new();
        let mut truncated = false;
        let mut scanned = 0;
        loop {
            if ctx.ctl.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                truncated = true;
                break;
            }
            match handle.rx.recv_timeout(left.min(Duration::from_millis(250))) {
                Ok(SearchEvent::Hits(batch)) => {
                    for (e, matches) in batch {
                        if self.scope.is_blocked(&e.path) {
                            continue;
                        }
                        let mut v = entry_json(&e);
                        if !matches.is_empty() {
                            v["matches"] = matches
                                .iter()
                                .take(5)
                                .map(|m| json!({"line": m.line, "text": m.text}))
                                .collect::<Vec<_>>()
                                .into();
                        }
                        hits.push(v);
                    }
                    if hits.len() >= max {
                        hits.truncate(max);
                        truncated = true;
                        break;
                    }
                }
                Ok(SearchEvent::Done(stats)) => {
                    scanned = stats.scanned;
                    truncated |= stats.truncated;
                    break;
                }
                Ok(_) => {}
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
        handle.cancel();
        Ok(json!({"hits": hits, "count": hits.len(), "truncated": truncated, "scanned": scanned}))
    }

    fn folder_size(&self, a: &PathsArg, ctx: &CallContext) -> Result<Value, ToolError> {
        let (rx, ctl) = soyuz_search::folder_sizes(a.paths.clone(), Arc::new(|| {}));
        let deadline = Instant::now() + READ_TIMEOUT;
        let mut out = Vec::new();
        while out.len() < a.paths.len() {
            if ctx.ctl.is_cancelled() || Instant::now() > deadline {
                ctl.cancel();
                return if ctx.ctl.is_cancelled() { Err(ToolError::Cancelled) } else { Err(failed("timed out")) };
            }
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(SizeEvent::Done { path, bytes, files, dirs, errors }) => {
                    out.push(json!({"path": path, "bytes": bytes, "files": files, "dirs": dirs, "errors": errors}))
                }
                Ok(_) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        }
        Ok(json!({"sizes": out}))
    }

    fn duplicates(&self, a: &DuplicatesArgs, ctx: &CallContext) -> Result<Value, ToolError> {
        let (rx, ctl) = soyuz_search::find_duplicates(a.roots.clone(), a.min_size.unwrap_or(1), Arc::new(|| {}));
        let deadline = Instant::now() + READ_TIMEOUT * 5;
        let mut groups = Vec::new();
        let mut wasted = 0;
        loop {
            if ctx.ctl.is_cancelled() {
                ctl.cancel();
                return Err(ToolError::Cancelled);
            }
            if Instant::now() > deadline {
                ctl.cancel();
                break;
            }
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(DupeEvent::Group(g)) => {
                    let files: Vec<Value> =
                        g.iter().filter(|e| !self.scope.is_blocked(&e.path)).map(entry_json).collect();
                    if files.len() > 1 && groups.len() < 200 {
                        groups.push(json!({"size": g.first().map(|e| e.size).unwrap_or(0), "files": files}));
                    }
                }
                Ok(DupeEvent::Done { wasted_bytes, .. }) => {
                    wasted = wasted_bytes;
                    break;
                }
                Ok(_) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        }
        Ok(json!({"groups": groups, "wasted_bytes": wasted}))
    }

    fn read_text(&self, a: &ReadTextArgs) -> Result<Value, ToolError> {
        let meta = soyuz_fs::stat(&a.path, true).map_err(failed)?;
        if meta.is_dir_like() {
            return Err(failed("not a file"));
        }
        use std::io::Read;
        let f = std::fs::File::open(&a.path).map_err(failed)?;
        let mut buf = Vec::new();
        f.take(self.max_read_bytes).read_to_end(&mut buf).map_err(failed)?;
        if buf.iter().take(8192).any(|&b| b == 0) {
            return Err(failed("binary file"));
        }
        let text = String::from_utf8_lossy(&buf);
        let start = a.start_line.unwrap_or(1).max(1);
        let end = a.end_line.unwrap_or(u64::MAX);
        let lines: Vec<&str> = text
            .lines()
            .enumerate()
            .filter(|(i, _)| (*i as u64 + 1) >= start && (*i as u64) < end)
            .map(|(_, l)| l)
            .collect();
        Ok(json!({
            "path": a.path,
            "size": meta.size,
            "truncated": meta.size > self.max_read_bytes,
            "start_line": start,
            "text": lines.join("\n"),
        }))
    }

    fn archive_list(&self, a: &ArchiveListArgs) -> Result<Value, ToolError> {
        let mut r = soyuz_archive::open(&a.path, None).map_err(failed)?;
        let entries = r.entries().map_err(failed)?;
        let limit = a.limit.unwrap_or(500).min(5000) as usize;
        let list: Vec<Value> = entries
            .iter()
            .take(limit)
            .map(|e| json!({"path": e.path, "dir": e.is_dir, "size": e.size, "modified": time(e.modified), "encrypted": e.encrypted}))
            .collect();
        Ok(json!({"total": entries.len(), "truncated": entries.len() > limit, "entries": list}))
    }

    fn trash_list(&self, a: &LimitArg) -> Result<Value, ToolError> {
        let mut items = soyuz_fs::trash::list().map_err(failed)?;
        items.sort_by_key(|t| std::cmp::Reverse(t.deleted_at));
        let limit = a.limit.unwrap_or(200).min(2000) as usize;
        let list: Vec<Value> = items
            .iter()
            .filter(|t| !self.scope.is_blocked(&t.original))
            .take(limit)
            .map(|t| {
                json!({"id": t.id, "original": t.original, "size": t.size, "kind": kind(&t.kind), "deleted_at": time(Some(t.deleted_at))})
            })
            .collect();
        Ok(json!({"total": items.len(), "items": list}))
    }
}

impl ToolHost for FsToolHost {
    fn scope(&self) -> &Scope {
        &self.scope
    }

    fn read(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        match input {
            ToolInput::ListDir(a) => self.list_dir(a),
            ToolInput::Stat(a) => soyuz_fs::stat(&a.path, false).map(|e| entry_json(&e)).map_err(failed),
            ToolInput::Search(a) => self.search(a, ctx),
            ToolInput::FolderSize(a) => self.folder_size(a, ctx),
            ToolInput::FindDuplicates(a) => self.duplicates(a, ctx),
            ToolInput::CompareDirs(a) => {
                let r = soyuz_search::compare_dirs(&a.left, &a.right, a.by_content, &ctx.ctl).map_err(failed)?;
                let cap = |v: &Vec<std::path::PathBuf>| v.iter().take(500).cloned().collect::<Vec<_>>();
                Ok(json!({
                    "only_left": cap(&r.only_left), "only_right": cap(&r.only_right),
                    "newer_left": cap(&r.newer_left), "newer_right": cap(&r.newer_right),
                    "differ": cap(&r.differ), "same": r.same,
                }))
            }
            ToolInput::ReadText(a) => self.read_text(a),
            ToolInput::Hash(a) => {
                let digest = soyuz_fs::hash_file(&a.path, hash_algo(a.algo), &ctx.ctl, &mut ()).map_err(failed)?;
                Ok(json!({"path": a.path, "algo": a.algo, "digest": digest}))
            }
            ToolInput::ArchiveList(a) => self.archive_list(a),
            ToolInput::TrashList(a) => self.trash_list(a),
            other => Err(ToolError::Unsupported(other.name().to_string())),
        }
    }

    fn plan(&self, input: &ToolInput, ctx: &CallContext) -> Result<Plan, ToolError> {
        let s = &self.scope;
        let ctl = &ctx.ctl;
        match input {
            ToolInput::Copy(a) => plan::transfer(s, false, &a.sources, &a.dest_dir, a.conflict, ctl),
            ToolInput::Move(a) => plan::transfer(s, true, &a.sources, &a.dest_dir, a.conflict, ctl),
            ToolInput::Rename(a) => plan::rename("rename", &[(a.path.clone(), a.new_name.clone())]),
            ToolInput::BatchRename(a) => {
                let p = plan::batch_rename(a)?;
                for item in &p.items {
                    if let Some(d) = &item.dst {
                        s.check_write(d)?;
                    }
                }
                Ok(p)
            }
            ToolInput::Mkdir(a) => plan::create(&a.path, true),
            ToolInput::CreateFile(a) => plan::create(&a.path, false),
            ToolInput::SetAttributes(a) => plan::set_attributes(s, a, ctl),
            ToolInput::Extract(a) => plan::extract(a),
            ToolInput::Compress(a) => plan::compress(s, a, ctl),
            ToolInput::RestoreFromTrash(a) => plan::restore(s, a),
            ToolInput::Trash(a) => plan::delete(s, &a.paths, false, ctl),
            ToolInput::DeletePermanent(a) => plan::delete(s, &a.paths, true, ctl),
            ToolInput::EmptyTrash(_) => plan::empty_trash(),
            ToolInput::SyncApply(a) => plan::sync(s, a, ctl),
            ToolInput::Tag(a) => Ok(plan::app_action(
                "tag",
                Risk::Write,
                format!("{} tags {:?} on {} item(s)", if a.remove { "Remove" } else { "Add" }, a.tags, a.paths.len()),
                PlanOp::Tag,
                &a.paths,
                serde_json::to_value(a).unwrap_or(Value::Null),
            )),
            ToolInput::Undo(a) => Ok(plan::app_action(
                "undo",
                Risk::Write,
                format!("Undo history entry {}", a.history_id),
                PlanOp::Undo,
                &[],
                json!({"history_id": a.history_id}),
            )),
            ToolInput::RunCustomCommand(a) => Ok(plan::app_action(
                "run_custom_command",
                Risk::Destructive,
                format!("Run custom command “{}”", a.name),
                PlanOp::RunCommand,
                &[],
                json!({"name": a.name}),
            )),
            other => Err(ToolError::Unsupported(other.name().to_string())),
        }
    }
}
