//! Keys and mouse of the AI assistant panel and its MCP approval cards (Phase T5).
//!
//! `ToggleAiPanel` (Ctrl+I; Alt+I on legacy terminals, where Ctrl+I arrives as Tab) opens
//! the panel with the keyboard in the composer (the core's `panel.focus_composer`); an
//! MCP request in `[ai.mcp] approval_ui = "panel"` mode opens it with the keyboard on the
//! first waiting external card ([`sync`]). The panel has the keyboard while it is open and
//! [`State::focused`]; a click on it gives it the keyboard, a click elsewhere takes it
//! away. The panel has parts ([`Part`]): the composer and the cards on the chat tab, the
//! list on the recipes and audit tabs, the audit filter.
//!
//! | Key | Composer | Cards | Recipes / audit |
//! |---|---|---|---|
//! | typing | edits the composer (`/` and `@` open a popup) | | |
//! | Enter | sends (picks the popup row) | expands a card, Start on the cost question, sends an idea | runs the recipe |
//! | Up / Down | move in the popup | select the previous / next card | move the cursor |
//! | Tab | to the cards (picks the popup row) | to the composer | |
//! | A / R / K / E | | Approve / Reject / Approve all of this kind / Edit names | |
//! | U / S | | undo the task / save it as a recipe | |
//! | Delete | | | delete the recipe (press twice) |
//! | `/` / R | | | audit: filter / reload |
//! | PageUp / PageDown | scroll the chat | scroll the chat | move the cursor |
//! | F1 F2 F3, Alt+1 2 3 | tabs | tabs | tabs |
//! | Ctrl+N | new chat | new chat | |
//! | Esc | closes the popup; stops a running task; else leaves the panel | stops a running task; else leaves | leaves |
//!
//! Other chords go to the core's global bindings (Ctrl+I closes the panel, Ctrl+Q, …).
//!
//! Safety: nothing here touches the disk. Every answer is a `PanelAct` for
//! `soyuz_appcore::ai::panel::apply` (assistant cards: `AiState::decide`; external cards:
//! `McpApprovals::decide`); approved plans run as undoable jobs (Alt+J shows them).

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::ai::TaskId;
use soyuz_appcore::ai::agent::{Decision, Finish};
use soyuz_appcore::ai::mcp::irreversible;
use soyuz_appcore::ai::panel::{self, CardAction, PanelAct, is_rename};
use soyuz_appcore::ai::state::{CardState, ChatItem, PanelTab, UndoState};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_core::config::{KeyName, Mods};

use super::InputState;
use crate::app::App;
use crate::hits::{AiButton, CardButton};
use crate::layout::HitTarget;
use crate::view::ai::{Built, PopupItem};
use crate::widgets::{EditOutcome, LineEdit, SelectList};

/// `AiCard` / `AiCardButton` id (and task) of what is not a plan card: a task end has
/// `id: OTHER, task`, the cost question `id: OTHER, task: OTHER`.
pub const OTHER: u64 = u64::MAX;
/// `AiMcpUndo(MCP_CLEAR)`: the approvals area's Clear (hide finished requests).
pub const MCP_CLEAR: usize = usize::MAX;
/// Lines the wheel scrolls.
const WHEEL_LINES: usize = 3;

/// The part of the panel that has the keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Part {
    /// The composer (chat tab).
    #[default]
    Composer,
    /// The chat's cards.
    Cards,
    /// The recipes / audit list.
    List,
    /// The audit filter.
    Filter,
}

/// A selectable item of the chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    /// An external client's card (`McpApprovals`).
    Mcp(u64),
    /// An assistant plan card.
    Plan { id: u64, task: u64 },
    /// The cost question.
    Confirm,
    /// The end of task `task`.
    End(u64),
    /// Idea `i` of the empty chat.
    Idea(usize),
}

impl Sel {
    /// The click target of the item's lines.
    pub fn target(self) -> HitTarget {
        match self {
            Sel::Mcp(id) => HitTarget::AiCard { id, task: 0 },
            Sel::Plan { id, task } => HitTarget::AiCard { id, task },
            Sel::Confirm => HitTarget::AiCard { id: OTHER, task: OTHER },
            Sel::End(task) => HitTarget::AiCard { id: OTHER, task },
            Sel::Idea(i) => HitTarget::AiRow(i),
        }
    }

    /// The item of an `AiCard` / `AiCardButton` target.
    pub fn of_card(id: u64, task: u64) -> Sel {
        match (id, task) {
            (OTHER, OTHER) => Sel::Confirm,
            (OTHER, t) => Sel::End(t),
            (id, 0) => Sel::Mcp(id),
            (id, task) => Sel::Plan { id, task },
        }
    }

    /// Button `b` of the item.
    pub fn button(self, b: CardButton) -> HitTarget {
        let (id, task) = match self {
            Sel::Mcp(id) => (id, 0),
            Sel::Plan { id, task } => (id, task),
            Sel::Confirm => (OTHER, OTHER),
            Sel::End(task) => (OTHER, task),
            Sel::Idea(_) => (OTHER, OTHER),
        };
        HitTarget::AiCardButton { id, task, button: b }
    }
}

/// What this part keeps between frames (`InputState::ai`).
#[derive(Debug)]
pub struct State {
    /// The panel has the keyboard.
    pub focused: bool,
    pub part: Part,
    /// The composer's cursor (the text is the core's `panel.composer`).
    pub edit: LineEdit,
    /// The audit filter's cursor (the text is the core's `audit.filter`).
    pub filter_edit: LineEdit,
    /// The selected chat item.
    pub sel: Option<Sel>,
    /// The recipes / audit cursor.
    pub list: SelectList,
    /// Body lines scrolled up from the bottom (0: the newest lines show).
    pub scroll_up: usize,
    /// Bring the selection into view on the next frame.
    pub follow: bool,
    /// The popup row under the cursor.
    pub popup: usize,
    /// Esc closed the popup for this composer text.
    pub popup_closed: Option<String>,
    /// Delete was pressed once on this recipe.
    pub delete_armed: Option<std::path::PathBuf>,
    /// The last MCP request seen (`panel.mcp_flash`).
    pub mcp_seen: Option<std::time::Instant>,
    /// The items that waited for an answer last frame (an answered card hands the
    /// selection to the next waiting one).
    pub waiting: Vec<Sel>,
    /// The selected card was answered: the next card that waits takes the selection
    /// (an edited plan comes back as a new card a moment later).
    pub handoff: bool,
    /// The body as built this frame (`view::ai::prepare`).
    pub view: Built,
}

impl Default for State {
    fn default() -> Self {
        State {
            focused: false,
            part: Part::Composer,
            edit: LineEdit::new(),
            filter_edit: LineEdit::new(),
            sel: None,
            list: SelectList::new(),
            scroll_up: 0,
            follow: false,
            popup: 0,
            popup_closed: None,
            delete_armed: None,
            mcp_seen: None,
            waiting: Vec::new(),
            handoff: false,
            view: Built::default(),
        }
    }
}

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    core.ai.panel.open && st.ai.focused
}

/// Every frame before the body is built (`view::ai::prepare`): the core's focus requests
/// (Ctrl+I, an MCP request) and the part that fits the tab.
pub fn sync(core: &mut AppCore, st: &mut State) {
    if !core.ai.panel.open {
        st.focused = false;
        st.delete_armed = None;
        return;
    }
    if std::mem::take(&mut core.ai.panel.focus_composer) {
        st.focused = true;
        st.part = Part::Composer;
        st.edit = LineEdit::at_end(&core.ai.panel.composer);
    }
    if let Some(t) = core.ai.panel.mcp_flash
        && st.mcp_seen != Some(t)
    {
        st.mcp_seen = Some(t);
        let first = core.services.mcp.approvals.lock().cards.iter().find(|c| c.is_pending()).map(|c| c.card.id);
        if let Some(id) = first {
            st.focused = true;
            st.part = Part::Cards;
            st.sel = Some(Sel::Mcp(id));
            st.follow = true;
        }
    }
    st.part = match (core.ai.panel.tab, st.part) {
        (PanelTab::Chat, Part::Composer | Part::Cards) => st.part,
        (PanelTab::Chat, _) => Part::Composer,
        (PanelTab::Audit, Part::Filter) => Part::Filter,
        _ if core.ai.panel.tab != PanelTab::Chat => Part::List,
        _ => st.part,
    };
}

/// After the body is built (`view::ai::prepare`): keep the selection, the list cursor and
/// the scroll position in range, bring the selection into view when asked, mark the
/// selected lines and set the visible window (`top`).
pub fn settle(st: &mut State, b: &mut Built) {
    let total = b.lines.len();
    let h = b.body_h;
    let max_up = total.saturating_sub(h);
    // The selection of the chat: an answered card hands it to the next waiting one.
    if let Some(s) = st.sel
        && st.waiting.contains(&s)
        && !b.pending.contains(&s)
    {
        st.handoff = true;
    }
    if st.handoff && b.first.is_some() {
        st.sel = b.first;
        st.follow = true;
        st.handoff = false;
    }
    st.waiting = b.pending.clone();
    if !b.sels.is_empty() && st.sel.is_none_or(|s| !b.has(s)) {
        st.sel = b.first.or_else(|| b.sels.last().map(|(s, ..)| *s));
    }
    let mut range = None;
    if st.part == Part::Cards
        && let Some(sel) = st.sel
        && let Some(&(_, a, z)) = b.sels.iter().find(|(s, ..)| *s == sel)
    {
        range = Some((a, z));
    }
    if st.part == Part::List && !b.rows.is_empty() {
        st.list.clamp(b.rows.len(), 1);
        range = b.rows.get(st.list.cursor).copied();
    }
    st.scroll_up = st.scroll_up.min(max_up);
    if std::mem::take(&mut st.follow)
        && let Some((a, z)) = range
        && h > 0
    {
        let top = total.saturating_sub(h + st.scroll_up);
        let new_top = if a < top {
            a
        } else if z > top + h {
            // The item's start stays in view when it is taller than the body.
            (z.saturating_sub(h)).min(a)
        } else {
            top
        };
        st.scroll_up = total.saturating_sub(h).saturating_sub(new_top).min(max_up);
    }
    b.top = total.saturating_sub(h + st.scroll_up);
    if st.focused
        && let Some((a, z)) = range
    {
        // Cards: the first line; list rows: every line.
        let end = if st.part == Part::Cards { (a + 1).min(z) } else { z };
        for l in b.lines.iter_mut().take(end).skip(a) {
            l.sel = true;
        }
    }
    st.popup = st.popup.min(b.popup.len().saturating_sub(1));
}

/// This frame's events while it has the focus.
pub fn route(app: &mut App, events: &[Event]) {
    // A press outside the panel takes the keyboard away (`mouse::route` handled it).
    for e in events {
        if let Event::Mouse(m) = e
            && matches!(m.kind, MouseEventKind::Down(_))
            && !matches!(crate::input::mouse::hit(&app.hits, m.column, m.row).map(|(_, t)| t), Some(t) if is_ours(t))
        {
            app.input.ai.focused = false;
        }
    }
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    let mut acts: Vec<PanelAct> = Vec::new();
    let mut i = 0;
    while i < frame.events.len() {
        if !app.input.ai.focused || !app.core.ai.panel.open || super::dialogs::is_open(&app.core, &app.input.dialogs) {
            break;
        }
        if frame.consumed[i] {
            i += 1;
            continue;
        }
        let ev = frame.events[i].clone();
        let handled = common_key(app, &ev, &mut acts)
            || match app.input.ai.part {
                Part::Composer => composer_key(app, &ev, &mut acts),
                Part::Cards => card_key(app, &ev, &mut acts),
                Part::List => list_key(app, &ev, &mut acts),
                Part::Filter => filter_key(app, &ev),
            };
        if handled {
            frame.consumed[i] = true;
            // The text a letter key types (cards and lists only: the composer takes it).
            if let (InputEvent::Chord(_), Some(InputEvent::Text(_))) = (&ev, frame.events.get(i + 1))
                && !matches!(app.input.ai.part, Part::Composer | Part::Filter)
                && typing_chord(&ev)
            {
                frame.consumed[i + 1] = true;
                i += 1;
            }
        }
        i += 1;
        if !acts.is_empty() {
            apply(app, std::mem::take(&mut acts));
        }
    }
    // Whatever is left: the core's global bindings.
    let text = matches!(app.input.ai.part, Part::Composer | Part::Filter);
    app.core.input(&app.host, &mut frame, soyuz_core::config::KeyContext::Global, text);
}

/// A letter key without Ctrl / Alt / Cmd (its `Text` follows).
fn typing_chord(ev: &InputEvent) -> bool {
    matches!(ev, InputEvent::Chord(c) if !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD))
}

/// Apply panel acts with the terminal UI's host.
pub fn apply(app: &mut App, acts: Vec<PanelAct>) {
    panel::apply(&mut app.core, &app.host, acts);
}

/// The chord of `ev` without Shift, if it has no Ctrl / Alt / Cmd.
fn plain(ev: &InputEvent) -> Option<(KeyName, bool)> {
    match ev {
        InputEvent::Chord(c) if !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) => {
            Some((c.key, c.mods.contains(Mods::SHIFT)))
        }
        _ => None,
    }
}

fn is_ctrl(ev: &InputEvent, key: KeyName) -> bool {
    matches!(ev, InputEvent::Chord(c) if c.key == key && c.mods.intersects(Mods::CTRL | Mods::CMD) && !c.mods.intersects(Mods::ALT | Mods::SHIFT))
}

fn is_alt(ev: &InputEvent, key: KeyName) -> bool {
    matches!(ev, InputEvent::Chord(c) if c.key == key && c.mods == Mods::ALT)
}

/// Keys of every part: tabs, new chat.
fn common_key(app: &mut App, ev: &InputEvent, acts: &mut Vec<PanelAct>) -> bool {
    let tab = |k: KeyName, n: KeyName| matches!(plain(ev), Some((x, false)) if x == k) || is_alt(ev, n);
    let t = if tab(KeyName::F1, KeyName::Num1) {
        PanelTab::Chat
    } else if tab(KeyName::F2, KeyName::Num2) {
        PanelTab::Recipes
    } else if tab(KeyName::F3, KeyName::Num3) {
        PanelTab::Audit
    } else if is_ctrl(ev, KeyName::N) && app.core.ai.panel.tab == PanelTab::Chat {
        acts.push(PanelAct::NewChat);
        return true;
    } else {
        return false;
    };
    set_tab(app, t, acts);
    true
}

fn set_tab(app: &mut App, t: PanelTab, acts: &mut Vec<PanelAct>) {
    let st = &mut app.input.ai;
    st.part = if t == PanelTab::Chat { Part::Composer } else { Part::List };
    st.list = SelectList::new();
    st.scroll_up = 0;
    st.delete_armed = None;
    acts.push(PanelAct::Tab(t));
}

/// Scroll the body by `lines` (negative: up).
fn scroll(app: &mut App, lines: i64) {
    let st = &mut app.input.ai;
    let max = st.view.lines.len().saturating_sub(st.view.body_h);
    st.scroll_up = if lines < 0 {
        (st.scroll_up + lines.unsigned_abs() as usize).min(max)
    } else {
        st.scroll_up.saturating_sub(lines as usize)
    };
}

fn page(app: &App) -> i64 {
    app.input.ai.view.body_h.saturating_sub(1).max(1) as i64
}

/// Esc: stop the running task (`escape_stops_task`, no dialog open), else leave.
fn escape(app: &mut App, acts: &mut Vec<PanelAct>) {
    if panel::escape_stops_task(&app.core) && !super::dialogs::is_open(&app.core, &app.input.dialogs) {
        acts.push(PanelAct::Stop);
    } else {
        app.input.ai.focused = false;
    }
}

fn composer_key(app: &mut App, ev: &InputEvent, acts: &mut Vec<PanelAct>) -> bool {
    let popup = !app.input.ai.view.popup.is_empty();
    match plain(ev) {
        Some((KeyName::Escape, false)) => {
            if popup {
                app.input.ai.popup_closed = Some(app.core.ai.panel.composer.clone());
            } else {
                escape(app, acts);
            }
            return true;
        }
        Some((KeyName::ArrowUp, false)) if popup => {
            app.input.ai.popup = app.input.ai.popup.saturating_sub(1);
            return true;
        }
        Some((KeyName::ArrowDown, false)) if popup => {
            let n = app.input.ai.view.popup.len();
            app.input.ai.popup = (app.input.ai.popup + 1).min(n.saturating_sub(1));
            return true;
        }
        Some((KeyName::Tab | KeyName::Enter, false)) if popup => {
            let i = app.input.ai.popup;
            pick(app, i);
            return true;
        }
        Some((KeyName::Tab, _)) => {
            if !app.input.ai.view.sels.is_empty() {
                app.input.ai.part = Part::Cards;
                app.input.ai.follow = true;
            }
            return true;
        }
        Some((KeyName::Enter, false)) => {
            if app.core.ai.running.is_none() && !app.core.ai.panel.composer.trim().is_empty() {
                acts.push(PanelAct::Send);
                app.input.ai.edit = LineEdit::new();
                app.input.ai.scroll_up = 0;
            }
            return true;
        }
        Some((KeyName::PageUp, false)) => {
            let p = page(app);
            scroll(app, -p);
            return true;
        }
        Some((KeyName::PageDown, false)) => {
            let p = page(app);
            scroll(app, p);
            return true;
        }
        Some((KeyName::Backspace, false))
            if app.core.ai.panel.composer.is_empty() && !app.core.ai.panel.mentions.is_empty() =>
        {
            app.core.ai.panel.mentions.pop();
            return true;
        }
        _ => {}
    }
    let st = &mut app.input.ai;
    match st.edit.apply(&mut app.core.ai.panel.composer, ev) {
        EditOutcome::Ignored => false,
        EditOutcome::Consumed => {
            st.popup_closed = None;
            true
        }
        // Up / Down without a popup and the like: nothing to do here.
        _ => true,
    }
}

/// Pick popup row `i`: a slash command fills the composer, a mention is added.
fn pick(app: &mut App, i: usize) {
    let Some(item) = app.input.ai.view.popup.get(i).cloned() else { return };
    let composer = &mut app.core.ai.panel.composer;
    match item {
        PopupItem::Slash(cmd) => *composer = format!("{cmd} "),
        PopupItem::Mention(paths) => {
            let last = composer.rsplit(char::is_whitespace).next().unwrap_or("").len();
            let cut = composer.len() - last;
            composer.truncate(cut);
            for p in paths {
                if !app.core.ai.panel.mentions.contains(&p) {
                    app.core.ai.panel.mentions.push(p);
                }
            }
        }
    }
    app.input.ai.edit = LineEdit::at_end(&app.core.ai.panel.composer);
    app.input.ai.popup = 0;
    app.input.ai.part = Part::Composer;
}

/// What a key does to a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKey {
    Approve,
    Reject,
    ApproveKind,
    Edit,
    /// Enter: expand / start / send.
    Enter,
    Undo,
    SaveRecipe,
}

/// The card command of `ev` (`None`: not one of the card keys).
pub fn card_key_of(ev: &InputEvent) -> Option<CardKey> {
    Some(match plain(ev)? {
        (KeyName::A, false) => CardKey::Approve,
        (KeyName::R, false) => CardKey::Reject,
        (KeyName::K, false) => CardKey::ApproveKind,
        (KeyName::E, false) => CardKey::Edit,
        (KeyName::Enter, false) => CardKey::Enter,
        (KeyName::U, false) => CardKey::Undo,
        (KeyName::S, false) => CardKey::SaveRecipe,
        _ => return None,
    })
}

fn move_sel(app: &mut App, delta: i64) {
    let st = &mut app.input.ai;
    let sels: Vec<_> = st.view.sels.iter().map(|(s, ..)| *s).collect();
    if sels.is_empty() {
        return;
    }
    let cur = st.sel.and_then(|s| sels.iter().position(|x| *x == s)).unwrap_or(sels.len() - 1) as i64;
    let next = (cur + delta).clamp(0, sels.len() as i64 - 1) as usize;
    st.sel = Some(sels[next]);
    st.follow = true;
    st.handoff = false;
}

fn card_key(app: &mut App, ev: &InputEvent, acts: &mut Vec<PanelAct>) -> bool {
    match plain(ev) {
        Some((KeyName::Escape, false)) => {
            escape(app, acts);
            return true;
        }
        Some((KeyName::Tab, _)) => {
            app.input.ai.part = Part::Composer;
            return true;
        }
        Some((KeyName::ArrowUp, false)) => {
            move_sel(app, -1);
            return true;
        }
        Some((KeyName::ArrowDown, false)) => {
            move_sel(app, 1);
            return true;
        }
        Some((KeyName::Home, false)) => {
            move_sel(app, i64::MIN / 2);
            return true;
        }
        Some((KeyName::End, false)) => {
            move_sel(app, i64::MAX / 2);
            return true;
        }
        Some((KeyName::PageUp, false)) => {
            let p = page(app);
            scroll(app, -p);
            return true;
        }
        Some((KeyName::PageDown, false)) => {
            let p = page(app);
            scroll(app, p);
            return true;
        }
        _ => {}
    }
    let Some(k) = card_key_of(ev) else { return false };
    if let Some(sel) = app.input.ai.sel {
        card_act(app, sel, k, acts);
    }
    true
}

/// Run `k` on item `sel`.
pub fn card_act(app: &mut App, sel: Sel, k: CardKey, acts: &mut Vec<PanelAct>) {
    match sel {
        Sel::Plan { id, task } => plan_act(app, id, task, k, acts),
        Sel::Mcp(id) => mcp_act(app, id, k, acts),
        Sel::Confirm => match k {
            CardKey::Approve | CardKey::Enter => acts.push(PanelAct::Confirm(true)),
            CardKey::Reject => acts.push(PanelAct::Confirm(false)),
            _ => {}
        },
        Sel::End(task) => end_act(app, task, k),
        Sel::Idea(i) => {
            if k == CardKey::Enter
                && let Some((_, prompt)) = crate::view::ai::chat::ideas(&app.core).get(i)
            {
                acts.push(PanelAct::Prompt(prompt.clone()));
            }
        }
    }
    // Undo / save on a card: the task's end.
    if matches!(k, CardKey::Undo | CardKey::SaveRecipe) && !matches!(sel, Sel::End(_)) {
        let task = match sel {
            Sel::Plan { task, .. } if has_end(&app.core, task) => Some(task),
            _ => last_end(&app.core),
        };
        if let Some(t) = task {
            end_act(app, t, k);
        }
    }
}

fn has_end(core: &AppCore, task: u64) -> bool {
    core.ai.chat.iter().any(|c| matches!(c, ChatItem::TaskEnd(t) if t.task.0 == task))
}

fn last_end(core: &AppCore) -> Option<u64> {
    core.ai.chat.iter().rev().find_map(|c| match c {
        ChatItem::TaskEnd(t) => Some(t.task.0),
        _ => None,
    })
}

fn plan_act(app: &mut App, id: u64, task: u64, k: CardKey, acts: &mut Vec<PanelAct>) {
    let live = app.core.ai.running.as_ref().is_some_and(|r| r.task.0 == task);
    let Some(card) = app.core.ai.card_mut(id, TaskId(task)) else { return };
    let pending = live && card.state == CardState::Pending;
    let can = card.plan.executable;
    let tid = TaskId(task);
    match k {
        CardKey::Approve if pending && can => acts.push(PanelAct::Decide(id, tid, Decision::Approve)),
        CardKey::Reject if pending => acts.push(PanelAct::Decide(id, tid, Decision::Reject(String::new()))),
        CardKey::ApproveKind if pending && can => acts.push(PanelAct::ApproveKind(id, tid, card.tool.clone())),
        CardKey::Enter => {
            card.expanded = !card.expanded;
            app.input.ai.follow = true;
        }
        CardKey::Edit if pending && is_rename(&card.tool) && !card.plan.items.is_empty() => {
            let card = card.clone();
            super::dialogs::plan_names::open(app, Sel::Plan { id, task }, &card);
        }
        _ => {}
    }
}

fn mcp_act(app: &mut App, id: u64, k: CardKey, acts: &mut Vec<PanelAct>) {
    let approvals = app.core.services.mcp.approvals.clone();
    let mut a = approvals.lock();
    let Some(c) = a.card_mut(id) else { return };
    let pending = c.is_pending();
    let can = c.card.plan.executable;
    match k {
        CardKey::Approve if pending && can => acts.push(PanelAct::Mcp(id, CardAction::Decide(Decision::Approve))),
        CardKey::Reject if pending => acts.push(PanelAct::Mcp(id, CardAction::Decide(Decision::Reject(String::new())))),
        CardKey::ApproveKind if pending && can && !irreversible(&c.card.tool) => {
            acts.push(PanelAct::Mcp(id, CardAction::ApproveKind))
        }
        CardKey::Enter => {
            c.card.expanded = !c.card.expanded;
            app.input.ai.follow = true;
        }
        CardKey::Edit if pending && is_rename(&c.card.tool) && !c.card.plan.items.is_empty() => {
            let card = c.card.clone();
            drop(a);
            super::dialogs::plan_names::open(app, Sel::Mcp(id), &card);
        }
        _ => {}
    }
}

/// Undo / save on the end of `task`.
fn end_act(app: &mut App, task: u64, k: CardKey) {
    let Some(t) = app.core.ai.chat.iter().find_map(|c| match c {
        ChatItem::TaskEnd(t) if t.task.0 == task => Some(t.as_ref().clone()),
        _ => None,
    }) else {
        return;
    };
    match k {
        CardKey::Undo if t.undo == UndoState::None && !t.executed.is_empty() => {
            apply(app, vec![PanelAct::Undo(t.task)]);
        }
        CardKey::SaveRecipe if can_save(&t) => super::dialogs::recipe_name::open(app, task),
        _ => {}
    }
}

/// "Save as recipe" is offered: a finished assistant task with changes, not saved yet.
pub fn can_save(t: &soyuz_appcore::ai::state::TaskEnd) -> bool {
    !t.recipe_run && matches!(t.reason, Finish::Done) && !t.executed.is_empty() && t.saved.is_none()
}

fn list_key(app: &mut App, ev: &InputEvent, acts: &mut Vec<PanelAct>) -> bool {
    let len = app.input.ai.view.rows.len();
    let tab = app.core.ai.panel.tab;
    if let Some((k, false)) = plain(ev) {
        match k {
            KeyName::Escape => {
                app.input.ai.focused = false;
                app.input.ai.delete_armed = None;
                return true;
            }
            KeyName::Enter if tab == PanelTab::Recipes => {
                let i = app.input.ai.list.cursor;
                if let Some((_, Ok(r))) = app.core.ai.recipes.list.get(i)
                    && app.core.ai.running.is_none()
                {
                    acts.push(PanelAct::RunRecipe(Box::new(r.clone())));
                }
                return true;
            }
            KeyName::Delete if tab == PanelTab::Recipes => {
                let i = app.input.ai.list.cursor;
                if let Some((path, _)) = app.core.ai.recipes.list.get(i).cloned() {
                    if app.input.ai.delete_armed.as_ref() == Some(&path) {
                        app.input.ai.delete_armed = None;
                        acts.push(PanelAct::DeleteRecipe(path));
                    } else {
                        app.input.ai.delete_armed = Some(path);
                    }
                }
                return true;
            }
            KeyName::Slash if tab == PanelTab::Audit => {
                app.input.ai.part = Part::Filter;
                app.input.ai.filter_edit = LineEdit::at_end(&app.core.ai.audit.filter);
                return true;
            }
            KeyName::R if tab == PanelTab::Audit => {
                acts.push(PanelAct::ReloadAudit);
                return true;
            }
            _ => {}
        }
    }
    let height = app.input.ai.view.body_h.max(1);
    if app.input.ai.list.apply(ev, len, height) {
        app.input.ai.delete_armed = None;
        app.input.ai.follow = true;
        return true;
    }
    false
}

fn filter_key(app: &mut App, ev: &InputEvent) -> bool {
    let st = &mut app.input.ai;
    match st.filter_edit.apply(&mut app.core.ai.audit.filter, ev) {
        EditOutcome::Ignored => false,
        EditOutcome::Consumed => {
            st.list = SelectList::new();
            true
        }
        _ => {
            st.part = Part::List;
            true
        }
    }
}

/// A click target of the panel.
fn is_ours(t: HitTarget) -> bool {
    matches!(
        t,
        HitTarget::AiTab(_)
            | HitTarget::AiComposer
            | HitTarget::AiButton(_)
            | HitTarget::AiCard { .. }
            | HitTarget::AiCardButton { .. }
            | HitTarget::AiRow(_)
            | HitTarget::AiMention(_)
            | HitTarget::AiMcpUndo(_)
            | HitTarget::AiChat
    )
}

/// A mouse event over one of its click targets (`input::mouse` dispatches by target);
/// returns whether it was taken.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) -> bool {
    if !is_ours(target) || !app.core.ai.panel.open {
        return false;
    }
    match ev.kind {
        MouseEventKind::ScrollUp => scroll(app, -(WHEEL_LINES as i64)),
        MouseEventKind::ScrollDown => scroll(app, WHEEL_LINES as i64),
        MouseEventKind::Down(MouseButton::Left) => {
            app.input.ai.focused = true;
            let mut acts = Vec::new();
            click(app, target, &mut acts);
            if !acts.is_empty() {
                apply(app, acts);
            }
        }
        _ => {}
    }
    true
}

fn click(app: &mut App, target: HitTarget, acts: &mut Vec<PanelAct>) {
    let tab = app.core.ai.panel.tab;
    match target {
        HitTarget::AiTab(i) => {
            let t = match i {
                0 => PanelTab::Chat,
                1 => PanelTab::Recipes,
                _ => PanelTab::Audit,
            };
            set_tab(app, t, acts);
        }
        HitTarget::AiComposer => {
            if tab == PanelTab::Audit {
                app.input.ai.part = Part::Filter;
                app.input.ai.filter_edit = LineEdit::at_end(&app.core.ai.audit.filter);
            } else {
                app.input.ai.part = Part::Composer;
            }
        }
        HitTarget::AiButton(b) => match b {
            AiButton::Send => {
                if app.core.ai.running.is_none() && !app.core.ai.panel.composer.trim().is_empty() {
                    acts.push(PanelAct::Send);
                    app.input.ai.edit = LineEdit::new();
                }
            }
            AiButton::Stop => acts.push(PanelAct::Stop),
            AiButton::NewChat => acts.push(PanelAct::NewChat),
            AiButton::Close => {
                app.input.ai.focused = false;
                acts.push(PanelAct::Close);
            }
            AiButton::Settings => acts.push(PanelAct::OpenSettings),
            AiButton::Profile => {
                let r = app.core.ai.panel.route;
                acts.push(PanelAct::SetRoute(match r {
                    soyuz_appcore::ai::model::Route::Fast => soyuz_appcore::ai::model::Route::Strong,
                    soyuz_appcore::ai::model::Route::Strong => soyuz_appcore::ai::model::Route::Fast,
                }));
            }
            AiButton::ApproveAll => {
                if let Some(r) = &app.core.ai.running {
                    acts.push(PanelAct::ApproveAllPending(r.task));
                }
            }
        },
        HitTarget::AiCard { id, task } => {
            let sel = Sel::of_card(id, task);
            let st = &mut app.input.ai;
            let again = st.part == Part::Cards && st.sel == Some(sel);
            st.part = Part::Cards;
            st.sel = Some(sel);
            st.handoff = false;
            if again && matches!(sel, Sel::Plan { .. } | Sel::Mcp(_)) {
                card_act(app, sel, CardKey::Enter, acts);
            }
        }
        HitTarget::AiCardButton { id, task, button } => {
            let sel = Sel::of_card(id, task);
            app.input.ai.part = Part::Cards;
            app.input.ai.sel = Some(sel);
            let k = match button {
                CardButton::Approve | CardButton::Go => CardKey::Approve,
                CardButton::Reject | CardButton::Cancel => CardKey::Reject,
                CardButton::ApproveKind => CardKey::ApproveKind,
                CardButton::Edit => CardKey::Edit,
                CardButton::Expand => CardKey::Enter,
                CardButton::Undo => CardKey::Undo,
                CardButton::SaveRecipe => CardKey::SaveRecipe,
            };
            card_act(app, sel, k, acts);
        }
        HitTarget::AiRow(i) => match tab {
            PanelTab::Chat => {
                app.input.ai.part = Part::Cards;
                app.input.ai.sel = Some(Sel::Idea(i));
                card_act(app, Sel::Idea(i), CardKey::Enter, acts);
            }
            _ => {
                let len = app.input.ai.view.rows.len();
                app.input.ai.part = Part::List;
                app.input.ai.list.select(i, len);
                app.input.ai.delete_armed = None;
            }
        },
        HitTarget::AiMention(i) => pick(app, i),
        HitTarget::AiMcpUndo(MCP_CLEAR) => acts.push(PanelAct::McpClear),
        HitTarget::AiMcpUndo(i) => {
            let clients = soyuz_appcore::ai::mcp::clients_with_history(app.core.fileops.jobs.history());
            if let Some((client, _)) = clients.get(i)
                && !app.core.ai.mcp_undo_running(client)
            {
                acts.push(PanelAct::McpUndoClient(client.clone()));
            }
        }
        _ => {}
    }
}

/// While `true`, every mouse event goes to [`on_mouse`] wherever it is (a drag that
/// leaves the targets, an overlay's capture); the target is `HitTarget::Overlay` when
/// nothing is under the pointer.
pub fn captures_mouse(_core: &AppCore, _st: &InputState) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    #[test]
    fn card_keys() {
        assert_eq!(card_key_of(&chord("A")), Some(CardKey::Approve));
        assert_eq!(card_key_of(&chord("R")), Some(CardKey::Reject));
        assert_eq!(card_key_of(&chord("K")), Some(CardKey::ApproveKind));
        assert_eq!(card_key_of(&chord("E")), Some(CardKey::Edit));
        assert_eq!(card_key_of(&chord("Enter")), Some(CardKey::Enter));
        assert_eq!(card_key_of(&chord("U")), Some(CardKey::Undo));
        assert_eq!(card_key_of(&chord("S")), Some(CardKey::SaveRecipe));
        assert_eq!(card_key_of(&chord("Ctrl+A")), None, "select all stays global");
        assert_eq!(card_key_of(&chord("Shift+A")), None);
        assert_eq!(card_key_of(&InputEvent::Text("a".into())), None);
    }

    #[test]
    fn card_targets_round_trip() {
        for s in [Sel::Mcp(3), Sel::Plan { id: 4, task: 9 }, Sel::Confirm, Sel::End(9)] {
            let HitTarget::AiCard { id, task } = s.target() else { panic!("{s:?}") };
            assert_eq!(Sel::of_card(id, task), s);
            let HitTarget::AiCardButton { id, task, .. } = s.button(CardButton::Approve) else { panic!() };
            assert_eq!(Sel::of_card(id, task), s);
        }
        assert_eq!(Sel::Idea(2).target(), HitTarget::AiRow(2));
    }

    fn built(n: usize, h: usize) -> Built {
        let lines = (0..n).map(|i| crate::view::ai::AiLine::plain(format!("{i}"), crate::theme::Role::Text)).collect();
        Built {
            lines,
            sels: vec![(Sel::Plan { id: 1, task: 1 }, 2, 5), (Sel::End(1), 30, 32)],
            body_h: h,
            ..Built::default()
        }
    }

    #[test]
    fn settle_sticks_to_the_bottom_and_follows_the_selection() {
        let mut st = State::default();
        let mut b = built(40, 10);
        settle(&mut st, &mut b);
        assert_eq!(b.top, 30, "the newest lines");
        assert_eq!(st.sel, Some(Sel::End(1)), "the last item when nothing waits");
        st.focused = true;
        st.part = Part::Cards;
        st.sel = Some(Sel::Plan { id: 1, task: 1 });
        st.follow = true;
        let mut b = built(40, 10);
        settle(&mut st, &mut b);
        assert_eq!(b.top, 2);
        assert!(b.lines[2].sel && !b.lines[3].sel, "the card's first line is marked");
        // Scrolled further up than possible: clamped.
        st.scroll_up = 99;
        let mut b = built(40, 10);
        settle(&mut st, &mut b);
        assert_eq!(b.top, 0);
        // A gone selection falls back to the first waiting item.
        st.sel = Some(Sel::Mcp(7));
        let mut b = built(40, 10);
        b.first = Some(Sel::Plan { id: 1, task: 1 });
        settle(&mut st, &mut b);
        assert_eq!(st.sel, Some(Sel::Plan { id: 1, task: 1 }));
    }
}
