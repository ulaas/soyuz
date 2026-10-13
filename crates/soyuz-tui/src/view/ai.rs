//! The view of the AI assistant panel and its MCP approval cards (Phase T5): drawn into
//! `Regions::ai` (right of the body at `[tui] ai_ratio`, or the whole body below 100
//! columns), after the GUI's panel (`soyuz-app/src/ui/ai`):
//!
//! ```text
//! │ * Assistant · Waiting for your approval      [+] [*] [x]
//! │ [Stop  Esc] [fake · move v] [Strong] [Metadata v] [Tools v] [Smart search]  1.2k tok *
//! │  Chat   Recipes   Audit log
//! │ > move a.txt
//! │ * I suggest moving a.txt.
//! │ | -> Move 1 item into Moved
//! │ |    move · 1 item · Waiting for you
//! │ |    data/a.txt -> data/Moved/a.txt            moved
//! │ |    [Approve] [Edit] [Reject] [Approve all "move"]
//! │ ... 2 plans wait for your approval     [Approve all]
//! │ > Ask or give a task…
//! │   (more lines of the message)
//! ```
//!
//! Rows top to bottom ([`parts`]): the header (title, status, new chat / settings / close
//! marks), the chip rows (Stop while a task runs, the model menu, fast / strong, the
//! current folder's access menu, the tools and MCP servers menu, smart search, the token
//! and cost meter with the monthly budget warning `*`; one or two rows), the tabs (chat,
//! recipes, audit log), the body, the footer (the smart search card, the "N plans wait"
//! bar, the slash / `@` popup, the mentions), the field (the composer on the chat tab,
//! growing to [`MAX_COMPOSER_ROWS`] rows and then scrolling; the filter on the audit tab)
//! and, while the panel has the keyboard, the key hints. Small panels drop the chips, the
//! tabs and the hints first. A chip menu (`input::ai::State::menu`) draws last, over the
//! panel and the files left of it.
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
//! keyboard); `AiTab`, `AiButton` (header, Stop, Approve all, the model chip), `AiCard`
//! (every line of a card, the cost question, a task end), `AiCardButton` (the card
//! buttons), `AiRow` (ideas, recipes, audit rows; from [`crate::input::ai::CHIPS`] on the
//! other chips and the smart search card's controls, [`crate::input::ai::Chip`]),
//! `AiMention` (popup rows), `AiMcpUndo` ("Undo everything <client> did";
//! [`crate::input::ai::MCP_CLEAR`] is the approvals area's Clear), `AiComposer` (a click
//! puts the cursor there) and, while a chip menu is open, `MenuItem`.

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
use soyuz_core::Location;
use soyuz_core::config::AiAccess;

use crate::dialog::safe;
use crate::hits::AiButton;
use crate::icons::{self, IconSet};
use crate::input::InputState;
use crate::input::ai::text_area;
use crate::input::ai::{Chip, MenuAct, Part, Sel};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme, to_color};
use crate::view::preview::Seg;
use crate::view::text;
use crate::widgets::Menu;

/// Footer rows at most (popup rows and the smart search card included).
pub const MAX_FOOTER: u16 = 14;
/// Rows of the slash / mention popup at most.
pub const MAX_POPUP: usize = 6;
/// The hints row shows from this panel height.
pub const HINTS_MIN_ROWS: u16 = 12;
/// The composer grows up to this many rows, then scrolls.
pub const MAX_COMPOSER_ROWS: usize = 6;
/// Chip rows at most (from [`TWO_CHIP_ROWS`] panel rows; one below).
pub const MAX_CHIP_ROWS: usize = 2;
/// The panel height from which the chips may take a second row.
pub const TWO_CHIP_ROWS: u16 = 16;
/// Lead of the composer's first row and of the rows after it.
const PROMPT: &str = "> ";
const INDENT: &str = "  ";

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

    /// The same span drawn in `role`.
    pub fn role(mut self, role: Role) -> Span {
        self.seg.role = Some(role);
        self
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

/// `spans` on as few lines of `w` cells as they need (one space apart), at most `max`
/// lines; a span wider than a line is cut.
pub fn flow(spans: Vec<Span>, w: usize, max: usize) -> Vec<AiLine> {
    let mut out: Vec<AiLine> = Vec::new();
    let mut cur: Vec<Span> = Vec::new();
    let mut used = 0;
    if max == 0 {
        return out;
    }
    for mut s in spans {
        if s.width() > w && !s.vline {
            s.seg.text = text::truncate_end(&s.seg.text, w);
        }
        let sw = s.width();
        if used > 0 && used + 1 + sw > w {
            out.push(AiLine::new(std::mem::take(&mut cur)));
            used = 0;
            if out.len() == max {
                return out;
            }
        }
        if used > 0 {
            cur.push(Span::new(" ", Role::Text));
            used += 1;
        }
        used += sw;
        cur.push(s);
    }
    if !cur.is_empty() && out.len() < max {
        out.push(AiLine::new(cur));
    }
    out
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
    /// The chips under the header (model, route, access, tools, smart search, meter).
    pub chips: Vec<AiLine>,
    pub footer: Vec<AiLine>,
    /// The text cursor in the footer (the smart search card's field): line, column.
    pub footer_cursor: Option<(usize, u16)>,
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

/// The composer / filter rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldVm {
    /// The first row's lead (the others are indented as wide).
    pub prefix: String,
    /// The visible rows of the text.
    pub lines: Vec<String>,
    pub placeholder: String,
    /// The cursor (row, column in the row's text) while the field has the keyboard.
    pub cursor: Option<(u16, u16)>,
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
    /// The chip rows: Stop while a task runs, model, route, folder access, tools, smart
    /// search and the token / cost meter (with the monthly budget warning).
    pub chips: Vec<AiLine>,
    pub new_chat: bool,
    pub tabs: Vec<String>,
    pub tab: usize,
    /// The visible body lines.
    pub lines: Vec<AiLine>,
    pub footer: Vec<AiLine>,
    pub footer_cursor: Option<(usize, u16)>,
    pub field: Option<FieldVm>,
    pub hints: String,
    /// An open chip menu and where it may draw.
    pub menu: Option<Menu<MenuAct>>,
    pub menu_screen: Rect,
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
        let rows = field_rows(core, regions);
        let p = parts(regions.ai, b.chips.len() as u16, b.footer.len() as u16, rows, st.focused);
        AiVm {
            visible: true,
            focused: st.focused,
            title: tr!("ai-panel-title").into(),
            status: safe(&status(core)),
            running: ai.running.is_some(),
            chips: b.chips.clone(),
            new_chat: ai.running.is_none(),
            tabs: vec![tr!("ai-tab-chat").into(), tr!("ai-tab-recipes").into(), tr!("ai-tab-audit").into()],
            tab,
            lines,
            footer: b.footer.clone(),
            footer_cursor: b.footer_cursor,
            field: field(core, input, p.field),
            hints: if st.focused { hints(core, input) } else { String::new() },
            menu: st.menu.clone(),
            menu_screen: st.menu_screen,
        }
    }
}

/// It is drawn this frame.
pub fn shown(core: &AppCore) -> bool {
    core.ai.panel.open
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

/// The profile and model of the next message: `(profile, model)`.
fn model_label(core: &AppCore) -> Option<(String, String)> {
    let ai = &core.ai;
    match &ai.panel.profile_override {
        Some(name) => core.config.ai.profile(name).map(|p| (p.name.clone(), p.model.clone())),
        None => model::route_label(&core.config.ai, ai.panel.route),
    }
}

/// "profile · model" (or "no model").
fn model_text(core: &AppCore) -> String {
    match model_label(core) {
        Some((prof, m)) if m.is_empty() => prof,
        Some((prof, m)) => format!("{prof} \u{00b7} {m}"),
        None => tr!("ai-no-model").into(),
    }
}

fn route_word(r: Route) -> &'static str {
    match r {
        Route::Fast => tr!("ai-route-fast"),
        Route::Strong => tr!("ai-route-strong"),
    }
}

/// The token / cost meter and whether this month's spend of the profile reached
/// `[ai] budget_warn_percent` of its monthly budget (the GUI's warning dot).
pub fn meter(core: &AppCore) -> (String, bool) {
    let ai = &core.ai;
    let (usage, cost) = match &ai.running {
        Some(r) => (r.usage, r.cost),
        None => (ai.session_usage, (ai.session_cost > 0.0).then_some(ai.session_cost)),
    };
    let text = match cost {
        Some(c) => {
            tr!("ai-meter-tokens-cost", tokens = tokens(usage.total()), cost = soyuz_core::locale::format_decimal(c, 3))
        }
        None => tr!("ai-meter-tokens", tokens = tokens(usage.total())),
    };
    let warn = model_label(core).is_some_and(|(prof, _)| {
        let cap = core.config.ai.profile(&prof).map_or(0.0, |p| p.monthly_budget_usd);
        let spent = ai.month.as_ref().map_or(0.0, |l| l.spend(&soyuz_ai::pricing::current_month(), &prof).cost_usd);
        budget_warns(spent, cap, core.config.ai.budget_warn_percent)
    });
    (text, warn)
}

/// The GUI's rule for the budget warning dot.
pub fn budget_warns(spent: f64, cap: f64, percent: u32) -> bool {
    cap > 0.0 && spent / cap * 100.0 >= f64::from(percent)
}

/// The access chip's word for `a`.
pub fn access_short(a: AiAccess) -> &'static str {
    match a {
        AiAccess::None => tr!("ai-access-short-none"),
        AiAccess::Metadata => tr!("ai-access-short-metadata"),
        AiAccess::Contents => tr!("ai-access-short-contents"),
    }
}

/// The chip rows for a panel `w` cells wide (at most `max` rows): Stop while a task
/// runs, the model menu, fast / strong, the folder's access menu, the tools menu, smart
/// search, then the meter.
pub fn chips(core: &AppCore, w: usize, max: usize, set: IconSet) -> Vec<AiLine> {
    let ai = &core.ai;
    let caret = icons::glyph(IconId::CaretDown, set);
    let mut spans = Vec::new();
    if ai.running.is_some() {
        spans.push(Span::button(tr!("ai-stop-esc"), HitTarget::AiButton(AiButton::Stop), true).role(Role::Danger));
    }
    let model = text::truncate_end(&safe(&model_text(core)), (w / 2).max(8));
    spans.push(Span::button(&format!("{model} {caret}"), HitTarget::AiButton(AiButton::Profile), true));
    if ai.panel.profile_override.is_none() {
        spans.push(Span::button(route_word(ai.panel.route), Chip::Route.target(), true));
    }
    if let Location::Dir(dir) = &core.ws.pane().location {
        let (access, explicit) = soyuz_appcore::ai::state::access_for(core, dir);
        let word = if explicit {
            access_short(access).to_string()
        } else {
            tr!("ai-access-inherited", access = access_short(access))
        };
        let chip = Span::button(&format!("{word} {caret}"), Chip::Access.target(), true);
        spans.push(if access == AiAccess::Contents { chip.role(Role::Warning) } else { chip });
    }
    let ext = ai.external.tools().len();
    let tools = if ext == 0 { tr!("ai-tools").to_string() } else { tr!("ai-tools-connected", count = ext) };
    spans.push(Span::button(&format!("{tools} {caret}"), Chip::Tools.target(), true));
    if ai.panel.tab == PanelTab::Chat {
        let smart = Span::button(tr!("ai-smart-chip"), Chip::Smart.target(), true);
        spans.push(if ai.smart.open { smart.role(Role::Accent) } else { smart });
    }
    let (m, warn) = meter(core);
    let mut meter = vec![Span::new(m, Role::Muted)];
    if warn {
        let dot = if set == IconSet::Ascii { " *" } else { " \u{25cf}" };
        meter.push(Span::new(dot, Role::Warning));
    }
    let meter_w: usize = meter.iter().map(Span::width).sum();
    let mut lines = flow(spans, w, max);
    // The meter goes after the chips on the last row when it fits, else on a row of its
    // own while there is one.
    let last_w = lines.last().map_or(0, |l| l.spans.iter().map(Span::width).sum::<usize>());
    let n = lines.len();
    match lines.last_mut() {
        Some(l) if last_w + 2 + meter_w <= w => {
            l.spans.push(Span::new("  ", Role::Text));
            l.spans.extend(meter);
        }
        _ if n < max => lines.push(AiLine::new(meter)),
        _ => {}
    }
    lines
}

fn hints(core: &AppCore, input: &InputState) -> String {
    let st = &input.ai;
    match (core.ai.panel.tab, st.part) {
        (PanelTab::Chat, Part::Composer) => tr!("tui-ai-hints-composer").into(),
        (PanelTab::Chat, Part::Smart) => tr!("tui-ai-hints-smart").into(),
        (PanelTab::Chat, _) => tr!("tui-ai-hints-cards").into(),
        (PanelTab::Recipes, _) if st.delete_armed.is_some() => tr!("tui-ai-delete-again").into(),
        (PanelTab::Recipes, _) => tr!("tui-ai-hints-recipes").into(),
        (PanelTab::Audit, Part::Filter) => tr!("tui-ai-hints-filter").into(),
        (PanelTab::Audit, _) => tr!("tui-ai-hints-audit").into(),
    }
}

/// The width the composer wraps at in a field `width` cells wide.
pub fn wrap_width(width: u16) -> usize {
    usize::from(width).saturating_sub(text::width(PROMPT)).max(1)
}

/// Rows the field wants: the composer's wrapped lines (1 to [`MAX_COMPOSER_ROWS`]), one
/// for the audit filter, none on the recipes tab.
fn field_rows(core: &AppCore, regions: &Regions) -> u16 {
    match core.ai.panel.tab {
        PanelTab::Chat => {
            let w = parts(regions.ai, 0, 0, 1, false).inner.width;
            let n = text_area::rows(&core.ai.panel.composer, Some(wrap_width(w))).len();
            n.clamp(1, MAX_COMPOSER_ROWS) as u16
        }
        PanelTab::Audit => 1,
        PanelTab::Recipes => 0,
    }
}

fn field(core: &AppCore, input: &InputState, r: Rect) -> Option<FieldVm> {
    let st = &input.ai;
    match core.ai.panel.tab {
        PanelTab::Chat => {
            let text = &core.ai.panel.composer;
            let has = st.focused && st.part == Part::Composer && st.menu.is_none();
            let w = wrap_width(r.width);
            let (lines, (row, col)) = st.composer.view(text, Some(w), w, usize::from(r.height.max(1)));
            Some(FieldVm {
                prefix: PROMPT.into(),
                lines: lines.iter().map(|l| safe(l)).collect(),
                placeholder: if core.ai.running.is_some() {
                    tr!("ai-composer-busy").into()
                } else {
                    tr!("ai-composer-hint").into()
                },
                cursor: has.then_some((row, col)),
                target: Some(HitTarget::AiComposer),
            })
        }
        PanelTab::Audit => {
            let text = &core.ai.audit.filter;
            let has = st.focused && st.part == Part::Filter;
            let mut edit = st.filter_edit.clone();
            let (shown, col) = edit.view(text, r.width.saturating_sub(2));
            Some(FieldVm {
                prefix: "/ ".into(),
                lines: vec![safe(&shown)],
                placeholder: tr!("ai-audit-filter-hint").into(),
                cursor: has.then_some((0, col)),
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
    /// The chip rows.
    pub info: Rect,
    pub tabs: Rect,
    pub body: Rect,
    pub footer: Rect,
    pub field: Rect,
    pub hints: Rect,
}

/// Split the panel `area` (`chips` chip rows wanted, `footer` rows wanted, `field` rows
/// of the composer / filter, the hints row while focused). Small panels drop the chips,
/// the tabs and the hints first; the body keeps at least one row while there is room.
pub fn parts(area: Rect, chips: u16, footer: u16, field: u16, hints: bool) -> Parts {
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
    if top < bottom {
        p.header = row(top);
        top += 1;
    }
    if h >= 7 && chips > 0 {
        let n = chips.min(if h >= TWO_CHIP_ROWS { MAX_CHIP_ROWS as u16 } else { 1 }).min(bottom - top);
        p.info = Rect { y: top, height: n, ..inner };
        top += n;
    }
    if h >= 5 && top < bottom {
        p.tabs = row(top);
        top += 1;
    }
    if hints && h >= HINTS_MIN_ROWS && bottom > top + 1 {
        bottom -= 1;
        p.hints = row(bottom);
    }
    if field > 0 && bottom > top + 1 {
        let room = bottom - top;
        let f = field.min(room - 1).min((room / 2).max(1));
        bottom -= f;
        p.field = Rect { y: bottom, height: f, ..inner };
    }
    let room = bottom.saturating_sub(top);
    let f = footer.min(MAX_FOOTER).min(room * 2 / 3);
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
        input.ai.menu = None;
        return;
    }
    match core.ai.panel.tab {
        PanelTab::Recipes if !core.ai.recipes.loaded => core.ai.load_recipes(),
        PanelTab::Audit if !core.ai.audit.loaded => core.ai.load_audit(),
        _ => {}
    }
    let set = IconSet::from_config(&core.config.tui);
    let inner_w = usize::from(parts(regions.ai, 0, 0, 0, false).inner.width);
    let w = inner_w.saturating_sub(1).max(1);
    let max_chips = if regions.ai.height >= TWO_CHIP_ROWS { MAX_CHIP_ROWS } else { 1 };
    let chip_lines = chips(core, w, max_chips, set);
    let foot = if core.ai.panel.tab == PanelTab::Chat {
        chat::footer(core, &input.ai, w, set)
    } else {
        chat::Footer::default()
    };
    let rows = field_rows(core, regions);
    let p = parts(regions.ai, chip_lines.len() as u16, foot.lines.len() as u16, rows, input.ai.focused);
    let st = &mut input.ai;
    let mut b = match core.ai.panel.tab {
        PanelTab::Chat => chat::lines(core, st, w, set),
        PanelTab::Recipes => pages::recipes(core, st, w),
        PanelTab::Audit => pages::audit(core, st, w),
    };
    let fh = usize::from(p.footer.height);
    b.chips = chip_lines.into_iter().take(usize::from(p.info.height)).collect();
    b.footer = foot.lines.into_iter().take(fh).collect();
    b.footer_cursor = foot.cursor.filter(|(l, _)| *l < fh);
    b.popup = foot.popup;
    b.body_h = usize::from(p.body.height);
    // The composer wraps at the field's width and keeps its cursor in view.
    st.field_rect = p.field;
    st.wrap = wrap_width(p.field.width);
    if core.ai.panel.tab == PanelTab::Chat {
        let h = usize::from(p.field.height.max(1));
        st.composer.scroll_to_cursor(&core.ai.panel.composer, Some(st.wrap), st.wrap, h);
    }
    // A chip menu may cover the panel and the files left of it, never the rows below.
    st.menu_screen = Rect { x: regions.area.x, width: regions.ai.right().saturating_sub(regions.area.x), ..regions.ai };
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
    let field_h = m.field.as_ref().map_or(0, |fv| fv.lines.len().max(1) as u16);
    let p = parts(area, m.chips.len() as u16, m.footer.len() as u16, field_h, m.focused);
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
        for (k, l) in m.chips.iter().enumerate() {
            let y = p.info.y + k as u16;
            if y >= p.info.bottom() {
                break;
            }
            draw_line(buf, Rect { y, height: 1, ..p.info }, l, theme, hits);
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
        if let Some((l, c)) = m.footer_cursor
            && (l as u16) < p.footer.height
        {
            let x = p.footer.x.saturating_add(1).saturating_add(c).min(p.footer.right().saturating_sub(1));
            cursor = Some(Position { x, y: p.footer.y + l as u16 });
        }
        if let (Some(fv), false) = (&m.field, p.field.is_empty()) {
            cursor = draw_field(buf, p.field, fv, theme, hits).or(cursor);
        }
        if !p.hints.is_empty() && !m.hints.is_empty() {
            put(buf, p.hints.x + 1, p.hints.y, p.hints.right(), &m.hints, theme.style(Role::KeyHint));
        }
        cursor
    };
    if let Some(menu) = &m.menu {
        menu.clone().draw(f, m.menu_screen, theme, hits);
    } else if let Some(c) = cursor {
        f.set_cursor_position(c);
    }
}

/// The composer / filter rows; returns the cursor's cell.
fn draw_field(buf: &mut Buffer, r: Rect, fv: &FieldVm, theme: &TuiTheme, hits: &mut Hits) -> Option<Position> {
    buf.set_style(r, theme.style(Role::Input));
    if let Some(t) = fv.target {
        hits.push(r, t);
    }
    let lead_w = text::width(&fv.prefix) as u16;
    let empty = fv.lines.iter().all(String::is_empty);
    for k in 0..r.height {
        let y = r.y + k;
        let lead = if k == 0 { fv.prefix.as_str() } else { INDENT };
        let x = put(buf, r.x, y, r.right(), lead, theme.style(Role::Accent));
        if k == 0 && empty && fv.cursor.is_none() {
            put(buf, x, y, r.right(), &fv.placeholder, theme.style(Role::Faint));
        } else if let Some(l) = fv.lines.get(usize::from(k)) {
            put(buf, x, y, r.right(), l, theme.style(Role::Input));
        }
    }
    let (row, col) = fv.cursor?;
    let row = row.min(r.height.saturating_sub(1));
    let x = r.x.saturating_add(lead_w).saturating_add(col).min(r.right().saturating_sub(1));
    Some(Position { x, y: r.y + row })
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
            chips: vec![AiLine::new(vec![
                Span::new("fake · move", Role::Muted),
                Span::button("Stop  Esc", HitTarget::AiButton(AiButton::Stop), true),
            ])],
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
            footer_cursor: None,
            field: Some(FieldVm {
                prefix: "> ".into(),
                lines: vec!["hello".into()],
                placeholder: "Ask".into(),
                cursor: Some((0, 5)),
                target: Some(HitTarget::AiComposer),
            }),
            hints: "Enter send".into(),
            menu: None,
            menu_screen: Rect::default(),
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
        let stop = lines[1].chars().take_while(|c| *c != '[').count() as u16;
        assert_eq!(hits.at(stop + 1, 1), Some(HitTarget::AiButton(AiButton::Stop)));
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
            for h in 0..24 {
                for footer in [0, 3, 9, 20] {
                    for (chips, field, hints) in [(1, 1, true), (0, 0, false), (2, 6, false), (2, 3, true)] {
                        let a = Rect::new(2, 1, w, h);
                        let p = parts(a, chips, footer, field, hints);
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
        let p = parts(Rect::new(0, 0, 40, 20), 1, 2, 1, true);
        assert_eq!((p.header.y, p.info.y, p.tabs.y, p.body.y), (0, 1, 2, 3));
        assert_eq!((p.footer.y, p.footer.height, p.field.y, p.hints.y), (16, 2, 18, 19));
        // A multi-line composer grows upwards; two chip rows on a tall panel.
        let p = parts(Rect::new(0, 0, 40, 30), 2, 0, 4, true);
        assert_eq!((p.info.height, p.tabs.y), (2, 3));
        assert_eq!((p.field.y, p.field.height, p.hints.y), (25, 4, 29));
    }

    #[test]
    fn multi_line_composer_and_footer_cursor() {
        let mut m = vm();
        m.field = Some(FieldVm {
            prefix: "> ".into(),
            lines: vec!["first".into(), "second".into()],
            placeholder: String::new(),
            cursor: Some((1, 3)),
            target: Some(HitTarget::AiComposer),
        });
        let (lines, hits) = draw(&m, 60, 16);
        assert!(lines[13].contains("> first"), "{lines:?}");
        assert!(lines[14].contains("  second"), "{lines:?}");
        assert_eq!(hits.at(10, 14), Some(HitTarget::AiComposer));
        assert!(budget_warns(80.0, 100.0, 80));
        assert!(!budget_warns(10.0, 0.0, 0), "no cap, no warning");
        assert!(!budget_warns(79.0, 100.0, 80));
    }

    #[test]
    fn chips_flow_onto_rows() {
        let spans: Vec<Span> = (0..6).map(|i| Span::button(&format!("chip{i}"), HitTarget::AiChat, true)).collect();
        let rows = flow(spans.clone(), 20, 2);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.spans.iter().map(Span::width).sum::<usize>() <= 20));
        assert_eq!(flow(spans.clone(), 200, 2).len(), 1);
        assert!(flow(spans, 3, 0).is_empty());
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
