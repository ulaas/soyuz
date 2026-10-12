//! Translated labels for `soyuz-core` enums (the core crate has no Fluent; its
//! `label()` methods stay English for logs and the CLI). UI code calls these instead.

use soyuz_core::Column;
use soyuz_core::config::{
    ActionCategory, AiAccess, AiProviderKind, AiRisk, KeyContext, KeyPreset, RuleAction, RuleConflict, RuleKind,
};

/// Column header label.
pub fn column(c: Column) -> &'static str {
    match c {
        Column::Name => tr!("core-column-name"),
        Column::Size => tr!("core-column-size"),
        Column::Kind => tr!("core-column-kind"),
        Column::Modified => tr!("core-column-modified"),
        Column::Created => tr!("core-column-created"),
        Column::Accessed => tr!("core-column-accessed"),
        Column::Attributes => tr!("core-column-attributes"),
        Column::Mode => tr!("core-column-mode"),
        Column::Owner => tr!("core-column-owner"),
        Column::Extension => tr!("core-column-extension"),
        Column::Path => tr!("core-column-path"),
        Column::Tags => tr!("core-column-tags"),
        Column::GitStatus => tr!("core-column-git"),
        Column::Dimensions => tr!("core-column-dimensions"),
        Column::Duration => tr!("core-column-duration"),
        Column::LinkTarget => tr!("core-column-link-target"),
        Column::Checksum => tr!("core-column-checksum"),
        Column::ItemCount => tr!("core-column-item-count"),
        Column::Script1 => tr!("core-column-script1"),
        Column::Script2 => tr!("core-column-script2"),
        Column::Script3 => tr!("core-column-script3"),
        Column::Script4 => tr!("core-column-script4"),
    }
}

/// Palette category.
pub fn action_category(c: ActionCategory) -> &'static str {
    match c {
        ActionCategory::Navigation => tr!("core-category-navigation"),
        ActionCategory::Tabs => tr!("core-category-tabs"),
        ActionCategory::View => tr!("core-category-view"),
        ActionCategory::Selection => tr!("core-category-selection"),
        ActionCategory::FileOps => tr!("core-category-file"),
        ActionCategory::Tools => tr!("core-category-tools"),
        ActionCategory::Jobs => tr!("core-category-jobs"),
    }
}

/// AI folder access level.
pub fn ai_access(a: AiAccess) -> &'static str {
    match a {
        AiAccess::None => tr!("core-ai-access-none"),
        AiAccess::Metadata => tr!("core-ai-access-metadata"),
        AiAccess::Contents => tr!("core-ai-access-contents"),
    }
}

/// AI provider kind (product names stay as they are).
pub fn ai_provider(k: AiProviderKind) -> &'static str {
    match k {
        AiProviderKind::LlamaCpp => tr!("core-ai-provider-llamacpp"),
        AiProviderKind::Custom => tr!("core-ai-provider-custom"),
        other => other.label(),
    }
}

/// MCP client trust level.
pub fn ai_risk(r: AiRisk) -> &'static str {
    match r {
        AiRisk::Read => tr!("core-ai-risk-read"),
        AiRisk::Write => tr!("core-ai-risk-write"),
        AiRisk::Destructive => tr!("core-ai-risk-destructive"),
    }
}

/// Key-binding context.
pub fn key_context(c: KeyContext) -> &'static str {
    match c {
        KeyContext::Global => tr!("core-key-context-global"),
        KeyContext::FileList => tr!("core-key-context-file-list"),
        KeyContext::Terminal => tr!("core-key-context-terminal"),
        KeyContext::Dialog => tr!("core-key-context-dialog"),
        KeyContext::Omnibar => tr!("core-key-context-omnibar"),
        KeyContext::Rename => tr!("core-key-context-rename"),
        KeyContext::Sidebar => tr!("core-key-context-sidebar"),
        KeyContext::Editor => tr!("core-key-context-editor"),
    }
}

/// Key-binding preset.
pub fn key_preset(p: KeyPreset) -> &'static str {
    match p {
        KeyPreset::Explorer => tr!("core-key-preset-explorer"),
        KeyPreset::Commander => tr!("core-key-preset-commander"),
        KeyPreset::Finder => tr!("core-key-preset-finder"),
        KeyPreset::Vim => tr!("core-key-preset-vim"),
    }
}

/// Automation rule target kind.
pub fn rule_kind(k: RuleKind) -> &'static str {
    match k {
        RuleKind::Files => tr!("core-rule-kind-files"),
        RuleKind::Folders => tr!("core-rule-kind-folders"),
        RuleKind::Any => tr!("core-rule-kind-any"),
    }
}

/// Automation rule conflict policy.
pub fn rule_conflict(c: RuleConflict) -> &'static str {
    match c {
        RuleConflict::KeepBoth => tr!("core-rule-conflict-keep-both"),
        RuleConflict::Skip => tr!("core-rule-conflict-skip"),
        RuleConflict::Overwrite => tr!("core-rule-conflict-overwrite"),
        RuleConflict::Ask => tr!("core-rule-conflict-ask"),
    }
}

/// Automation rule step.
pub fn rule_action(a: &RuleAction) -> &'static str {
    match a {
        RuleAction::Move { .. } => tr!("core-rule-action-move"),
        RuleAction::Copy { .. } => tr!("core-rule-action-copy"),
        RuleAction::Rename { .. } => tr!("core-rule-action-rename"),
        RuleAction::Tag { .. } => tr!("core-rule-action-tag"),
        RuleAction::Trash => tr!("core-rule-action-trash"),
        RuleAction::Script { .. } => tr!("core-rule-action-script"),
    }
}
