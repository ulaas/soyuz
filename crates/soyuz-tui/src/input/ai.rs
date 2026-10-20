//! Keys and mouse of the AI assistant panel and its MCP approval cards (Phase T5).
//!
//! `ToggleAiPanel` (Ctrl+I; Alt+I on legacy terminals, where Ctrl+I arrives as Tab) opens
//! the panel with the keyboard in the composer (the core's `panel.focus_composer`); an
//! MCP request in `[ai.mcp] approval_ui = "panel"` mode opens it with the keyboard on the
//! first waiting external card ([`sync`]). The panel has the keyboard while it is open and
//! [`State::focused`]; a click on it gives it the keyboard, a click elsewhere takes it
//! away. The panel has parts ([`Part`]): the composer, the smart search card and the cards
//! on the chat tab, the list on the recipes and audit tabs, the audit filter.
//!
//! | Key | Composer | Cards | Recipes / audit |
//! |---|---|---|---|
//! | typing | edits the composer (`/` and `@` open a popup) | | |
//! | Enter | sends (picks the popup row) | expands a card, Start on the cost question, sends an idea | runs the recipe |
//! | Alt+Enter, Ctrl+J (Shift+Enter with the kitty protocol) | a new line | | |
//! | Up / Down | the popup, else the composer's lines | select the previous / next card | move the cursor |
//! | Tab | to the cards (picks the popup row) | to the composer | |
//! | A / R / K | | Approve / Reject / Approve all of this kind | |
//! | E / Shift+E | | Edit (names of a rename, else the JSON arguments) / Edit in `$EDITOR` | |
//! | U / S | | undo the task / save it as a recipe | |
//! | Delete | | | delete the recipe (press twice) |
//! | `/` / R | | | audit: filter / reload |
//! | PageUp / PageDown | scroll the chat | scroll the chat | move the cursor |
//! | F1 F2 F3, Alt+1 2 3 | tabs | tabs | tabs |
//! | Alt+M / Alt+R / Alt+F / Alt+O / Alt+S | model menu / fast or strong / folder access menu / tools menu / smart search | same | |
//! | Ctrl+N | new chat | new chat | |
//! | Esc | closes the popup; stops a running task; else leaves the panel | stops a running task; else leaves | leaves |
//!
//! The smart search card (`AiSmartSearch`, Alt+S, its chip): typing edits the plain words
//! (the filters follow) or, after Tab / Down, the query; Enter searches, Alt+Enter asks the
//! assistant, Backspace in an empty field removes the last filter, Esc closes the card.
//!
//! Other chords go to the core's global bindings (Ctrl+I closes the panel, Ctrl+Q, …).
//! Every frame, before the input is routed, [`arm`] lets Esc stop a running task while the
//! panel is open but the file list, sidebar, preview or jobs have the keyboard (the GUI
//! stops it from anywhere), and makes `ToggleAiPanel` give an open, unfocused panel the
//! keyboard instead of closing it; it also hands finished `$EDITOR` runs back and keeps
//! the "Working · Ns" header and the MCP countdowns ticking once a second.
//!
//! Plan edits: E opens the name table of a rename plan (`TuiDialog::PlanNames`) or the
//! JSON editor of the arguments (`TuiDialog::PlanJson`); Shift+E (and Ctrl+E in the JSON
//! editor) writes the arguments to a temporary file under the config folder's `tmp` and
//! suspends to the configured editor (`SuspendRequest::Editor` with
//! `EditPurpose::Scratch`); [`arm`] reads the file back when the editor exits, validates it
//! (`edit::args_from_json`) and sends `Decision::Edit`, or opens the JSON editor again with
//! the error. The file is a few hundred bytes, read once right after the suspend.
//!
//! Safety: nothing here touches the user's files. Every answer is a `PanelAct` for
//! `soyuz_appcore::ai::panel::apply` (assistant cards: `AiState::decide`; external cards:
//! `McpApprovals::decide`); approved plans run as undoable jobs (Alt+J shows them).

pub mod text_area;

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossterm::event::{Event, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use soyuz_ai::tools::Risk;
use soyuz_appcore::ai::TaskId;
use soyuz_appcore::ai::agent::{Decision, Finish};
use soyuz_appcore::ai::mcp::irreversible;
use soyuz_appcore::ai::model::Route;
use soyuz_appcore::ai::panel::{self, CardAction, PanelAct, is_rename};
use soyuz_appcore::ai::smart_search;
use soyuz_appcore::ai::state::{CardState, ChatItem, PanelTab, PlanCard, UndoState};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Host as _;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::frontend::{EditPurpose, SuspendRequest};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::{Action, AiAccess, KeyName, Mods};
use soyuz_mcp::client::ServerStatus;

use self::text_area::TextArea;
use super::InputState;
use super::dialogs::TuiDialog;
use super::dialogs::plan_json;
use crate::app::App;
use crate::hits::{AiButton, CardButton};
use crate::layout::HitTarget;
use crate::view::ai::{Built, PopupItem};
use crate::widgets::{EditOutcome, LineEdit, Menu, MenuItem, MenuOutcome, SelectList};

/// `AiCard` / `AiCardButton` id (and task) of what is not a plan card: a task end has
/// `id: OTHER, task`, the cost question `id: OTHER, task: OTHER`.
pub const OTHER: u64 = u64::MAX;
/// `AiMcpUndo(MCP_CLEAR)`: the approvals area's Clear (hide finished requests).
pub const MCP_CLEAR: usize = usize::MAX;
/// `AiRow(CHIPS + n)`: the chips and the smart search card's controls ([`Chip`]), a range
/// far above any list row.
pub const CHIPS: usize = usize::MAX - 1024;
/// Lines the wheel scrolls.
const WHEEL_LINES: usize = 3;
/// The tag of this panel's scratch edit tokens (`EditPurpose::Scratch`), so they never
/// meet another owner's.
const TOKEN_TAG: u64 = 0x4149_0000_0000_0000;
/// The next scratch edit number.
static NEXT_EDIT: AtomicU64 = AtomicU64::new(1);

/// The part of the panel that has the keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Part {
    /// The composer (chat tab).
    #[default]
    Composer,
    /// The smart search card above the composer (chat tab).
    Smart,
    /// The chat's cards.
    Cards,
    /// The recipes / audit list.
    List,
    /// The audit filter.
    Filter,
}

/// A chip of the panel or a control of the smart search card (`AiRow(CHIPS + n)`; the
/// model chip is `AiButton::Profile`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    /// Fast / strong routing.
    Route,
    /// The current folder's assistant access.
    Access,
    /// The tools and MCP servers.
    Tools,
    /// Show / hide the smart search card.
    Smart,
    SmartClose,
    SmartInput,
    SmartQuery,
    SmartRun,
    SmartAsk,
    /// Remove filter `n` of the smart search.
    Pill(u8),
}

impl Chip {
    const PILLS: usize = 64;

    fn index(self) -> usize {
        match self {
            Chip::Route => 0,
            Chip::Access => 1,
            Chip::Tools => 2,
            Chip::Smart => 3,
            Chip::SmartClose => 4,
            Chip::SmartInput => 5,
            Chip::SmartQuery => 6,
            Chip::SmartRun => 7,
            Chip::SmartAsk => 8,
            Chip::Pill(n) => Chip::PILLS + usize::from(n),
        }
    }

    /// Its click target.
    pub fn target(self) -> HitTarget {
        HitTarget::AiRow(CHIPS + self.index())
    }

    /// The chip of an `AiRow` index, if it is one.
    pub fn of(i: usize) -> Option<Chip> {
        let n = i.checked_sub(CHIPS)?;
        Some(match n {
            0 => Chip::Route,
            1 => Chip::Access,
            2 => Chip::Tools,
            3 => Chip::Smart,
            4 => Chip::SmartClose,
            5 => Chip::SmartInput,
            6 => Chip::SmartQuery,
            7 => Chip::SmartRun,
            8 => Chip::SmartAsk,
            n if (Chip::PILLS..Chip::PILLS + 256).contains(&n) => Chip::Pill((n - Chip::PILLS) as u8),
            _ => return None,
        })
    }
}

/// What a row of the panel's menus does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAct {
    /// The profile for this chat (`None`: routing from the settings).
    Profile(Option<String>),
    Route(Route),
    /// The assistant access of a folder (`None`: inherit).
    Access(PathBuf, Option<AiAccess>),
    /// Ask the assistant to use this tool (text for the composer).
    Tool(String),
    /// The AI settings.
    Settings,
    /// A caption (disabled).
    Nothing,
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

/// A plan's arguments out in `$EDITOR` (one at a time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct External {
    pub token: u64,
    pub sel: Sel,
    pub path: PathBuf,
    /// The text the editor started with.
    pub original: String,
    /// Asked from the JSON editor dialog (the result goes back into it).
    pub from_dialog: bool,
}

/// What this part keeps between frames (`InputState::ai`).
#[derive(Debug)]
pub struct State {
    /// The panel has the keyboard.
    pub focused: bool,
    pub part: Part,
    /// The composer's cursor and scroll (the text is the core's `panel.composer`).
    pub composer: TextArea,
    /// The width the composer's lines wrap at and its rect (`view::ai::prepare`).
    pub wrap: usize,
    pub field_rect: Rect,
    /// An Enter in this frame has typed text after it (a paste without bracketed paste):
    /// a new line, not Send.
    pub burst: bool,
    /// The smart search card's field with the keyboard (0: the words, 1: the query) and
    /// their cursors (the texts are the core's `ai.smart`).
    pub smart_field: u8,
    pub smart_edit: LineEdit,
    pub query_edit: LineEdit,
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
    /// The open menu of a chip (model, access, tools).
    pub menu: Option<Menu<MenuAct>>,
    /// Where the menu may draw (the panel and the files left of it).
    pub menu_screen: Rect,
    /// The plan arguments out in `$EDITOR`.
    pub external: Option<External>,
    /// The body as built this frame (`view::ai::prepare`).
    pub view: Built,
}

impl Default for State {
    fn default() -> Self {
        State {
            focused: false,
            part: Part::Composer,
            composer: TextArea::new(),
            wrap: 0,
            field_rect: Rect::default(),
            burst: false,
            smart_field: 0,
            smart_edit: LineEdit::new(),
            query_edit: LineEdit::new(),
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
            menu: None,
            menu_screen: Rect::default(),
            external: None,
            view: Built::default(),
        }
    }
}

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    core.ai.panel.open && st.ai.focused
}

/// Every frame before the body is built (`view::ai::prepare`): the core's focus requests
/// (Ctrl+I, an MCP request, smart search) and the part that fits the tab.
pub fn sync(core: &mut AppCore, st: &mut State) {
    if !core.ai.panel.open {
        st.focused = false;
        st.delete_armed = None;
        st.menu = None;
        return;
    }
    if std::mem::take(&mut core.ai.panel.focus_composer) {
        st.focused = true;
        st.part = Part::Composer;
        st.composer = TextArea::at_end(&core.ai.panel.composer);
    }
    if std::mem::take(&mut core.ai.smart.focus) && core.ai.smart.open {
        st.focused = true;
        st.part = Part::Smart;
        st.smart_field = 0;
        st.smart_edit = LineEdit::at_end(&core.ai.smart.input);
        core.ai.panel.tab = PanelTab::Chat;
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
    if st.part == Part::Smart && !core.ai.smart.open {
        st.part = Part::Composer;
    }
    st.part = match (core.ai.panel.tab, st.part) {
        (PanelTab::Chat, Part::Composer | Part::Cards | Part::Smart) => st.part,
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

// ---- every frame ----------------------------------------------------------------------

/// Every frame, before the input is routed (`input::route`): hand finished `$EDITOR`
/// runs back, run the JSON editor's "Edit in $EDITOR", keep the clock ticking while a task
/// runs, and take Esc (stop the running task) and `ToggleAiPanel` (focus an open panel)
/// from the parts without a text field. Returns the events the rest of the input sees.
pub fn arm<'a>(app: &mut App, events: &'a [Event]) -> Cow<'a, [Event]> {
    finish_external(app);
    dialog_requests(app);
    tick(app);
    if !app.core.ai.panel.open || events.is_empty() {
        return Cow::Borrowed(events);
    }
    use crate::input::TuiFocus as F;
    if !matches!(app.focus(), F::List | F::Sidebar | F::Preview | F::Jobs) {
        return Cow::Borrowed(events);
    }
    let cmd_is_ctrl = app.core.config.keys.cmd_is_ctrl;
    let toggle = app.core.keymap.chords_for(&Action::ToggleAiPanel);
    let mut out: Vec<Event> = Vec::new();
    let mut taken = false;
    for ev in events {
        let chord = match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                crate::keys::frame(std::slice::from_ref(ev), cmd_is_ctrl, cfg!(windows)).events.into_iter().find_map(
                    |e| match e {
                        InputEvent::Chord(c) => Some(c),
                        _ => None,
                    },
                )
            }
            _ => None,
        };
        if let Some(c) = chord {
            if c.key == KeyName::Escape && c.mods.is_empty() && panel::escape_stops_task(&app.core) {
                apply(app, vec![PanelAct::Stop]);
                taken = true;
                continue;
            }
            if toggle.contains(&c) && !app.input.ai.focused {
                focus_panel(app);
                taken = true;
                continue;
            }
        }
        out.push(ev.clone());
    }
    if taken { Cow::Owned(out) } else { Cow::Borrowed(events) }
}

/// `ToggleAiPanel` on an open panel without the keyboard: give it the keyboard (the
/// composer on the chat tab).
fn focus_panel(app: &mut App) {
    let st = &mut app.input.ai;
    st.focused = true;
    if app.core.ai.panel.tab == PanelTab::Chat {
        st.part = Part::Composer;
        st.composer = TextArea::at_end(&app.core.ai.panel.composer);
    }
}

/// A redraw once a second while a task runs ("Working · 3s") or an MCP request counts
/// down.
fn tick(app: &mut App) {
    if !app.core.ai.panel.open {
        return;
    }
    let started = app.core.ai.running.as_ref().map(|r| r.started);
    let countdown =
        app.core.services.mcp.approvals.lock().cards.iter().any(|c| c.is_pending() && c.card.expires.is_some());
    let ms = match started {
        Some(t) => 1000 - (t.elapsed().as_millis() % 1000) as u64,
        None if countdown => 1000,
        None => return,
    };
    app.host.redraw_after(Duration::from_millis(ms.max(20)));
}

/// The JSON editor dialog asked for `$EDITOR` (Ctrl+E, its button) or needs the screen
/// size.
fn dialog_requests(app: &mut App) {
    let area = app.regions().area;
    let Some(TuiDialog::PlanJson(s)) = app.input.dialogs.tui.top_mut() else { return };
    s.screen = (area.width, area.height);
    if !std::mem::take(&mut s.editor) {
        return;
    }
    let Some(sel) = s.card else { return };
    let text = s.text.clone();
    start_external(app, sel, text, true);
}

/// Write `text` to a scratch file and suspend to the editor on it.
pub fn start_external(app: &mut App, sel: Sel, text: String, from_dialog: bool) {
    if app.input.ai.external.is_some() || app.suspend_pending() {
        return;
    }
    let token = TOKEN_TAG | NEXT_EDIT.fetch_add(1, Ordering::Relaxed);
    let dir = soyuz_core::config::config_dir().join("tmp");
    let path = dir.join(format!("ai-plan-{token:x}.json"));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, text.as_bytes())) {
        app.core.ui.toasts.push(Toast::error(tr!("tui-ai-editor-failed")).body(e.to_string()));
        return;
    }
    app.input.ai.external = Some(External { token, sel, path: path.clone(), original: text, from_dialog });
    app.request_suspend(SuspendRequest::Editor { path, purpose: EditPurpose::Scratch(token) });
}

/// The editor of [`State::external`] exited: read the file back, validate it and send the
/// edit, or show the error in the JSON editor.
fn finish_external(app: &mut App) {
    let Some(token) = app.input.ai.external.as_ref().map(|x| x.token) else { return };
    let Some(done) = app.take_finished_edit(token) else { return };
    let Some(x) = app.input.ai.external.take() else { return };
    let read = std::fs::read_to_string(&x.path);
    let _ = std::fs::remove_file(&x.path);
    let Some(card) = plan_json::waiting_card(&app.core, x.sel) else { return };
    match done.exit {
        // No editor ran (a toast said why): the built-in editor instead.
        None => {
            if !x.from_dialog {
                plan_json::open_with(app, x.sel, &card, x.original, None);
            }
            return;
        }
        // The editor failed (a toast said so): nothing is sent.
        Some(c) if c != 0 => return,
        Some(_) => {}
    }
    let text = match read {
        Ok(t) => text_area::clean(&t),
        Err(e) => {
            app.core.ui.toasts.push(Toast::error(tr!("tui-ai-editor-failed")).body(e.to_string()));
            return;
        }
    };
    if x.from_dialog {
        if let Some(TuiDialog::PlanJson(s)) = app.input.dialogs.tui.top_mut()
            && s.card == Some(x.sel)
        {
            s.set_text(text);
            if plan_json::submit(&mut app.core, s) {
                app.input.dialogs.tui.pop();
            }
        }
        return;
    }
    if text.trim() == x.original.trim() {
        return;
    }
    match panel::edit::args_from_json(&card, &text) {
        Ok(args) => plan_json::decide(&mut app.core, x.sel, args),
        Err(e) => plan_json::open_with(app, x.sel, &card, text, Some(e)),
    }
}

// ---- keys -----------------------------------------------------------------------------

/// This frame's events while it has the focus.
pub fn route(app: &mut App, events: &[Event]) {
    // A press outside the panel takes the keyboard away (`mouse::route` handled it).
    for e in events {
        if let Event::Mouse(m) = e
            && matches!(m.kind, MouseEventKind::Down(_))
            && app.input.ai.menu.is_none()
            && !matches!(crate::input::mouse::hit(&app.hits, m.column, m.row).map(|(_, t)| t),
                Some(t) if is_ours(t) || matches!(t, HitTarget::MenuItem { .. }))
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
        if app.input.ai.menu.is_some() {
            menu_key(app, &ev);
            frame.consumed[i] = true;
            i += 1;
            continue;
        }
        app.input.ai.burst = frame.events[i + 1..].iter().any(|e| matches!(e, InputEvent::Text(_)));
        let handled = common_key(app, &ev, &mut acts)
            || match app.input.ai.part {
                Part::Composer => composer_key(app, &ev, &mut acts),
                Part::Smart => smart_key(app, &ev, &mut acts),
                Part::Cards => card_key(app, &ev, &mut acts),
                Part::List => list_key(app, &ev, &mut acts),
                Part::Filter => filter_key(app, &ev),
            };
        if handled {
            frame.consumed[i] = true;
            // The text a letter key types (cards and lists only: the fields take it).
            if let (InputEvent::Chord(_), Some(InputEvent::Text(_))) = (&ev, frame.events.get(i + 1))
                && !matches!(app.input.ai.part, Part::Composer | Part::Filter | Part::Smart)
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
    let text = matches!(app.input.ai.part, Part::Composer | Part::Filter | Part::Smart);
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

/// Alt+Enter, Ctrl+J or Shift+Enter (the kitty protocol tells it from Enter): a new line.
pub fn is_newline(ev: &InputEvent) -> bool {
    match ev {
        InputEvent::Chord(c) => {
            (c.key == KeyName::Enter && (c.mods == Mods::ALT || c.mods == Mods::SHIFT))
                || (c.key == KeyName::J && (c.mods == Mods::CTRL || c.mods == Mods::CMD))
        }
        _ => false,
    }
}

/// Keys of every part: tabs, new chat, the chips.
fn common_key(app: &mut App, ev: &InputEvent, acts: &mut Vec<PanelAct>) -> bool {
    let tab = |k: KeyName, n: KeyName| matches!(plain(ev), Some((x, false)) if x == k) || is_alt(ev, n);
    let chat = app.core.ai.panel.tab == PanelTab::Chat;
    let t = if tab(KeyName::F1, KeyName::Num1) {
        PanelTab::Chat
    } else if tab(KeyName::F2, KeyName::Num2) {
        PanelTab::Recipes
    } else if tab(KeyName::F3, KeyName::Num3) {
        PanelTab::Audit
    } else if is_ctrl(ev, KeyName::N) && chat {
        acts.push(PanelAct::NewChat);
        return true;
    } else if is_alt(ev, KeyName::M) {
        open_menu(app, MenuKind::Model, None);
        return true;
    } else if is_alt(ev, KeyName::R) {
        if app.core.ai.panel.profile_override.is_none() {
            acts.push(PanelAct::SetRoute(other_route(app.core.ai.panel.route)));
        }
        return true;
    } else if is_alt(ev, KeyName::F) {
        open_menu(app, MenuKind::Access, None);
        return true;
    } else if is_alt(ev, KeyName::O) {
        open_menu(app, MenuKind::Tools, None);
        return true;
    } else if is_alt(ev, KeyName::S) && chat {
        toggle_smart(app);
        return true;
    } else {
        return false;
    };
    set_tab(app, t, acts);
    true
}

fn other_route(r: Route) -> Route {
    match r {
        Route::Fast => Route::Strong,
        Route::Strong => Route::Fast,
    }
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

/// Send the composer's text (nothing while a task runs or the text is blank).
fn send(app: &mut App, acts: &mut Vec<PanelAct>) {
    if app.core.ai.running.is_none() && !app.core.ai.panel.composer.trim().is_empty() {
        acts.push(PanelAct::Send);
        app.input.ai.composer = TextArea::new();
        app.input.ai.scroll_up = 0;
    }
}

fn composer_key(app: &mut App, ev: &InputEvent, acts: &mut Vec<PanelAct>) -> bool {
    let popup = !app.input.ai.view.popup.is_empty();
    if is_newline(ev) {
        let st = &mut app.input.ai;
        st.composer.newline(&mut app.core.ai.panel.composer);
        st.popup_closed = None;
        return true;
    }
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
            if app.input.ai.burst {
                // Pasted lines arriving as keys: the Enter between them is a new line.
                app.input.ai.composer.newline(&mut app.core.ai.panel.composer);
            } else {
                send(app, acts);
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
    let wrap = Some(st.wrap.max(1));
    match st.composer.apply(&mut app.core.ai.panel.composer, ev, wrap) {
        EditOutcome::Ignored => false,
        EditOutcome::Consumed => {
            st.popup_closed = None;
            true
        }
        // Up on the first line, Down on the last and the like: nothing to do here.
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
    app.input.ai.composer = TextArea::at_end(&app.core.ai.panel.composer);
    app.input.ai.popup = 0;
    app.input.ai.part = Part::Composer;
}

// ---- smart search card ----------------------------------------------------------------

/// Show the smart search card with the keyboard in it, or hide it.
fn toggle_smart(app: &mut App) {
    let s = &mut app.core.ai.smart;
    s.open = !s.open;
    if s.open {
        s.focus = true;
    } else if app.input.ai.part == Part::Smart {
        app.input.ai.part = Part::Composer;
    }
}

fn smart_field(app: &mut App, field: u8) {
    let st = &mut app.input.ai;
    st.part = Part::Smart;
    st.smart_field = field;
    if field == 0 {
        st.smart_edit = LineEdit::at_end(&app.core.ai.smart.input);
    } else {
        st.query_edit = LineEdit::at_end(&app.core.ai.smart.query_text);
    }
}

fn smart_key(app: &mut App, ev: &InputEvent, acts: &mut Vec<PanelAct>) -> bool {
    if is_alt(ev, KeyName::Enter) {
        if !app.core.ai.smart.input.trim().is_empty() && app.core.ai.running.is_none() {
            acts.push(PanelAct::SmartAskModel);
        }
        return true;
    }
    let field = app.input.ai.smart_field;
    match plain(ev) {
        Some((KeyName::Escape, false)) => {
            app.core.ai.smart.open = false;
            app.input.ai.part = Part::Composer;
            return true;
        }
        Some((KeyName::Tab, false)) | Some((KeyName::ArrowDown, false)) if field == 0 => {
            smart_field(app, 1);
            return true;
        }
        Some((KeyName::ArrowUp, false)) | Some((KeyName::Tab, true)) if field == 1 => {
            smart_field(app, 0);
            return true;
        }
        Some((KeyName::Tab, false)) => {
            app.input.ai.part = Part::Composer;
            return true;
        }
        Some((KeyName::Enter, false)) => {
            if !app.core.ai.smart.query_text.trim().is_empty() {
                acts.push(PanelAct::SmartRun);
            }
            return true;
        }
        Some((KeyName::Backspace, false)) if field == 0 && app.core.ai.smart.input.is_empty() => {
            let s = &mut app.core.ai.smart;
            if s.pills.pop().is_some() {
                s.query_text = smart_search::to_text(&s.pills);
            }
            return true;
        }
        _ => {}
    }
    let st = &mut app.input.ai;
    let s = &mut app.core.ai.smart;
    let (text, edit) =
        if field == 0 { (&mut s.input, &mut st.smart_edit) } else { (&mut s.query_text, &mut st.query_edit) };
    let before = text.clone();
    let out = edit.apply(text, ev);
    if *text != before {
        if field == 0 {
            acts.push(PanelAct::SmartParse);
        } else {
            s.pills = smart_search::parse_text(&s.query_text);
        }
    }
    !matches!(out, EditOutcome::Ignored)
}

// ---- cards ----------------------------------------------------------------------------

/// What a key does to a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKey {
    Approve,
    Reject,
    ApproveKind,
    /// The names of a rename plan, the JSON arguments otherwise.
    Edit,
    /// The JSON arguments (any tool).
    EditJson,
    /// The JSON arguments in `$EDITOR`.
    EditExternal,
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
        (KeyName::E, true) => CardKey::EditExternal,
        (KeyName::J, false) => CardKey::EditJson,
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

/// An edit key on a waiting card: the name table, the JSON editor or `$EDITOR`.
fn edit_card(app: &mut App, sel: Sel, card: &PlanCard, k: CardKey) {
    match k {
        CardKey::Edit if is_rename(&card.tool) && !card.plan.items.is_empty() => {
            super::dialogs::plan_names::open(app, sel, card);
        }
        // Items and destination; tools without that form edit as JSON.
        CardKey::Edit if super::dialogs::plan_form::open(app, sel, card) => {}
        CardKey::Edit | CardKey::EditJson => plan_json::open(app, sel, card),
        CardKey::EditExternal => start_external(app, sel, panel::edit::card_args_json(card), false),
        _ => {}
    }
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
        CardKey::Edit | CardKey::EditJson | CardKey::EditExternal if pending => {
            let card = card.clone();
            edit_card(app, Sel::Plan { id, task }, &card, k);
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
        CardKey::Edit | CardKey::EditJson | CardKey::EditExternal if pending => {
            let card = c.card.clone();
            drop(a);
            edit_card(app, Sel::Mcp(id), &card, k);
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

// ---- menus ----------------------------------------------------------------------------

/// Which chip menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Model,
    Access,
    Tools,
}

/// The items of chip menu `kind`.
pub fn menu_items(core: &AppCore, kind: MenuKind) -> Vec<MenuItem<MenuAct>> {
    let caption = |s: &str| MenuItem::run(s, MenuAct::Nothing).enabled(false);
    let mut v = Vec::new();
    match kind {
        MenuKind::Model => {
            let over = core.ai.panel.profile_override.as_deref();
            v.push(caption(tr!("ai-model-caption")));
            v.push(MenuItem::run(tr!("ai-model-auto"), MenuAct::Profile(None)).checked(over.is_none()));
            for p in &core.config.ai.profiles {
                let label =
                    if p.model.is_empty() { p.name.clone() } else { format!("{} \u{00b7} {}", p.name, p.model) };
                let on = over == Some(p.name.as_str());
                v.push(MenuItem::run(crate::dialog::safe(&label), MenuAct::Profile(Some(p.name.clone()))).checked(on));
            }
            if over.is_none() {
                let r = core.ai.panel.route;
                v.push(MenuItem::Separator);
                v.push(MenuItem::run(tr!("ai-route-fast"), MenuAct::Route(Route::Fast)).checked(r == Route::Fast));
                v.push(
                    MenuItem::run(tr!("ai-route-strong"), MenuAct::Route(Route::Strong)).checked(r == Route::Strong),
                );
            }
            v.push(MenuItem::Separator);
            v.push(MenuItem::run(tr!("ai-manage-providers"), MenuAct::Settings));
        }
        MenuKind::Access => {
            let Location::Dir(dir) = &core.ws.pane().location else { return v };
            let (access, explicit) = soyuz_appcore::ai::state::access_for(core, dir);
            v.push(caption(tr!("ai-access-menu-caption")));
            for (a, label) in [
                (AiAccess::None, tr!("ai-access-menu-none")),
                (AiAccess::Metadata, tr!("ai-access-menu-metadata")),
                (AiAccess::Contents, tr!("ai-access-menu-contents")),
            ] {
                v.push(MenuItem::run(label, MenuAct::Access(dir.clone(), Some(a))).checked(explicit && access == a));
            }
            v.push(MenuItem::run(tr!("ai-access-inherit"), MenuAct::Access(dir.clone(), None)).checked(!explicit));
        }
        MenuKind::Tools => {
            let max_risk = soyuz_appcore::ai::agent::Limits::from_config(&core.config.ai.agent).max_risk;
            let builtin = soyuz_ai::tools::catalog().iter().filter(|d| d.risk <= max_risk).count();
            let ceiling = risk_word(max_risk);
            v.push(caption("Soyuz"));
            v.push(caption(&tr!("ai-tools-builtin", count = builtin, risk = ceiling)));
            v.push(MenuItem::Separator);
            v.push(caption(tr!("ai-tools-caption-connected")));
            let status = core.ai.external.status();
            let tools = core.ai.external.tools();
            if status.is_empty() {
                v.push(caption(tr!("ai-tools-none")));
            }
            let connecting = core.ai.external_connecting();
            for (name, st) in &status {
                let n = tools.iter().filter(|t| &t.server == name).count();
                let text = server_status(st, n, connecting);
                v.push(caption(&crate::dialog::safe(&format!("{name} \u{00b7} {text}"))));
                for t in tools.iter().filter(|t| &t.server == name) {
                    let allowed = t.risk <= max_risk;
                    let risk = risk_label(t.risk);
                    let label = crate::dialog::safe(&format!("  {} \u{00b7} {risk}", t.remote));
                    v.push(MenuItem::run(label, MenuAct::Tool(t.name.clone())).enabled(allowed));
                }
            }
            v.push(MenuItem::Separator);
            v.push(MenuItem::run(tr!("ai-manage-connections"), MenuAct::Settings));
        }
    }
    v
}

/// The risk ceiling's word for `ai-tools-builtin`.
fn risk_word(r: Risk) -> &'static str {
    match r {
        Risk::Read => "read",
        Risk::Write => "write",
        Risk::Destructive => "destructive",
    }
}

/// A tool's risk.
fn risk_label(r: Risk) -> &'static str {
    match r {
        Risk::Read => tr!("ai-risk-read"),
        Risk::Write => tr!("ai-risk-write"),
        Risk::Destructive => tr!("ai-risk-destructive"),
    }
}

/// A server's status line in the tools menu (`tools`: its tools).
pub fn server_status(st: &ServerStatus, tools: usize, connecting: bool) -> String {
    match st {
        ServerStatus::Connected { .. } => tr!("ai-tools-count", count = tools),
        ServerStatus::Failed(e) => e.clone(),
        ServerStatus::Disabled => tr!("ai-tools-disabled").into(),
        ServerStatus::NotConnected if connecting => tr!("ai-tools-connecting").into(),
        ServerStatus::NotConnected => tr!("ai-tools-not-connected").into(),
    }
}

/// Open chip menu `kind` at `at` (else under its chip in the last frame, else at the
/// panel's top). The panel takes the keyboard.
pub fn open_menu(app: &mut App, kind: MenuKind, at: Option<(u16, u16)>) {
    let items = menu_items(&app.core, kind);
    if items.is_empty() {
        return;
    }
    let target = match kind {
        MenuKind::Model => HitTarget::AiButton(AiButton::Profile),
        MenuKind::Access => Chip::Access.target(),
        MenuKind::Tools => Chip::Tools.target(),
    };
    let anchor = at
        .or_else(|| app.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| (r.x, r.y)))
        .unwrap_or((app.regions().ai.x + 1, app.regions().ai.y));
    app.input.ai.focused = true;
    app.input.ai.menu = Some(Menu::new(items, anchor));
}

fn menu_key(app: &mut App, ev: &InputEvent) {
    let screen = app.input.ai.menu_screen;
    let Some(menu) = app.input.ai.menu.as_mut() else { return };
    match menu.apply(ev, screen) {
        MenuOutcome::Activated(a) => run_menu(app, a),
        MenuOutcome::Close => app.input.ai.menu = None,
        MenuOutcome::Consumed | MenuOutcome::Ignored => {}
    }
}

fn run_menu(app: &mut App, a: MenuAct) {
    app.input.ai.menu = None;
    let act = match a {
        MenuAct::Profile(p) => PanelAct::SetProfile(p),
        MenuAct::Route(r) => PanelAct::SetRoute(r),
        MenuAct::Access(path, access) => PanelAct::FolderAccess(path, access),
        MenuAct::Settings => PanelAct::OpenSettings,
        MenuAct::Tool(name) => {
            let c = &mut app.core.ai.panel.composer;
            if !c.is_empty() && !c.ends_with(char::is_whitespace) {
                c.push(' ');
            }
            c.push_str(&tr!("ai-use-tool", name = name.as_str()));
            c.push(' ');
            app.core.ai.panel.tab = PanelTab::Chat;
            app.core.ai.panel.focus_composer = true;
            return;
        }
        MenuAct::Nothing => return,
    };
    apply(app, vec![act]);
}

// ---- mouse ----------------------------------------------------------------------------

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

/// A mouse event over one of its click targets (`input::mouse` dispatches by target), or
/// anywhere while a chip menu is open; returns whether it was taken.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) -> bool {
    if app.input.ai.menu.is_some() {
        menu_mouse(app, target, ev);
        return true;
    }
    if !is_ours(target) || !app.core.ai.panel.open {
        return false;
    }
    match ev.kind {
        MouseEventKind::ScrollUp => scroll(app, -(WHEEL_LINES as i64)),
        MouseEventKind::ScrollDown => scroll(app, WHEEL_LINES as i64),
        MouseEventKind::Down(MouseButton::Left) => {
            app.input.ai.focused = true;
            let mut acts = Vec::new();
            click(app, target, ev, &mut acts);
            if !acts.is_empty() {
                apply(app, acts);
            }
        }
        _ => {}
    }
    true
}

fn menu_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) {
    let MouseEventKind::Down(button) = ev.kind else { return };
    let screen = app.input.ai.menu_screen;
    let Some(menu) = app.input.ai.menu.as_mut() else { return };
    match target {
        HitTarget::MenuItem { depth, index } if button == MouseButton::Left => {
            match menu.click(usize::from(depth), index, screen) {
                MenuOutcome::Activated(a) => run_menu(app, a),
                MenuOutcome::Close => app.input.ai.menu = None,
                MenuOutcome::Consumed | MenuOutcome::Ignored => {}
            }
        }
        HitTarget::MenuItem { .. } => {}
        _ => app.input.ai.menu = None,
    }
}

fn click(app: &mut App, target: HitTarget, ev: &MouseEvent, acts: &mut Vec<PanelAct>) {
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
                // Put the cursor where the click was (after the "> " / "  " lead).
                let st = &mut app.input.ai;
                let r = st.field_rect;
                if r.contains(ratatui::layout::Position { x: ev.column, y: ev.row }) {
                    let row = usize::from(ev.row - r.y);
                    let col = usize::from(ev.column.saturating_sub(r.x + 2));
                    st.composer.click(&app.core.ai.panel.composer, Some(st.wrap.max(1)), row, col);
                }
            }
        }
        HitTarget::AiButton(b) => match b {
            AiButton::Send => send(app, acts),
            AiButton::Stop => acts.push(PanelAct::Stop),
            AiButton::NewChat => acts.push(PanelAct::NewChat),
            AiButton::Close => {
                app.input.ai.focused = false;
                acts.push(PanelAct::Close);
            }
            AiButton::Settings => acts.push(PanelAct::OpenSettings),
            AiButton::Profile => open_menu(app, MenuKind::Model, Some((ev.column, ev.row))),
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
        HitTarget::AiRow(i) if Chip::of(i).is_some() => {
            if let Some(c) = Chip::of(i) {
                chip_click(app, c, ev, acts);
            }
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

fn chip_click(app: &mut App, c: Chip, ev: &MouseEvent, acts: &mut Vec<PanelAct>) {
    let at = Some((ev.column, ev.row));
    match c {
        Chip::Route => {
            if app.core.ai.panel.profile_override.is_none() {
                acts.push(PanelAct::SetRoute(other_route(app.core.ai.panel.route)));
            }
        }
        Chip::Access => open_menu(app, MenuKind::Access, at),
        Chip::Tools => open_menu(app, MenuKind::Tools, at),
        Chip::Smart => toggle_smart(app),
        Chip::SmartClose => {
            app.core.ai.smart.open = false;
            app.input.ai.part = Part::Composer;
        }
        Chip::SmartInput => smart_field(app, 0),
        Chip::SmartQuery => smart_field(app, 1),
        Chip::SmartRun => {
            if !app.core.ai.smart.query_text.trim().is_empty() {
                acts.push(PanelAct::SmartRun);
            }
        }
        Chip::SmartAsk => {
            if !app.core.ai.smart.input.trim().is_empty() && app.core.ai.running.is_none() {
                acts.push(PanelAct::SmartAskModel);
            }
        }
        Chip::Pill(n) => {
            let s = &mut app.core.ai.smart;
            if usize::from(n) < s.pills.len() {
                s.pills.remove(usize::from(n));
                s.query_text = smart_search::to_text(&s.pills);
            }
        }
    }
}

/// While `true`, every mouse event goes to [`on_mouse`] wherever it is (a drag that
/// leaves the targets, an overlay's capture); the target is `HitTarget::Overlay` when
/// nothing is under the pointer. A chip menu captures the mouse while it is open.
pub fn captures_mouse(core: &AppCore, st: &InputState) -> bool {
    core.ai.panel.open && st.ai.menu.is_some()
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
        assert_eq!(card_key_of(&chord("J")), Some(CardKey::EditJson));
        assert_eq!(card_key_of(&chord("Shift+E")), Some(CardKey::EditExternal));
        assert_eq!(card_key_of(&chord("Enter")), Some(CardKey::Enter));
        assert_eq!(card_key_of(&chord("U")), Some(CardKey::Undo));
        assert_eq!(card_key_of(&chord("S")), Some(CardKey::SaveRecipe));
        assert_eq!(card_key_of(&chord("Ctrl+A")), None, "select all stays global");
        assert_eq!(card_key_of(&chord("Shift+A")), None);
        assert_eq!(card_key_of(&InputEvent::Text("a".into())), None);
    }

    #[test]
    fn newline_chords() {
        for k in ["Alt+Enter", "Ctrl+J", "Shift+Enter"] {
            assert!(is_newline(&chord(k)), "{k}");
        }
        for k in ["Enter", "J", "Ctrl+Enter", "Alt+J", "Ctrl+Shift+J"] {
            assert!(!is_newline(&chord(k)), "{k}");
        }
    }

    #[test]
    fn chip_targets_round_trip() {
        for c in [
            Chip::Route,
            Chip::Access,
            Chip::Tools,
            Chip::Smart,
            Chip::SmartClose,
            Chip::SmartInput,
            Chip::SmartQuery,
            Chip::SmartRun,
            Chip::SmartAsk,
            Chip::Pill(0),
            Chip::Pill(9),
        ] {
            let HitTarget::AiRow(i) = c.target() else { panic!("{c:?}") };
            assert_eq!(Chip::of(i), Some(c));
        }
        assert_eq!(Chip::of(3), None, "list rows are not chips");
        assert_eq!(Chip::of(MCP_CLEAR), None);
    }

    #[test]
    fn server_status_lines() {
        assert_eq!(server_status(&ServerStatus::Disabled, 0, false), tr!("ai-tools-disabled"));
        assert_eq!(server_status(&ServerStatus::Failed("no route".into()), 0, false), "no route");
        assert_eq!(server_status(&ServerStatus::NotConnected, 0, true), tr!("ai-tools-connecting"));
        assert_eq!(server_status(&ServerStatus::NotConnected, 0, false), tr!("ai-tools-not-connected"));
        let up = ServerStatus::Connected { server_name: "x".into(), tools: 2, modern: true };
        assert!(!server_status(&up, 2, false).is_empty());
        assert_eq!(risk_word(Risk::Read), "read");
        assert_eq!(risk_word(Risk::Destructive), "destructive");
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
