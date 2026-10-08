//! Typed tool arguments. Each struct derives a JSON Schema (schemars) that is sent to the
//! model; doc comments become the field descriptions.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Paths an argument set touches, for scope enforcement.
pub trait ToolArgs {
    /// Paths that are read.
    fn read_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    /// Paths that are created or changed.
    fn write_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

fn yes() -> bool {
    true
}

// ---------------------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}
impl ToolArgs for NoArgs {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathArg {
    /// Absolute path.
    pub path: PathBuf,
}
impl ToolArgs for PathArg {
    fn read_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDirArgs {
    /// Absolute folder path.
    pub path: PathBuf,
    /// Include hidden files (default false).
    #[serde(default)]
    pub include_hidden: bool,
    /// Maximum entries to return (default 500, max 5000).
    #[serde(default)]
    pub limit: Option<u32>,
}
impl ToolArgs for ListDirArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum NameMatch {
    /// Case-insensitive substring.
    #[default]
    Substring,
    Glob,
    Regex,
    Fuzzy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KindFilter {
    File,
    Dir,
    Any,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    /// Folders to search (absolute).
    pub roots: Vec<PathBuf>,
    /// Name pattern.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub name_match: NameMatch,
    /// Text to find inside files (requires content access in this folder's AI scope).
    #[serde(default)]
    pub content: Option<String>,
    /// Treat `content` as a regular expression.
    #[serde(default)]
    pub content_regex: bool,
    /// File extensions without the dot.
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub min_size: Option<u64>,
    #[serde(default)]
    pub max_size: Option<u64>,
    /// RFC 3339 date-time.
    #[serde(default)]
    pub modified_after: Option<String>,
    /// RFC 3339 date-time.
    #[serde(default)]
    pub modified_before: Option<String>,
    #[serde(default)]
    pub kind: Option<KindFilter>,
    #[serde(default)]
    pub max_depth: Option<u32>,
    #[serde(default)]
    pub include_hidden: bool,
    /// Maximum hits (default 200, max 2000).
    #[serde(default)]
    pub max_results: Option<u32>,
}
impl ToolArgs for SearchArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathsArg {
    /// Absolute paths.
    pub paths: Vec<PathBuf>,
}
impl ToolArgs for PathsArg {
    fn read_paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicatesArgs {
    pub roots: Vec<PathBuf>,
    /// Ignore files smaller than this many bytes (default 1).
    #[serde(default)]
    pub min_size: Option<u64>,
}
impl ToolArgs for DuplicatesArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareArgs {
    pub left: PathBuf,
    pub right: PathBuf,
    /// Compare file contents, not just size and date.
    #[serde(default)]
    pub by_content: bool,
}
impl ToolArgs for CompareArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        vec![self.left.clone(), self.right.clone()]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadTextArgs {
    pub path: PathBuf,
    /// First line, 1-based (default 1).
    #[serde(default)]
    pub start_line: Option<u64>,
    /// Last line, inclusive.
    #[serde(default)]
    pub end_line: Option<u64>,
}
impl ToolArgs for ReadTextArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum HashAlgoArg {
    #[default]
    Blake3,
    Sha256,
    Sha1,
    Md5,
    Crc32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HashArgs {
    pub path: PathBuf,
    #[serde(default)]
    pub algo: HashAlgoArg,
}
impl ToolArgs for HashArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LimitArg {
    /// Maximum items (default 200).
    #[serde(default)]
    pub limit: Option<u32>,
}
impl ToolArgs for LimitArg {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveListArgs {
    pub path: PathBuf,
    #[serde(default)]
    pub limit: Option<u32>,
}
impl ToolArgs for ArchiveListArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}

// ---------------------------------------------------------------------------------------
// Write (jobs)
// ---------------------------------------------------------------------------------------

/// How to resolve a name clash at the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictArg {
    /// Ask the user when the job runs.
    #[default]
    Ask,
    Skip,
    /// Keep both (auto-rename the new item).
    KeepBoth,
    Overwrite,
    OverwriteIfNewer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferArgs {
    pub sources: Vec<PathBuf>,
    /// Destination folder.
    pub dest_dir: PathBuf,
    #[serde(default)]
    pub conflict: ConflictArg,
    /// Only return the plan, do not request execution.
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for TransferArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        self.sources.clone()
    }
    fn write_paths(&self) -> Vec<PathBuf> {
        vec![self.dest_dir.clone()]
    }
}

/// Move needs write access to the sources too (they disappear).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    pub sources: Vec<PathBuf>,
    pub dest_dir: PathBuf,
    #[serde(default)]
    pub conflict: ConflictArg,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for MoveArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        self.sources.iter().cloned().chain(std::iter::once(self.dest_dir.clone())).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameArgs {
    pub path: PathBuf,
    /// New file name (no separators).
    pub new_name: String,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for RenameArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenamePair {
    pub path: PathBuf,
    pub new_name: String,
}

/// A simple rename rule applied to every path's file name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameRule {
    #[serde(default)]
    pub find: String,
    #[serde(default)]
    pub replace: String,
    /// `find` is a regular expression (`$1` style captures in `replace`).
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    /// Apply find/replace and suffix to the stem only, keeping the extension (default true).
    #[serde(default = "yes")]
    pub keep_extension: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchRenameArgs {
    /// Explicit renames.
    #[serde(default)]
    pub pairs: Vec<RenamePair>,
    /// Or: paths plus a rule.
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub rule: Option<RenameRule>,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for BatchRenameArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        self.pairs.iter().map(|p| p.path.clone()).chain(self.paths.iter().cloned()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    /// Absolute path of the new folder / file.
    pub path: PathBuf,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for CreateArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TagArgs {
    pub paths: Vec<PathBuf>,
    pub tags: Vec<String>,
    /// Remove these tags instead of adding them.
    #[serde(default)]
    pub remove: bool,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for TagArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAttributesArgs {
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub readonly: Option<bool>,
    #[serde(default)]
    pub hidden: Option<bool>,
    /// RFC 3339 modification time.
    #[serde(default)]
    pub modified: Option<String>,
    /// Unix permission bits (e.g. 420 for 0o644); ignored on Windows.
    #[serde(default)]
    pub mode: Option<u32>,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for SetAttributesArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractArgs {
    pub archive: PathBuf,
    pub dest_dir: PathBuf,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for ExtractArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        vec![self.archive.clone()]
    }
    fn write_paths(&self) -> Vec<PathBuf> {
        vec![self.dest_dir.clone()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormatArg {
    #[default]
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    SevenZ,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompressArgs {
    pub sources: Vec<PathBuf>,
    /// Archive file to create.
    pub dest: PathBuf,
    #[serde(default)]
    pub format: ArchiveFormatArg,
    /// 0–9 (default 6).
    #[serde(default)]
    pub level: Option<u8>,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for CompressArgs {
    fn read_paths(&self) -> Vec<PathBuf> {
        self.sources.clone()
    }
    fn write_paths(&self) -> Vec<PathBuf> {
        vec![self.dest.clone()]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreArgs {
    /// Trash item ids from `trash_list`.
    pub ids: Vec<String>,
    /// Restore into this folder instead of the original locations.
    #[serde(default)]
    pub to: Option<PathBuf>,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for RestoreArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        self.to.iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------------------------
// Destructive
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteArgs {
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for DeleteArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncDirection {
    /// Make `right` match `left`.
    #[default]
    LeftToRight,
    RightToLeft,
    /// Copy newer / missing files both ways, never delete.
    Both,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncArgs {
    pub left: PathBuf,
    pub right: PathBuf,
    #[serde(default)]
    pub direction: SyncDirection,
    /// Delete files that exist only on the target side (one-way sync only).
    #[serde(default)]
    pub delete_extra: bool,
    #[serde(default)]
    pub by_content: bool,
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for SyncArgs {
    fn write_paths(&self) -> Vec<PathBuf> {
        vec![self.left.clone(), self.right.clone()]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DryRunOnly {
    #[serde(default)]
    pub dry_run: bool,
}
impl ToolArgs for DryRunOnly {}

// ---------------------------------------------------------------------------------------
// UI control / meta
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToastArgs {
    /// Short message (max 200 characters).
    pub message: String,
}
impl ToolArgs for ToastArgs {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomCommandArgs {
    /// Name of a custom command configured by the user.
    pub name: String,
}
impl ToolArgs for CustomCommandArgs {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryIdArg {
    /// History entry id from `history_list`.
    pub history_id: u64,
}
impl ToolArgs for HistoryIdArg {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobIdArg {
    /// Job id; omit for all active jobs.
    #[serde(default)]
    pub job_id: Option<u64>,
}
impl ToolArgs for JobIdArg {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelJobArgs {
    pub job_id: u64,
}
impl ToolArgs for CancelJobArgs {}
