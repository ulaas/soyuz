//! Settings → AI: provider profiles (add from templates, edit, delete, make
//! default), API keys that go straight to the OS credential store, live tr!("settings-ai-test")
//! with a tool-calling probe, per-task models, privacy, scope, network limits, monthly
//! budgets with this month's spend, and the price table.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, Stroke, TextEdit, Ui};
use soyuz_ai::{BudgetStatus, PriceTable, ProbeReport, Secret, UsageLedger};
use soyuz_core::config::{
    Ai, AiAccess, AiApproval, AiFolderRule, AiPrice, AiPrivacy, AiProfile, AiProviderKind, AiRiskCeiling, AiTaskModel,
    AiToolMode, default_prices,
};

use super::widgets::{
    code, dd, dropdown, group, matches, number, segmented, small_button, subheading, switch, text_choice,
};
use crate::app::SoyuzApp;
use crate::services::ai_service::{self, KeyStatus, Wake};
use crate::theme::icons::ph;
use crate::theme::{icons, palette};

const CTX: &str = "AI assistant models providers";

enum Test {
    Running(Receiver<ProbeReport>),
    Done(ProbeReport),
}

/// Page-local state (lives in `SettingsState`).
#[derive(Default)]
pub struct AiPageState {
    /// Secret typed into a profile's password field, by profile name.
    key_input: HashMap<String, String>,
    key_status: HashMap<String, KeyStatus>,
    key_pending: HashMap<String, Receiver<KeyStatus>>,
    tests: HashMap<String, Test>,
    cred_ops: Vec<(String, Receiver<Result<(), String>>)>,
    ledger: Option<UsageLedger>,
    ledger_rx: Option<Receiver<Result<UsageLedger, String>>>,
    ledger_error: Option<String>,
    notice: Option<(String, bool)>,
    roots_text: Option<String>,
    blocked_text: Option<String>,
    /// Models listed by each profile's provider (by profile name), for the model dropdown.
    models: HashMap<String, Models>,
}

/// A provider's model list, fetched when a model dropdown opens.
enum Models {
    Loading(Receiver<Result<Vec<String>, String>>),
    Done(Vec<String>),
    Failed(String),
}

fn wake_of(ui: &Ui) -> Wake {
    let c = ui.ctx().clone();
    Arc::new(move || c.request_repaint())
}

impl AiPageState {
    fn poll(&mut self, ui: &Ui) {
        self.key_pending.retain(|reference, rx| match rx.try_recv() {
            Ok(s) => {
                self.key_status.insert(reference.clone(), s);
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for m in self.models.values_mut() {
            if let Models::Loading(rx) = m
                && let Ok(r) = rx.try_recv()
            {
                *m = match r {
                    Ok(v) => Models::Done(v),
                    Err(e) => Models::Failed(e),
                };
            }
        }
        for t in self.tests.values_mut() {
            if let Test::Running(rx) = t
                && let Ok(r) = rx.try_recv()
            {
                *t = Test::Done(r);
            }
        }
        let mut refresh = false;
        self.cred_ops.retain(|(what, rx)| match rx.try_recv() {
            Ok(Ok(())) => {
                self.notice = Some((what.clone(), false));
                refresh = true;
                false
            }
            Ok(Err(e)) => {
                self.notice = Some((tr!("settings-ai-op-failed", what = what, error = e), true));
                refresh = true;
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        if refresh {
            self.key_status.clear();
        }
        if self.ledger.is_none() && self.ledger_rx.is_none() && self.ledger_error.is_none() {
            self.ledger_rx = Some(ai_service::load_ledger(wake_of(ui)));
        }
        if let Some(rx) = &self.ledger_rx
            && let Ok(r) = rx.try_recv()
        {
            match r {
                Ok(l) => self.ledger = Some(l),
                Err(e) => self.ledger_error = Some(e),
            }
            self.ledger_rx = None;
        }
    }

    fn status(&mut self, ui: &Ui, reference: &str) -> Option<KeyStatus> {
        if let Some(s) = self.key_status.get(reference) {
            return Some(s.clone());
        }
        if !self.key_pending.contains_key(reference) {
            self.key_pending.insert(reference.to_string(), ai_service::key_status(reference.to_string(), wake_of(ui)));
        }
        None
    }
}

fn card(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    Frame::new()
        .fill(p.bg_surface2.gamma_multiply(if p.dark { 0.7 } else { 1.0 }))
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        });
}

/// Default model id per kind (verified against provider docs 2026-10; editable).
fn default_model(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Anthropic => "claude-opus-5-5",
        AiProviderKind::Openai => "gpt-6-luna",
        AiProviderKind::OpenaiResponses => "gpt-6.1-sol",
        AiProviderKind::Gemini => "gemini-3.8-flash",
        AiProviderKind::Ollama => "qwen3:8b",
        _ => "",
    }
}

/// Well-known model ids per provider kind (the default first); the provider's own list
/// is fetched when the model dropdown opens.
fn known_models(kind: AiProviderKind) -> &'static [&'static str] {
    match kind {
        AiProviderKind::Anthropic => &[
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5",
        ],
        AiProviderKind::Openai => &["gpt-6-luna"],
        AiProviderKind::OpenaiResponses => &["gpt-6.1-sol"],
        AiProviderKind::Gemini => &["gemini-3.8-flash"],
        AiProviderKind::Ollama => &["qwen3:8b"],
        _ => &[],
    }
}

/// Dropdown items for a model field: `first` (the "use the profile's model" entry of
/// task routing), the known models, then the provider's listed ones.
fn model_items(
    kind: AiProviderKind,
    listed: &[String],
    status: Option<&str>,
    first: Option<&str>,
) -> Vec<dd::Item<String>> {
    let mut items = Vec::new();
    if let Some(f) = first {
        items.push(dd::Item::new(String::new(), f).icon(ph::ARROW_BEND_UP_LEFT));
    }
    let known = known_models(kind);
    let g_known = tr!("settings-ai-models-known");
    for (i, m) in known.iter().enumerate() {
        let item = dd::Item::new(m.to_string(), *m).group(g_known);
        items.push(if i == 0 { item.secondary(tr!("settings-ai-models-default")) } else { item });
    }
    let g_listed = tr!("settings-ai-models-listed");
    for m in listed.iter().filter(|m| !known.contains(&m.as_str())) {
        items.push(dd::Item::new(m.clone(), m.clone()).group(g_listed));
    }
    if let Some(st) = status {
        items.push(dd::Item::new("?status".to_string(), st).group(g_listed).disabled(Some(st.to_string())));
    }
    items
}

fn unique_name(ai: &Ai, base: &str) -> String {
    let base = base.to_lowercase().replace([' ', '(', ')', '.'], "-").replace("--", "-");
    let base = base.trim_matches('-').to_string();
    (1..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
        .find(|n| ai.profile(n).is_none())
        .unwrap_or(base)
}

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    app.gui.settings.ai.poll(ui);
    let searching = !q.trim().is_empty();
    let mut any = false;

    any |= general(ui, app, q);

    // --- Profiles ------------------------------------------------------------------
    if !searching || matches(q, &["provider profile model api key test connection", CTX, tr!("settings-ai-profiles")]) {
        any = true;
        ui.add_space(14.0);
        ui.label(RichText::new(super::widgets::upper(tr!("settings-ai-profiles"))).size(10.5).color(p.text_tertiary));
        ui.add_space(6.0);
        if let Some((msg, err)) = &app.gui.settings.ai.notice {
            let color = if *err { p.danger } else { p.success };
            ui.label(RichText::new(msg).size(11.5).color(color));
            ui.add_space(4.0);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr!("settings-views-add-column")).size(11.5).color(p.text_tertiary));
            for kind in AiProviderKind::ALL {
                let icon = if kind.is_local() { ph::DESKTOP_TOWER } else { ph::CLOUD };
                if small_button(ui, Some(icon), crate::i18n::labels::ai_provider(kind), false).clicked() {
                    let d = &mut app.gui.settings.draft.ai;
                    let name = unique_name(d, kind.label().split(' ').next().unwrap_or("profile"));
                    let key = if kind.is_local() || kind == AiProviderKind::Custom {
                        String::new()
                    } else {
                        format!("credential:{}", ai_service::credential_name(&name))
                    };
                    d.profiles.push(AiProfile {
                        name: name.clone(),
                        kind,
                        model: default_model(kind).into(),
                        key,
                        ..AiProfile::default()
                    });
                    if d.default_profile.is_empty() {
                        d.default_profile = name;
                    }
                }
            }
        });
        let n = app.gui.settings.draft.ai.profiles.len();
        let mut remove = None;
        for i in 0..n {
            ui.add_space(10.0);
            if profile_card(ui, app, i) {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            let d = &mut app.gui.settings.draft.ai;
            let removed = d.profiles.remove(i);
            if d.default_profile == removed.name {
                d.default_profile = d.profiles.first().map(|p| p.name.clone()).unwrap_or_default();
            }
            app.gui.settings.ai.notice = Some((tr!("settings-ai-profile-removed", name = &removed.name), false));
        }
        if n == 0 {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(icons::icon(ph::SPARKLE, 28.0).color(p.text_tertiary));
                ui.label(RichText::new(tr!("settings-ai-empty")).color(p.text_secondary));
                ui.label(RichText::new(tr!("settings-ai-empty-hint")).size(11.5).color(p.text_tertiary));
            });
        }
    }

    any |= prices(ui, app, q);
    any |= super::ai_connections::page(ui, app, q);
    any
}

fn general(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let s = &mut app.gui.settings;
    let d = &mut s.draft.ai;
    let names: Vec<(String, String)> = d.profiles.iter().map(|p| (p.name.clone(), p.name.clone())).collect();
    let kinds: Vec<(String, AiProviderKind)> = d.profiles.iter().map(|p| (p.name.clone(), p.kind)).collect();
    let default_kind = d.default_profile().map_or(AiProviderKind::Anthropic, |p| p.kind);
    let mut any = false;
    any |= group(ui, q, CTX, tr!("settings-ai-assistant"), |r| {
        r.row(tr!("settings-ai-enabled"), tr!("settings-ai-enabled-desc"), |ui| {
            switch(ui, &mut d.enabled);
        });
        r.row_kw(
            tr!("settings-ai-palette"),
            tr!("settings-ai-palette-desc"),
            "palette ask question natural language",
            |ui| {
                switch(ui, &mut d.palette);
            },
        );
        r.row_kw(
            tr!("settings-ai-privacy"),
            tr!("settings-ai-privacy-desc"),
            "privacy contents metadata upload",
            |ui| {
                segmented(
                    ui,
                    &mut d.privacy,
                    &[
                        (AiPrivacy::None, tr!("settings-behaviour-nothing")),
                        (AiPrivacy::MetadataOnly, tr!("settings-ai-metadata-only")),
                        (AiPrivacy::Contents, tr!("settings-ai-contents")),
                    ],
                );
            },
        );
        r.row(tr!("settings-ai-default-profile"), tr!("settings-ai-default-profile-desc"), |ui| {
            let mut opts: Vec<(String, &str)> = names.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
            opts.insert(0, (String::new(), tr!("settings-ai-first-profile")));
            dropdown(ui, "ai_default", &mut d.default_profile, &opts, 180.0);
        });
        for (label, desc, id, task) in [
            (tr!("settings-ai-fast-model"), tr!("settings-ai-fast-model-desc"), "fast", &mut d.tasks.fast),
            (tr!("settings-ai-strong-model"), tr!("settings-ai-strong-model-desc"), "strong", &mut d.tasks.strong),
        ] {
            r.row_kw(label, desc, "per task routing", |ui| {
                task_model(ui, id, task, &names, &kinds, default_kind);
            });
        }
    });
    any |= group(ui, q, CTX, tr!("settings-ai-scope"), |r| {
        r.block(tr!("settings-ai-roots"), tr!("settings-ai-roots-desc"), |ui| {
            let t = s.ai.roots_text.get_or_insert_with(|| {
                d.scope.allowed_roots.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n")
            });
            if ui
                .add(
                    TextEdit::multiline(t)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                )
                .changed()
            {
                d.scope.allowed_roots =
                    t.lines().map(str::trim).filter(|l| !l.is_empty()).map(std::path::PathBuf::from).collect();
            }
        });
        r.block(tr!("settings-ai-blocked"), tr!("settings-ai-blocked-desc"), |ui| {
            let t = s.ai.blocked_text.get_or_insert_with(|| d.scope.blocked.join("\n"));
            if ui
                .add(
                    TextEdit::multiline(t)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                )
                .changed()
            {
                d.scope.blocked = t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            }
        });
        r.row_kw(tr!("settings-ai-sensitive"), tr!("settings-ai-sensitive-desc"), "security secrets override", |ui| {
            switch(ui, &mut d.scope.allow_sensitive);
        });
        r.row(tr!("settings-ai-read-limit"), tr!("settings-ai-read-limit-desc"), |ui| {
            number(ui, &mut d.scope.max_read_kib, 16..=4096, " KiB");
        });
    });
    any |= group(ui, q, CTX, tr!("settings-ai-network"), |r| {
        r.row(tr!("settings-ai-timeout"), tr!("settings-ai-timeout-desc"), |ui| {
            number(ui, &mut d.request_timeout_secs, 5..=600, " s");
        });
        r.row(tr!("settings-ai-idle-timeout"), tr!("settings-ai-idle-timeout-desc"), |ui| {
            number(ui, &mut d.idle_timeout_secs, 10..=900, " s");
        });
        r.row_kw(tr!("settings-ai-retries"), tr!("settings-ai-retries-desc"), "429 retry", |ui| {
            number(ui, &mut d.max_retries, 0..=10, "");
        });
        r.row_kw(tr!("settings-ai-budget-warn"), tr!("settings-ai-budget-warn-desc"), "cost spend", |ui| {
            number(ui, &mut d.budget_warn_percent, 10..=100, " %");
        });
    });
    any |= agent_settings(ui, &mut s.draft.ai, q);
    any
}

/// `[ai.agent]` and `[[ai.folders]]` (in-app assistant: limits, approvals, privacy).
fn agent_settings(ui: &mut Ui, d: &mut Ai, q: &str) -> bool {
    let p = palette(ui.ctx());
    let mut any = group(ui, q, CTX, tr!("settings-ai-tasks"), |r| {
        let a = &mut d.agent;
        r.row_kw(tr!("settings-ai-approvals"), tr!("settings-ai-approvals-desc"), "approve auto plan confirm", |ui| {
            segmented(
                ui,
                &mut a.approval,
                &[
                    (AiApproval::Always, tr!("settings-ai-always-ask")),
                    (AiApproval::AutoWrite, tr!("settings-ai-auto-writes")),
                ],
            );
        });
        r.row_kw(tr!("settings-ai-max-risk"), tr!("settings-ai-max-risk-desc"), "risk destructive delete", |ui| {
            segmented(
                ui,
                &mut a.max_risk,
                &[
                    (AiRiskCeiling::Read, tr!("settings-ai-read-only")),
                    (AiRiskCeiling::Write, tr!("settings-ai-write")),
                    (AiRiskCeiling::Destructive, tr!("settings-ai-destructive")),
                ],
            );
        });
        r.row_kw(tr!("settings-ai-steps"), tr!("settings-ai-steps-desc"), "limit", |ui| {
            number(ui, &mut a.max_steps, 1..=200, "");
        });
        r.row_kw(tr!("settings-ai-token-budget"), tr!("settings-ai-token-budget-desc"), "limit tokens", |ui| {
            number(ui, &mut a.token_budget, 0..=50_000_000, "");
        });
        r.row_kw(tr!("settings-ai-time-limit"), tr!("settings-ai-time-limit-desc"), "limit timeout", |ui| {
            number(ui, &mut a.time_limit_secs, 10..=86_400, " s");
        });
        r.row_kw(tr!("settings-ai-auto-reads"), tr!("settings-ai-auto-reads-desc"), "limit list search", |ui| {
            number(ui, &mut a.max_auto_reads, 1..=1000, "");
        });
        r.row_kw(tr!("settings-ai-compact"), tr!("settings-ai-compact-desc"), "context window", |ui| {
            number(ui, &mut a.compact_after_kib, 8..=4096, " KiB");
        });
        r.row_kw(tr!("settings-ai-keep-recent"), tr!("settings-ai-keep-recent-desc"), "context", |ui| {
            number(ui, &mut a.keep_recent_results, 0..=100, "");
        });
        r.row_kw(tr!("settings-ai-listing"), tr!("settings-ai-listing-desc"), "listing context", |ui| {
            number(ui, &mut a.listing_entries, 0..=2000, "");
        });
        r.row_kw(tr!("settings-ai-confirm-cost"), tr!("settings-ai-confirm-cost-desc"), "cost estimate price", |ui| {
            let name = super::widgets::row_label(ui);
            let dv = ui.add(
                egui::DragValue::new(&mut a.confirm_cost_usd)
                    .range(0.0..=1000.0)
                    .speed(0.05)
                    .prefix("$ ")
                    .max_decimals(2),
            );
            super::widgets::named(&dv, &name);
        });
        r.row_kw(
            tr!("settings-ai-redact"),
            tr!("settings-ai-redact-desc"),
            "privacy redaction mask email key token",
            |ui| {
                switch(ui, &mut a.redact_cloud);
            },
        );
        r.row_kw(
            tr!("settings-ai-name-excerpt"),
            tr!("settings-ai-name-excerpt-desc"),
            "rename suggest names contents excerpt",
            |ui| {
                number(ui, &mut a.name_excerpt_chars, 0..=20_000, &format!(" {}", tr!("settings-unit-chars")));
            },
        );
        r.row_kw(
            tr!("settings-ai-vision-px"),
            tr!("settings-ai-vision-px-desc"),
            "rename suggest names vision image photo",
            |ui| {
                number(ui, &mut a.vision_max_px, 0..=2048, " px");
            },
        );
        r.row_kw(tr!("settings-ai-name-files"), tr!("settings-ai-name-files-desc"), "rename suggest names", |ui| {
            number(ui, &mut a.name_max_files, 1..=500, "");
        });
    });
    any |= group(ui, q, CTX, tr!("settings-ai-folders"), |r| {
        r.block(tr!("settings-ai-per-folder"), tr!("settings-ai-per-folder-desc"), |ui| {
            let mut remove = None;
            for (i, rule) in d.folders.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let mut text = rule.path.to_string_lossy().into_owned();
                    if code(ui, &mut text, tr!("settings-ai-folder-hint"), (ui.available_width() - 330.0).max(140.0))
                        .changed()
                    {
                        rule.path = std::path::PathBuf::from(text);
                    }
                    segmented(
                        ui,
                        &mut rule.access,
                        &[
                            (AiAccess::None, tr!("settings-none")),
                            (AiAccess::Metadata, tr!("settings-ai-metadata")),
                            (AiAccess::Contents, tr!("settings-ai-contents")),
                        ],
                    );
                    if small_button(ui, Some(ph::TRASH), "", true)
                        .on_hover_text(tr!("settings-ai-remove-rule"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                d.folders.remove(i);
            }
            if d.folders.is_empty() {
                ui.label(RichText::new(tr!("settings-ai-no-rules")).size(11.5).color(p.text_tertiary));
            }
            if small_button(ui, Some(ph::PLUS), tr!("settings-ai-add-rule"), false).clicked() {
                d.folders.push(AiFolderRule { path: soyuz_fs::home(), access: AiAccess::None });
            }
        });
    });
    any
}

fn task_model(
    ui: &mut Ui,
    id: &str,
    task: &mut AiTaskModel,
    names: &[(String, String)],
    kinds: &[(String, AiProviderKind)],
    default_kind: AiProviderKind,
) {
    // In a right-to-left row the profile picker goes rightmost, the model left of it.
    let mut opts: Vec<(String, &str)> = names.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
    opts.insert(0, (String::new(), tr!("settings-ai-default-profile")));
    dropdown(ui, &format!("ai_task_{id}"), &mut task.profile, &opts, 150.0);
    let kind = kinds.iter().find(|(n, _)| *n == task.profile).map_or(default_kind, |(_, k)| *k);
    let items = model_items(kind, &[], None, Some(tr!("settings-ai-task-model-hint")));
    text_choice(ui, &format!("ai_task_model_{id}"), &mut task.model, &items, 190.0, tr!("settings-ai-model-hint"));
}

/// One profile card; returns `true` when the user deleted it.
fn profile_card(ui: &mut Ui, app: &mut SoyuzApp, i: usize) -> bool {
    let p = palette(ui.ctx());
    let s = &mut app.gui.settings;
    let (d, st) = (&mut s.draft.ai, &mut s.ai);
    let mut delete = false;
    let is_default = d.default_profile == d.profiles[i].name || (d.default_profile.is_empty() && i == 0);
    let ai_snapshot = d.clone();
    let prices = PriceTable::new(&d.prices);
    let warn = d.budget_warn_percent;
    let names_taken: Vec<String> =
        d.profiles.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, p)| p.name.clone()).collect();
    let mut make_default = false;
    let prof = &mut d.profiles[i];
    card(ui, |ui| {
        // Header: name, kind, default star, delete.
        ui.horizontal(|ui| {
            ui.label(
                icons::icon(if prof.kind.is_local() { ph::DESKTOP_TOWER } else { ph::CLOUD }, 16.0).color(p.accent),
            );
            let before = prof.name.clone();
            if ui
                .add(
                    TextEdit::singleline(&mut prof.name)
                        .hint_text(tr!("settings-name"))
                        .desired_width(160.0)
                        .margin(Margin::symmetric(8, 4)),
                )
                .changed()
                && is_default
            {
                make_default = true;
            }
            if prof.name.trim().is_empty() || names_taken.contains(&prof.name) {
                ui.label(RichText::new(tr!("settings-ai-name-unique")).size(11.0).color(p.warning));
            }
            let mut kind = prof.kind;
            let opts: Vec<(AiProviderKind, &str)> =
                AiProviderKind::ALL.iter().map(|k| (*k, crate::i18n::labels::ai_provider(*k))).collect();
            if dropdown(ui, &format!("ai_kind_{i}"), &mut kind, &opts, 200.0) {
                prof.kind = kind;
                if prof.model.is_empty() {
                    prof.model = default_model(kind).into();
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if small_button(ui, Some(ph::TRASH), "", true)
                    .on_hover_text(tr!("settings-ai-delete-profile"))
                    .clicked()
                {
                    delete = true;
                }
                if is_default {
                    ui.label(RichText::new(tr!("settings-ai-default-tag")).size(11.5).color(p.accent));
                    ui.label(icons::icon(ph::STAR, 12.0).color(p.accent));
                } else if small_button(ui, Some(ph::STAR), tr!("settings-ai-make-default"), false).clicked() {
                    make_default = true;
                }
            });
            if before != prof.name {
                // Keep page state attached to the renamed profile.
                if let Some(t) = st.tests.remove(&before) {
                    st.tests.insert(prof.name.clone(), t);
                }
            }
        });
        ui.add_space(6.0);
        egui::Grid::new(("ai_profile_grid", i)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            let lbl = |ui: &mut Ui, t: &str| {
                ui.label(RichText::new(t).size(12.0).color(p.text_secondary));
            };
            lbl(ui, tr!("settings-ai-base-url"));
            code(ui, &mut prof.base_url, prof.kind.default_base_url(), 320.0);
            ui.end_row();
            lbl(ui, tr!("settings-ai-model"));
            ui.horizontal(|ui| {
                model_choice(ui, st, prof, &ai_snapshot, i);
            });
            ui.end_row();
            lbl(ui, tr!("settings-ai-api-key"));
            key_row(ui, st, prof);
            ui.end_row();
            lbl(ui, tr!("settings-ai-tools"));
            ui.horizontal(|ui| {
                segmented(
                    ui,
                    &mut prof.tool_mode,
                    &[
                        (AiToolMode::Auto, tr!("settings-auto")),
                        (AiToolMode::Native, tr!("settings-ai-native")),
                        (AiToolMode::Json, tr!("settings-ai-json")),
                    ],
                );
            });
            ui.end_row();
            lbl(ui, tr!("settings-ai-max-tokens"));
            let dv = ui.add(egui::DragValue::new(&mut prof.max_tokens).range(256..=128_000).speed(64.0));
            super::widgets::named(&dv, tr!("settings-ai-max-tokens"));
            ui.end_row();
            lbl(ui, tr!("settings-ai-vision"));
            ui.horizontal(|ui| {
                switch(ui, &mut prof.vision);
                ui.label(RichText::new(tr!("settings-ai-vision-desc")).size(11.0).color(p.text_tertiary));
            });
            ui.end_row();
            lbl(ui, tr!("settings-ai-budget"));
            ui.horizontal(|ui| {
                let dv = ui.add(
                    egui::DragValue::new(&mut prof.monthly_budget_usd).range(0.0..=100_000.0).speed(1.0).prefix("$ "),
                );
                super::widgets::named(&dv, tr!("settings-ai-budget"));
                ui.label(RichText::new(tr!("settings-ai-budget-hint")).size(11.0).color(p.text_tertiary));
            });
            ui.end_row();
            lbl(ui, tr!("settings-ai-this-month"));
            spend_row(ui, st, prof, &prices, warn);
            ui.end_row();
        });
        if prof.kind == AiProviderKind::Custom {
            ui.add_space(4.0);
            headers_editor(ui, prof, i);
        }
        ui.add_space(6.0);
        test_row(ui, st, prof, &ai_snapshot);
    });
    if make_default {
        d.default_profile = d.profiles[i].name.clone();
    }
    delete
}

/// The profile's model: known ids, the provider's list (fetched in the background when
/// the dropdown opens and a key is available or the provider is local), "Custom…".
fn model_choice(ui: &mut Ui, st: &mut AiPageState, prof: &mut AiProfile, ai: &Ai, i: usize) {
    let mut listed: Vec<String> = match st.tests.get(&prof.name) {
        Some(Test::Done(r)) => r.models.clone(),
        _ => Vec::new(),
    };
    let status = match st.models.get(&prof.name) {
        Some(Models::Done(v)) => {
            listed.extend(v.iter().cloned());
            None
        }
        Some(Models::Loading(_)) => Some(tr!("settings-ai-models-loading").to_string()),
        Some(Models::Failed(e)) => Some(tr!("settings-ai-models-failed", error = e.as_str())),
        None => None,
    };
    listed.sort();
    listed.dedup();
    let items = model_items(prof.kind, &listed, status.as_deref(), None);
    let out = dd::Dropdown::new(format!("ai_model_{i}"))
        .width(240.0)
        .name(tr!("settings-ai-model"))
        .custom(tr!("dropdown-custom"))
        .placeholder(if prof.model.trim().is_empty() { tr!("settings-ai-model-hint") } else { tr!("settings-custom") })
        .show(ui, &mut prof.model, &items);
    if out.opened && !matches!(st.models.get(&prof.name), Some(Models::Loading(_) | Models::Done(_))) {
        let typed = st.key_input.get(&prof.name).map(|s| s.trim()).filter(|s| !s.is_empty()).map(Secret::new);
        let key_ok = prof.kind.is_local()
            || typed.is_some()
            || matches!(st.key_status.get(&prof.key), Some(KeyStatus::Stored | KeyStatus::FromEnv(_)));
        if key_ok {
            let rx = ai_service::list_models(prof.clone(), ai, typed, wake_of(ui));
            st.models.insert(prof.name.clone(), Models::Loading(rx));
        }
    }
    // A custom id: edit it inline.
    let flag = egui::Id::unique(("soyuz.ai.model_custom", i));
    let mut editing = ui.ctx().data(|d| d.get_temp::<bool>(flag)).unwrap_or(false);
    if out.changed {
        editing = false;
    }
    let focus = out.custom.is_some();
    if let Some(t) = out.custom {
        editing = true;
        if !t.is_empty() {
            prof.model = t;
        }
    }
    let known = items.iter().any(|it| it.value == prof.model);
    if editing || (!known && !prof.model.trim().is_empty()) {
        let r = code(ui, &mut prof.model, tr!("settings-ai-model-hint"), 200.0);
        if focus {
            r.request_focus();
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(flag, editing));
}

fn key_row(ui: &mut Ui, st: &mut AiPageState, prof: &mut AiProfile) {
    let p = palette(ui.ctx());
    if cfg!(target_os = "linux") && !prof.kind.is_local() && store_available(ui) == Some(false) {
        ui.horizontal_top(|ui| {
            ui.label(icons::icon(ph::WARNING, 13.0).color(p.warning));
            ui.add(
                egui::Label::new(
                    RichText::new(tr!("settings-ai-store-unavailable", store = store_name()))
                        .size(11.5)
                        .color(p.warning),
                )
                .wrap(),
            );
        });
    }
    ui.horizontal(|ui| {
        let reference = prof.key.clone();
        match st.status(ui, &reference) {
            None => {
                ui.add(egui::Spinner::new().size(12.0));
            }
            Some(KeyStatus::None) if prof.kind.is_local() => {
                ui.label(RichText::new(tr!("settings-ai-key-not-needed")).size(11.5).color(p.text_tertiary));
            }
            Some(KeyStatus::None) => {
                ui.label(RichText::new(tr!("settings-ai-no-key")).size(11.5).color(p.warning));
            }
            Some(KeyStatus::Stored) => {
                ui.label(icons::icon(ph::LOCK_KEY, 13.0).color(p.success));
                ui.label(
                    RichText::new(tr!("settings-ai-key-stored", store = store_name())).size(11.5).color(p.success),
                );
            }
            Some(KeyStatus::Missing) => {
                ui.label(RichText::new(tr!("settings-ai-key-missing")).size(11.5).color(p.warning));
            }
            Some(KeyStatus::FromEnv(v)) => {
                ui.label(RichText::new(tr!("settings-ai-key-env", var = v)).size(11.5).color(p.success));
            }
            Some(KeyStatus::EnvMissing(v)) => {
                ui.label(RichText::new(tr!("settings-ai-key-env-missing", var = v)).size(11.5).color(p.warning));
            }
            Some(KeyStatus::Invalid(e)) => {
                ui.label(RichText::new(e).size(11.5).color(p.danger));
            }
        }
        let input = st.key_input.entry(prof.name.clone()).or_default();
        ui.add(
            TextEdit::singleline(input)
                .password(true)
                .hint_text(tr!("settings-ai-key-hint"))
                .desired_width(170.0)
                .margin(Margin::symmetric(8, 4)),
        );
        let has_input = !input.trim().is_empty();
        if has_input && small_button(ui, Some(ph::KEY), tr!("settings-ai-save-key"), false).clicked() {
            let secret = Secret::new(std::mem::take(input).trim().to_string());
            let name = match soyuz_ai::credentials::parse_ref(&prof.key) {
                Ok(soyuz_ai::credentials::KeyRef::Credential(n)) => n,
                _ => ai_service::credential_name(&prof.name),
            };
            prof.key = format!("credential:{name}");
            st.cred_ops.push((
                tr!("settings-ai-key-saved", store = store_name()),
                ai_service::store_secret(name, secret, wake_of(ui)),
            ));
        }
        if let Ok(soyuz_ai::credentials::KeyRef::Credential(name)) = soyuz_ai::credentials::parse_ref(&prof.key)
            && st.key_status.get(&prof.key) == Some(&KeyStatus::Stored)
            && small_button(ui, Some(ph::TRASH), tr!("settings-remote-forget"), true)
                .on_hover_text(tr!("settings-ai-forget-key"))
                .clicked()
        {
            st.cred_ops.push((tr!("settings-ai-key-deleted").into(), ai_service::delete_secret(name, wake_of(ui))));
        }
        ui.menu_button(icons::icon(ph::DOTS_THREE, 14.0), |ui| {
            ui.label(RichText::new(tr!("settings-ai-key-ref")).size(11.5));
            code(ui, &mut prof.key, tr!("settings-ai-key-ref-hint"), 240.0);
            ui.label(RichText::new(tr!("settings-ai-key-ref-desc")).size(11.0).color(p.text_tertiary));
        });
    });
}

fn store_name() -> &'static str {
    if cfg!(windows) {
        tr!("settings-ai-store-windows")
    } else if cfg!(target_os = "linux") {
        tr!("settings-ai-store-linux")
    } else {
        tr!("settings-ai-store-other")
    }
}

/// Whether the OS credential store answers (`None` while checking). Linux asks the
/// Secret Service over D-Bus, which can take seconds: checked once on a worker thread.
fn store_available(ui: &Ui) -> Option<bool> {
    use std::sync::atomic::{AtomicU8, Ordering};
    // 0 = not started, 1 = checking, 2 = unavailable, 3 = available.
    static STATE: AtomicU8 = AtomicU8::new(0);
    match STATE.load(Ordering::Acquire) {
        2 => Some(false),
        3 => Some(true),
        1 => None,
        _ => {
            if STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                let wake = wake_of(ui);
                let spawned = std::thread::Builder::new().name("soyuz-keyring-probe".into()).spawn(move || {
                    let ok = soyuz_ai::credentials::store_available();
                    STATE.store(if ok { 3 } else { 2 }, Ordering::Release);
                    wake();
                });
                if spawned.is_err() {
                    STATE.store(2, Ordering::Release);
                }
            }
            None
        }
    }
}

fn spend_row(ui: &mut Ui, st: &AiPageState, prof: &AiProfile, prices: &PriceTable, warn: u32) {
    let p = palette(ui.ctx());
    let Some(ledger) = &st.ledger else {
        let msg = st.ledger_error.as_deref().unwrap_or(tr!("settings-ai-loading"));
        ui.label(RichText::new(msg).size(11.5).color(p.text_tertiary));
        return;
    };
    let month = soyuz_ai::pricing::current_month();
    let spend = ledger.spend(&month, &prof.name);
    let priced = prices.price(&prof.model).is_some();
    ui.horizontal(|ui| {
        let tokens = spend.usage.total();
        let mut parts = vec![tr!(
            "settings-ai-spend",
            cost = format!("${}", soyuz_core::locale::format_decimal(spend.cost_usd, 2)),
            tokens = tokens,
            requests = spend.requests
        )];
        if !priced {
            parts.push(tr!("settings-ai-spend-unpriced").to_string());
        }
        let color = match ledger.budget(&month, &prof.name, prof.monthly_budget_usd, warn) {
            BudgetStatus::Unlimited => p.text_secondary,
            BudgetStatus::Ok { fraction } => {
                parts.push(tr!("settings-ai-spend-share", percent = format!("{:.0}", fraction * 100.0)));
                p.text_secondary
            }
            BudgetStatus::Warn { fraction } => {
                parts.push(tr!("settings-ai-spend-share", percent = format!("{:.0}", fraction * 100.0)));
                p.warning
            }
            BudgetStatus::Exceeded => {
                parts.push(tr!("settings-ai-spend-exceeded").to_string());
                p.danger
            }
        };
        ui.label(RichText::new(parts.join(" · ")).size(11.5).color(color));
    });
}

fn headers_editor(ui: &mut Ui, prof: &mut AiProfile, i: usize) {
    let p = palette(ui.ctx());
    subheading(ui, tr!("settings-ai-headers"));
    let mut remove = None;
    let keys: Vec<String> = prof.headers.keys().cloned().collect();
    for (j, k) in keys.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(k).monospace().color(p.text_secondary));
            if let Some(v) = prof.headers.get_mut(k) {
                code(ui, v, tr!("settings-ai-header-value"), 260.0);
            }
            if small_button(ui, Some(ph::X), "", true).clicked() {
                remove = Some(j);
            }
        });
    }
    if let Some(j) = remove {
        prof.headers.remove(&keys[j]);
    }
    let id = egui::Id::unique(("ai_new_header", i));
    let mut new_key: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        code(ui, &mut new_key, "Header-Name", 160.0);
        if small_button(ui, Some(ph::PLUS), tr!("settings-ai-add-header"), false).clicked()
            && !new_key.trim().is_empty()
        {
            prof.headers.insert(new_key.trim().to_string(), String::new());
            new_key.clear();
        }
    });
    ui.label(RichText::new(tr!("settings-ai-headers-desc")).size(11.0).color(p.text_tertiary));
    ui.data_mut(|d| d.insert_temp(id, new_key));
}

fn test_row(ui: &mut Ui, st: &mut AiPageState, prof: &AiProfile, ai: &Ai) {
    let p = palette(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        let running = matches!(st.tests.get(&prof.name), Some(Test::Running(_)));
        if running {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(RichText::new(tr!("settings-ai-testing")).size(12.0).color(p.text_secondary));
        } else if small_button(ui, Some(ph::PLUGS_CONNECTED), tr!("settings-ai-test"), false)
            .on_hover_text(tr!("settings-ai-test-tooltip"))
            .clicked()
        {
            let typed = st.key_input.get(&prof.name).map(|s| s.trim()).filter(|s| !s.is_empty()).map(Secret::new);
            st.tests.insert(
                prof.name.clone(),
                Test::Running(ai_service::test_profile(prof.clone(), ai, typed, wake_of(ui))),
            );
        }
        if let Some(Test::Done(r)) = st.tests.get(&prof.name) {
            let (icon, color) =
                if r.error.is_none() { (ph::CHECK_CIRCLE, p.success) } else { (ph::X_CIRCLE, p.danger) };
            ui.label(icons::icon(icon, 14.0).color(color));
            ui.label(RichText::new(r.summary()).size(12.0).color(color));
            if r.model_listed == Some(false) {
                ui.label(icons::icon(ph::WARNING, 13.0).color(p.warning));
                ui.label(RichText::new(tr!("settings-ai-model-unlisted")).size(11.5).color(p.warning));
            }
            if let Some(e) = &r.models_error
                && r.error.is_none()
            {
                ui.label(
                    RichText::new(tr!("settings-ai-test-models-error", error = e.to_string()))
                        .size(11.0)
                        .color(p.text_tertiary),
                );
            }
            if r.usage.total() > 0 {
                ui.label(
                    RichText::new(tr!("settings-ai-tokens", count = r.usage.total())).size(11.0).color(p.text_tertiary),
                );
            }
        }
    });
}

fn prices(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.gui.settings.draft.ai;
    let mut remove = None;
    let mut add = false;
    let mut reset = false;
    let drawn = group(ui, q, CTX, tr!("settings-ai-prices"), |r| {
        r.block(tr!("settings-ai-price-table"), tr!("settings-ai-price-table-desc"), |ui| {
            egui::Grid::new("ai_prices").num_columns(6).spacing([8.0, 4.0]).show(ui, |ui| {
                for h in [
                    tr!("settings-ai-model"),
                    tr!("settings-ai-price-input"),
                    tr!("settings-ai-price-output"),
                    tr!("settings-ai-price-cache-read"),
                    tr!("settings-ai-price-cache-write"),
                    "",
                ] {
                    ui.label(RichText::new(h).size(11.0));
                }
                ui.end_row();
                for (i, row) in d.prices.iter_mut().enumerate() {
                    code(ui, &mut row.model, "model-*", 160.0);
                    for v in [
                        &mut row.input_per_mtok,
                        &mut row.output_per_mtok,
                        &mut row.cache_read_per_mtok,
                        &mut row.cache_write_per_mtok,
                    ] {
                        ui.add(egui::DragValue::new(v).range(0.0..=1000.0).speed(0.05).max_decimals(3).prefix("$"));
                    }
                    if small_button(ui, Some(ph::TRASH), "", true).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                add = small_button(ui, Some(ph::PLUS), tr!("settings-ai-add-price"), false).clicked();
                reset = small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("settings-restore-defaults"), false)
                    .clicked();
            });
        });
    });
    if let Some(i) = remove {
        d.prices.remove(i);
    }
    if add {
        d.prices.push(AiPrice { model: "model-*".into(), ..AiPrice::default() });
    }
    if reset {
        d.prices = default_prices();
    }
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_profile_names() {
        let mut ai = Ai::default();
        assert_eq!(unique_name(&ai, "LM Studio"), "lm-studio");
        ai.profiles.push(AiProfile { name: "anthropic".into(), ..Default::default() });
        assert_eq!(unique_name(&ai, "Anthropic"), "anthropic-2");
    }
}
