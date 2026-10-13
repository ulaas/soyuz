//! The recipes and audit log tabs of the AI panel (the GUI's `ui/ai/pages.rs`): saved
//! recipes (Enter runs one in the current folder with fresh plans and the same approvals,
//! Delete twice removes it) and the audit log (`ai-audit.jsonl`, newest first, filtered
//! by the text after `/`; the source / session / model filters the GUI sets apply too).
//! Each list row is two lines, a `HitTarget::AiRow` target.

use soyuz_appcore::ai::audit::{self, AuditRecord};
use soyuz_appcore::ai::recipe;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::{AiLine, Built, Span};
use crate::dialog::safe;
use crate::input::ai::State;
use crate::layout::HitTarget;
use crate::theme::Role;
use crate::view::text;

fn row(b: &mut Built, first: Vec<Span>, second: Vec<Span>) {
    let i = b.rows.len();
    let start = b.lines.len();
    let t = Some(HitTarget::AiRow(i));
    b.lines.push(AiLine { spans: first, target: t, sel: false });
    b.lines.push(AiLine { spans: second, target: t, sel: false });
    b.rows.push((start, b.lines.len()));
}

fn note(b: &mut Built, w: usize, s: &str, role: Role) {
    for l in text::wrap(s, w) {
        b.lines.push(AiLine::plain(l, role));
    }
}

/// The recipes tab.
pub fn recipes(core: &AppCore, st: &State, w: usize) -> Built {
    let mut b = Built::default();
    let r = &core.ai.recipes;
    note(&mut b, w, tr!("ai-recipes-intro"), Role::Muted);
    b.lines.push(AiLine::default());
    if r.loading.is_some() && r.list.is_empty() {
        b.lines.push(AiLine::plain(tr!("common-loading"), Role::Muted));
        return b;
    }
    if r.list.is_empty() {
        note(&mut b, w, tr!("ai-recipes-empty-title"), Role::Title);
        note(&mut b, w, tr!("ai-recipes-empty-body"), Role::Muted);
        return b;
    }
    let dir = recipe::dir();
    for (path, rec) in &r.list {
        let armed = st.delete_armed.as_ref() == Some(path);
        match rec {
            Ok(rec) => {
                let tools: Vec<&str> = rec.steps.iter().map(|s| s.tool.as_str()).collect();
                let first = vec![
                    Span::bold(text::truncate_end(&safe(&rec.name), w), Role::Title),
                    Span::new(format!("  {}", safe(&tools.join(", "))), Role::Muted),
                ];
                let file = path.strip_prefix(&dir).unwrap_or(path).display().to_string();
                let created = rec.created.get(..16).unwrap_or(&rec.created).replace('T', " ");
                let second = if armed {
                    vec![Span::new(format!("  {}", tr!("tui-ai-delete-again")), Role::Danger)]
                } else {
                    vec![Span::new(format!("  {} \u{00b7} {}", safe(&file), safe(&created)), Role::Faint)]
                };
                row(&mut b, first, second);
            }
            Err(e) => {
                let first = vec![Span::new(format!("! {}", safe(&path.display().to_string())), Role::Warning)];
                let second = vec![Span::new(format!("  {}", safe(e)), Role::Danger)];
                row(&mut b, first, second);
            }
        }
    }
    b
}

/// The audit rows that pass the filters, newest first.
pub fn audit_rows(core: &AppCore) -> Vec<&AuditRecord> {
    let a = &core.ai.audit;
    let f = a.filter.to_lowercase();
    a.events
        .iter()
        .rev()
        .filter(|e| a.session.is_empty() || e.session == a.session)
        .filter(|e| a.source.is_empty() || audit::source_label(e) == a.source)
        .filter(|e| a.model.is_empty() || e.via == a.model)
        .filter(|e| {
            f.is_empty()
                || e.detail.to_lowercase().contains(&f)
                || e.args.to_lowercase().contains(&f)
                || e.event.contains(&f)
                || e.outcome.contains(&f)
                || e.tool.contains(&f)
                || e.client.to_lowercase().contains(&f)
        })
        .collect()
}

/// The audit log tab.
pub fn audit(core: &AppCore, _st: &State, w: usize) -> Built {
    let mut b = Built::default();
    let a = &core.ai.audit;
    if let Some(e) = &a.error {
        note(&mut b, w, &safe(e), Role::Danger);
    }
    if a.loading.is_some() && a.events.is_empty() {
        b.lines.push(AiLine::plain(tr!("common-loading"), Role::Muted));
        return b;
    }
    let rows = audit_rows(core);
    let count = tr!(
        "ai-audit-count",
        shown = rows.len(),
        total = a.events.len(),
        path = audit::default_path().display().to_string()
    );
    b.lines.push(AiLine::plain(text::truncate_end(&safe(&count), w), Role::Faint));
    for e in rows {
        let mcp = e.source == audit::SOURCE_MCP;
        let role = match (e.event.as_str(), e.outcome.as_str()) {
            (_, "rejected" | "refused" | "timed_out" | "cancelled") => Role::Muted,
            (_, "error") => Role::Danger,
            ("tool_call", "plan") | ("plan", _) | ("taint", _) | ("client_registered", _) => Role::Warning,
            ("approval", _) => Role::Success,
            _ => Role::Text,
        };
        let tool = if e.tool.is_empty() { String::new() } else { format!(" \u{00b7} {}", e.tool) };
        let outcome = if e.outcome.is_empty() || e.outcome == "ok" || e.outcome == e.event {
            String::new()
        } else {
            format!(" \u{2192} {}", e.outcome)
        };
        let job =
            e.job_id.map(|j| format!(" \u{00b7} {}", tr!("ai-audit-job", id = j.to_string()))).unwrap_or_default();
        let time = e.ts.get(11..19).unwrap_or(&e.ts).to_string();
        let head = safe(&format!("{time}  {}{tool}{outcome}{job}", e.event));
        let badge = if mcp {
            format!("MCP \u{00b7} {}", if e.client.is_empty() { "?" } else { &e.client })
        } else if e.client == "recipe" {
            tr!("ai-audit-badge-recipe").to_string()
        } else {
            tr!("ai-audit-badge-assistant").to_string()
        };
        let badge = safe(&badge);
        let room = w.saturating_sub(text::width(&badge) + 2);
        let first = vec![
            Span::new(text::fit(&text::truncate_end(&head, room), room), role),
            Span::new(format!(" {badge}"), if mcp { Role::Warning } else { Role::Accent }),
        ];
        let body = if !e.detail.is_empty() {
            e.detail.replace('\n', " ")
        } else if !e.args.is_empty() {
            e.args.clone()
        } else {
            e.via.clone()
        };
        let second =
            vec![Span::new(format!("  {}", text::truncate_end(&safe(&body), w.saturating_sub(2))), Role::Muted)];
        row(&mut b, first, second);
    }
    b
}
