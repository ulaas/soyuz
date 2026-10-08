//! Job specifications, states and summaries.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use soyuz_archive::ArchiveFormat;
use soyuz_core::{ConflictResolution, ErrorAction, FsError, HashAlgo};
use soyuz_fs::{AttrSet, CopyOptions, DeleteOptions, LinkKind, MoveOptions, TimesSet, TrashItem};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct JobId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    /// Serialize jobs sharing a `(src_dev, dst_dev)` pair.
    #[default]
    PerDevice,
    Parallel,
    Serial,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteMode {
    Trash,
    Permanent,
    Wipe { passes: u8 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateKind {
    Dir,
    File { template: Option<PathBuf> },
    Symlink { target: PathBuf, kind: LinkKind },
    Hardlink { src: PathBuf },
    Junction { target: PathBuf },
}

/// One step of a folder sync (paths relative to the plan roots).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    CopyLR(PathBuf),
    CopyRL(PathBuf),
    DeleteL(PathBuf),
    DeleteR(PathBuf),
}

/// M12; produced by `soyuz_search::compare_dirs` + the app's sync dialog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncPlan {
    pub left: PathBuf,
    pub right: PathBuf,
    pub actions: Vec<SyncAction>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// `rename_to`: single source only. Duplicate = Copy into the same dir with conflict KeepBoth.
    Copy {
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        rename_to: Option<String>,
    },
    Move {
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        rename_to: Option<String>,
    },
    Delete {
        paths: Vec<PathBuf>,
        mode: DeleteMode,
    },
    /// Batch rename; two-phase (temp names) when the pair graph has cycles.
    Rename {
        pairs: Vec<(PathBuf, PathBuf)>,
    },
    Create {
        path: PathBuf,
        kind: CreateKind,
    },
    SetMeta {
        paths: Vec<PathBuf>,
        times: Option<TimesSet>,
        attrs: Option<AttrSet>,
        mode: Option<u32>,
    },
    Hash {
        paths: Vec<PathBuf>,
        algo: HashAlgo,
    },
    Extract {
        archive: PathBuf,
        /// `ArchiveEntry::index` values; `None` = everything.
        entries: Option<Vec<usize>>,
        dest_dir: PathBuf,
        password: Option<String>,
        strip_components: usize,
    },
    Compress {
        sources: Vec<PathBuf>,
        dest: PathBuf,
        format: ArchiveFormat,
        level: u8,
        password: Option<String>,
    },
    RestoreTrash {
        items: Vec<TrashItem>,
        to: Option<PathBuf>,
    },
    EmptyTrash,
    /// Permanently delete these trash items (the trash view's per-item "Delete forever").
    PurgeTrash {
        items: Vec<TrashItem>,
    },
    /// M12.
    Sync {
        plan: SyncPlan,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKindBrief {
    Copy,
    Move,
    Delete,
    Trash,
    Rename,
    Create,
    SetMeta,
    Hash,
    Extract,
    Compress,
    RestoreTrash,
    EmptyTrash,
    Sync,
    Undo,
}

impl JobKind {
    pub fn brief(&self) -> JobKindBrief {
        match self {
            JobKind::Copy { .. } => JobKindBrief::Copy,
            JobKind::Move { .. } => JobKindBrief::Move,
            JobKind::Delete { mode: DeleteMode::Trash, .. } => JobKindBrief::Trash,
            JobKind::Delete { .. } => JobKindBrief::Delete,
            JobKind::Rename { .. } => JobKindBrief::Rename,
            JobKind::Create { .. } => JobKindBrief::Create,
            JobKind::SetMeta { .. } => JobKindBrief::SetMeta,
            JobKind::Hash { .. } => JobKindBrief::Hash,
            JobKind::Extract { .. } => JobKindBrief::Extract,
            JobKind::Compress { .. } => JobKindBrief::Compress,
            JobKind::RestoreTrash { .. } => JobKindBrief::RestoreTrash,
            JobKind::EmptyTrash | JobKind::PurgeTrash { .. } => JobKindBrief::EmptyTrash,
            JobKind::Sync { .. } => JobKindBrief::Sync,
        }
    }

    /// Sources + destination, for overlap serialization (`soyuz_fs::path_overlaps`). Jobs
    /// that change the trash itself (restore, purge, empty) also hold [`TRASH_ROOT`], which
    /// serializes them with each other.
    pub fn touched_roots(&self) -> Vec<PathBuf> {
        match self {
            JobKind::Copy { sources, dest_dir, .. } | JobKind::Move { sources, dest_dir, .. } => {
                sources.iter().cloned().chain(std::iter::once(dest_dir.clone())).collect()
            }
            JobKind::Delete { paths, .. } | JobKind::SetMeta { paths, .. } | JobKind::Hash { paths, .. } => {
                paths.clone()
            }
            JobKind::Rename { pairs } => pairs.iter().flat_map(|(a, b)| [a.clone(), b.clone()]).collect(),
            JobKind::Create { path, .. } => vec![path.clone()],
            JobKind::Extract { archive, dest_dir, .. } => vec![archive.clone(), dest_dir.clone()],
            JobKind::Compress { sources, dest, .. } => {
                sources.iter().cloned().chain(std::iter::once(dest.clone())).collect()
            }
            JobKind::RestoreTrash { items, to } => items
                .iter()
                .map(|i| match to {
                    Some(t) => t.clone(),
                    None => i.original.clone(),
                })
                .chain(std::iter::once(PathBuf::from(TRASH_ROOT)))
                .collect(),
            JobKind::EmptyTrash | JobKind::PurgeTrash { .. } => vec![PathBuf::from(TRASH_ROOT)],
            JobKind::Sync { plan } => vec![plan.left.clone(), plan.right.clone()],
        }
    }
}

/// Pseudo root (not a filesystem path) held by every job that changes the trash itself.
pub const TRASH_ROOT: &str = "<soyuz:trash>";

/// A pseudo root such as [`TRASH_ROOT`]: never resolved on disk, and it overlaps only itself.
pub(crate) fn is_synthetic_root(p: &Path) -> bool {
    p.as_os_str() == TRASH_ROOT
}

/// Overlap of two touched roots (pseudo roots only overlap themselves).
pub(crate) fn roots_overlap(a: &Path, b: &Path) -> bool {
    match (is_synthetic_root(a), is_synthetic_root(b)) {
        (false, false) => soyuz_fs::path_overlaps(a, b),
        (true, true) => a == b,
        _ => false,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JobOptions {
    pub priority: Priority,
    pub queue: QueueMode,
    pub start_paused: bool,
    /// Pre-answered conflict resolution.
    pub conflict: Option<ConflictResolution>,
    pub errors: Option<ErrorAction>,
    pub copy: CopyOptions,
    pub delete: DeleteOptions,
    pub mv: MoveOptions,
    /// Record for undo.
    pub journal: bool,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    pub kind: JobKind,
    pub options: JobOptions,
}

impl JobSpec {
    /// Spec with default options and journaling on.
    pub fn new(kind: JobKind) -> Self {
        JobSpec { kind, options: JobOptions { journal: true, ..JobOptions::default() } }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Planning,
    Running,
    Paused,
    WaitingForUser,
    Finishing,
    Done(JobSummary),
    Failed(FsError),
    Cancelled,
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobState::Done(_) | JobState::Failed(_) | JobState::Cancelled)
    }

    /// `Planning | Running | Finishing` (the states counted against `max_parallel`).
    pub fn is_active(&self) -> bool {
        matches!(self, JobState::Planning | JobState::Running | JobState::Finishing)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JobSummary {
    pub items_ok: u64,
    pub items_skipped: u64,
    pub items_failed: u64,
    pub bytes: u64,
    pub elapsed: Duration,
    /// Capped at [`MAX_SUMMARY_ERRORS`].
    pub errors: Vec<(PathBuf, FsError)>,
    /// E.g. "owner not preserved (no SeRestorePrivilege)".
    pub notes: Vec<String>,
}

/// Cap for [`JobSummary::errors`].
pub const MAX_SUMMARY_ERRORS: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    /// Since job start.
    pub at: Duration,
    pub level: LogLevel,
    pub path: Option<PathBuf>,
    pub msg: String,
}

/// Human label used when `JobOptions::label` is `None` (e.g. `Copy 3 items to “dst”`).
pub(crate) fn default_label(kind: &JobKind) -> String {
    fn name(p: &std::path::Path) -> String {
        format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(p))
    }
    fn items(paths: &[PathBuf]) -> String {
        match paths {
            [one] => name(one),
            many => format!("{} items", many.len()),
        }
    }
    match kind {
        JobKind::Copy { sources, dest_dir, .. } => format!("Copy {} to {}", items(sources), name(dest_dir)),
        JobKind::Move { sources, dest_dir, .. } => format!("Move {} to {}", items(sources), name(dest_dir)),
        JobKind::Delete { paths, mode: DeleteMode::Trash } => format!("Move {} to trash", items(paths)),
        JobKind::Delete { paths, mode: DeleteMode::Permanent } => format!("Delete {}", items(paths)),
        JobKind::Delete { paths, mode: DeleteMode::Wipe { .. } } => format!("Wipe {}", items(paths)),
        JobKind::Rename { pairs } => match pairs.as_slice() {
            [(a, b)] => format!("Rename {} to {}", name(a), name(b)),
            many => format!("Rename {} items", many.len()),
        },
        JobKind::Create { path, kind: CreateKind::Dir } => format!("New folder {}", name(path)),
        JobKind::Create { path, .. } => format!("Create {}", name(path)),
        JobKind::SetMeta { paths, .. } => format!("Change properties of {}", items(paths)),
        JobKind::Hash { paths, algo } => format!("{} checksum of {}", algo.label(), items(paths)),
        JobKind::Extract { archive, dest_dir, .. } => format!("Extract {} to {}", name(archive), name(dest_dir)),
        JobKind::Compress { sources, dest, .. } => format!("Compress {} into {}", items(sources), name(dest)),
        JobKind::RestoreTrash { items: it, .. } => match it.as_slice() {
            [one] => format!("Restore {}", name(&one.original)),
            many => format!("Restore {} items", many.len()),
        },
        JobKind::EmptyTrash => "Empty trash".to_string(),
        JobKind::PurgeTrash { items: it } => match it.as_slice() {
            [one] => format!("Delete {} forever", name(&one.original)),
            many => format!("Delete {} items forever", many.len()),
        },
        JobKind::Sync { plan } => format!("Sync {} with {}", name(&plan.left), name(&plan.right)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_and_roots() {
        let k = JobKind::Copy { sources: vec!["/a".into(), "/b".into()], dest_dir: "/d".into(), rename_to: None };
        assert_eq!(k.brief(), JobKindBrief::Copy);
        assert_eq!(k.touched_roots(), vec![PathBuf::from("/a"), "/b".into(), "/d".into()]);
        let t = JobKind::Delete { paths: vec!["/x".into()], mode: DeleteMode::Trash };
        assert_eq!(t.brief(), JobKindBrief::Trash);
        assert!(JobState::Cancelled.is_terminal());
        assert!(JobState::Running.is_active());
        assert!(!JobState::Paused.is_active());
        assert!(!JobState::WaitingForUser.is_active());
        // Trash-changing jobs share the pseudo root; it overlaps nothing on disk.
        assert_eq!(JobKind::EmptyTrash.touched_roots(), vec![PathBuf::from(TRASH_ROOT)]);
        let t = PathBuf::from(TRASH_ROOT);
        assert!(roots_overlap(&t, &t));
        assert!(!roots_overlap(&t, Path::new("/")));
        assert!(!roots_overlap(&std::env::current_dir().unwrap(), &t));
    }

    #[test]
    fn spec_serde_round_trip() {
        let spec = JobSpec::new(JobKind::Rename { pairs: vec![("/a".into(), "/b".into())] });
        let json = serde_json::to_string(&spec).unwrap();
        let back: JobSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, spec.kind);
        assert!(back.options.journal);
    }
}
