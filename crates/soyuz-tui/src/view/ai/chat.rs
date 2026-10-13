//! The chat tab's lines: the approvals area of external MCP clients on top (their cards,
//! "Undo everything <client> did", Clear), then the conversation (your messages, the
//! assistant's markdown through the preview's renderer, tool rows, plan cards, notices,
//! the cost question, task ends with "Undo everything" and "Save as recipe") or, for an
//! empty chat, the ideas; and the footer above the composer (the smart search card, the
//! "N plans wait" bar, the slash / `@` popup, the mentions). Like the GUI's
//! `ui/ai/{chat,cards,mcp_area,pages}.rs`.

use soyuz_appcore::ai::agent::Finish;
use soyuz_appcore::ai::mcp::{clients_with_history, irreversible};
use soyuz_appcore::ai::panel::{is_rename, mention_candidates, plan_totals, tokens};
use soyuz_appcore::ai::state::{self, CardOrigin, CardState, ChatItem, PlanCard, TaskEnd, ToolStatus, UndoState};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::tr;

use super::{AiLine, Built, MAX_POPUP, PopupItem, Span};
use crate::dialog::safe;
use crate::hits::{AiButton, CardButton};
use crate::icons::{self, IconSet};
use crate::input::ai::{Chip, MCP_CLEAR, Part, Sel, State};
use crate::layout::HitTarget;
use crate::theme::Role;
use crate::view::preview::markdown;
use crate::view::text;
use crate::widgets::LineEdit;

/// Items of a plan shown while a card is folded.
pub const FOLDED_ITEMS: usize = 8;
/// Items shown at most while it is unfolded.
pub const MAX_ITEMS: usize = 200;

/// The empty chat's ideas: `(label, prompt)`.
pub fn ideas(core: &AppCore) -> Vec<(String, String)> {
    let mut v = vec![
        (tr!("ai-idea-organize").to_string(), "/organize".to_string()),
        (tr!("ai-idea-explain").to_string(), "/explain".to_string()),
        (tr!("ai-idea-dedupe").to_string(), "/dedupe".to_string()),
    ];
    if !core.ws.pane().selection.is_empty() {
        v.insert(0, (tr!("ai-idea-rename").to_string(), "/rename".to_string()));
    }
    v
}

/// Icon of a tool (the GUI's `cards::tool_style`).
pub fn tool_icon(tool: &str) -> (IconId, Role) {
    match tool {
        "copy" => (IconId::Copy, Role::Accent),
        "move" => (IconId::ArrowBendDownRight, Role::Accent),
        "rename" | "batch_rename" => (IconId::PencilSimple, Role::Info),
        "mkdir" => (IconId::FolderPlus, Role::Success),
        "create_file" => (IconId::FilePlus, Role::Success),
        "trash" => (IconId::Trash, Role::Danger),
        "delete_permanent" | "empty_trash" => (IconId::XCircle, Role::Danger),
        "compress" | "extract" => (IconId::Archive, Role::Archive),
        "set_attributes" => (IconId::Sliders, Role::Warning),
        "tag" => (IconId::Tag, Role::Accent),
        "sync_apply" => (IconId::ArrowsClockwise, Role::Warning),
        "restore_from_trash" | "undo" => (IconId::ArrowCounterClockwise, Role::Success),
        "run_custom_command" => (IconId::TerminalWindow, Role::Warning),
        _ => (IconId::Lightning, Role::Accent),
    }
}

/// The badge of a card state.
pub fn state_badge(state: &CardState) -> (String, Role) {
    match state {
        CardState::Pending => (tr!("ai-card-state-pending").into(), Role::Warning),
        CardState::Approved { auto: true } => (tr!("ai-card-state-auto").into(), Role::Accent),
        CardState::Approved { auto: false } => (tr!("ai-card-state-approved").into(), Role::Accent),
        CardState::Running(_) => (tr!("ai-card-state-running").into(), Role::Accent),
        CardState::Done { ok: true, .. } => (tr!("ai-card-state-done").into(), Role::Success),
        CardState::Done { ok: false, .. } => (tr!("ai-card-state-failed").into(), Role::Danger),
        CardState::Rejected => (tr!("ai-card-state-rejected").into(), Role::Muted),
        CardState::Edited => (tr!("ai-card-state-edited").into(), Role::Muted),
        CardState::Preview => (tr!("ai-card-state-preview").into(), Role::Muted),
        CardState::Withdrawn(why) => (safe(why), Role::Muted),
    }
}

/// Collects lines and the line ranges of the selectable items.
struct Out {
    b: Built,
    w: usize,
    set: IconSet,
}

impl Out {
    fn push(&mut self, l: AiLine) {
        self.b.lines.push(l);
    }

    /// Wrapped paragraph lines of `s` (`first` prefixes the first line, `indent` blanks
    /// the others), each with `target`.
    fn para(&mut self, first: Vec<Span>, s: &str, role: Role, target: Option<HitTarget>) {
        let lead = first.iter().map(Span::width).sum::<usize>();
        let w = self.w.saturating_sub(lead).max(1);
        for (k, line) in text::wrap(s, w).into_iter().enumerate() {
            let mut spans = if k == 0 {
                first.clone()
            } else {
                first
                    .iter()
                    .map(|sp| if sp.vline { sp.clone() } else { Span::new(" ".repeat(sp.width()), Role::Text) })
                    .collect()
            };
            spans.push(Span::new(line, role));
            self.push(AiLine { spans, target, sel: false });
        }
    }

    /// Buttons, wrapped onto as many lines as they need.
    fn buttons(&mut self, lead: Vec<Span>, buttons: Vec<Span>, target: Option<HitTarget>) {
        let lead_w = lead.iter().map(Span::width).sum::<usize>();
        let mut cur = lead.clone();
        let mut used = lead_w;
        for b in buttons {
            let bw = b.width();
            if used > lead_w && used + 1 + bw > self.w {
                self.push(AiLine { spans: std::mem::replace(&mut cur, lead.clone()), target, sel: false });
                used = lead_w;
            }
            if used > lead_w {
                cur.push(Span::new(" ", Role::Text));
                used += 1;
            }
            used += bw;
            cur.push(b);
        }
        if used > lead_w {
            self.push(AiLine { spans: cur, target, sel: false });
        }
    }

    fn sel(&mut self, s: Sel, start: usize, pending: bool) {
        let end = self.b.lines.len();
        if end > start {
            self.b.sels.push((s, start, end));
            if pending {
                self.b.first.get_or_insert(s);
                self.b.pending.push(s);
            }
        }
    }
}

/// The chat tab's body.
pub fn lines(core: &AppCore, _st: &State, w: usize, set: IconSet) -> Built {
    let mut o = Out { b: Built::default(), w: w.max(1), set };
    let area = mcp_area(core, &mut o);
    if core.ai.chat.is_empty() {
        if !area {
            empty(core, &mut o);
        }
        return o.b;
    }
    let running = core.ai.running.as_ref().map(|r| r.task);
    for item in &core.ai.chat {
        match item {
            ChatItem::User { text, mentions } => {
                o.push(AiLine::default());
                let mark = if set == IconSet::Ascii { "> " } else { "\u{203a} " };
                // Every line of a multi-line message is its own paragraph.
                for (k, line) in text.split('\n').enumerate() {
                    let lead = if k == 0 { mark } else { "  " };
                    o.para(vec![Span::new(lead, Role::Accent)], &safe(line), Role::Title, None);
                }
                if !mentions.is_empty() {
                    let names: Vec<String> = mentions
                        .iter()
                        .take(8)
                        .map(|m| format!("@{}", safe(&soyuz_core::entry::display_name(m))))
                        .collect();
                    let mut s = names.join(" ");
                    if mentions.len() > 8 {
                        s.push_str(&format!(" +{}", mentions.len() - 8));
                    }
                    o.para(vec![Span::new("  ", Role::Text)], &s, Role::Accent, None);
                }
            }
            ChatItem::Assistant { text, reasoning, streaming, .. } => assistant(&mut o, text, reasoning, *streaming),
            ChatItem::Tool { tool, detail, status, .. } => {
                let (mark, role, brief) = match status {
                    ToolStatus::Running => ("\u{2026}", Role::Info, detail.as_str()),
                    ToolStatus::Ok(b) => (icons::glyph(IconId::SealCheck, set), Role::Success, b.as_str()),
                    ToolStatus::Failed(b) => ("!", Role::Warning, b.as_str()),
                };
                let line = format!("{} {}", safe(tool), safe(&brief.replace('\n', " ")));
                o.push(AiLine::new(vec![
                    Span::new("  ", Role::Text),
                    Span::new(format!("{mark} "), role),
                    Span::new(text::truncate_end(&line, o.w.saturating_sub(4)), Role::Muted),
                ]));
            }
            ChatItem::Plan(card) => {
                let start = o.b.lines.len();
                let live = running == Some(card.task);
                let sel = Sel::Plan { id: card.id, task: card.task.0 };
                card_lines(&mut o, card, live, sel);
                o.sel(sel, start, live && card.state == CardState::Pending);
            }
            ChatItem::Notice { text, warn } => {
                let (mark, role) = if *warn { ("! ", Role::Warning) } else { ("i ", Role::Muted) };
                o.para(vec![Span::new("  ", Role::Text), Span::new(mark, role)], &safe(text), role, None);
            }
            ChatItem::Confirm { estimate, model, .. } => {
                let start = o.b.lines.len();
                confirm(&mut o, *estimate, model);
                o.sel(Sel::Confirm, start, true);
            }
            ChatItem::TaskEnd(t) => {
                let start = o.b.lines.len();
                task_end(&mut o, t);
                o.sel(Sel::End(t.task.0), start, false);
            }
        }
    }
    o.b
}

/// The approvals area (`[ai.mcp] approval_ui = "panel"`); whether it showed anything.
fn mcp_area(core: &AppCore, o: &mut Out) -> bool {
    let a = core.services.mcp.approvals.lock();
    let undo = clients_with_history(core.fileops.jobs.history());
    if a.cards.is_empty() && undo.is_empty() {
        return false;
    }
    let waiting = a.pending();
    let mut head = vec![
        Span::bold(format!("{} ", icons::glyph(IconId::Lightning, o.set)), Role::Warning),
        Span::bold(tr!("ai-mcp-title"), Role::Title),
        Span::new(" \u{00b7} ", Role::Faint),
        if waiting > 0 {
            Span::new(tr!("ai-mcp-waiting", count = waiting), Role::Warning)
        } else {
            Span::new(tr!("ai-mcp-clients"), Role::Muted)
        },
    ];
    if a.cards.iter().any(|c| c.resolved.is_some()) {
        head.push(Span::new("  ", Role::Text));
        head.push(Span::button(tr!("ai-mcp-clear"), HitTarget::AiMcpUndo(MCP_CLEAR), true));
    }
    o.push(AiLine::new(head));
    if !undo.is_empty() {
        let buttons = undo
            .iter()
            .enumerate()
            .map(|(i, (client, _))| {
                let busy = core.ai.mcp_undo_running(client);
                let label =
                    if busy { tr!("ai-undoing").to_string() } else { tr!("ai-mcp-undo-client", client = safe(client)) };
                Span::button(&label, HitTarget::AiMcpUndo(i), !busy)
            })
            .collect();
        o.buttons(vec![Span::new("  ", Role::Text)], buttons, None);
    }
    // Waiting ones first (oldest first), then the finished ones.
    let mut order: Vec<usize> = (0..a.cards.len()).collect();
    order.sort_by_key(|&i| (a.cards[i].resolved.is_some(), a.cards[i].card.id));
    for i in order {
        let c = &a.cards[i];
        let start = o.b.lines.len();
        let sel = Sel::Mcp(c.card.id);
        card_lines(o, &c.card, true, sel);
        o.sel(sel, start, c.is_pending());
    }
    let line = "\u{2500}".repeat(o.w.min(60));
    o.push(AiLine::plain(line, Role::Faint));
    true
}

fn empty(core: &AppCore, o: &mut Out) {
    o.push(AiLine::default());
    o.para(
        vec![Span::bold(format!("{} ", icons::glyph(IconId::Sparkle, o.set)), Role::Accent)],
        tr!("ai-empty-title"),
        Role::Title,
        None,
    );
    o.para(vec![Span::new("  ", Role::Text)], tr!("ai-empty-body"), Role::Muted, None);
    o.push(AiLine::default());
    for (i, (label, prompt)) in ideas(core).into_iter().enumerate() {
        let start = o.b.lines.len();
        let t = Some(HitTarget::AiRow(i));
        let room = o.w.saturating_sub(text::width(&prompt) + 6);
        o.push(AiLine {
            spans: vec![
                Span::new("  ", Role::Text),
                Span::new(text::fit(label.as_str(), room), Role::Text),
                Span::new("  ", Role::Text),
                Span::new(prompt, Role::Faint),
            ],
            target: t,
            sel: false,
        });
        o.sel(Sel::Idea(i), start, false);
    }
}

fn assistant(o: &mut Out, text_: &str, reasoning: &str, streaming: bool) {
    o.push(AiLine::default());
    let spark = format!("{} ", icons::glyph(IconId::Sparkle, o.set));
    if !reasoning.trim().is_empty() {
        let r = safe(&reasoning.trim().replace('\n', " "));
        let s = format!("{}: {r}", tr!("ai-thinking"));
        let mut sp = Span::new(text::truncate_end(&s, o.w.saturating_sub(2)), Role::Faint);
        sp.seg.italic = true;
        o.push(AiLine::new(vec![Span::new("  ", Role::Text), sp]));
    }
    if text_.trim().is_empty() {
        if streaming {
            o.push(AiLine::new(vec![Span::new(spark, Role::Accent), Span::new("\u{2026}", Role::Faint)]));
        }
        return;
    }
    let blocks = soyuz_appcore::services::preview_text::parse_markdown(text_);
    let md = markdown(&blocks, o.w.saturating_sub(2).max(1));
    for (k, l) in md.into_iter().enumerate() {
        let mut spans = vec![if k == 0 { Span::new(spark.clone(), Role::Accent) } else { Span::new("  ", Role::Text) }];
        spans.extend(l.segs.into_iter().map(|seg| Span { seg, target: None, vline: false }));
        o.push(AiLine::new(spans));
    }
    if streaming {
        o.push(AiLine::new(vec![Span::new("  ", Role::Text), Span::new("\u{2026}", Role::Faint)]));
    }
}

/// A plan card (the assistant's or an external client's).
fn card_lines(o: &mut Out, card: &PlanCard, live: bool, sel: Sel) {
    let t = Some(sel.target());
    let pending = card.state == CardState::Pending && live;
    let destructive = format!("{:?}", card.plan.risk) == "Destructive";
    let gutter_role =
        if pending { if card.tainted || destructive { Role::Warning } else { Role::Accent } } else { Role::Border };
    let g = || vec![Span::gutter(gutter_role)];
    o.push(AiLine::default());
    // The external client strip.
    if let CardOrigin::Mcp { client, via } = &card.origin {
        let mut s = format!(
            "{}: {} \u{00b7} {}",
            tr!("ai-card-external"),
            safe(client),
            tr!("ai-card-via", transport = safe(via))
        );
        if card.state == CardState::Pending
            && let Some(exp) = card.expires
        {
            let left = exp.saturating_duration_since(std::time::Instant::now()).as_secs();
            s.push_str(" \u{00b7} ");
            s.push_str(&tr!("ai-card-left", time = format!("{}:{:02}", left / 60, left % 60)));
        }
        o.para(g(), &s, Role::Warning, t);
    }
    // Header: icon, summary; then tool · totals · state.
    let (icon, role) = tool_icon(&card.tool);
    let summary = safe(card.plan.summary.trim_start_matches("AI: "));
    let mut first = g();
    first.push(Span::bold(format!("{} ", icons::glyph(icon, o.set)), role));
    o.para(first, &summary, Role::Title, t);
    let (badge, badge_role) = state_badge(&card.state);
    let mut sub = g();
    sub.push(Span::new("  ", Role::Text));
    let info = format!("{} \u{00b7} {} \u{00b7} ", safe(&card.tool), plan_totals(&card.plan));
    sub.push(Span::new(info, Role::Muted));
    sub.push(Span::new(badge, badge_role));
    o.push(AiLine { spans: sub, target: t, sel: false });
    // Risk, taint and warnings.
    let mut banners: Vec<(String, Role)> = Vec::new();
    if destructive && card.state != CardState::Preview {
        banners.push((tr!("ai-card-destructive").into(), Role::Danger));
    }
    if card.origin != CardOrigin::Assistant && card.state == CardState::Pending {
        banners.push((tr!("ai-card-external-warning").into(), Role::Warning));
    }
    if card.tainted && card.state != CardState::Preview {
        banners.push((tr!("ai-card-tainted").into(), Role::Warning));
    }
    for w in &card.plan.warnings {
        banners.push((safe(w), Role::Warning));
    }
    if !card.plan.executable && card.state == CardState::Pending {
        banners.push((tr!("ai-card-blocked").into(), Role::Danger));
    }
    for (b, r) in banners {
        let mut lead = g();
        lead.push(Span::new("! ", r));
        o.para(lead, &b, r, t);
    }
    // Items: the first few while folded, all (up to a cap) while unfolded.
    let n = card.plan.items.len();
    let shown = if card.expanded { n.min(MAX_ITEMS) } else { n.min(FOLDED_ITEMS) };
    for i in &card.plan.items[..shown] {
        let row = crate::view::dialogs::approval_mcp::row(
            &format!("{:?}", i.op),
            i.src.as_deref(),
            i.dst.as_deref(),
            i.conflict.is_some(),
        );
        let right_w = text::width(&row.right);
        let room = o.w.saturating_sub(right_w + 6);
        let mut spans = g();
        spans.push(Span::new("  ", Role::Text));
        spans.push(Span::new(text::fit(&text::truncate_middle(&row.text, room), room), Role::Text));
        spans.push(Span::new(" ", Role::Text));
        spans.push(Span::new(row.right, row.role.unwrap_or(Role::Muted)));
        o.push(AiLine { spans, target: t, sel: false });
    }
    if n > shown {
        let mut spans = g();
        spans.push(Span::new("  ", Role::Text));
        let more = tr!("ai-diff-more", count = n - shown);
        spans.push(Span::new(more, Role::Faint));
        if !card.expanded {
            spans.push(Span::new("  ", Role::Text));
            spans.push(Span::button(tr!("tui-ai-show-all"), sel.button(CardButton::Expand), true));
        }
        o.push(AiLine { spans, target: t, sel: false });
    }
    if let CardState::Done { message, ok } = &card.state {
        o.para(g(), &safe(message), if *ok { Role::Success } else { Role::Danger }, t);
    }
    // Buttons.
    if pending {
        let can = card.plan.executable;
        let mut b = vec![Span::button(tr!("ai-card-approve"), sel.button(CardButton::Approve), can)];
        let label = if is_rename(&card.tool) && !card.plan.items.is_empty() {
            tr!("tui-ai-edit-names")
        } else {
            tr!("tui-ai-edit-json")
        };
        b.push(Span::button(label, sel.button(CardButton::Edit), true));
        b.push(Span::button(tr!("ai-card-reject"), sel.button(CardButton::Reject), true));
        let (label, bulk) = match &card.origin {
            CardOrigin::Assistant => (tr!("ai-card-approve-kind", tool = safe(&card.tool)), true),
            CardOrigin::Mcp { client, .. } => (
                tr!("ai-card-approve-kind-client", tool = safe(&card.tool), client = safe(client)),
                !irreversible(&card.tool),
            ),
        };
        if bulk {
            b.push(Span::button(&label, sel.button(CardButton::ApproveKind), can));
        }
        o.buttons(g(), b, t);
    }
}

fn confirm(o: &mut Out, estimate: f64, model: &str) {
    let t = Some(Sel::Confirm.target());
    o.push(AiLine::default());
    let g = || vec![Span::gutter(Role::Warning)];
    let cost = tr!("ai-confirm-cost", cost = soyuz_core::locale::format_decimal(estimate, 2));
    let mut first = g();
    first.push(Span::new("$ ", Role::Warning));
    o.para(first, &cost, Role::Title, t);
    o.para(g(), &tr!("ai-confirm-body", model = safe(model)), Role::Muted, t);
    let b = vec![
        Span::button(tr!("ai-start"), Sel::Confirm.button(CardButton::Go), true),
        Span::button(tr!("common-cancel"), Sel::Confirm.button(CardButton::Cancel), true),
    ];
    o.buttons(g(), b, t);
}

fn task_end(o: &mut Out, t: &TaskEnd) {
    let sel = Sel::End(t.task.0);
    let target = Some(sel.target());
    let role = match &t.reason {
        Finish::Done => Role::Success,
        Finish::Cancelled => Role::Muted,
        Finish::Error(_) => Role::Danger,
        _ => Role::Warning,
    };
    let changes = t.executed.len();
    let mut parts = vec![t.reason.label()];
    if changes > 0 {
        parts.push(tr!("ai-end-changes", count = changes));
    }
    if t.usage.total() > 0 {
        parts.push(tr!("ai-end-tokens", tokens = tokens(t.usage.total())));
    }
    if let Some(c) = t.cost {
        parts.push(tr!("ai-end-cost", cost = soyuz_core::locale::format_decimal(c, 3)));
    }
    let label = safe(&parts.join(" \u{00b7} "));
    let lw = text::width(&label) + 2;
    let side = o.w.saturating_sub(lw) / 2;
    let rule = |n: usize| "\u{2500}".repeat(n.min(30));
    o.push(AiLine::default());
    if o.w > lw + 2 {
        o.push(AiLine {
            spans: vec![
                Span::new(rule(side), Role::Faint),
                Span::new(format!(" {label} "), role),
                Span::new(rule(o.w.saturating_sub(side + lw)), Role::Faint),
            ],
            target,
            sel: false,
        });
    } else {
        o.para(Vec::new(), &label, role, target);
    }
    let mut b: Vec<Span> = Vec::new();
    if let Finish::Error(e) = &t.reason
        && (e.contains("Settings") || e.contains(tr!("ai-settings-word")))
    {
        b.push(Span::button(tr!("ai-open-settings"), HitTarget::AiButton(AiButton::Settings), true));
    }
    if changes > 0 {
        match &t.undo {
            UndoState::None => b.push(Span::button(tr!("ai-undo-all"), sel.button(CardButton::Undo), true)),
            UndoState::Running => b.push(Span::button(tr!("ai-undoing"), sel.button(CardButton::Undo), false)),
            UndoState::Done(_) => {}
        }
        if crate::input::ai::can_save(t) {
            b.push(Span::button(tr!("ai-save-recipe"), sel.button(CardButton::SaveRecipe), true));
        }
    }
    o.buttons(vec![Span::new("  ", Role::Text)], b, target);
    if let UndoState::Done(msg) = &t.undo {
        o.para(vec![Span::new("  ", Role::Text)], &safe(msg), Role::Muted, target);
    }
    if let Some(s) = &t.saved {
        o.para(vec![Span::new("  ", Role::Text)], &safe(s), Role::Success, target);
    }
}

/// The lines above the composer, what the popup rows pick and the text cursor of the
/// smart search card's field (line, column).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Footer {
    pub lines: Vec<AiLine>,
    pub popup: Vec<PopupItem>,
    pub cursor: Option<(usize, u16)>,
}

/// The footer: the smart search card, the "N plans wait" bar, the slash / `@` popup and
/// the mentions of the next message.
pub fn footer(core: &AppCore, st: &State, w: usize, set: IconSet) -> Footer {
    let mut f = Footer::default();
    if core.ai.smart.open {
        f.cursor = smart_card(core, st, w, set, &mut f.lines);
    }
    let out = &mut f.lines;
    let popup = &mut f.popup;
    // "N plans wait for your approval  [Approve all]".
    if let Some(r) = &core.ai.running {
        let pending: Vec<bool> = core
            .ai
            .chat
            .iter()
            .filter_map(|c| match c {
                ChatItem::Plan(x) if x.task == r.task && x.state == CardState::Pending => Some(x.plan.executable),
                _ => None,
            })
            .collect();
        if pending.len() > 1 {
            let label = tr!("ai-pending-plans", count = pending.len());
            let button = Span::button(
                tr!("ai-approve-all"),
                HitTarget::AiButton(AiButton::ApproveAll),
                pending.iter().any(|e| *e),
            );
            let room = w.saturating_sub(button.width() + 4);
            out.push(AiLine::new(vec![
                Span::new(format!("{} ", icons::glyph(IconId::Hourglass, set)), Role::Warning),
                Span::new(text::fit(&text::truncate_end(&label, room), room), Role::Warning),
                Span::new(" ", Role::Text),
                button,
            ]));
        }
    }
    // The popup: slash commands or `@` mentions, while the composer has the keyboard.
    let composer = &core.ai.panel.composer;
    let typing = st.focused && st.part == Part::Composer && st.popup_closed.as_deref() != Some(composer.as_str());
    if typing {
        let last = composer.rsplit(char::is_whitespace).next().unwrap_or("");
        if composer.starts_with('/') && !composer.contains(char::is_whitespace) {
            let low = composer.to_lowercase();
            for (c, _, _) in state::SLASH.iter().filter(|(c, _, _)| c.starts_with(&low)).take(MAX_POPUP) {
                popup.push(PopupItem::Slash((*c).to_string()));
                let desc = state::slash_description(c);
                out.push(popup_line(popup.len() - 1, c, desc, st.popup, w));
            }
        } else if let Some(q) = last.strip_prefix('@') {
            for m in mention_candidates(core, q).into_iter().take(MAX_POPUP) {
                let icon = icons::glyph(m.icon, set);
                let label = format!("{icon} {}", safe(&m.label));
                popup.push(PopupItem::Mention(m.paths));
                out.push(popup_line(popup.len() - 1, &label, "", st.popup, w));
            }
        }
    }
    // The mentions of the next message.
    let mentions = &core.ai.panel.mentions;
    if !mentions.is_empty() {
        let names: Vec<String> =
            mentions.iter().map(|m| format!("@{}", safe(&soyuz_core::entry::display_name(m)))).collect();
        out.push(AiLine::new(vec![Span::new(text::truncate_end(&names.join(" "), w), Role::Accent)]));
    }
    f
}

/// One field of the smart search card.
struct SmartField<'a> {
    lead: &'a str,
    value: &'a str,
    edit: &'a LineEdit,
    placeholder: &'a str,
    focused: bool,
    target: HitTarget,
}

impl SmartField<'_> {
    /// `lead` then the visible text (or the placeholder), the whole line a click target;
    /// the cursor's column while it has the keyboard.
    fn line(&self, w: usize) -> (AiLine, Option<u16>) {
        let lead_w = text::width(self.lead);
        let room = w.saturating_sub(lead_w).max(1) as u16;
        let mut e = self.edit.clone();
        let (shown, col) = e.view(self.value, room);
        let mut spans = vec![Span::new(self.lead, Role::Accent)];
        if self.value.is_empty() && !self.focused {
            spans.push(Span::new(text::truncate_end(self.placeholder, usize::from(room)), Role::Faint));
        } else {
            spans.push(Span::new(safe(&shown), Role::Input));
        }
        let line = AiLine { spans, target: Some(self.target), sel: false };
        (line, self.focused.then_some(lead_w as u16 + col))
    }
}

/// The smart search card (the GUI's `pages::smart_card`): plain words, the filters they
/// became (click one to remove it), the query (editable), the error, Search and Ask the
/// assistant. Returns the text cursor while one of its fields has the keyboard.
fn smart_card(core: &AppCore, st: &State, w: usize, set: IconSet, out: &mut Vec<AiLine>) -> Option<(usize, u16)> {
    let s = &core.ai.smart;
    let has = st.focused && st.part == Part::Smart && st.menu.is_none();
    let mut cursor = None;
    // Title and close.
    let close = Span::button(icons::glyph(IconId::X, set), Chip::SmartClose.target(), true);
    let title = format!("{} {}", icons::glyph(IconId::MagnifyingGlass, set), tr!("ai-smart-title"));
    let room = w.saturating_sub(close.width() + 1);
    out.push(AiLine::new(vec![
        Span::bold(text::fit(&text::truncate_end(&title, room), room), Role::Accent),
        Span::new(" ", Role::Text),
        close,
    ]));
    // The plain words.
    let words = SmartField {
        lead: "> ",
        value: &s.input,
        edit: &st.smart_edit,
        placeholder: tr!("ai-smart-hint"),
        focused: has && st.smart_field == 0,
        target: Chip::SmartInput.target(),
    };
    let (line, col) = words.line(w);
    if let Some(c) = col {
        cursor = Some((out.len(), c));
    }
    out.push(line);
    // The filters.
    if s.pills.is_empty() {
        out.push(AiLine::plain(text::truncate_end(tr!("ai-smart-empty"), w), Role::Muted));
    } else {
        let x = if set == IconSet::Ascii { "x" } else { "\u{00d7}" };
        let pills = s
            .pills
            .iter()
            .enumerate()
            .take(usize::from(u8::MAX))
            .map(|(i, p)| {
                Span::button(&format!("{}:{} {x}", p.key.word(), safe(&p.value)), Chip::Pill(i as u8).target(), true)
            })
            .collect();
        out.extend(super::flow(pills, w, 2));
    }
    // The query.
    let lead = format!("{}: ", tr!("ai-smart-query"));
    let query = SmartField {
        lead: &lead,
        value: &s.query_text,
        edit: &st.query_edit,
        placeholder: "",
        focused: has && st.smart_field == 1,
        target: Chip::SmartQuery.target(),
    };
    let (line, col) = query.line(w);
    if let Some(c) = col {
        cursor = Some((out.len(), c));
    }
    out.push(line);
    if let Some(e) = &s.error {
        out.push(AiLine::plain(text::truncate_end(&safe(e), w), Role::Danger));
    }
    let buttons = vec![
        Span::button(tr!("ai-smart-search"), Chip::SmartRun.target(), !s.query_text.trim().is_empty()),
        Span::button(
            tr!("ai-smart-ask"),
            Chip::SmartAsk.target(),
            !s.input.trim().is_empty() && core.ai.running.is_none(),
        ),
    ];
    out.extend(super::flow(buttons, w, 2));
    out.push(AiLine::plain("\u{2500}".repeat(w.min(60)), Role::Faint));
    cursor
}

fn popup_line(i: usize, label: &str, desc: &str, cursor: usize, w: usize) -> AiLine {
    let lw = text::width(label);
    let mut spans = vec![Span::new(format!(" {label}"), Role::Accent)];
    if !desc.is_empty() && w > lw + 4 {
        spans.push(Span::new(format!("  {}", text::truncate_end(desc, w - lw - 4)), Role::Muted));
    }
    AiLine { spans, target: Some(HitTarget::AiMention(i)), sel: i == cursor }
}
