//! The view of the AI assistant panel and its MCP approval cards (Phase T5): drawn into
//! `Regions::ai` (right of the body at `[tui] ai_ratio`, or the whole body below 100
//! columns), after the GUI's panel (`soyuz-app/src/ui/ai`):
//!
//! ```text
//! │ * Assistant · Waiting for your approval      [+] [*] [x]
//! │ fake · move · Strong · 1.2k tok               [Stop Esc]
//! │  Chat   Recipes   Audit log
//! │ > move a.txt
//! │ * I suggest moving a.txt.
//! │ | -> Move 1 item into Moved
//! │ |    move · 1 item · Waiting for you
//! │ |    data/a.txt -> data/Moved/a.txt            moved
//! │ |    [Approve] [Reject] [Approve all "move"]
//! │ ... 2 plans wait for your approval     [Approve all]
//! │ > Ask or give a task…   / commands · @ mention
//! ```
//!
//! Rows top to bottom ([`parts`]): the header (title, status, new chat / settings / close
//! marks), the info row (profile · model, route, tokens and cost, Stop while a task
//! runs), the tabs (chat, recipes, audit log), the body, the footer (the "N plans wait"
//! bar, the slash / `@` popup, the mentions), the field (the composer on the chat tab,
//! the filter on the audit tab) and, while the panel has the keyboard, the key hints.
//! Small panels drop the info row, the tabs and the hints first.
//!
//! The body is built in [`prepare`] (`chat::lines` for the chat tab with the MCP
//! approvals area on top, `pages::recipes` / `pages::audit` for the others) into
//! `input::ai::State::view` together with the line ranges of every selectable item (plan
//! cards, MCP cards, the cost question, task ends, the empty chat's ideas, list rows), so
//! the keyboard can keep the selection in sight; [`AiVm::from_core`] takes the visible
//! window. Everything the model, the files or an MCP client wrote is untrusted and passes
//! through [`crate::dialog::safe`].
//!
//! Click targets: the whole panel is `AiChat` (wheel scrolls, a click gives the panel the
//! keyboard); `AiTab`, `AiButton` (header, Stop, Approve all), `AiCard` (every line of a
//! card, the cost question, a task end), `AiCardButton` (the card buttons), `AiRow`
//! (ideas, recipes, audit rows), `AiMention` (popup rows), `AiMcpUndo` ("Undo everything
//! <client> did"; [`crate::input::ai::MCP_CLEAR`] is the approvals area's Clear) and
//! `AiComposer`.

pub mod chat;
pub mod pages;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use soyuz_appcore::ai::model::{self, Route};
use soyuz_appcore::ai::panel::tokens;
use soyuz_appcore::ai::state::{CardState, ChatItem, PanelTab};
use soyuz_appcore::color::Rgba8;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::tr;

use crate::dialog::safe;
use crate::hits::AiButton;
use crate::icons::{self, IconSet};
use crate::input::InputState;
use crate::input::ai::{Part, Sel};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme, to_color};
use crate::view::preview::Seg;
use crate::view::text;

/// Footer rows at most (popup rows included).
pub const MAX_FOOTER: u16 = 8;
/// Rows of the slash / mention popup at most.
pub const MAX_POPUP: usize = 6;
/// The hints row shows from this panel height.
pub const HINTS_MIN_ROWS: u16 = 12;

/// A piece of a line: styled text, maybe a click target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub seg: Seg,
    pub target: Option<HitTarget>,
    /// Draw the theme's vertical line instead of the text (card gutters).
    pub vline: bool,
}

impl Span {
    pub fn new(text: impl Into<String>, role: Role) -> Span {
        Span { seg: Seg::new(text, role), ..Span::default() }
    }

    pub fn bold(text: impl Into<String>, role: Role) -> Span {
        let mut s = Span::new(text, role);
        s.seg.bold = true;
        s
    }

    /// `[label]`, a target while enabled.
    pub fn button(label: &str, target: HitTarget, enabled: bool) -> Span {
        Span {
            seg: Seg::new(format!("[{label}]"), if enabled { Role::Button } else { Role::Disabled }),
            target: enabled.then_some(target),
            vline: false,
        }
    }

    /// A card gutter (`│ `).
    pub fn gutter(role: Role) -> Span {
        Span { seg: Seg::new("  ", role), vline: true, ..Span::default() }
    }

    pub fn width(&self) -> usize {
        if self.vline { 2 } else { text::width(&self.seg.text) }
    }
}

/// One line of the panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiLine {
    pub spans: Vec<Span>,
    /// The whole line is this target (spans with their own target win).
    pub target: Option<HitTarget>,
    /// Drawn as the selected row.
    pub sel: bool,
}

impl AiLine {
    pub fn new(spans: Vec<Span>) -> AiLine {
        AiLine { spans, ..AiLine::default() }
    }

    pub fn plain(text: impl Into<String>, role: Role) -> AiLine {
        AiLine::new(vec![Span::new(text, role)])
    }

    pub fn with_target(mut self, t: HitTarget) -> AiLine {
        self.target = Some(t);
        self
    }

    /// The line's text (tests).
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| if s.vline { "| " } else { s.seg.text.as_str() }).collect()
    }
}

/// What a slash / mention popup row picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopupItem {
    Slash(String),
    Mention(Vec<std::path::PathBuf>),
}

/// The panel's body as built by [`prepare`] (kept in `input::ai::State::view`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Built {
    /// Every line of the body (the visible window is `top .. top + body_h`).
    pub lines: Vec<AiLine>,
    /// Line ranges of the selectable items, in order.
    pub sels: Vec<(Sel, usize, usize)>,
    /// The item that waits for an answer first (selected when nothing is).
    pub first: Option<Sel>,
    /// Every item that waits for an answer.
    pub pending: Vec<Sel>,
    /// Line ranges of the list rows (recipes, audit).
    pub rows: Vec<(usize, usize)>,
    pub footer: Vec<AiLine>,
    /// What the popup rows pick (`AiMention(i)`).
    pub popup: Vec<PopupItem>,
    pub body_h: usize,
    pub top: usize,
}

impl Built {
    /// The item `sel` is still shown.
    pub fn has(&self, sel: Sel) -> bool {
        self.sels.iter().any(|(s, ..)| *s == sel)
    }
}

/// The composer / filter row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldVm {
    pub prefix: String,
    /// The visible part of the text.
    pub text: String,
    pub placeholder: String,
    /// The cursor column in `text` while the field has the keyboard.
    pub cursor: Option<u16>,
    pub target: Option<HitTarget>,
}

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiVm {
    pub visible: bool,
    /// The panel has the keyboard.
    pub focused: bool,
    pub title: String,
    pub status: String,
    pub running: bool,
    /// Profile · model, route, tokens and cost.
    pub info: String,
    /// "Stop  Esc" while a task runs.
    pub stop: Option<String>,
    pub new_chat: bool,
    pub tabs: Vec<String>,
    pub tab: usize,
    /// The visible body lines.
    pub lines: Vec<AiLine>,
    pub footer: Vec<AiLine>,
    pub field: Option<FieldVm>,
    pub hints: String,
}

impl AiVm {
    pub fn from_core(core: &AppCore, input: &InputState, regions: &Regions) -> AiVm {
        if !shown(core) || regions.ai.is_empty() {
            return AiVm::default();
        }
        let st = &input.ai;
        let ai = &core.ai;
        let b = &st.view;
        let end = (b.top + b.body_h).min(b.lines.len());
        let lines = b.lines.get(b.top.min(end)..end).map(<[AiLine]>::to_vec).unwrap_or_default();
        let tab = match ai.panel.tab {
            PanelTab::Chat => 0,
            PanelTab::Recipes => 1,
            PanelTab::Audit => 2,
        };
        let parts = parts(regions.ai, b.footer.len() as u16, has_field(core), st.focused);
        AiVm {
            visible: true,
            focused: st.focused,
            title: tr!("ai-panel-title").into(),
            status: safe(&status(core)),
            running: ai.running.is_some(),
            info: safe(&info(core)),
            stop: ai.running.is_some().then(|| tr!("ai-stop-esc").to_string()),
            new_chat: ai.running.is_none(),
            tabs: vec![tr!("ai-tab-chat").into(), tr!("ai-tab-recipes").into(), tr!("ai-tab-audit").into()],
            tab,
            lines,
            footer: b.footer.clone(),
            field: field(core, input, parts.field.width),
            hints: if st.focused { hints(core, input) } else { String::new() },
        }
    }
}

/// It is drawn this frame.
pub fn shown(core: &AppCore) -> bool {
    core.ai.panel.open
}

/// The field row: the composer (chat) or the filter (audit).
fn has_field(core: &AppCore) -> bool {
    matches!(core.ai.panel.tab, PanelTab::Chat | PanelTab::Audit)
}

/// "Working · 3s", "Waiting for your approval", the external requests, idle.
pub fn status(core: &AppCore) -> String {
    let ai = &core.ai;
    match &ai.running {
        Some(r) if r.recipe => tr!("ai-status-recipe").into(),
        Some(r) => {
            let waiting = ai
                .chat
                .iter()
                .any(|c| matches!(c, ChatItem::Plan(x) if x.task == r.task && x.state == CardState::Pending));
            if waiting {
                tr!("ai-status-waiting").into()
            } else {
                tr!("ai-status-working", secs = r.started.elapsed().as_secs())
            }
        }
        None => match core.services.mcp.approvals.lock().pending() {
            0 => tr!("ai-status-idle").into(),
            n => tr!("ai-status-external", count = n),
        },
    }
}

/// "profile · model · Strong · 1.2k tok · $0.010".
pub fn info(core: &AppCore) -> String {
    let ai = &core.ai;
    let route = ai.panel.route;
    let label = match &ai.panel.profile_override {
        Some(name) => core.config.ai.profile(name).map(|p| (p.name.clone(), p.model.clone())),
        None => model::route_label(&core.config.ai, route),
    };
    let mut parts = vec![match &label {
        Some((prof, m)) if m.is_empty() => prof.clone(),
        Some((prof, m)) => format!("{prof} \u{00b7} {m}"),
        None => tr!("ai-no-model").into(),
    }];
    if ai.panel.profile_override.is_none() {
        parts.push(match route {
            Route::Fast => tr!("ai-route-fast").into(),
            Route::Strong => tr!("ai-route-strong").into(),
        });
    }
    let (usage, cost) = match &ai.running {
        Some(r) => (r.usage, r.cost),
        None => (ai.session_usage, (ai.session_cost > 0.0).then_some(ai.session_cost)),
    };
    parts.push(match cost {
        Some(c) => {
            tr!("ai-meter-tokens-cost", tokens = tokens(usage.total()), cost = soyuz_core::locale::format_decimal(c, 3))
        }
        None => tr!("ai-meter-tokens", tokens = tokens(usage.total())),
    });
    parts.join(" \u{00b7} ")
}

fn hints(core: &AppCore, input: &InputState) -> String {
    let st = &input.ai;
    match (core.ai.panel.tab, st.part) {
        (PanelTab::Chat, Part::Composer) => tr!("tui-ai-hints-composer").into(),
        (PanelTab::Chat, _) => tr!("tui-ai-hints-cards").into(),
        (PanelTab::Recipes, _) if st.delete_armed.is_some() => tr!("tui-ai-delete-again").into(),
        (PanelTab::Recipes, _) => tr!("tui-ai-hints-recipes").into(),
        (PanelTab::Audit, Part::Filter) => tr!("tui-ai-hints-filter").into(),
        (PanelTab::Audit, _) => tr!("tui-ai-hints-audit").into(),
    }
}

fn field(core: &AppCore, input: &InputState, width: u16) -> Option<FieldVm> {
    let st = &input.ai;
    let prefix = "> ".to_string();
    let w = width.saturating_sub(text::width(&prefix) as u16);
    match core.ai.panel.tab {
        PanelTab::Chat => {
            let text = &core.ai.panel.composer;
            let has = st.focused && st.part == Part::Composer;
            let mut edit = st.edit.clone();
            let (shown, col) = edit.view(text, w);
            Some(FieldVm {
                prefix,
                text: safe(&shown),
                placeholder: if core.ai.running.is_some() {
                    tr!("ai-composer-busy").into()
                } else {
                    tr!("ai-composer-hint").into()
                },
                cursor: has.then_some(col),
                target: Some(HitTarget::AiComposer),
            })
        }
        PanelTab::Audit => {
            let text = &core.ai.audit.filter;
            let has = st.focused && st.part == Part::Filter;
            let mut edit = st.filter_edit.clone();
            let (shown, col) = edit.view(text, w);
            Some(FieldVm {
                prefix: "/ ".into(),
                text: safe(&shown),
                placeholder: tr!("ai-audit-filter-hint").into(),
                cursor: has.then_some(col),
                target: Some(HitTarget::AiComposer),
            })
        }
        PanelTab::Recipes => None,
    }
}

/// The panel's rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    /// The panel without its left line.
    pub inner: Rect,
    pub header: Rect,
    pub info: Rect,
    pub tabs: Rect,
    pub body: Rect,
    pub footer: Rect,
    pub field: Rect,
    pub hints: Rect,
}

/// Split the panel `area` (`footer` rows wanted, a field row, the hints row while
/// focused). Small panels drop the info row, the tabs and the hints first; the body
/// keeps at least one row while there is room.
pub fn parts(area: Rect, footer: u16, field: bool, hints: bool) -> Parts {
    let mut p = Parts::default();
    if area.is_empty() {
        return p;
    }
    // A left line separates the panel from the files.
    let inner = if area.width >= 3 { Rect { x: area.x + 1, width: area.width - 1, ..area } } else { area };
    p.inner = inner;
    let row = |y: u16| Rect { y, height: 1, ..inner };
    let h = inner.height;
    let mut top = inner.y;
    let mut bottom = inner.bottom();
    let take_top = |top: &mut u16, bottom: u16| -> Rect {
        if *top < bottom {
            let r = row(*top);
            *top += 1;
            r
        } else {
            Rect::default()
        }
    };
    p.header = take_top(&mut top, bottom);
    if h >= 7 {
        p.info = take_top(&mut top, bottom);
    }
    if h >= 5 {
        p.tabs = take_top(&mut top, bottom);
    }
    if hints && h >= HINTS_MIN_ROWS && bottom > top + 1 {
        bottom -= 1;
        p.hints = row(bottom);
    }
    if field && bottom > top + 1 {
        bottom -= 1;
        p.field = row(bottom);
    }
    let room = bottom.saturating_sub(top);
    let f = footer.min(MAX_FOOTER).min(room / 2);
    if f > 0 {
        bottom -= f;
        p.footer = Rect { y: bottom, height: f, ..inner };
    }
    p.body = Rect { y: top, height: bottom.saturating_sub(top), ..inner };
    p
}

/// Step 1 of a frame (`view::prepare`): consume the core's focus requests, load the
/// pages, build the body and keep the scroll position and the selection in range.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    crate::input::ai::sync(core, &mut input.ai);
    if !shown(core) || regions.ai.is_empty() {
        input.ai.view = Built::default();
        return;
    }
    match core.ai.panel.tab {
        PanelTab::Recipes if !core.ai.recipes.loaded => core.ai.load_recipes(),
        PanelTab::Audit if !core.ai.audit.loaded => core.ai.load_audit(),
        _ => {}
    }
    let set = IconSet::from_config(&core.config.tui);
    let inner_w = usize::from(parts(regions.ai, 0, false, false).inner.width);
    let w = inner_w.saturating_sub(1).max(1);
    let (footer, popup) = if core.ai.panel.tab == PanelTab::Chat {
        chat::footer(core, &input.ai, w, set)
    } else {
        (Vec::new(), Vec::new())
    };
    let p = parts(regions.ai, footer.len() as u16, has_field(core), input.ai.focused);
    let st = &mut input.ai;
    let mut b = match core.ai.panel.tab {
        PanelTab::Chat => chat::lines(core, st, w, set),
        PanelTab::Recipes => pages::recipes(core, st, w),
        PanelTab::Audit => pages::audit(core, st, w),
    };
    b.footer = footer.into_iter().take(usize::from(p.footer.height)).collect();
    b.popup = popup;
    b.body_h = usize::from(p.body.height);
    if std::mem::take(&mut core.ai.panel.scroll_to_bottom) {
        st.scroll_up = 0;
    }
    crate::input::ai::settle(st, &mut b);
    st.view = b;
}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_ai(f: &mut Frame<'_>, area: Rect, m: &AiVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    f.render_widget(Clear, area);
    let p = parts(area, m.footer.len() as u16, m.field.is_some(), m.focused);
    let cursor = {
        let buf = f.buffer_mut();
        let base = theme.style(Role::Text);
        buf.set_style(area, base);
        hits.push(area, HitTarget::AiChat);
        let set = theme.icons();
        // The left line.
        if p.inner != area {
            let line = crate::borders::lines(theme.borders()).vertical;
            let style = theme.style(if m.focused { Role::BorderFocused } else { Role::Border });
            for y in area.y..area.bottom() {
                buf.set_string(area.x, y, line, style);
            }
        }
        header(buf, p.header, m, theme, set, hits);
        if !p.info.is_empty() {
            let end = p.info.right();
            let mut right = end;
            if let Some(stop) = &m.stop {
                let s = format!("[{stop}]");
                let w = text::width(&s) as u16;
                if w + 4 < p.info.width {
                    right = end - w;
                    put(buf, right, p.info.y, end, &s, theme.style(Role::Danger));
                    hits.push(Rect::new(right, p.info.y, w, 1), HitTarget::AiButton(AiButton::Stop));
                    right = right.saturating_sub(1);
                }
            }
            put(buf, p.info.x + 1, p.info.y, right, &m.info, theme.style(Role::Muted));
        }
        if !p.tabs.is_empty() {
            let mut x = p.tabs.x + 1;
            for (i, t) in m.tabs.iter().enumerate() {
                let role = if i == m.tab { Role::TabActive } else { Role::TabInactive };
                let start = x;
                x = put(buf, x, p.tabs.y, p.tabs.right(), &format!(" {t} "), theme.style(role));
                hits.push(Rect::new(start, p.tabs.y, x - start, 1), HitTarget::AiTab(i as u8));
                x = (x + 1).min(p.tabs.right());
            }
        }
        for (k, l) in m.lines.iter().enumerate() {
            let y = p.body.y + k as u16;
            if y >= p.body.bottom() {
                break;
            }
            draw_line(buf, Rect { y, height: 1, ..p.body }, l, theme, hits);
        }
        for (k, l) in m.footer.iter().enumerate() {
            let y = p.footer.y + k as u16;
            if y >= p.footer.bottom() {
                break;
            }
            draw_line(buf, Rect { y, height: 1, ..p.footer }, l, theme, hits);
        }
        let mut cursor = None;
        if let (Some(fv), false) = (&m.field, p.field.is_empty()) {
            let r = p.field;
            buf.set_style(r, theme.style(Role::Input));
            if let Some(t) = fv.target {
                hits.push(r, t);
            }
            let x = put(buf, r.x, r.y, r.right(), &fv.prefix, theme.style(Role::Accent));
            if fv.text.is_empty() && fv.cursor.is_none() {
                put(buf, x, r.y, r.right(), &fv.placeholder, theme.style(Role::Faint));
            } else {
                put(buf, x, r.y, r.right(), &fv.text, theme.style(Role::Input));
            }
            if let Some(c) = fv.cursor {
                let cx = x.saturating_add(c).min(r.right().saturating_sub(1));
                cursor = Some(Position { x: cx, y: r.y });
            }
        }
        if !p.hints.is_empty() && !m.hints.is_empty() {
            put(buf, p.hints.x + 1, p.hints.y, p.hints.right(), &m.hints, theme.style(Role::KeyHint));
        }
        cursor
    };
    if let Some(c) = cursor {
        f.set_cursor_position(c);
    }
}

fn header(buf: &mut Buffer, r: Rect, m: &AiVm, theme: &TuiTheme, set: IconSet, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let y = r.y;
    let end = r.right();
    // The marks on the right: new chat, settings, close.
    let mut marks: Vec<(String, AiButton)> = Vec::new();
    if m.new_chat {
        marks.push((icons::glyph(IconId::NotePencil, set).to_string(), AiButton::NewChat));
    }
    marks.push((icons::glyph(IconId::GearSix, set).to_string(), AiButton::Settings));
    marks.push((icons::glyph(IconId::X, set).to_string(), AiButton::Close));
    let marks_w: u16 = marks.iter().map(|(g, _)| text::width(g) as u16 + 2).sum();
    let mut right = end;
    if marks_w + 6 <= r.width {
        let mut x = end - marks_w;
        right = x;
        for (g, b) in &marks {
            let s = format!(" {g} ");
            let w = text::width(&s) as u16;
            put(buf, x, y, end, &s, theme.style(Role::Muted));
            hits.push(Rect::new(x, y, w, 1), HitTarget::AiButton(*b));
            x += w;
        }
    }
    let mut x = r.x + 1;
    let spark = icons::glyph(IconId::Sparkle, set);
    x = put(buf, x, y, right, &format!("{spark} "), theme.style(Role::Accent));
    x = put(buf, x, y, right, &m.title, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    if !m.status.is_empty() {
        x = put(buf, x, y, right, " \u{00b7} ", theme.style(Role::Faint));
        let role = if m.running { Role::Accent } else { Role::Muted };
        put(buf, x, y, right, &m.status, theme.style(role));
    }
}

fn seg_style(s: &Seg, theme: &TuiTheme) -> Style {
    let mut st = theme.style(s.role.unwrap_or(Role::Text));
    if let Some([r, g, b]) = s.color {
        let c = to_color(Rgba8::rgb(r, g, b), theme.mode());
        if c != ratatui::style::Color::Reset {
            st = st.fg(c);
        }
    }
    if s.bold {
        st = st.add_modifier(Modifier::BOLD);
    }
    if s.italic {
        st = st.add_modifier(Modifier::ITALIC);
    }
    if s.underline {
        st = st.add_modifier(Modifier::UNDERLINED);
    }
    st
}

/// Draw one line into the one-row `r` (one blank column on the left).
fn draw_line(buf: &mut Buffer, r: Rect, l: &AiLine, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    if l.sel {
        buf.set_style(r, theme.style(Role::Selected));
    }
    if let Some(t) = l.target {
        hits.push(r, t);
    }
    let sel = theme.style(Role::Selected);
    let mut x = r.x + 1.min(r.width);
    let end = r.right();
    for s in &l.spans {
        if x >= end {
            break;
        }
        let mut style = seg_style(&s.seg, theme);
        if l.sel {
            style = sel.patch(style);
        }
        let start = x;
        x = if s.vline {
            let line = crate::borders::lines(theme.borders()).vertical;
            let nx = put(buf, x, r.y, end, line, style);
            put(buf, nx, r.y, end, " ", style)
        } else {
            put(buf, x, r.y, end, &s.seg.text, style)
        };
        if let Some(t) = s.target
            && x > start
        {
            hits.push(Rect::new(start, r.y, x - start, 1), t);
        }
    }
}

/// Write `s` at `(x, y)`, at most up to `end` (cut with "…"); returns the next column.
fn put(buf: &mut Buffer, x: u16, y: u16, end: u16, s: &str, style: Style) -> u16 {
    if x >= end || s.is_empty() {
        return x;
    }
    let max = usize::from(end - x);
    let s = text::truncate_end(s, max);
    let (nx, _) = buf.set_stringn(x, y, &s, max, style);
    nx.min(end)
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn vm() -> AiVm {
        AiVm {
            visible: true,
            focused: true,
            title: "Assistant".into(),
            status: "Waiting for your approval".into(),
            running: true,
            info: "fake · move · Strong · 1.2k tok".into(),
            stop: Some("Stop  Esc".into()),
            new_chat: false,
            tabs: vec!["Chat".into(), "Recipes".into(), "Audit log".into()],
            tab: 0,
            lines: vec![
                AiLine::plain("> move it", Role::Title),
                AiLine::new(vec![
                    Span::gutter(Role::Accent),
                    Span::button(
                        "Approve",
                        HitTarget::AiCardButton { id: 1, task: 2, button: crate::hits::CardButton::Approve },
                        true,
                    ),
                    Span::new(" ", Role::Text),
                    Span::button(
                        "Reject",
                        HitTarget::AiCardButton { id: 1, task: 2, button: crate::hits::CardButton::Reject },
                        true,
                    ),
                ])
                .with_target(HitTarget::AiCard { id: 1, task: 2 }),
            ],
            footer: vec![AiLine::plain("2 plans wait", Role::Warning)],
            field: Some(FieldVm {
                prefix: "> ".into(),
                text: "hello".into(),
                placeholder: "Ask".into(),
                cursor: Some(5),
                target: Some(HitTarget::AiComposer),
            }),
            hints: "Enter send".into(),
        }
    }

    fn draw(m: &AiVm, w: u16, h: u16) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_ai(f, Rect::new(0, 0, w, h), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, hits)
    }

    #[test]
    fn panel_rows_and_targets() {
        let (lines, hits) = draw(&vm(), 60, 14);
        assert!(lines[0].contains("Assistant") && lines[0].contains("Waiting"), "{lines:?}");
        assert!(lines[1].contains("fake") && lines[1].ends_with("[Stop  Esc]"), "{lines:?}");
        assert!(lines[2].contains("Chat") && lines[2].contains("Audit log"), "{lines:?}");
        assert!(lines[3].contains("> move it"), "{lines:?}");
        assert!(lines[4].contains("[Approve] [Reject]"), "{lines:?}");
        assert!(lines[11].contains("2 plans wait"), "{lines:?}");
        assert!(lines[12].contains("> hello"), "{lines:?}");
        assert!(lines[13].contains("Enter send"), "{lines:?}");
        let x = lines[4].find("[Approve]").unwrap() as u16;
        assert_eq!(
            hits.at(x + 1, 4),
            Some(HitTarget::AiCardButton { id: 1, task: 2, button: crate::hits::CardButton::Approve })
        );
        assert_eq!(hits.at(40, 4), Some(HitTarget::AiCard { id: 1, task: 2 }));
        assert_eq!(hits.at(30, 8), Some(HitTarget::AiChat));
        assert_eq!(hits.at(8, 12), Some(HitTarget::AiComposer));
        assert_eq!(hits.at(59, 0), Some(HitTarget::AiButton(AiButton::Close)));
        assert!(matches!(hits.at(4, 2), Some(HitTarget::AiTab(0))));
    }

    #[test]
    fn parts_shrink_without_overlap() {
        for w in 0..30 {
            for h in 0..20 {
                for footer in [0, 3, 9] {
                    for (field, hints) in [(true, true), (false, false), (true, false)] {
                        let a = Rect::new(2, 1, w, h);
                        let p = parts(a, footer, field, hints);
                        let rows = [p.header, p.info, p.tabs, p.body, p.footer, p.field, p.hints];
                        let total: u16 = rows.iter().map(|r| r.height).sum();
                        assert!(total <= h, "{w}x{h}: {p:?}");
                        for r in rows {
                            assert!(r.is_empty() || a.contains(Position { x: r.x, y: r.y }), "{p:?}");
                        }
                    }
                }
            }
        }
        let p = parts(Rect::new(0, 0, 40, 20), 2, true, true);
        assert_eq!((p.header.y, p.info.y, p.tabs.y, p.body.y), (0, 1, 2, 3));
        assert_eq!((p.footer.y, p.footer.height, p.field.y, p.hints.y), (16, 2, 18, 19));
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let m = vm();
        for w in 0..24 {
            for h in 0..10 {
                let _ = draw(&m, w, h);
            }
        }
    }
}
