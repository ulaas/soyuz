//! Keys and mouse of the editor panel (`TuiFocus::Editor`) — Phase T4.
//!
//! The editor has the keyboard ([`State::focused`]) after a document asks for it (a new
//! document, F4 on an open one, a tab click, Esc in a bar: the core's `focus_request`
//! flags, taken in `view::editor::prepare`) or a click into it; Esc with nothing to close
//! (the core's `release_focus`), a click elsewhere or an action that moves the focus
//! hands it back to the file list.
//!
//! Keys, per frame ([`route`]):
//! 1. **Paste bursts.** Windows consoles deliver a paste as a stream of key presses
//!    (crossterm has no bracketed paste there). A run of at least [`BURST_MIN`] typing
//!    keys (characters, Enter, Tab, no Ctrl / Alt except AltGr) arriving in one batch is
//!    inserted as one paste (one undo step; Enter is a line break, not a command). A
//!    bracketed paste (`Event::Paste`) is a paste as well.
//! 2. **Terminal UI keys** first: Ctrl+H (Backspace or Ctrl+Backspace in many terminals)
//!    and Alt+Backspace delete the word before the caret, like Ctrl+Backspace; Esc closes
//!    the terminal UI's own bars (save as, resize) and then answers the external-change
//!    banner with "Keep my version"; Tab switches the hex find bar between bytes and text.
//! 3. **The editor commands** (`editors::handle_keys`, as in the GUI): every chord bound
//!    in `KeyContext::Editor` (`[keys.bindings]`; the GUI's defaults plus the terminal
//!    UI's, `crate::overlay::EDITOR_DEFAULTS`): save (Ctrl+S), save as (Ctrl+Shift+S /
//!    Alt+Shift+S), find (Ctrl+F), replace (Ctrl+R), go to (Ctrl+G), F3 / Shift+F3 next /
//!    previous, close (Ctrl+W, asks to save, `dialogs::unsaved`), Ctrl+PageUp / PageDown
//!    switch documents, Alt+M maximize, Alt+Shift+M the dock side, Alt+Z soft wrap, Alt+E
//!    the notice's action (reload a changed file, edit a lossy file anyway, allow editing
//!    a large hex file, OK), Alt+C / Alt+W / Alt+R the find options, Alt+A replace all,
//!    Alt+S resize an image; Esc closes a bar or leaves the editor. Global bindings the
//!    editor does not reserve (Ctrl+Q, the palette, F-keys, Alt+J the jobs panel, which
//!    takes the keyboard) run as actions; `FocusEditor` (Alt+E) comes back.
//! 4. **The focused bar's field** (a [`LineEdit`]) or **the document**: the core's text /
//!    hex editing (`TextDoc::input`, `HexDoc::input`: typing, Backspace / Delete, Home /
//!    End, Ctrl+arrows by word, Ctrl+Home / End, Shift selects, Ctrl+A, Ctrl+Z / Ctrl+Y,
//!    Ctrl+D, Tab indents). Up / Down / PageUp / PageDown move by visual rows here
//!    ([`vertical`]): the terminal measures wide characters in two cells. Copy / cut
//!    (Ctrl+C / Ctrl+X, Ctrl+Insert / Shift+Delete) go to the host's clipboard (OSC 52 and
//!    the local clipboard); Ctrl+V / Shift+Insert paste the editor's last copy when the
//!    terminal did not paste by itself (a terminal paste arrives as text). Images take
//!    the core's image keys (`ImageDoc::key`: R / Shift+R rotate, H / V flip, C crop,
//!    Ctrl+Z / Ctrl+Y); the crop bar (x, y, width, height) and the resize bar (width,
//!    height: an empty field keeps the aspect ratio, `50%` scales) are fields, Tab / Up /
//!    Down move between them, Enter applies.
//!
//! Mouse ([`click`], [`wheel`]): a tab selects (middle button closes), `×` closes, the
//! panel buttons dock / maximize / hide; in the text a press places the caret (Shift
//! extends, a double press selects the word, a triple the line) and dragging selects; in
//! the hex view a press picks the byte and the column; a bar row focuses its field; the
//! banner's buttons reload or keep the edits; the wheel scrolls.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use soyuz_appcore::core::{AppCore, Focus};
use soyuz_appcore::editors::image::ops::{ImgOp, NormRect, SaveMode};
use soyuz_appcore::editors::io::Want;
use soyuz_appcore::editors::text::TextDoc;
use soyuz_appcore::editors::text::buffer::{Pos, Sel};
use soyuz_appcore::editors::{self, DocKind, EditInput, KeyMods};
use soyuz_appcore::host::Host as _;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::config::{Action, KeyChord, KeyName, Mods};

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;
use crate::view::editor::image::{CROP_FIELDS, RESIZE_FIELDS, crop_fields, parse_crop, parse_resize};
use crate::view::editor::layout::LineLayout;
use crate::view::editor::text::line_rows;
use crate::view::editor::{BANNER_KEEP, BANNER_RELOAD, BAR_ROW, BTN_DOCK, BTN_HIDE, BTN_MAXIMIZE, EditorVm};
use crate::widgets::{EditOutcome, LineEdit};

/// Typing keys in one batch that make a paste burst.
pub const BURST_MIN: usize = 4;
/// Presses this close together on the same cell count as a double / triple press.
const MULTI_PRESS: Duration = Duration::from_millis(500);

/// Which part of a document has the keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bar {
    /// The text / bytes.
    #[default]
    Text,
    Find,
    Replace,
    Goto,
    SaveAs,
    /// The image's crop fields ([`DocView::fields`]).
    Crop,
    /// The image's resize fields ([`DocView::fields`]).
    Resize,
}

/// The text area's size and settings of the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geom {
    /// Text columns.
    pub cols: usize,
    /// Text rows.
    pub rows: usize,
    pub wrap: bool,
    pub tab: usize,
}

/// What the terminal UI keeps per document: scrolling, the vertical motion's column,
/// the bars' text cursors.
#[derive(Clone, Debug, Default)]
pub struct DocView {
    /// First line shown.
    pub top: usize,
    /// First wrapped row of that line shown.
    pub top_row: usize,
    /// First display column shown (no soft wrap).
    pub left: usize,
    /// The column Up / Down keep (in the row).
    pub want_x: Option<usize>,
    pub bar: Bar,
    pub find_edit: LineEdit,
    pub replace_edit: LineEdit,
    pub goto_edit: LineEdit,
    pub geom: Geom,
    /// The crop / resize bar's fields, the one with the keyboard, their cursors and the
    /// last error.
    pub fields: Vec<String>,
    pub field: usize,
    pub field_edits: Vec<LineEdit>,
    pub field_error: Option<String>,
}

/// The "Save as" bar of a document.
#[derive(Debug, Default)]
pub struct SaveAs {
    pub doc: u64,
    pub text: String,
    pub edit: LineEdit,
    pub error: Option<String>,
    /// The copy is being written.
    pub busy: bool,
    rx: Option<Receiver<Result<(), String>>>,
    target: Option<PathBuf>,
    /// How the copy opens (text or image; `None`: text).
    want: Option<Want>,
}

/// What the editor keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The editor has the keyboard.
    pub focused: bool,
    /// Per document id.
    pub views: HashMap<u64, DocView>,
    pub save_as: Option<SaveAs>,
    /// The frame's model, built by `view::editor::prepare`.
    pub vm: EditorVm,
    /// The editor's last copy or cut (Ctrl+V when the terminal does not paste).
    pub clip: Option<String>,
    /// The last press in the text: time, cell, count (double / triple press).
    last_press: Option<(Instant, u16, u16, u8)>,
}

/// Whether the editor has the keyboard (it must be shown).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    st.editor.focused && crate::view::editor::placement(core).is_some()
}

/// A run of the frame's events.
#[derive(Debug, PartialEq)]
pub enum Part {
    /// Keys and mouse events, in order.
    Keys(Vec<Event>),
    /// Text to insert as one paste (a paste burst or a bracketed paste).
    Text(String),
}

/// What a typing key of a burst inserts (`None`: not a typing key).
fn burst_char(ev: &Event) -> Option<Option<char>> {
    let Event::Key(k) = ev else { return None };
    if k.kind == KeyEventKind::Release {
        // Windows reports releases between the presses: they do not break a burst.
        return Some(None);
    }
    let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
    let plain = !k.modifiers.intersects(ctrl_alt | KeyModifiers::SUPER | KeyModifiers::META);
    match k.code {
        KeyCode::Char(c) if plain && !c.is_control() => Some(Some(c)),
        // AltGr on Windows (Ctrl+Alt with a character that is not a letter or digit).
        KeyCode::Char(c) if k.modifiers.contains(ctrl_alt) && !c.is_ascii_alphanumeric() && !c.is_control() => {
            Some(Some(c))
        }
        KeyCode::Enter if k.modifiers.is_empty() => Some(Some('\n')),
        KeyCode::Tab if k.modifiers.is_empty() => Some(Some('\t')),
        _ => None,
    }
}

/// Split the frame's `events` into key runs and pastes: bracketed pastes, and runs of at
/// least [`BURST_MIN`] typing keys (with a character among them) in one batch.
pub fn split(events: &[Event]) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    let push_keys = |out: &mut Vec<Part>, evs: &[Event]| {
        if evs.is_empty() {
            return;
        }
        if let Some(Part::Keys(k)) = out.last_mut() {
            k.extend_from_slice(evs);
        } else {
            out.push(Part::Keys(evs.to_vec()));
        }
    };
    let mut i = 0;
    while i < events.len() {
        if let Event::Paste(s) = &events[i] {
            if !s.is_empty() {
                out.push(Part::Text(s.clone()));
            }
            i += 1;
            continue;
        }
        // The longest run of typing keys from here.
        let mut j = i;
        let mut text = String::new();
        let mut presses = 0;
        while j < events.len() {
            match burst_char(&events[j]) {
                Some(Some(c)) => {
                    text.push(c);
                    presses += 1;
                }
                Some(None) => {}
                None => break,
            }
            j += 1;
        }
        let has_char = text.chars().any(|c| c != '\n' && c != '\t');
        if presses >= BURST_MIN && has_char {
            out.push(Part::Text(text));
            i = j;
        } else {
            let end = j.max(i + 1);
            push_keys(&mut out, &events[i..end]);
            i = end;
        }
    }
    out
}

/// The core's modifiers of a chord.
fn key_mods(c: KeyChord) -> KeyMods {
    KeyMods {
        shift: c.mods.contains(Mods::SHIFT),
        command: c.mods.intersects(Mods::CTRL | Mods::CMD),
        alt: c.mods.contains(Mods::ALT),
    }
}

fn is(c: &KeyChord, mods: Mods, key: KeyName) -> bool {
    c.key == key && c.mods == mods
}

/// Keys while the editor has focus.
pub fn route(app: &mut App, events: &[Event]) {
    // A press outside the editor (handled by `mouse::route` already) takes the focus away.
    for ev in events {
        if let Event::Mouse(m) = ev
            && matches!(m.kind, MouseEventKind::Down(_))
        {
            let t = super::mouse::hit(&app.hits, m.column, m.row).map(|(_, t)| t);
            if !matches!(t, Some(HitTarget::EditorTab(_) | HitTarget::EditorClose(_) | HitTarget::EditorCell { .. })) {
                app.input.editor.focused = false;
            }
        }
    }
    for part in split(events) {
        if !has_focus(&app.core, &app.input) || super::dialogs::is_open(&app.core, &app.input.dialogs) {
            break;
        }
        match part {
            Part::Text(t) => insert_text(app, &t),
            Part::Keys(evs) => keys(app, &evs),
        }
    }
}

/// The view state of document `id`.
fn view(st: &mut State, id: u64) -> &mut DocView {
    st.views.entry(id).or_default()
}

/// A paste (burst or bracketed) into the focused bar or the document.
fn insert_text(app: &mut App, text: &str) {
    let Some(d) = app.core.editors.active_doc_mut() else { return };
    let id = d.id;
    let bar = view(&mut app.input.editor, id).bar;
    if bar != Bar::Text {
        let line: String = text.split(['\r', '\n']).next().unwrap_or("").to_string();
        bar_event(app, id, bar, &InputEvent::Text(line));
        return;
    }
    let tab = crate::view::editor::text::tab_width(&app.core.config.editor);
    let Some(d) = app.core.editors.active_doc_mut() else { return };
    match &mut d.kind {
        DocKind::Text(t) => {
            t.input(EditInput::Paste(text.to_string()), tab);
        }
        DocKind::Hex(h) => {
            h.input(EditInput::Text(text.replace(['\r', '\n'], "")));
        }
        _ => {}
    }
    view(&mut app.input.editor, id).want_x = None;
}

/// One run of key events.
fn keys(app: &mut App, evs: &[Event]) {
    let cmd_is_ctrl = app.core.config.keys.cmd_is_ctrl;
    let mut frame = crate::keys::frame(evs, cmd_is_ctrl, cfg!(windows));
    // Mouse drags select.
    for ev in evs {
        if let Event::Mouse(m) = ev {
            mouse_move(app, m);
        }
    }
    // 2. The terminal UI's own chords.
    for i in 0..frame.events.len() {
        let InputEvent::Chord(c) = frame.events[i] else { continue };
        // Ctrl+H (a legacy Ctrl+Backspace) and Alt+Backspace delete the word before.
        if is(&c, Mods::CTRL, KeyName::H) || is(&c, Mods::ALT, KeyName::Backspace) {
            frame.events[i] = InputEvent::Chord(KeyChord::new(Mods::CTRL, KeyName::Backspace));
            continue;
        }
        if local_key(app, c) {
            frame.consumed[i] = true;
        }
    }
    if !has_focus(&app.core, &app.input) {
        return;
    }
    // 3. The core's editor keys and the global bindings.
    let focus_before = (app.core.ui.focus, app.core.ui.address.editing, app.core.palette.open);
    let out = editors::handle_keys(&mut app.core, &mut frame);
    if out.release_focus {
        app.input.editor.focused = false;
    }
    let had_actions = !out.actions.is_empty();
    // Moving the keyboard to another panel leaves the editor (the jobs panel opens from a
    // frontend request, so the core's focus does not change).
    let leaves = out.actions.iter().any(|a| {
        matches!(a, Action::FocusActivityPanel | Action::FocusSidebar | Action::FocusList | Action::FocusTerminal)
    });
    for a in out.actions {
        soyuz_appcore::actions::execute(&mut app.core, a, &app.host);
    }
    let focus_after = (app.core.ui.focus, app.core.ui.address.editing, app.core.palette.open);
    if leaves || had_actions && (focus_after != focus_before || app.core.ui.filter_focus) {
        app.input.editor.focused = false;
    }
    if !has_focus(&app.core, &app.input) {
        return;
    }
    // 4. The bar or the document.
    for i in 0..frame.events.len() {
        if frame.consumed[i] {
            continue;
        }
        let ev = frame.events[i].clone();
        if matches!(ev, InputEvent::Other) {
            continue;
        }
        let Some(d) = app.core.editors.active_doc() else { return };
        let id = d.id;
        let bar = view(&mut app.input.editor, id).bar;
        if bar == Bar::Text {
            doc_event(app, id, &ev);
        } else {
            bar_event(app, id, bar, &ev);
        }
    }
}

/// The terminal UI's own editor keys (not commands): Esc on its own bars and on the
/// external-change banner, Tab in the hex find bar. `true` when `c` was one.
fn local_key(app: &mut App, c: KeyChord) -> bool {
    let Some(d) = app.core.editors.active_doc_mut() else { return false };
    let id = d.id;
    let bar = app.input.editor.views.get(&id).map(|v| v.bar).unwrap_or_default();
    let esc = is(&c, Mods::empty(), KeyName::Escape);
    if esc && matches!(bar, Bar::SaveAs | Bar::Resize) {
        if bar == Bar::SaveAs {
            app.input.editor.save_as = None;
        }
        view(&mut app.input.editor, id).bar = Bar::Text;
        return true;
    }
    // Esc with no bar of the core's open: "Keep my version" on the banner.
    if esc && d.external.is_some() {
        let bars_open = match &d.kind {
            DocKind::Text(t) => t.find.open || t.goto.is_some(),
            DocKind::Hex(h) => h.find.is_some() || h.goto.is_some(),
            DocKind::Image(i) => i.crop_mode || i.crop.is_some(),
            _ => false,
        };
        if !bars_open {
            d.external = None;
            return true;
        }
    }
    if let DocKind::Hex(h) = &mut d.kind
        && bar == Bar::Find
        && is(&c, Mods::empty(), KeyName::Tab)
    {
        h.toggle_find_mode();
        return true;
    }
    false
}

/// The dock button: the other side (saved in `[editor] dock`).
fn toggle_dock(app: &mut App) {
    soyuz_appcore::editors::commands::execute(&mut app.core, Action::EditorToggleDock, &app.host);
}

/// One event for the focused bar's field.
fn bar_event(app: &mut App, id: u64, bar: Bar, ev: &InputEvent) {
    let shift_enter = matches!(ev, InputEvent::Chord(c) if is(c, Mods::SHIFT, KeyName::Enter));
    if matches!(bar, Bar::Crop | Bar::Resize) {
        field_event(app, id, bar, ev);
        return;
    }
    if bar == Bar::SaveAs {
        let Some(s) = app.input.editor.save_as.as_mut() else { return };
        if s.busy {
            return;
        }
        match s.edit.apply(&mut s.text, ev) {
            EditOutcome::Submit => start_save_as(app),
            EditOutcome::Consumed => s.error = None,
            _ => {}
        }
        return;
    }
    let st = &mut app.input.editor;
    let v = st.views.entry(id).or_default();
    let Some(d) = app.core.editors.docs.iter_mut().find(|d| d.id == id) else { return };
    match &mut d.kind {
        DocKind::Text(t) => match bar {
            Bar::Find => {
                if shift_enter {
                    t.find_step(false);
                    return;
                }
                let before = t.find.query.text.clone();
                match v.find_edit.apply(&mut t.find.query.text, ev) {
                    EditOutcome::Submit | EditOutcome::Down => t.find_step(true),
                    EditOutcome::Up => t.find_step(false),
                    EditOutcome::Tab | EditOutcome::ShiftTab if t.find.replace => {
                        v.bar = Bar::Replace;
                        v.replace_edit = LineEdit::at_end(&t.find.replace_text);
                    }
                    _ => {}
                }
                if t.find.query.text != before {
                    t.find_incremental();
                }
            }
            Bar::Replace => match v.replace_edit.apply(&mut t.find.replace_text, ev) {
                EditOutcome::Submit => t.replace_current(),
                EditOutcome::Tab | EditOutcome::ShiftTab => {
                    v.bar = Bar::Find;
                    v.find_edit = LineEdit::at_end(&t.find.query.text);
                }
                _ => {}
            },
            Bar::Goto => {
                let Some(text) = t.goto.as_mut() else { return };
                if v.goto_edit.apply(text, ev) == EditOutcome::Submit {
                    t.submit_goto();
                }
            }
            Bar::Text | Bar::SaveAs | Bar::Crop | Bar::Resize => {}
        },
        DocKind::Hex(h) => match bar {
            Bar::Find => {
                if shift_enter {
                    h.find_step(false);
                    return;
                }
                let Some(f) = h.find.as_mut() else { return };
                let before = f.text.clone();
                let out = v.find_edit.apply(&mut f.text, ev);
                if f.text != before {
                    f.status = None;
                    f.found = false;
                }
                match out {
                    EditOutcome::Submit | EditOutcome::Down => h.find_step(true),
                    EditOutcome::Up => h.find_step(false),
                    _ => {}
                }
            }
            Bar::Goto => {
                let Some(text) = h.goto.as_mut() else { return };
                if v.goto_edit.apply(text, ev) == EditOutcome::Submit {
                    h.submit_goto();
                    if h.goto.as_deref() == Some("") {
                        v.goto_edit = LineEdit::new();
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
}

/// One event for the document itself.
fn doc_event(app: &mut App, id: u64, ev: &InputEvent) {
    let tab = crate::view::editor::text::tab_width(&app.core.config.editor);
    let st = &mut app.input.editor;
    let v = st.views.entry(id).or_default();
    let Some(d) = app.core.editors.docs.iter_mut().find(|d| d.id == id) else { return };
    if let DocKind::Image(i) = &mut d.kind {
        // The core's image keys (rotate, flip, crop, undo / redo); the terminal fits the
        // image, so its zoom keys change nothing here.
        if let InputEvent::Chord(c) = ev {
            i.key(Some(c.key), key_mods(*c), 1.0, 1.0);
        }
        return;
    }
    let input = match ev {
        InputEvent::Text(t) => EditInput::Text(t.clone()),
        InputEvent::Chord(c) => {
            let ctrl = c.mods.intersects(Mods::CTRL | Mods::CMD) && !c.mods.contains(Mods::ALT);
            let shift_only = c.mods == Mods::SHIFT;
            match c.key {
                KeyName::C if ctrl && !c.mods.contains(Mods::SHIFT) => EditInput::Copy,
                KeyName::Insert if ctrl => EditInput::Copy,
                KeyName::X if ctrl => EditInput::Cut,
                KeyName::Delete if shift_only => EditInput::Cut,
                KeyName::V if ctrl => match st.clip.clone() {
                    Some(s) => EditInput::Paste(s),
                    None => return,
                },
                KeyName::Insert if shift_only => match st.clip.clone() {
                    Some(s) => EditInput::Paste(s),
                    None => return,
                },
                KeyName::ArrowUp | KeyName::ArrowDown | KeyName::PageUp | KeyName::PageDown
                    if !c.mods.contains(Mods::ALT) && matches!(d.kind, DocKind::Text(_)) =>
                {
                    let DocKind::Text(t) = &mut d.kind else { return };
                    let rows = v.geom.rows.saturating_sub(1).max(1) as isize;
                    let delta = match c.key {
                        KeyName::ArrowUp => -1,
                        KeyName::ArrowDown => 1,
                        KeyName::PageUp => -rows,
                        _ => rows,
                    };
                    vertical(t, v, delta, c.mods.contains(Mods::SHIFT));
                    return;
                }
                _ => EditInput::Key(Some(c.key), key_mods(*c)),
            }
        }
        InputEvent::Other => return,
    };
    v.want_x = None;
    let copied = match &mut d.kind {
        DocKind::Text(t) => t.input(input, tab),
        DocKind::Hex(h) => h.input(input),
        _ => None,
    };
    if let Some(text) = copied {
        app.host.copy_text(text.clone());
        app.input.editor.clip = Some(text);
    }
}

/// Up / Down / PageUp / PageDown: move the caret `delta` visual rows (Shift extends),
/// keeping its column in the row ([`DocView::want_x`]) in terminal cells.
pub fn vertical(t: &mut TextDoc, v: &mut DocView, delta: isize, extend: bool) {
    let g = v.geom;
    let g = if g.cols == 0 { Geom { cols: 80, rows: 24, tab: 4, ..g } } else { g };
    let head = t.buf.clamp(t.sel.head);
    let lay = LineLayout::new(t.buf.line(head.line), g.tab);
    let rows = line_rows(&lay, &g);
    let r = lay.row_of(&rows, head.col);
    let x = lay.col_of(head.col) - lay.row_col(&rows, r);
    let want = *v.want_x.get_or_insert(x);
    let (line, row) = crate::view::editor::text::walk(t, &g, (head.line, r), delta);
    let lay = LineLayout::new(t.buf.line(line), g.tab);
    let rows = line_rows(&lay, &g);
    let col = lay.byte_in_row(&rows, row, want);
    let p = Pos::new(line, col);
    t.sel = if extend { Sel::new(t.sel.anchor, p) } else { Sel::caret(p) };
    t.want_col = None;
    t.scroll_to_caret = true;
}

/// Take the core's request for a terminal UI bar (`EditorSaveAs`, `EditorResizeImage`:
/// `editors::commands`); called every frame by `view::editor::prepare`.
pub fn take_request(core: &mut AppCore, st: &mut State) {
    match core.editors.request.take() {
        Some(Action::EditorSaveAs) => open_save_as(core, st),
        Some(Action::EditorResizeImage) => open_resize(core, st),
        _ => {}
    }
}

/// `EditorSaveAs`: open the "Save as" bar on the active document's path.
fn open_save_as(core: &mut AppCore, st: &mut State) {
    let Some(d) = core.editors.active_doc() else { return };
    let want = match d.kind {
        DocKind::Text(_) => Want::Text,
        DocKind::Image(_) => Want::Auto,
        _ => {
            core.ui.toasts.push(Toast::info(tr!("tui-editor-save-as-text-only")));
            return;
        }
    };
    let id = d.id;
    let text = d.path.display().to_string();
    let edit = LineEdit::at_end(&text);
    st.save_as = Some(SaveAs { doc: id, text, edit, want: Some(want), ..SaveAs::default() });
    st.focused = true;
    view(st, id).bar = Bar::SaveAs;
}

/// `EditorResizeImage`: open the resize bar with the image's size.
fn open_resize(core: &mut AppCore, st: &mut State) {
    let Some(d) = core.editors.active_doc_mut() else { return };
    let DocKind::Image(i) = &mut d.kind else {
        core.ui.toasts.push(Toast::info(tr!("tui-editor-resize-image-only")));
        return;
    };
    i.command("cancel");
    let (w, _) = i.size();
    let v = view(st, d.id);
    v.fields = vec![w.to_string(), String::new()];
    v.field = 0;
    v.field_edits = v.fields.iter().map(|s| LineEdit::at_end(s)).collect();
    v.field_error = None;
    v.bar = Bar::Resize;
    st.focused = true;
}

/// One event for the crop / resize bar of image document `id`: digits (and `%` for the
/// resize) edit the focused field, Tab / Up / Down move between the fields, Enter applies.
fn field_event(app: &mut App, id: u64, bar: Bar, ev: &InputEvent) {
    let max_mp = app.core.config.editor.max_image_mp.max(1);
    let st = &mut app.input.editor;
    let v = st.views.entry(id).or_default();
    let n = if bar == Bar::Crop { CROP_FIELDS } else { RESIZE_FIELDS };
    v.fields.resize(n, String::new());
    v.field_edits.resize_with(n, LineEdit::new);
    v.field = v.field.min(n - 1);
    if let InputEvent::Text(t) = ev
        && !t.chars().all(|c| c.is_ascii_digit() || (bar == Bar::Resize && c == '%'))
    {
        return;
    }
    let k = v.field;
    let out = v.field_edits[k].apply(&mut v.fields[k], ev);
    match out {
        EditOutcome::Tab | EditOutcome::Down => v.field = (k + 1) % n,
        EditOutcome::ShiftTab | EditOutcome::Up => v.field = (k + n - 1) % n,
        EditOutcome::Consumed => v.field_error = None,
        _ => {}
    }
    if out != EditOutcome::Submit {
        return;
    }
    let Some(DocKind::Image(i)) = app.core.editors.docs.iter_mut().find(|d| d.id == id).map(|d| &mut d.kind) else {
        return;
    };
    let size = i.size();
    if bar == Bar::Crop {
        let Some((x, y, w, h)) = parse_crop(&v.fields, size) else {
            v.field_error = Some(tr!("tui-editor-crop-invalid").into());
            return;
        };
        let (iw, ih) = (size.0 as f32, size.1 as f32);
        let rect = NormRect { x0: x as f32 / iw, y0: y as f32 / ih, x1: (x + w) as f32 / iw, y1: (y + h) as f32 / ih };
        if (x, y, w, h) != (0, 0, size.0, size.1) {
            i.push(ImgOp::Crop(rect));
        }
        i.command("cancel");
        v.fields = crop_fields(i.size());
        v.bar = Bar::Text;
        return;
    }
    let Some((w, h)) = parse_resize(&v.fields, size) else {
        v.field_error = Some(tr!("tui-editor-resize-invalid").into());
        return;
    };
    if u64::from(w) * u64::from(h) > u64::from(max_mp) * 1_000_000 {
        v.field_error = Some(tr!("tui-editor-resize-too-large", limit = max_mp.to_string()));
        return;
    }
    if (w, h) != size {
        i.push(ImgOp::Resize { w, h });
    }
    v.bar = Bar::Text;
}

/// What a "Save as" writes: the text's bytes, or the edited image (encoded on the worker).
enum CopyJob {
    Bytes(Vec<u8>),
    Image { full: std::sync::Arc<image::DynamicImage>, ops: Vec<ImgOp>, mode: SaveMode, quality: u8 },
}

/// Enter in the "Save as" bar: write the text (or the edited image, in the format of the
/// new name's extension) to the new file on a worker thread (never over an existing
/// file); [`poll`] then opens the copy in place of the document.
fn start_save_as(app: &mut App) {
    let quality = app.core.config.editor.jpeg_quality;
    let Some(s) = app.input.editor.save_as.as_mut() else { return };
    let Some(d) = app.core.editors.docs.iter().find(|d| d.id == s.doc) else { return };
    let raw = s.text.trim();
    if raw.is_empty() {
        return;
    }
    let mut target = PathBuf::from(raw);
    if target.is_relative()
        && let Some(dir) = d.path.parent()
    {
        target = dir.join(target);
    }
    if target == d.path {
        // The same file: a plain save.
        let idx = app.core.editors.docs.iter().position(|x| x.id == s.doc).unwrap_or(0);
        app.input.editor.save_as = None;
        editors::save(&mut app.core, idx);
        return;
    }
    let copy = match &d.kind {
        DocKind::Text(t) => {
            let lines = t.buf.lines().iter().map(String::as_str);
            CopyJob::Bytes(editors::text::encoding::encode_lines(lines, t.encoding, t.eol))
        }
        DocKind::Image(i) => {
            let format = image::ImageFormat::from_path(&target).ok();
            let mode = soyuz_appcore::editors::image::ops::save_mode(format);
            if mode == SaveMode::Unsupported {
                s.error = Some(tr!("tui-editor-save-as-format").into());
                return;
            }
            CopyJob::Image { full: i.full.clone(), ops: i.ops.clone(), mode, quality }
        }
        _ => return,
    };
    let (tx, rx) = crossbeam_channel::bounded(1);
    let wake = app.core.wake.clone();
    let path = target.clone();
    let spawned = std::thread::Builder::new().name("soyuz-tui-save-as".into()).spawn(move || {
        let bytes = match copy {
            CopyJob::Bytes(b) => Ok(b),
            CopyJob::Image { full, ops, mode, quality } => {
                let img = soyuz_appcore::editors::image::ImageDoc::edited(&full, &ops);
                soyuz_appcore::editors::image::encode(&img, mode, quality)
            }
        };
        let r = if std::fs::symlink_metadata(&path).is_ok() {
            Err(tr!("tui-editor-save-as-exists").to_string())
        } else {
            bytes.and_then(|b| soyuz_fs::write_atomic(&path, &b).map_err(|e| e.to_string()))
        };
        let _ = tx.send(r);
        wake();
    });
    match spawned {
        Ok(_) => {
            s.busy = true;
            s.error = None;
            s.rx = Some(rx);
            s.target = Some(target);
        }
        Err(e) => s.error = Some(e.to_string()),
    }
}

/// Apply a finished "Save as" (called every frame by `view::editor::prepare`): the copy
/// replaces the document (the original file stays as it was on disk).
pub fn poll(core: &mut AppCore, st: &mut State) {
    let Some(s) = st.save_as.as_mut() else { return };
    let Some(rx) = &s.rx else { return };
    let Ok(r) = rx.try_recv() else { return };
    s.rx = None;
    s.busy = false;
    match r {
        Ok(()) => {
            let doc = s.doc;
            let target = s.target.take();
            let want = s.want.unwrap_or(Want::Text);
            st.save_as = None;
            if let Some(idx) = core.editors.docs.iter().position(|d| d.id == doc) {
                editors::close(core, idx, true);
            }
            if let Some(p) = target {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                editors::open_path(core, &p, want);
                core.ui.toasts.push_replacing(Toast::success(tr!("editors-saved", name = name)));
            }
        }
        Err(e) => s.error = Some(e),
    }
}

/// A press on a text cell at `(x, y)` of `rect` showing line `top` from column `left`.
fn text_pos(t: &TextDoc, tab: usize, top: usize, left: usize, rect: Rect, x: u16) -> Pos {
    let line = top.min(t.buf.line_count() - 1);
    let lay = LineLayout::new(t.buf.line(line), tab);
    let col = left.saturating_add(usize::from(x.saturating_sub(rect.x)));
    Pos::new(line, lay.byte_at(col))
}

/// Dragging with the left button extends the text selection.
fn mouse_move(app: &mut App, m: &MouseEvent) {
    match m.kind {
        MouseEventKind::Drag(MouseButton::Left) => {}
        MouseEventKind::Up(_) => {
            if let Some(DocKind::Text(t)) = app.core.editors.active_doc_mut().map(|d| &mut d.kind) {
                t.dragging = false;
            }
            return;
        }
        _ => return,
    }
    let tab = crate::view::editor::text::tab_width(&app.core.config.editor);
    let hit = super::mouse::hit(&app.hits, m.column, m.row);
    let Some(d) = app.core.editors.active_doc_mut() else { return };
    let id = d.id;
    let DocKind::Text(t) = &mut d.kind else { return };
    if !t.dragging {
        return;
    }
    match hit {
        Some((rect, HitTarget::EditorCell { doc, top, left })) if doc == id && top != BAR_ROW => {
            t.sel.head = text_pos(t, tab, top, left, rect, m.column);
        }
        _ => {
            // Above or below the text: one row further.
            let v = app.input.editor.views.entry(id).or_default();
            let up = app.hits.iter().any(|(r, t)| {
                matches!(t, HitTarget::EditorCell { doc, top, .. } if *doc == id && *top != BAR_ROW) && m.row < r.y
            });
            let anchor = t.sel.anchor;
            vertical(t, v, if up { -1 } else { 1 }, true);
            t.sel.anchor = anchor;
        }
    }
}

/// A mouse press on one of the editor's targets.
pub fn click(app: &mut App, target: HitTarget, m: &MouseEvent) {
    let MouseEventKind::Down(button) = m.kind else { return };
    match target {
        HitTarget::EditorTab(BTN_MAXIMIZE) => app.core.editors.maximized = !app.core.editors.maximized,
        HitTarget::EditorTab(BTN_DOCK) => toggle_dock(app),
        HitTarget::EditorClose(BTN_HIDE) => {
            app.core.editors.open = false;
            app.core.editors.maximized = false;
            app.core.ui.focus = Focus::List;
            app.input.editor.focused = false;
        }
        HitTarget::EditorTab(i) if button == MouseButton::Middle => editors::close(&mut app.core, i, false),
        HitTarget::EditorClose(i) => editors::close(&mut app.core, i, false),
        HitTarget::EditorTab(i) => {
            if i < app.core.editors.docs.len() {
                app.core.editors.active = i;
                app.core.editors.docs[i].request_focus();
            }
        }
        HitTarget::EditorCell { doc, top, left } => {
            if button != MouseButton::Left {
                return;
            }
            if top == BAR_ROW && matches!(left, BANNER_RELOAD | BANNER_KEEP) {
                let action = if left == BANNER_RELOAD { Some(Action::EditorNoticeAction) } else { None };
                let idx = app.core.editors.active;
                match action {
                    Some(a) => soyuz_appcore::editors::commands::run(&mut app.core, &a),
                    None => {
                        if let Some(d) = app.core.editors.docs.get_mut(idx) {
                            d.external = None;
                        }
                    }
                }
                return;
            }
            let Some(d) = app.core.editors.active_doc_mut().filter(|d| d.id == doc) else { return };
            app.input.editor.focused = true;
            app.input.jobs.focused = false;
            app.input.preview.focused = false;
            let v = app.input.editor.views.entry(doc).or_default();
            if top == BAR_ROW && matches!(v.bar, Bar::Crop | Bar::Resize) {
                // A field row of the crop / resize bar (they come first).
                if left < v.fields.len() {
                    v.field = left;
                    return;
                }
            }
            if top == BAR_ROW {
                v.bar = match (&d.kind, left) {
                    (DocKind::Text(t), 0) if t.find.open => Bar::Find,
                    (DocKind::Text(t), 1) if t.find.open && t.find.replace => Bar::Replace,
                    (DocKind::Hex(h), 0) if h.find.is_some() => Bar::Find,
                    _ if app.input.editor.save_as.as_ref().is_some_and(|s| s.doc == doc) => Bar::SaveAs,
                    _ => Bar::Goto,
                };
                return;
            }
            v.bar = Bar::Text;
            v.want_x = None;
            let rect = super::mouse::hit(&app.hits, m.column, m.row).map(|(r, _)| r).unwrap_or_default();
            let tab = crate::view::editor::text::tab_width(&app.core.config.editor);
            let shift = m.modifiers.contains(KeyModifiers::SHIFT);
            match &mut d.kind {
                DocKind::Text(t) => {
                    let at = text_pos(t, tab, top, left, rect, m.column);
                    let now = Instant::now();
                    let count = match app.input.editor.last_press {
                        Some((when, x, y, n)) if now - when < MULTI_PRESS && (x, y) == (m.column, m.row) => n % 3 + 1,
                        _ => 1,
                    };
                    app.input.editor.last_press = Some((now, m.column, m.row, count));
                    match count {
                        2 => t.select_word(at),
                        3 => t.select_line(at.line),
                        _ => t.press(at, shift),
                    }
                }
                DocKind::Image(_) => {}
                DocKind::Hex(h) => {
                    let digits = crate::view::editor::hex::offset_digits(h.len);
                    if let Some((col, text)) =
                        crate::view::editor::hex::hit_byte(digits, m.column.saturating_sub(rect.x))
                    {
                        h.click(top as u64, col, text);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// The wheel over one of the editor's targets (`rows`: negative scrolls up).
pub fn wheel(app: &mut App, target: HitTarget, rows: i32) {
    let HitTarget::EditorCell { doc, .. } = target else { return };
    let Some(d) = app.core.editors.active_doc_mut().filter(|d| d.id == doc) else { return };
    let v = app.input.editor.views.entry(doc).or_default();
    match &mut d.kind {
        DocKind::Text(t) => {
            let (l, r) = crate::view::editor::text::walk(t, &v.geom, (v.top, v.top_row), rows as isize);
            v.top = l;
            v.top_row = r;
        }
        DocKind::Hex(h) => {
            let max = h.total_rows().saturating_sub(1) as f64;
            h.scroll_row = (h.scroll_row + f64::from(rows)).clamp(0.0, max);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEvent;

    use super::*;

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn release(c: char) -> Event {
        let mut k = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        k.kind = KeyEventKind::Release;
        Event::Key(k)
    }

    fn code(c: KeyCode, m: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(c, m))
    }

    #[test]
    fn typing_stays_keys_and_bursts_become_text() {
        let typed = [key('a'), key('b')];
        assert_eq!(split(&typed), vec![Part::Keys(typed.to_vec())]);
        // A Windows paste: presses and releases, a line break, an upper-case letter.
        let burst = [
            key('f'),
            release('f'),
            key('n'),
            code(KeyCode::Enter, KeyModifiers::NONE),
            code(KeyCode::Char('X'), KeyModifiers::SHIFT),
            code(KeyCode::Tab, KeyModifiers::NONE),
            key('y'),
        ];
        assert_eq!(split(&burst), vec![Part::Text("fn\nX\ty".into())]);
        // Commands around a burst stay keys, in order.
        let save = code(KeyCode::Char('s'), KeyModifiers::CONTROL);
        let mut mixed = vec![save.clone()];
        mixed.extend(burst.iter().cloned());
        mixed.push(save.clone());
        assert_eq!(
            split(&mixed),
            vec![Part::Keys(vec![save.clone()]), Part::Text("fn\nX\ty".into()), Part::Keys(vec![save])]
        );
        // Enter / Tab alone are commands, never a burst.
        let enters = vec![code(KeyCode::Enter, KeyModifiers::NONE); 6];
        assert_eq!(split(&enters), vec![Part::Keys(enters.clone())]);
        // A bracketed paste.
        assert_eq!(split(&[Event::Paste("a\r\nb".into())]), vec![Part::Text("a\r\nb".into())]);
    }

    #[test]
    fn vertical_motion_keeps_the_terminal_column() {
        let d = soyuz_appcore::editors::text::encoding::decode("日本語x\nabcdefgh\n".as_bytes());
        let mut t = TextDoc::new("a.txt", d);
        let mut v = DocView { geom: Geom { cols: 80, rows: 10, wrap: false, tab: 4 }, ..DocView::default() };
        // The caret after 日本 (column 4) goes to column 4 of the next line.
        t.sel = Sel::caret(Pos::new(0, 6));
        vertical(&mut t, &mut v, 1, false);
        assert_eq!(t.sel.head, Pos::new(1, 4));
        vertical(&mut t, &mut v, -1, true);
        assert_eq!(t.sel.head, Pos::new(0, 6), "back on the same cell");
        assert_eq!(t.sel.anchor, Pos::new(1, 4), "Shift extends");
        // Column 3 is the right half of 本: the caret goes before it.
        t.sel = Sel::caret(Pos::new(1, 3));
        v.want_x = None;
        vertical(&mut t, &mut v, -1, false);
        assert_eq!(t.sel.head, Pos::new(0, 3));
    }

    #[test]
    fn vertical_motion_in_wrapped_rows() {
        let d = soyuz_appcore::editors::text::encoding::decode(b"abcdefghij\nxy");
        let mut t = TextDoc::new("a.txt", d);
        let mut v = DocView { geom: Geom { cols: 4, rows: 10, wrap: true, tab: 4 }, ..DocView::default() };
        t.sel = Sel::caret(Pos::new(0, 1));
        vertical(&mut t, &mut v, 1, false);
        assert_eq!(t.sel.head, Pos::new(0, 5), "the next wrapped row of the same line");
        vertical(&mut t, &mut v, 2, false);
        assert_eq!(t.sel.head, Pos::new(1, 1));
    }
}
