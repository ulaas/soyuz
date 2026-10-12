//! The built-in text and hex editors (`soyuz_appcore::editors`) — Phase T4.
//!
//! [`placement`] decides whether and where the editor panel goes (`Regions::editor`:
//! `[editor] dock`, maximized over the panes like the GUI's button). [`prepare`] runs with
//! the core mutable: it takes the documents' focus requests, pumps the hex pages, keeps
//! the caret in view (the terminal UI's own scroll positions, `input::editor::DocView`),
//! runs the syntax highlighter over the visible lines and builds the whole
//! [`EditorVm`] into `InputState::editor` ([`EditorVm::from_core`] hands it out).
//! [`draw_editor`] draws it and records the click targets:
//!
//! - `EditorTab(i)` / `EditorClose(i)`: document tab `i` and its close mark; the panel
//!   buttons on the right of the tab row use the sentinels [`BTN_DOCK`], [`BTN_MAXIMIZE`]
//!   (`EditorTab`) and [`BTN_HIDE`] (`EditorClose`);
//! - `EditorCell { doc, top, left }`: one per text row (`top` the line, `left` the display
//!   column of the row's first cell), per hex row (`top` the file row), the image (`top`
//!   and `left` 0), per bar row (`top == usize::MAX`, `left` the bar's index), and the
//!   external-change banner's buttons (`top == usize::MAX`, `left` [`BANNER_RELOAD`] /
//!   [`BANNER_KEEP`]).
//!
//! The panel: a tab row (one tab per document: kind mark, name, `●` while unsaved, `×`
//! to close), the external-change banner when the file changed on disk under unsaved
//! edits (Reload with the notice key, Keep my version with Esc, or a click), then the
//! document: text ([`text`]), hex ([`hex`]) or image ([`image`]), each with its bars
//! (find / replace, go to, crop, resize, save as), an optional notice and a status line.
//! Cells are laid out with [`layout`]: wide characters take two cells, tabs expand,
//! controls show as `¤`. The bars and notices name the editor commands' current chords
//! ([`Keys`], from the key map: `KeyContext::Editor`).

pub mod hex;
pub mod image;
pub mod layout;
pub mod text;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::editors::DocKind;
use soyuz_appcore::tr;
use soyuz_core::config::{Action, EditorDock, Keymap};

use crate::icons::IconSet;
use crate::input::InputState;
use crate::input::editor::{Bar, DocView};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::view::text as fit;

/// `EditorTab` sentinel: switch the dock side.
pub const BTN_DOCK: usize = usize::MAX - 1;
/// `EditorTab` sentinel: maximize / restore.
pub const BTN_MAXIMIZE: usize = usize::MAX - 2;
/// `EditorClose` sentinel: hide the panel (the documents stay open).
pub const BTN_HIDE: usize = usize::MAX;
/// `EditorCell::top` of a bar row.
pub const BAR_ROW: usize = usize::MAX;
/// `EditorCell::left` (with `top == BAR_ROW`) of the banner's Reload / OK button.
pub const BANNER_RELOAD: usize = usize::MAX - 1;
/// `EditorCell::left` (with `top == BAR_ROW`) of the banner's "Keep my version" button.
pub const BANNER_KEEP: usize = usize::MAX - 2;

/// The chords of the editor commands the bars, notices and hints name (the first chord
/// of each; `—` when the command has none).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub notice: String,
    pub replace_all: String,
    pub case: String,
    pub word: String,
    pub regex: String,
    pub resize: String,
    pub save: String,
}

impl Keys {
    pub fn from_keymap(km: &Keymap) -> Keys {
        let l = |a: Action| soyuz_appcore::keys::chord_label(km, &a).unwrap_or_else(|| "—".into());
        Keys {
            notice: l(Action::EditorNoticeAction),
            replace_all: l(Action::EditorReplaceAll),
            case: l(Action::EditorMatchCase),
            word: l(Action::EditorWholeWord),
            regex: l(Action::EditorRegex),
            resize: l(Action::EditorResizeImage),
            save: l(Action::EditorSave),
        }
    }
}

/// The external-change banner: the message and its buttons (label, `EditorCell::left`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BannerVm {
    pub text: String,
    pub buttons: Vec<(String, usize)>,
}

/// Where the open editor panel goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorPlacement {
    pub dock: EditorDock,
    /// It fills the panes' area.
    pub maximized: bool,
}

/// What a tab holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabKind {
    #[default]
    Loading,
    Failed,
    Text,
    Hex,
    Image,
}

/// One document tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabVm {
    pub name: String,
    pub kind: TabKind,
    pub active: bool,
    pub dirty: bool,
    pub saving: bool,
}

/// The active document's part of the panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BodyVm {
    /// No document open: the empty state with the "Edit in Soyuz" key.
    #[default]
    Empty,
    /// The file is being read.
    Loading,
    /// Lines of a message (open failed, images).
    Message(Vec<String>),
    Text(text::TextVm),
    Hex(hex::HexVm),
    Image(image::ImageVm),
}

/// One row of a bar (find, replace, go to, save as) above the document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarVm {
    pub label: String,
    /// The visible part of the field's text.
    pub field: String,
    /// Width of the field in cells.
    pub field_w: u16,
    /// The text cursor's column in the field while the bar has the keyboard.
    pub cursor: Option<u16>,
    /// Option chips after the field (`Aa`, `W`, `.*`, `HEX`), on / off.
    pub chips: Vec<(String, bool)>,
    /// Status or hint after the field.
    pub note: String,
    /// The note is an error.
    pub error: bool,
}

/// The editor panel as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorVm {
    /// The panel is drawn.
    pub visible: bool,
    /// It has the keyboard.
    pub focused: bool,
    pub maximized: bool,
    pub dock: EditorDock,
    /// A separator column on the left (docked right of the panes).
    pub edge: bool,
    /// The active document's id (0: none).
    pub doc: u64,
    pub tabs: Vec<TabVm>,
    pub body: BodyVm,
    /// The empty state's text and hint.
    pub empty: (String, String),
    /// The file changed on disk under unsaved edits.
    pub banner: Option<BannerVm>,
}

/// Where the editor panel goes, `None` while it is closed (`core.editors.open`,
/// `core.editors.maximized`, `[editor] dock`).
pub fn placement(core: &AppCore) -> Option<EditorPlacement> {
    core.editors.open.then_some(EditorPlacement { dock: core.config.editor.dock, maximized: core.editors.maximized })
}

/// The panel's parts in `area`: the separator column, the tab row and the body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    pub edge: Option<Rect>,
    pub tabs: Rect,
    pub body: Rect,
}

/// Split `area` (`edge`: a separator column on the left).
pub fn split(area: Rect, edge: bool) -> Parts {
    let mut inner = area;
    let mut parts = Parts::default();
    if edge && inner.width > 1 {
        parts.edge = Some(Rect { width: 1, ..inner });
        inner.x += 1;
        inner.width -= 1;
    }
    if inner.height == 0 {
        return parts;
    }
    parts.tabs = Rect { height: 1, ..inner };
    parts.body = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };
    parts
}

/// The banner's row (when `banner`) and the document's part of `body`.
pub fn banner_split(body: Rect, banner: bool) -> (Option<Rect>, Rect) {
    if !banner || body.height == 0 {
        return (None, body);
    }
    (Some(Rect { height: 1, ..body }), Rect { y: body.y + 1, height: body.height - 1, ..body })
}

/// The external-change banner of a document (`None`: the file did not change).
fn banner_vm(ext: Option<soyuz_appcore::editors::External>, keys: &Keys) -> Option<BannerVm> {
    use soyuz_appcore::editors::External;
    let button = |label: &str, key: &str| format!("[{label} {key}]");
    Some(match ext? {
        External::Changed => BannerVm {
            text: tr!("editors-external-changed").into(),
            buttons: vec![
                (button(tr!("editors-external-reload"), &keys.notice), BANNER_RELOAD),
                (button(tr!("editors-external-keep"), "Esc"), BANNER_KEEP),
            ],
        },
        External::Deleted => BannerVm {
            text: tr!("editors-external-deleted").into(),
            buttons: vec![(button(tr!("common-ok"), &keys.notice), BANNER_RELOAD)],
        },
    })
}

/// Step 1 of a frame (`view::prepare`): focus requests, scrolling, highlighting, the
/// hex pages, a finished "Save as", and the frame's [`EditorVm`].
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    crate::input::editor::poll(core, &mut input.editor);
    crate::input::editor::take_request(core, &mut input.editor);
    let was_focused = input.editor.focused;
    prepare_editor(core, &mut input.editor, regions);
    if input.editor.focused && !was_focused {
        // The editor took the keyboard (Alt+E, a new document): the panels that had it let
        // go, so Esc in the editor goes back to the file list.
        input.jobs.focused = false;
        input.preview.focused = false;
    }
}

fn prepare_editor(core: &mut AppCore, st: &mut crate::input::editor::State, regions: &Regions) {
    let Some(pl) = placement(core) else {
        st.vm = EditorVm::default();
        st.focused = false;
        return;
    };
    let area = regions.editor;
    let edge = !pl.maximized
        && area.width > 1
        && regions.panes.iter().any(|p| !p.rect.is_empty() && p.rect.right() <= area.x && p.rect.y < area.bottom());
    let parts = split(area, edge);
    let ids: Vec<u64> = core.editors.docs.iter().map(|d| d.id).collect();
    st.views.retain(|id, _| ids.contains(id));
    if st.save_as.as_ref().is_some_and(|s| !ids.contains(&s.doc)) {
        st.save_as = None;
    }
    let active = core.editors.active;
    let tabs = core
        .editors
        .docs
        .iter()
        .enumerate()
        .map(|(i, d)| TabVm {
            name: crate::dialog::safe(&d.name),
            kind: match d.kind {
                DocKind::Loading => TabKind::Loading,
                DocKind::Failed(_) => TabKind::Failed,
                DocKind::Text(_) => TabKind::Text,
                DocKind::Hex(_) => TabKind::Hex,
                DocKind::Image(_) => TabKind::Image,
            },
            active: i == active,
            dirty: d.is_dirty(),
            saving: d.saving,
        })
        .collect();
    let key = soyuz_appcore::keys::chord_label(&core.keymap, &soyuz_core::config::Action::OpenInEditor)
        .unwrap_or_else(|| "F4".into());
    let empty = (tr!("editors-empty").to_string(), tr!("editors-empty-hint", key = key));
    let dark = crate::theme::TuiTheme::cached(core, false).is_dark();
    let wake = core.wake.clone();
    let cfg = core.config.editor.clone();
    let keys = Keys::from_keymap(&core.keymap);
    let mut doc_id = 0;
    let mut banner = None;
    let body = match core.editors.docs.get_mut(active) {
        None => BodyVm::Empty,
        Some(d) => {
            doc_id = d.id;
            let v = st.views.entry(d.id).or_default();
            // The core asks for the keyboard (a new document, a tab switch, Esc in a bar).
            match &mut d.kind {
                DocKind::Text(t) => {
                    if std::mem::take(&mut t.focus_request) {
                        st.focused = true;
                        v.bar = Bar::Text;
                    }
                    if std::mem::take(&mut t.find.focus_find) {
                        st.focused = true;
                        v.bar = Bar::Find;
                        v.find_edit = crate::widgets::LineEdit::at_end(&t.find.query.text);
                    }
                    if std::mem::take(&mut t.goto_focus) {
                        st.focused = true;
                        v.bar = Bar::Goto;
                        v.goto_edit = crate::widgets::LineEdit::new();
                    }
                }
                DocKind::Hex(h) => {
                    if std::mem::take(&mut h.focus_request) {
                        st.focused = true;
                        v.bar = Bar::Text;
                    }
                    if let Some(f) = h.find.as_mut()
                        && std::mem::take(&mut f.focus)
                    {
                        st.focused = true;
                        v.bar = Bar::Find;
                        v.find_edit = crate::widgets::LineEdit::at_end(&f.text);
                    }
                    if std::mem::take(&mut h.goto_focus) {
                        st.focused = true;
                        v.bar = Bar::Goto;
                        v.goto_edit = crate::widgets::LineEdit::new();
                    }
                }
                DocKind::Image(i) => {
                    if std::mem::take(&mut i.focus_request) {
                        st.focused = true;
                        if !matches!(v.bar, Bar::Crop | Bar::Resize | Bar::SaveAs) {
                            v.bar = Bar::Text;
                        }
                    }
                }
                _ => {}
            }
            let save_as = st.save_as.as_mut().filter(|s| s.doc == d.id);
            let focused = st.focused;
            banner = banner_vm(d.external, &keys);
            let (_, body) = banner_split(parts.body, banner.is_some());
            match &mut d.kind {
                DocKind::Loading => BodyVm::Loading,
                DocKind::Failed(e) => BodyVm::Message(
                    tr!("editors-open-failed", name = crate::dialog::safe(&d.name), error = crate::dialog::safe(e))
                        .lines()
                        .map(str::to_string)
                        .collect(),
                ),
                DocKind::Image(i) => {
                    BodyVm::Image(image::prepare(i, v, save_as, body, focused, &keys, cfg.jpeg_quality))
                }
                DocKind::Text(t) => {
                    let env = text::Env { cfg: &cfg, focused, dark, wake: &wake, keys: &keys };
                    BodyVm::Text(text::prepare(t, v, save_as, body, &env))
                }
                DocKind::Hex(h) => BodyVm::Hex(hex::prepare(h, v, save_as, body, focused, &keys.notice)),
            }
        }
    };
    st.vm = EditorVm {
        visible: true,
        focused: st.focused,
        maximized: pl.maximized,
        dock: pl.dock,
        edge,
        doc: doc_id,
        tabs,
        body,
        empty,
        banner,
    };
}

impl EditorVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> EditorVm {
        let mut vm = input.editor.vm.clone();
        vm.visible = placement(core).is_some();
        vm.focused = vm.visible && input.editor.focused;
        vm
    }
}

/// Write `s` at `(x, y)`, at most `max` cells and never outside `clip`.
pub(crate) fn put(buf: &mut Buffer, clip: Rect, x: u16, y: u16, s: &str, max: usize, style: Style) -> u16 {
    let clip = clip.intersection(buf.area);
    if y < clip.y || y >= clip.bottom() || x < clip.x || x >= clip.right() {
        return x;
    }
    let room = usize::from(clip.right() - x).min(max);
    let (end, _) = buf.set_stringn(x, y, s, room, style);
    end
}

/// Fill row `y` of `clip` from `x` to its end with `style`.
pub(crate) fn fill_row(buf: &mut Buffer, clip: Rect, x: u16, y: u16, style: Style) {
    let clip = clip.intersection(buf.area);
    if y < clip.y || y >= clip.bottom() || x >= clip.right() {
        return;
    }
    let x = x.max(clip.x);
    buf.set_style(Rect::new(x, y, clip.right() - x, 1), style);
}

/// Draw the panel into `area` (`Regions::editor`).
pub fn draw_editor(f: &mut Frame<'_>, area: Rect, m: &EditorVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    let buf = f.buffer_mut();
    buf.set_style(area, theme.base());
    let parts = split(area, m.edge);
    if let Some(e) = parts.edge {
        let line = if theme.ascii_only() { "|" } else { "│" };
        let role = if m.focused { Role::BorderFocused } else { Role::Border };
        for y in e.y..e.bottom() {
            put(buf, e, e.x, y, line, 1, theme.style(role));
        }
    }
    draw_tabs(buf, parts.tabs, m, theme, hits);
    let (banner_row, body) = banner_split(parts.body, m.banner.is_some());
    if let (Some(r), Some(b)) = (banner_row, &m.banner) {
        draw_banner(buf, r, b, m.doc, theme, hits);
    }
    if body.is_empty() {
        return;
    }
    match &m.body {
        BodyVm::Empty => {
            let mid = body.y + body.height / 2;
            centre(buf, body, mid.saturating_sub(1).max(body.y), &m.empty.0, theme.style(Role::Muted));
            centre(buf, body, mid.saturating_sub(1).max(body.y) + 1, &m.empty.1, theme.style(Role::Faint));
        }
        BodyVm::Loading => {
            centre(buf, body, body.y + body.height / 2, "…", theme.style(Role::Muted));
        }
        BodyVm::Message(lines) => {
            let top = body.y + body.height.saturating_sub(lines.len() as u16) / 2;
            for (i, l) in lines.iter().enumerate() {
                let role = if i == 0 { Role::Text } else { Role::Muted };
                centre(buf, body, top + i as u16, l, theme.style(role));
            }
        }
        BodyVm::Text(t) => text::draw(f, body, t, m.doc, theme, hits),
        BodyVm::Hex(h) => hex::draw(f, body, h, m.doc, theme, hits),
        BodyVm::Image(i) => image::draw(f, body, i, m.doc, theme, hits),
    }
}

/// The external-change banner: the message, then its buttons on the right.
fn draw_banner(buf: &mut Buffer, r: Rect, b: &BannerVm, doc: u64, theme: &TuiTheme, hits: &mut Hits) {
    fill_row(buf, r, r.x, r.y, theme.style(Role::Warning).add_modifier(Modifier::REVERSED));
    let bw: u16 = b.buttons.iter().map(|(s, _)| fit::width(s) as u16 + 1).sum();
    let room = r.width.saturating_sub(bw + 1);
    let text = fit::truncate_end(&b.text, usize::from(room));
    let style = theme.style(Role::Warning).add_modifier(Modifier::REVERSED);
    put(buf, r, r.x + 1, r.y, &text, usize::from(room), style);
    let mut x = r.right().saturating_sub(bw);
    for (label, left) in &b.buttons {
        let w = (fit::width(label) as u16).min(r.right().saturating_sub(x));
        if w == 0 || x < r.x {
            break;
        }
        put(buf, r, x, r.y, label, usize::from(w), theme.style(Role::Button));
        hits.push(Rect::new(x, r.y, w, 1), HitTarget::EditorCell { doc, top: BAR_ROW, left: *left });
        x += w + 1;
    }
}

fn centre(buf: &mut Buffer, r: Rect, y: u16, s: &str, style: Style) {
    let s = fit::truncate_end(s, usize::from(r.width));
    let w = fit::width(&s) as u16;
    put(buf, r, r.x + r.width.saturating_sub(w) / 2, y, &s, usize::from(r.width), style);
}

/// The tab row: the tabs, then the panel buttons on the right.
fn draw_tabs(buf: &mut Buffer, r: Rect, m: &EditorVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let ascii = theme.icons() == IconSet::Ascii;
    fill_row(buf, r, r.x, r.y, theme.style(Role::StatusBar));
    // Panel buttons: dock side, maximize / restore, hide.
    let dock = match (m.dock, ascii) {
        (EditorDock::Right, true) => "[_]",
        (EditorDock::Bottom, true) => "[|]",
        (EditorDock::Right, false) => "▄",
        (EditorDock::Bottom, false) => "▐",
    };
    let max = match (m.maximized, ascii) {
        (false, true) => "[+]",
        (true, true) => "[-]",
        (false, false) => "⊞",
        (true, false) => "⊟",
    };
    let hide = if ascii { "[x]" } else { "×" };
    let mut buttons = vec![(hide, HitTarget::EditorClose(BTN_HIDE)), (max, HitTarget::EditorTab(BTN_MAXIMIZE))];
    if !m.maximized {
        buttons.push((dock, HitTarget::EditorTab(BTN_DOCK)));
    }
    let mut right = r.right();
    let bw: u16 = buttons.iter().map(|(s, _)| fit::width(s) as u16 + 1).sum();
    if r.width > bw + 8 {
        for (s, t) in &buttons {
            let w = fit::width(s) as u16;
            right -= w;
            put(buf, r, right, r.y, s, usize::from(w), theme.style(Role::Muted));
            hits.push(Rect::new(right, r.y, w, 1), *t);
            right -= 1;
        }
    }
    let tabs_end = right.saturating_sub(1).max(r.x);
    let mut x = r.x;
    let n = m.tabs.len().max(1);
    let room = usize::from(tabs_end.saturating_sub(r.x));
    // Each tab: " k name ● " (`k` the kind mark, `●` unsaved or `×` to close).
    let per = (room / n).clamp(8, 32);
    for (i, t) in m.tabs.iter().enumerate() {
        if x >= tabs_end {
            break;
        }
        let mark = match (t.kind, ascii) {
            (TabKind::Text, _) => "T",
            (TabKind::Hex, _) => "H",
            (TabKind::Image, _) => "I",
            (TabKind::Failed, _) => "!",
            (TabKind::Loading, true) => ".",
            (TabKind::Loading, false) => "…",
        };
        let close = match (t.dirty || t.saving, ascii) {
            (true, true) => "*",
            (true, false) => "●",
            (false, true) => "x",
            (false, false) => "×",
        };
        let name_w = per.saturating_sub(6).max(1);
        let name = fit::truncate_middle(&t.name, name_w);
        let label = format!(" {mark} {name} ");
        let w = (fit::width(&label) as u16).min(tabs_end - x);
        let role = if t.active { Role::TabActive } else { Role::TabInactive };
        let style = theme.style(Role::StatusBar).patch(theme.style(role));
        let end = put(buf, r, x, r.y, &label, usize::from(w), style);
        hits.push(Rect::new(x, r.y, end - x, 1), HitTarget::EditorTab(i));
        x = end;
        if x < tabs_end {
            let role = if t.dirty { Role::Accent } else { Role::Faint };
            put(buf, r, x, r.y, close, 1, style.patch(theme.style(role)));
            hits.push(Rect::new(x, r.y, 1, 1), HitTarget::EditorClose(i));
            x += 1;
        }
        if x < tabs_end {
            put(buf, r, x, r.y, " ", 1, theme.style(Role::StatusBar));
            x += 1;
        }
    }
}

/// Draw the bars of a document (find, replace, go to, save as) from row `y` down; returns
/// the first row after them. Each row records `EditorCell { top: BAR_ROW, left: i }`.
pub(crate) fn draw_bars(
    buf: &mut Buffer,
    r: Rect,
    bars: &[BarVm],
    doc: u64,
    theme: &TuiTheme,
    hits: &mut Hits,
    cursor: &mut Option<(u16, u16)>,
) -> u16 {
    let mut y = r.y;
    for (i, b) in bars.iter().enumerate() {
        if y >= r.bottom() {
            break;
        }
        fill_row(buf, r, r.x, y, theme.style(Role::StatusBar));
        let mut x = put(buf, r, r.x + 1, y, &b.label, usize::from(r.width), theme.style(Role::Muted));
        x = x.saturating_add(1);
        let fw = b.field_w.min(r.right().saturating_sub(x));
        if fw > 0 {
            buf.set_style(Rect::new(x, y, fw, 1), theme.style(Role::Input));
            put(buf, r, x, y, &b.field, usize::from(fw), theme.style(Role::Input));
            if let Some(c) = b.cursor {
                let cx = x + c.min(fw - 1);
                if let Some(cell) = buf.cell_mut((cx, y)) {
                    cell.modifier.insert(Modifier::REVERSED);
                }
                *cursor = Some((cx, y));
            }
            x += fw;
        }
        for (label, on) in &b.chips {
            x = x.saturating_add(1);
            let role = if *on { Role::ButtonFocused } else { Role::Button };
            x = put(buf, r, x, y, label, usize::from(r.width), theme.style(role));
        }
        if !b.note.is_empty() {
            let role = if b.error { Role::Danger } else { Role::Muted };
            put(buf, r, x.saturating_add(2), y, &b.note, usize::from(r.width), theme.style(role));
        }
        hits.push(Rect::new(r.x, y, r.width, 1), HitTarget::EditorCell { doc, top: BAR_ROW, left: i });
        y += 1;
    }
    y
}

/// The width a bar gives its field in a body `width` cells wide after a `label`.
pub(crate) fn field_width(width: u16, label: &str, reserve: u16) -> u16 {
    let lw = fit::width(label) as u16 + 2;
    width.saturating_sub(lw + reserve).clamp(width.saturating_sub(lw).min(8), 48)
}

/// Shared by the text and hex views: a bar for a [`crate::widgets::LineEdit`] field.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bar(
    label: String,
    text: &str,
    edit: &mut crate::widgets::LineEdit,
    width: u16,
    reserve: u16,
    focused: bool,
    chips: Vec<(String, bool)>,
    note: String,
    error: bool,
) -> BarVm {
    let field_w = field_width(width, &label, reserve);
    let (field, col) = edit.view(&crate::dialog::safe(text), field_w);
    BarVm { label, field, field_w, cursor: focused.then_some(col), chips, note, error }
}

/// Whether `v`'s bar `b` has the keyboard.
pub(crate) fn bar_focused(focused: bool, v: &DocView, b: Bar) -> bool {
    focused && v.bar == b
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn screen(m: &EditorVm, w: u16, h: u16) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_editor(f, Rect::new(0, 0, w, h), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, hits)
    }

    fn vm() -> EditorVm {
        EditorVm {
            visible: true,
            focused: true,
            doc: 1,
            tabs: vec![
                TabVm { name: "notes.txt".into(), kind: TabKind::Text, active: true, dirty: true, saving: false },
                TabVm { name: "日本語.bin".into(), kind: TabKind::Hex, ..TabVm::default() },
            ],
            body: BodyVm::Empty,
            empty: ("No files open".into(), "Select a file and press F4".into()),
            ..EditorVm::default()
        }
    }

    #[test]
    fn tabs_and_panel_buttons() {
        let (lines, hits) = screen(&vm(), 60, 6);
        // The plain theme draws ASCII marks.
        assert!(lines[0].starts_with(" T notes.txt *  H "), "{lines:?}");
        assert!(lines[0].ends_with("[_] [+] [x]"), "the panel buttons: {lines:?}");
        assert_eq!(hits.at(3, 0), Some(HitTarget::EditorTab(0)));
        assert_eq!(hits.at(58, 0), Some(HitTarget::EditorClose(BTN_HIDE)));
        assert_eq!(hits.at(54, 0), Some(HitTarget::EditorTab(BTN_MAXIMIZE)));
        assert!(lines.iter().any(|l| l.contains("No files open")), "{lines:?}");
    }

    #[test]
    fn banner_row_and_buttons() {
        let mut m = vm();
        m.banner = Some(BannerVm {
            text: "This file changed on disk.".into(),
            buttons: vec![("[Reload Alt+E]".into(), BANNER_RELOAD), ("[Keep Esc]".into(), BANNER_KEEP)],
        });
        let (lines, hits) = screen(&m, 60, 6);
        assert!(lines[1].contains("This file changed on disk.") && lines[1].contains("[Reload Alt+E]"), "{lines:?}");
        let reload = lines[1].find("[Reload").unwrap() as u16;
        assert_eq!(hits.at(reload, 1), Some(HitTarget::EditorCell { doc: 1, top: BAR_ROW, left: BANNER_RELOAD }));
        assert!(lines.iter().any(|l| l.contains("No files open")), "the body moved down: {lines:?}");
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let mut m = vm();
        let mut t = text::tests::sample_vm();
        t.bars.push(BarVm {
            label: "Find".into(),
            field: "日本".into(),
            field_w: 6,
            cursor: Some(2),
            ..BarVm::default()
        });
        let bodies = [
            BodyVm::Empty,
            BodyVm::Loading,
            BodyVm::Message(vec!["a".into(), "日本語".into()]),
            BodyVm::Text(t),
            BodyVm::Hex(hex::tests::sample_vm()),
            BodyVm::Image(image::tests::sample_vm()),
        ];
        for (k, body) in bodies.into_iter().enumerate() {
            m.body = body;
            m.banner = (k % 2 == 1).then(|| BannerVm {
                text: "This file changed on disk.".into(),
                buttons: vec![("[Reload Alt+E]".into(), BANNER_RELOAD), ("[Keep Esc]".into(), BANNER_KEEP)],
            });
            for edge in [false, true] {
                m.edge = edge;
                for w in 0..=20 {
                    for h in 0..=6 {
                        let _ = screen(&m, w, h);
                    }
                }
            }
        }
    }
}
