//! `[ai.agent]` and `[[ai.folders]]` (roadmap A5 / A7): limits and approval policy of the
//! in-app agent harness, redaction, and per-folder AI permissions inherited down the tree.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// When approved-by-default is allowed for write plans.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiApproval {
    /// Every plan waits for the user (default).
    #[default]
    Always,
    /// Conflict-free plans of the *write* class run without asking, except when the plan
    /// was proposed right after the assistant read untrusted file contents. Destructive
    /// plans always ask.
    AutoWrite,
}

/// Highest risk class the in-app assistant is offered.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiRiskCeiling {
    /// Read and UI tools only.
    Read,
    /// Plus copy / move / rename / create (always as plans).
    Write,
    /// Plus trash / delete / sync (always need approval).
    #[default]
    Destructive,
}

/// Per-folder AI access (A7), inherited by everything below the folder.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiAccess {
    /// The assistant cannot see the folder at all.
    None,
    /// Names, sizes, dates and types only.
    #[default]
    Metadata,
    /// File contents may be read (and sent to the model).
    Contents,
}

impl AiAccess {
    pub const ALL: [AiAccess; 3] = [AiAccess::None, AiAccess::Metadata, AiAccess::Contents];

    pub fn label(self) -> &'static str {
        match self {
            AiAccess::None => "No access",
            AiAccess::Metadata => "Metadata only",
            AiAccess::Contents => "Contents allowed",
        }
    }
}

/// One `[[ai.folders]]` rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AiFolderRule {
    /// Folder the rule applies to (and everything below it).
    pub path: PathBuf,
    /// What AI tools may see there.
    pub access: AiAccess,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiAgent {
    /// Model turns per task before the harness stops.
    pub max_steps: u32,
    /// Tokens (input + output, all turns) per task; 0 = unlimited.
    pub token_budget: u64,
    /// Wall-clock limit per task, seconds.
    pub time_limit_secs: u64,
    /// Read tools the agent may run on its own per task.
    pub max_auto_reads: u32,
    /// Summarise old tool results once the conversation is larger than this (KiB of text).
    pub compact_after_kib: u64,
    /// Most recent tool results kept verbatim when compacting.
    pub keep_recent_results: u32,
    pub approval: AiApproval,
    pub max_risk: AiRiskCeiling,
    /// Ask before starting a task whose estimated cost exceeds this (USD; 0 = never ask).
    pub confirm_cost_usd: f64,
    /// Entries of the current folder summarised into the task context.
    pub listing_entries: u32,
    /// Mask e-mail addresses, API keys and tokens in file data sent to cloud providers.
    pub redact_cloud: bool,
    /// "Suggest names": characters of each text / document file sent to the model (only
    /// where the folder's AI access is "contents"; 0 = names and metadata only).
    pub name_excerpt_chars: u32,
    /// "Suggest names": longest side of the downscaled image sent to vision models (px;
    /// 0 = never send images).
    pub vision_max_px: u32,
    /// "Suggest names": most files looked at per request.
    pub name_max_files: u32,
}

impl Default for AiAgent {
    fn default() -> Self {
        AiAgent {
            max_steps: 24,
            token_budget: 400_000,
            time_limit_secs: 600,
            max_auto_reads: 60,
            compact_after_kib: 96,
            keep_recent_results: 6,
            approval: AiApproval::Always,
            max_risk: AiRiskCeiling::Destructive,
            confirm_cost_usd: 0.5,
            listing_entries: 80,
            redact_cloud: true,
            name_excerpt_chars: 1500,
            vision_max_px: 768,
            name_max_files: 40,
        }
    }
}

/// Effective access for `path`: the deepest matching rule wins, else `default`.
/// Matching is component-wise and case-insensitive on Windows.
pub fn folder_access(rules: &[AiFolderRule], path: &Path, default: AiAccess) -> AiAccess {
    let key = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| {
                let s = c.as_os_str().to_string_lossy().replace('\\', "/");
                if cfg!(windows) { s.to_lowercase() } else { s }
            })
            .filter(|s| !s.is_empty() && s != "/")
            .collect()
    };
    let target = key(path);
    let mut best: Option<(usize, AiAccess)> = None;
    for r in rules {
        if r.path.as_os_str().is_empty() {
            continue;
        }
        let k = key(&r.path);
        if k.len() <= target.len() && target[..k.len()] == k[..] && best.is_none_or(|(n, _)| k.len() >= n) {
            best = Some((k.len(), r.access));
        }
    }
    best.map_or(default, |(_, a)| a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepest_rule_wins() {
        let rules = vec![
            AiFolderRule { path: "/data".into(), access: AiAccess::Contents },
            AiFolderRule { path: "/data/private".into(), access: AiAccess::None },
        ];
        assert_eq!(folder_access(&rules, Path::new("/data/x.txt"), AiAccess::Metadata), AiAccess::Contents);
        assert_eq!(folder_access(&rules, Path::new("/data/private/a/b"), AiAccess::Metadata), AiAccess::None);
        assert_eq!(folder_access(&rules, Path::new("/data/privateer"), AiAccess::Metadata), AiAccess::Contents);
        assert_eq!(folder_access(&rules, Path::new("/other"), AiAccess::Metadata), AiAccess::Metadata);
    }
}
