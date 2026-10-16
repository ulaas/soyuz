//! The one dropdown used across the app (settings, dialogs): a themed button that shows
//! the current value (with an optional icon, color swatches, a font preview), and a popup
//! list with keyboard navigation, an optional search box (shown for long lists), fuzzy
//! filtering with Turkish-aware folding, grouped sections, disabled items with reasons,
//! virtualized rows (hundreds of system fonts), a check mark on the current item and an
//! optional "Custom…" entry for settings that also take a free value.
//!
//! The filtering / ordering / navigation logic is the pure [`filter_rows`] /
//! [`navigate`] / [`type_ahead`] model below, unit-tested without egui.

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, EventFilter, FontFamily, FontId, Id, Key, Margin, Modifiers, Order, Rect,
    Response, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2,
};

use crate::palette::fuzzy::Fuzzy;
use crate::theme::icons::ph;
use crate::theme::{icons, palette};

// ---------------------------------------------------------------------------------------
// Items

/// One entry of a dropdown.
#[derive(Clone, Debug)]
pub struct Item<T> {
    pub value: T,
    pub label: String,
    /// Right-aligned secondary text (a hint, "Embedded", a file name…); also searched.
    pub secondary: String,
    /// Leading phosphor icon.
    pub icon: Option<&'static str>,
    pub icon_color: Option<Color32>,
    /// Leading color dots (accent presets, theme previews).
    pub swatches: Vec<Color32>,
    /// Draw the label in this font (font pickers).
    pub font: Option<FontFamily>,
    /// Section header this item sits under (empty: none). Items of a group should be
    /// adjacent; sections appear in the order of their first item.
    pub group: String,
    /// Not selectable; the text is the reason, shown as a tooltip.
    pub disabled: Option<String>,
    /// Extra words matched by the search (not shown).
    pub keywords: String,
}

impl<T> Item<T> {
    pub fn new(value: T, label: impl Into<String>) -> Self {
        Item {
            value,
            label: label.into(),
            secondary: String::new(),
            icon: None,
            icon_color: None,
            swatches: Vec::new(),
            font: None,
            group: String::new(),
            disabled: None,
            keywords: String::new(),
        }
    }

    pub fn secondary(mut self, s: impl Into<String>) -> Self {
        self.secondary = s.into();
        self
    }

    pub fn icon(mut self, glyph: &'static str) -> Self {
        self.icon = Some(glyph);
        self
    }

    pub fn icon_color(mut self, c: Color32) -> Self {
        self.icon_color = Some(c);
        self
    }

    pub fn swatch(mut self, c: Color32) -> Self {
        self.swatches.push(c);
        self
    }

    pub fn font(mut self, f: Option<FontFamily>) -> Self {
        self.font = f;
        self
    }

    pub fn group(mut self, g: impl Into<String>) -> Self {
        self.group = g.into();
        self
    }

    /// Disable the item when `reason` is `Some`.
    pub fn disabled(mut self, reason: Option<String>) -> Self {
        self.disabled = reason;
        self
    }

    pub fn keywords(mut self, k: impl Into<String>) -> Self {
        self.keywords = k.into();
        self
    }

    fn meta(&self) -> Meta<'_> {
        Meta {
            label: &self.label,
            secondary: &self.secondary,
            keywords: &self.keywords,
            group: &self.group,
            disabled: self.disabled.is_some(),
        }
    }
}

/// Plain `(value, label)` pairs as items.
pub fn items<T: Clone>(options: &[(T, &str)]) -> Vec<Item<T>> {
    options.iter().map(|(v, l)| Item::new(v.clone(), *l)).collect()
}

// ---------------------------------------------------------------------------------------
// Pure model

/// What the filter needs to know about an item.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meta<'a> {
    pub label: &'a str,
    pub secondary: &'a str,
    pub keywords: &'a str,
    pub group: &'a str,
    pub disabled: bool,
}

/// One visible row of the popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Section header; the index is an item of that section (for its group name).
    Header(usize),
    Item(usize),
    /// The "Custom…" entry.
    Custom,
}

fn fold(s: &str) -> String {
    s.chars().map(soyuz_core::locale::fold_char_for_match).collect()
}

/// Substring score: every word of the query occurs in the label, secondary text or
/// keywords (folded). Whole-query prefixes of the label rank first, then labels that
/// contain it (earlier is better), then other word matches.
fn substring_score(m: &Meta<'_>, q: &str, words: &[&str]) -> Option<u32> {
    let label = fold(m.label);
    let hay = format!("{label} {} {}", fold(m.secondary), fold(m.keywords));
    if !words.iter().all(|w| hay.contains(w)) {
        return None;
    }
    Some(match label.find(q) {
        Some(0) => 3_000_000,
        Some(pos) => 2_000_000 - (pos as u32).min(10_000),
        None => 1_000_000,
    })
}

/// The visible rows for `query`: sections in the order of their first item, a header
/// before each named section that has matches, items in their original order (no
/// query) or best match first (stable), then the custom entry when enabled. Substring
/// matches win; only when nothing contains the query does fuzzy matching kick in.
pub fn filter_rows(items: &[Meta<'_>], query: &str, custom: bool) -> Vec<Row> {
    let q = fold(query.trim());
    let words: Vec<&str> = q.split_whitespace().collect();
    let filtering = !words.is_empty();
    let substring = filtering && items.iter().any(|m| substring_score(m, &q, &words).is_some());
    let mut fz = Fuzzy::new(if substring { "" } else { query });
    let mut score = |m: &Meta<'_>| -> Option<u32> {
        if !filtering {
            Some(0)
        } else if substring {
            substring_score(m, &q, &words)
        } else {
            // The label counts most; secondary text and keywords still find it.
            let label = fz.score(m.label).map(|s| s.saturating_mul(2));
            let other = [m.secondary, m.keywords].iter().filter(|t| !t.is_empty()).filter_map(|t| fz.score(t)).max();
            label.max(other)
        }
    };
    // Section order: first appearance.
    let mut groups: Vec<&str> = Vec::new();
    for m in items {
        if !groups.contains(&m.group) {
            groups.push(m.group);
        }
    }
    let mut rows = Vec::with_capacity(items.len() + groups.len() + 1);
    for g in groups {
        let mut hits: Vec<(u32, usize)> = items
            .iter()
            .enumerate()
            .filter(|(_, m)| m.group == g)
            .filter_map(|(i, m)| score(m).map(|s| (s, i)))
            .collect();
        if hits.is_empty() {
            continue;
        }
        if filtering {
            hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        if !g.is_empty() {
            rows.push(Row::Header(hits[0].1));
        }
        rows.extend(hits.into_iter().map(|(_, i)| Row::Item(i)));
    }
    if custom {
        rows.push(Row::Custom);
    }
    rows
}

/// Char indices of `label` to highlight for `query`: the first occurrence of each query
/// word (folded), else the fuzzy match positions.
pub fn match_indices(label: &str, query: &str) -> Vec<u32> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = label.chars().map(soyuz_core::locale::fold_char_for_match).collect();
    let mut out = Vec::new();
    for w in q.split_whitespace() {
        let w: Vec<char> = w.chars().collect();
        if let Some(start) = (0..hay.len().saturating_sub(w.len() - 1)).find(|&i| hay[i..].starts_with(&w)) {
            out.extend((start..start + w.len()).map(|i| i as u32));
        }
    }
    if out.is_empty() {
        return Fuzzy::new(query).indices(label).map(|(_, v)| v).unwrap_or_default();
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Keyboard movement in the popup list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    /// By this many rows.
    PageUp(usize),
    PageDown(usize),
    Home,
    End,
}

fn selectable(rows: &[Row], items: &[Meta<'_>], r: usize) -> bool {
    match rows.get(r) {
        Some(Row::Item(i)) => !items.get(*i).is_some_and(|m| m.disabled),
        Some(Row::Custom) => true,
        _ => false,
    }
}

/// The new cursor row (an index into `rows`) after `nav`, skipping headers and disabled
/// items. Up / Down stop at the ends; without a cursor, Down goes to the first row and
/// Up to the last. `None` when nothing is selectable.
pub fn navigate(rows: &[Row], items: &[Meta<'_>], cursor: Option<usize>, nav: Nav) -> Option<usize> {
    let ok = |r: usize| selectable(rows, items, r);
    let first = (0..rows.len()).find(|&r| ok(r));
    let last = (0..rows.len()).rev().find(|&r| ok(r));
    let cur = cursor.filter(|&c| c < rows.len());
    let forward = |from: usize| (from..rows.len()).find(|&r| ok(r));
    let backward = |from: usize| (0..=from.min(rows.len().saturating_sub(1))).rev().find(|&r| ok(r));
    match nav {
        Nav::Home => first,
        Nav::End => last,
        Nav::Down => match cur {
            None => first,
            Some(c) => forward(c + 1).or(Some(c).filter(|&c| ok(c))).or(last),
        },
        Nav::Up => match cur {
            None => last,
            Some(0) => first,
            Some(c) => backward(c - 1).or(Some(c).filter(|&c| ok(c))).or(first),
        },
        Nav::PageDown(n) => match cur {
            None => first,
            Some(c) => backward((c + n.max(1)).min(rows.len().saturating_sub(1)))
                .filter(|&r| r > c)
                .or_else(|| forward(c + 1))
                .or(Some(c)),
        },
        Nav::PageUp(n) => match cur {
            None => last,
            Some(c) => forward(c.saturating_sub(n.max(1))).filter(|&r| r < c).or_else(|| backward(c)).or(Some(c)),
        },
    }
}

/// Type-ahead without a search box: the first selectable row whose label starts with
/// `prefix` (folded), else the first that contains it.
pub fn type_ahead(rows: &[Row], items: &[Meta<'_>], prefix: &str) -> Option<usize> {
    let p = soyuz_core::locale::fold_for_match(prefix.trim_start());
    if p.is_empty() {
        return None;
    }
    let label = |r: &Row| match r {
        Row::Item(i) => items.get(*i).map(|m| soyuz_core::locale::fold_for_match(m.label)),
        _ => None,
    };
    let candidates = || (0..rows.len()).filter(|&r| selectable(rows, items, r));
    candidates()
        .find(|&r| label(&rows[r]).is_some_and(|l| l.starts_with(&p)))
        .or_else(|| candidates().find(|&r| label(&rows[r]).is_some_and(|l| l.contains(&p))))
}

/// Lists longer than this get a search box.
pub const SEARCH_THRESHOLD: usize = 8;

// ---------------------------------------------------------------------------------------
// Widget

const BUTTON_H: f32 = 28.0;
const ROW_H: f32 = 28.0;
const HEADER_H: f32 = 24.0;
const SEARCH_H: f32 = 34.0;
const MAX_POPUP_H: f32 = 380.0;
const MAX_POPUP_W: f32 = 460.0;

#[derive(Clone, Default)]
struct PopupState {
    open: bool,
    query: String,
    cursor: Option<usize>,
    /// Scroll the cursor row into view (`Some(true)`: centered, on open).
    scroll: Option<bool>,
    /// Type-ahead buffer time (no search box).
    typed_at: f64,
    /// Popup width, measured on open.
    width: f32,
    /// Popup rect last frame (outside-click detection).
    rect: Option<Rect>,
    /// Just opened (focus the search box).
    fresh: bool,
}

fn any_open_id() -> Id {
    Id::unique("soyuz.dropdown.any_open")
}

fn active_id() -> Id {
    Id::unique("soyuz.dropdown.active")
}

fn open_request_id() -> Id {
    Id::unique("soyuz.dropdown.open_request")
}

/// Whether a dropdown popup was open last frame. Window-level key handlers (Esc closes
/// Settings, global shortcuts) leave the keyboard to it while this is true.
pub fn any_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<(u64, bool)>(any_open_id()))
        .is_some_and(|(pass, open)| open && ctx.cumulative_pass_nr().saturating_sub(pass) <= 1)
}

fn mark_open(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(any_open_id(), (pass, true)));
}

/// Open the dropdown with `key` on its next frame, with `query` typed into its search
/// (`SOYUZ_SCRIPT` step `dropdown:<key>[=query]`, screenshots).
pub fn request_open(ctx: &egui::Context, key: &str, query: &str) {
    ctx.data_mut(|d| d.insert_temp(open_request_id(), (key.to_string(), query.to_string())));
    ctx.request_repaint();
}

/// The result of [`Dropdown::show`].
pub struct Output {
    /// The button.
    pub response: Response,
    /// The value changed this frame.
    pub changed: bool,
    /// The custom entry was chosen; carries the search text typed at the time.
    pub custom: Option<String>,
    /// The popup opened this frame (start loading lazy items, e.g. a model list).
    pub opened: bool,
}

/// A dropdown; configure with the builder methods, then [`show`](Self::show).
pub struct Dropdown {
    key: String,
    width: f32,
    name: String,
    search: Option<bool>,
    custom: Option<String>,
    placeholder: Option<String>,
    enabled: bool,
}

impl Dropdown {
    /// `key` identifies the dropdown (state, scripted opening); unique per screen.
    pub fn new(key: impl Into<String>) -> Self {
        Dropdown {
            key: key.into(),
            width: 200.0,
            name: String::new(),
            search: None,
            custom: None,
            placeholder: None,
            enabled: true,
        }
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    /// Accessible name (usually the visible row label).
    pub fn name(mut self, n: impl Into<String>) -> Self {
        self.name = n.into();
        self
    }

    /// Force the search box on / off (default: on for more than [`SEARCH_THRESHOLD`] items).
    pub fn search(mut self, on: bool) -> Self {
        self.search = Some(on);
        self
    }

    /// Add a "Custom…" entry with this label (see [`Output::custom`]).
    pub fn custom(mut self, label: impl Into<String>) -> Self {
        self.custom = Some(label.into());
        self
    }

    /// Text on the button when the value is not among the items.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    fn id(&self) -> Id {
        Id::unique(("soyuz.dropdown", self.key.as_str()))
    }

    pub fn show<T: PartialEq + Clone>(self, ui: &mut Ui, value: &mut T, items: &[Item<T>]) -> Output {
        let id = self.id();
        let ctx = ui.ctx().clone();
        let mut st: PopupState = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
        let selected = items.iter().position(|it| it.value == *value);
        let metas: Vec<Meta<'_>> = items.iter().map(Item::meta).collect();
        let search = self.search.unwrap_or(items.len() > SEARCH_THRESHOLD);
        let (mut changed, mut custom, mut opened) = (false, None, false);

        // ---- button
        let sense = if self.enabled { Sense::click() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(vec2(self.width, BUTTON_H), sense);
        let current_label = match selected {
            Some(i) => items[i].label.clone(),
            None => self.placeholder.clone().unwrap_or_else(|| tr!("settings-custom").to_string()),
        };
        {
            let (name, value_text, open) = (self.name.clone(), current_label.clone(), st.open);
            let enabled = self.enabled;
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, &name));
            crate::ui::a11y::with(&ctx, resp.id, |n| {
                n.set_role(Role::ComboBox);
                if !name.is_empty() {
                    n.set_label(name.as_str());
                }
                n.set_value(value_text.as_str());
                n.set_expanded(open);
            });
        }
        let mut open_now = false;
        let mut close = false;
        if resp.clicked() {
            if st.open {
                close = true;
            } else {
                open_now = true;
            }
        }
        if resp.has_focus() && !st.open && self.enabled {
            // Arrow keys stay with the button (egui would move focus with them).
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(
                    resp.id,
                    EventFilter { tab: false, horizontal_arrows: false, vertical_arrows: true, escape: false },
                )
            });
            let keys = ctx.input_mut(|i| {
                i.consume_key(Modifiers::ALT, Key::ArrowDown)
                    || i.consume_key(Modifiers::NONE, Key::ArrowDown)
                    || i.consume_key(Modifiers::NONE, Key::ArrowUp)
                    || i.consume_key(Modifiers::NONE, Key::F4)
            });
            if keys {
                open_now = true;
            }
        }
        // Scripted opening.
        if let Some((k, q)) = ctx.data(|d| d.get_temp::<(String, String)>(open_request_id()))
            && k == self.key
        {
            ctx.data_mut(|d| d.remove::<(String, String)>(open_request_id()));
            open_now = true;
            st.query = q;
            resp.scroll_to_me(Some(Align::Center));
        }
        if open_now && !st.open {
            st.open = true;
            st.fresh = true;
            if !st.query.is_empty() && !search {
                st.query.clear();
            }
            st.width = popup_width(ui, items, self.width, self.custom.as_deref());
            let rows = filter_rows(&metas, &st.query, self.custom.is_some());
            st.cursor = if st.query.is_empty() {
                selected.and_then(|s| rows.iter().position(|r| *r == Row::Item(s)))
            } else {
                None
            }
            .or_else(|| navigate(&rows, &metas, None, Nav::Home));
            st.scroll = Some(true);
            opened = true;
            ctx.data_mut(|d| d.insert_temp(active_id(), id));
        }

        paint_button(ui, &resp, rect, items, selected, &current_label, st.open, self.enabled);

        // Only one popup at a time: another dropdown opened since.
        if st.open && !open_now && ctx.data(|d| d.get_temp::<Id>(active_id())).is_some_and(|a| a != id) {
            close = true;
        }
        // ---- popup
        if st.open && !close {
            mark_open(&ctx);
            let rows = filter_rows(&metas, &st.query, self.custom.is_some());
            if st.cursor.is_some_and(|c| !selectable(&rows, &metas, c)) {
                st.cursor = navigate(&rows, &metas, None, Nav::Home);
            }
            let search_id = id.with("search");
            // Keys (before the search box sees them).
            // Keys typed into a native popup window arrive here one frame later.
            if let Some(evs) = ctx.data_mut(|d| d.remove_temp::<Vec<egui::Event>>(id.with("fwd"))) {
                ctx.input_mut(|i| i.events.extend(evs));
            }
            let page = ((MAX_POPUP_H - SEARCH_H) / ROW_H) as usize;
            let (nav, enter, esc, tab) = ctx.input_mut(|i| {
                let mut nav = None;
                for (k, n) in [
                    (Key::ArrowDown, Nav::Down),
                    (Key::ArrowUp, Nav::Up),
                    (Key::PageDown, Nav::PageDown(page)),
                    (Key::PageUp, Nav::PageUp(page)),
                    (Key::Home, Nav::Home),
                    (Key::End, Nav::End),
                ] {
                    if i.consume_key(Modifiers::NONE, k) {
                        nav = Some(n);
                    }
                }
                let close_alt = i.consume_key(Modifiers::ALT, Key::ArrowUp) || i.consume_key(Modifiers::NONE, Key::F4);
                (
                    nav,
                    i.consume_key(Modifiers::NONE, Key::Enter)
                        || (!search && i.consume_key(Modifiers::NONE, Key::Space)),
                    i.consume_key(Modifiers::NONE, Key::Escape) || close_alt,
                    i.key_pressed(Key::Tab),
                )
            });
            if let Some(n) = nav {
                st.cursor = navigate(&rows, &metas, st.cursor, n);
                st.scroll = Some(false);
            }
            // Type-ahead when there is no search box.
            if !search && resp.has_focus() {
                let typed: String = ctx.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::Text(t) => Some(t.clone()),
                            _ => None,
                        })
                        .collect()
                });
                if !typed.is_empty() {
                    let now = ctx.input(|i| i.time);
                    if now - st.typed_at > 1.0 {
                        st.query.clear();
                    }
                    st.typed_at = now;
                    st.query.push_str(&typed);
                    if let Some(r) = type_ahead(&rows, &metas, &st.query) {
                        st.cursor = Some(r);
                        st.scroll = Some(false);
                    }
                }
            }
            let mut pick: Option<Row> = None;
            if enter {
                pick = st.cursor.and_then(|c| rows.get(c).copied());
                if pick.is_none() {
                    close = true;
                }
            }
            if esc || tab {
                close = true;
            }
            if !close && pick.is_none() {
                let res = popup(ui, id, &self, &mut st, &rows, items, selected, rect, search, search_id);
                if let Some(r) = res {
                    pick = Some(r);
                }
            }
            if let Some(r) = pick {
                match r {
                    Row::Item(i) if items[i].disabled.is_none() => {
                        if items[i].value != *value {
                            *value = items[i].value.clone();
                            changed = true;
                        }
                        close = true;
                    }
                    Row::Custom => {
                        custom = Some(if search { st.query.trim().to_string() } else { String::new() });
                        close = true;
                    }
                    _ => {}
                }
            }
            // Outside click closes.
            if !close {
                let pressed_outside = ctx.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|pos| {
                            !rect.contains(pos) && !st.rect.is_some_and(|r| r.expand(2.0).contains(pos))
                        })
                });
                if pressed_outside {
                    close = true;
                }
            }
            if close {
                let had_focus =
                    ctx.memory(|m| m.focused()) == Some(search_id) || resp.has_focus() || esc || pick.is_some();
                if had_focus && !tab {
                    resp.request_focus();
                }
            }
        }
        if close {
            st.open = false;
            st.query.clear();
            st.rect = None;
            ctx.request_repaint();
        }
        if resp.has_focus() {
            crate::ui::widgets::focus_ring(ui, rect);
        }
        if let Some(i) = selected
            && let Some(reason) = &items[i].disabled
        {
            resp.clone().on_hover_text(reason);
        }
        ctx.data_mut(|d| d.insert_temp(id, st));
        Output { response: resp, changed, custom, opened }
    }
}

fn popup_width<T>(ui: &Ui, items: &[Item<T>], min: f32, custom: Option<&str>) -> f32 {
    let font = FontId::proportional(13.0);
    let small = FontId::proportional(11.5);
    let p = palette(ui.ctx());
    let mut w: f32 = min;
    // Long lists (system fonts): measuring the first few hundred is plenty.
    for it in items.iter().take(300) {
        let mut x = 76.0;
        if !it.swatches.is_empty() {
            x += it.swatches.len() as f32 * 14.0 + 4.0;
        } else if it.icon.is_some() {
            x += 20.0;
        }
        let f = it.font.clone().map_or(font.clone(), |fam| FontId::new(13.0, fam));
        x += ui.painter().layout_no_wrap(it.label.clone(), f, p.text_primary).size().x;
        if !it.secondary.is_empty() {
            x += 16.0 + ui.painter().layout_no_wrap(it.secondary.clone(), small.clone(), p.text_primary).size().x;
        }
        w = w.max(x);
    }
    if let Some(c) = custom {
        w = w.max(60.0 + ui.painter().layout_no_wrap(c.to_string(), font, p.text_primary).size().x);
    }
    w.min(MAX_POPUP_W.max(min))
}

#[allow(clippy::too_many_arguments)]
fn paint_button<T>(
    ui: &Ui,
    resp: &Response,
    rect: Rect,
    items: &[Item<T>],
    selected: Option<usize>,
    label: &str,
    open: bool,
    enabled: bool,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = palette(ui.ctx());
    let painter = ui.painter();
    let hover = ui.ctx().animate_bool_with_time(
        resp.id.with("hover"),
        resp.hovered() && enabled,
        crate::theme::anim(ui.ctx(), 0.10),
    );
    let cr = CornerRadius::same(7);
    let fill = super::super::settings::widgets::lerp_color(
        p.bg_surface1.gamma_multiply(if p.dark { 0.8 } else { 1.0 }),
        p.bg_surface2,
        hover,
    );
    painter.rect_filled(rect, cr, fill);
    let stroke = if open {
        Stroke::new(1.0, p.accent)
    } else if hover > 0.5 {
        Stroke::new(1.0, p.border_strong)
    } else {
        Stroke::new(1.0, p.border_subtle)
    };
    painter.rect_stroke(rect, cr, stroke, StrokeKind::Inside);
    let fg = if enabled { p.text_primary } else { p.text_tertiary };
    let mut x = rect.left() + 10.0;
    let cy = rect.center().y;
    let item = selected.map(|i| &items[i]);
    if let Some(it) = item {
        x = paint_lead(painter, it, x, cy, enabled, &p);
    }
    let caret_w = 22.0;
    let max_w = (rect.right() - caret_w - x).max(8.0);
    let font = item.and_then(|it| it.font.clone()).map_or(FontId::proportional(12.5), |f| FontId::new(13.0, f));
    let g = crate::ui::widgets::trunc_galley(painter, label, font, fg, max_w);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, fg);
    let caret = if open { ph::CARET_UP } else { ph::CARET_DOWN };
    painter.text(
        pos2(rect.right() - 12.0, cy),
        Align2::CENTER_CENTER,
        caret,
        icons::font(12.0),
        if enabled { p.text_secondary } else { p.text_tertiary },
    );
}

/// Leading swatches / icon of an item at `x`; returns where the label starts.
fn paint_lead<T>(
    painter: &egui::Painter,
    it: &Item<T>,
    mut x: f32,
    cy: f32,
    enabled: bool,
    p: &crate::theme::Palette,
) -> f32 {
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    if !it.swatches.is_empty() {
        for c in &it.swatches {
            painter.circle_filled(pos2(x + 6.0, cy), 6.0, dim(*c));
            painter.circle_stroke(pos2(x + 6.0, cy), 6.0, Stroke::new(1.0, p.border_subtle));
            x += 14.0;
        }
        x += 6.0;
    } else if let Some(g) = it.icon {
        let c = if enabled { it.icon_color.unwrap_or(p.text_secondary) } else { p.text_tertiary };
        painter.text(pos2(x + 7.0, cy), Align2::CENTER_CENTER, g, icons::font(14.0), c);
        x += 22.0;
    }
    x
}

/// Label with the matched characters highlighted.
fn label_job(text: &str, font: FontId, color: Color32, hl: Color32, matched: &[u32], max_w: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = TextFormat { font_id: font.clone(), color, ..Default::default() };
    let strong = TextFormat { font_id: font, color: hl, ..Default::default() };
    if matched.is_empty() {
        job.append(text, 0.0, plain);
    } else {
        let mut buf = String::new();
        let mut on = false;
        for (ci, ch) in text.chars().enumerate() {
            let m = matched.binary_search(&(ci as u32)).is_ok();
            if m != on && !buf.is_empty() {
                job.append(&buf, 0.0, if on { strong.clone() } else { plain.clone() });
                buf.clear();
            }
            on = m;
            buf.push(ch);
        }
        if !buf.is_empty() {
            job.append(&buf, 0.0, if on { strong } else { plain });
        }
    }
    job.wrap = TextWrapping::truncate_at_width(max_w.max(8.0));
    job
}

#[allow(clippy::too_many_arguments)]
fn popup<T: PartialEq>(
    ui: &Ui,
    id: Id,
    dd: &Dropdown,
    st: &mut PopupState,
    rows: &[Row],
    items: &[Item<T>],
    selected: Option<usize>,
    button: Rect,
    search: bool,
    search_id: Id,
) -> Option<Row> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let screen = ctx.content_rect();
    let pad = 4.0;
    let list_h: f32 = rows.iter().map(|r| if matches!(r, Row::Header(_)) { HEADER_H } else { ROW_H }).sum::<f32>();
    let search_h = if search { SEARCH_H + 4.0 } else { 0.0 };
    let no_items = !rows.iter().any(|r| matches!(r, Row::Item(_)));
    let empty_h = if no_items { ROW_H } else { 0.0 };
    let want = list_h + search_h + empty_h + pad * 2.0;
    let below = screen.bottom() - button.bottom() - 10.0;
    let above = button.top() - screen.top() - 10.0;
    let place_below = below >= want.min(MAX_POPUP_H) || below >= above;
    let native = crate::ui::native::active(&ctx)
        && (crate::ui::native::popup::is_native_open(id.with("native"))
            || want.min(MAX_POPUP_H) > below.max(above) + 0.5);
    let room = if place_below { below } else { above };
    let h = if native {
        want.min(MAX_POPUP_H)
    } else {
        want.min(MAX_POPUP_H).min(room).max((ROW_H * 3.0 + search_h + pad * 2.0).min(want))
    };
    let w = st.width.max(button.width()).min(screen.width() - 16.0);
    let x = button.left().clamp(screen.left() + 8.0, (screen.right() - 8.0 - w).max(screen.left() + 8.0));
    let y = if place_below { button.bottom() + 4.0 } else { button.top() - 4.0 - h };
    let t = ctx.animate_bool_with_time(id.with("anim"), true, crate::theme::anim(&ctx, 0.12));
    let slide = (1.0 - t) * if place_below { -6.0 } else { 6.0 };
    if t < 1.0 {
        ctx.request_repaint();
    }
    let mut pick = None;
    let native_key = id.with("native");
    let frame = egui::Frame::new()
        .fill(p.bg_surface1)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::same(pad as i8));
    let shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha((p.shadow_alpha * 255.0).min(255.0) as u8),
    };
    let mut draw = |ui: &mut Ui| {
        let inner_w = w - pad * 2.0;
        ui.set_width(inner_w);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        crate::ui::a11y::container(&ctx, ui.scope_id(), Role::ListBox, &dd.name);
        if search {
            search_box(ui, st, search_id, inner_w);
            ui.add_space(4.0);
        }
        if no_items {
            let (r, _) = ui.allocate_exact_size(vec2(inner_w, ROW_H), Sense::hover());
            ui.painter().text(
                pos2(r.left() + 12.0, r.center().y),
                Align2::LEFT_CENTER,
                tr!("dropdown-no-matches"),
                FontId::proportional(12.5),
                p.text_tertiary,
            );
        }
        let list_max = h - search_h - pad * 2.0 - empty_h;
        let pointer_moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        egui::ScrollArea::vertical()
            .id_salt(id.with("scroll"))
            .max_height(list_max)
            .min_scrolled_height(list_max.min(list_h))
            .auto_shrink([false, true])
            .show_viewport(ui, |ui, viewport| {
                let origin = ui.max_rect().min;
                let (_, _) = ui.allocate_exact_size(vec2(inner_w, list_h), Sense::hover());
                let n_items = rows.iter().filter(|r| !matches!(r, Row::Header(_))).count();
                let mut y = 0.0;
                let mut pos_in_set = 0usize;
                for (ri, row) in rows.iter().enumerate() {
                    let rh = if matches!(row, Row::Header(_)) { HEADER_H } else { ROW_H };
                    let r = Rect::from_min_size(origin + vec2(0.0, y), vec2(inner_w, rh));
                    let visible = y + rh >= viewport.top() - ROW_H && y <= viewport.bottom() + ROW_H;
                    let wants_scroll = st.scroll.is_some() && st.cursor == Some(ri);
                    y += rh;
                    if !matches!(row, Row::Header(_)) {
                        pos_in_set += 1;
                    }
                    if wants_scroll {
                        let center = st.scroll == Some(true);
                        ui.scroll_to_rect(r, if center { Some(Align::Center) } else { None });
                        st.scroll = None;
                    }
                    if !visible {
                        continue;
                    }
                    match *row {
                        Row::Header(i) => {
                            let text = crate::ui::widgets::caption_upper(&items[i].group);
                            ui.painter().text(
                                pos2(r.left() + 10.0, r.bottom() - 6.0),
                                Align2::LEFT_BOTTOM,
                                text,
                                FontId::proportional(10.5),
                                p.text_tertiary,
                            );
                        }
                        Row::Item(i) => {
                            let it = &items[i];
                            let rid = id.with(("row", i));
                            let enabled = it.disabled.is_none();
                            let resp = ui.interact(r, rid, if enabled { Sense::click() } else { Sense::hover() });
                            if resp.hovered() && pointer_moved && enabled {
                                st.cursor = Some(ri);
                            }
                            let is_cursor = st.cursor == Some(ri);
                            let is_sel = selected == Some(i);
                            if is_cursor {
                                ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface3);
                            } else if resp.hovered() && enabled {
                                ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface2);
                            }
                            let painter = ui.painter_at(r);
                            let cy = r.center().y;
                            let lx = paint_lead(&painter, it, r.left() + 10.0, cy, enabled, &p);
                            let right = r.right() - 30.0;
                            let mut sec_w = 0.0;
                            if !it.secondary.is_empty() {
                                let lf = it.font.clone().map_or(FontId::proportional(13.0), |f| FontId::new(13.0, f));
                                let label_w = painter.layout_no_wrap(it.label.clone(), lf, p.text_primary).size().x;
                                let g = crate::ui::widgets::trunc_galley(
                                    &painter,
                                    &it.secondary,
                                    FontId::proportional(11.5),
                                    p.text_tertiary,
                                    (right - lx - label_w - 16.0).max((right - lx) * 0.35),
                                );
                                sec_w = g.size().x + 12.0;
                                painter.galley(pos2(right - g.size().x, cy - g.size().y / 2.0), g, p.text_tertiary);
                            }
                            let color = if enabled { p.text_primary } else { p.text_tertiary.gamma_multiply(0.8) };
                            let font = it.font.clone().map_or(FontId::proportional(13.0), |f| FontId::new(13.0, f));
                            let matched = match_indices(&it.label, &st.query);
                            let hl = if p.dark { p.accent_hover } else { p.accent };
                            let job = label_job(&it.label, font, color, hl, &matched, right - lx - sec_w);
                            let g = painter.layout_job(job);
                            painter.galley(pos2(lx, cy - g.size().y / 2.0), g, color);
                            if is_sel {
                                painter.text(
                                    pos2(r.right() - 15.0, cy),
                                    Align2::CENTER_CENTER,
                                    ph::CHECK,
                                    icons::font(13.0),
                                    p.accent,
                                );
                            }
                            let label = if it.secondary.is_empty() {
                                it.label.clone()
                            } else {
                                format!("{}, {}", it.label, it.secondary)
                            };
                            crate::ui::a11y::with(&ctx, resp.id, |n| {
                                n.set_role(Role::ListBoxOption);
                                n.set_label(label.as_str());
                                n.set_selected(is_sel);
                                n.set_position_in_set(pos_in_set);
                                n.set_size_of_set(n_items);
                                if !enabled {
                                    n.set_disabled();
                                }
                            });
                            if let Some(reason) = &it.disabled {
                                resp.clone().on_hover_text(reason);
                            }
                            if resp.clicked() {
                                pick = Some(Row::Item(i));
                            }
                        }
                        Row::Custom => {
                            let rid = id.with("custom");
                            let resp = ui.interact(r, rid, Sense::click());
                            if resp.hovered() && pointer_moved {
                                st.cursor = Some(ri);
                            }
                            if st.cursor == Some(ri) {
                                ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface3);
                            }
                            if ri > 0 {
                                ui.painter().hline(r.x_range().shrink(6.0), r.top(), Stroke::new(1.0, p.border_subtle));
                            }
                            let painter = ui.painter_at(r);
                            let cy = r.center().y;
                            painter.text(
                                pos2(r.left() + 17.0, cy),
                                Align2::CENTER_CENTER,
                                ph::PENCIL_SIMPLE,
                                icons::font(14.0),
                                p.text_secondary,
                            );
                            let base = dd.custom.clone().unwrap_or_default();
                            let q = st.query.trim();
                            let text = if search && !q.is_empty() { tr!("dropdown-use-typed", text = q) } else { base };
                            let g = crate::ui::widgets::trunc_galley(
                                &painter,
                                &text,
                                FontId::proportional(13.0),
                                p.text_primary,
                                r.width() - 44.0,
                            );
                            painter.galley(pos2(r.left() + 32.0, cy - g.size().y / 2.0), g, p.text_primary);
                            crate::ui::a11y::with(&ctx, resp.id, |n| {
                                n.set_role(Role::ListBoxOption);
                                n.set_label(text.as_str());
                                n.set_selected(false);
                                n.set_position_in_set(pos_in_set);
                                n.set_size_of_set(n_items);
                            });
                            if resp.clicked() {
                                pick = Some(Row::Custom);
                            }
                        }
                    }
                }
            });
    };
    if native {
        let side = crate::ui::native::geometry::Side::Below;
        let out = crate::ui::native::popup::native_area(&ctx, native_key, button, side, vec2(w, h), &frame, |ui| {
            forward_keys(ui, id, search);
            draw(ui);
        });
        st.rect = None;
        if out.lost {
            st.open = false;
            st.query.clear();
        }
    } else {
        let area = egui::Area::new(id.with("popup"))
            .order(Order::Foreground)
            .fixed_pos(pos2(x, y + slide))
            .constrain(false)
            .show(&ctx, |ui| {
                ui.set_opacity(t.max(0.05));
                frame.shadow(shadow).show(ui, |ui| draw(ui));
            });
        st.rect = Some(area.response.rect);
    }
    pick
}

fn search_box(ui: &mut Ui, st: &mut PopupState, search_id: Id, w: f32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(w, SEARCH_H), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(6), p.bg_sunken);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        ph::MAGNIFYING_GLASS,
        icons::font(13.0),
        p.text_tertiary,
    );
    let field = Rect::from_min_max(pos2(rect.left() + 26.0, rect.top()), pos2(rect.right() - 6.0, rect.bottom()));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(Align::Center)));
    let resp = child.add(
        TextEdit::singleline(&mut st.query)
            .id(search_id)
            .frame(egui::Frame::NONE)
            .hint_text(tr!("dropdown-search"))
            .desired_width(field.width())
            .margin(Margin::symmetric(4, 6)),
    );
    crate::ui::a11y::name(&resp, tr!("dropdown-search"));
    if resp.changed() {
        st.cursor = None;
        st.scroll = Some(false);
    }
    if st.fresh {
        resp.request_focus();
        st.fresh = false;
    }
    // Up / Down / Esc / Enter belong to the list, not to focus movement.
    ui.ctx().memory_mut(|m| {
        m.set_focus_lock_filter(
            search_id,
            EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true },
        )
    });
}

/// A choice over strings that also takes a free value: the items, a "Custom…" entry,
/// and an inline text field next to the button while the value is custom (not one of
/// the items) or the user picked "Custom…". Returns `true` when the value changed.
#[allow(clippy::too_many_arguments)]
pub fn text_choice(
    ui: &mut Ui,
    key: &str,
    name: &str,
    value: &mut String,
    items: &[Item<String>],
    width: f32,
    custom_label: &str,
    hint: &str,
) -> bool {
    let flag = Id::unique(("soyuz.dropdown.custom_edit", key));
    let mut editing: bool = ui.ctx().data(|d| d.get_temp(flag)).unwrap_or(false);
    let known = items.iter().any(|it| it.value == *value);
    let is_custom = !known && !value.trim().is_empty();
    let before = value.clone();
    let out = Dropdown::new(key)
        .width(width)
        .name(name)
        .custom(custom_label)
        .placeholder(if value.trim().is_empty() { hint.to_string() } else { tr!("settings-custom").to_string() })
        .show(ui, value, items);
    if out.changed {
        editing = false;
    }
    let mut focus = false;
    if let Some(typed) = out.custom {
        editing = true;
        focus = true;
        if !typed.is_empty() {
            *value = typed;
        }
    }
    if editing || is_custom {
        let fid = Id::unique(("soyuz.dropdown.custom_field", key));
        let r = ui.add(
            TextEdit::singleline(value)
                .id(fid)
                .hint_text(hint)
                .font(egui::TextStyle::Monospace)
                .desired_width(width.max(160.0))
                .margin(Margin::symmetric(8, 5)),
        );
        crate::ui::a11y::name(&r, &format!("{name} ({custom_label})"));
        if focus {
            r.request_focus();
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(flag, editing));
    *value != before
}

/// Keys pressed in the dropdown's native popup window go to the owner window's key
/// handling (next frame), like the in-window popup, which never takes the keyboard.
fn forward_keys(ui: &Ui, id: Id, search: bool) {
    let evs: Vec<egui::Event> = ui.input(|i| {
        i.events
            .iter()
            .filter(|e| match e {
                egui::Event::Key { .. } => true,
                egui::Event::Text(_) => !search,
                _ => false,
            })
            .cloned()
            .collect()
    });
    if !evs.is_empty() {
        ui.ctx().data_mut(|d| d.insert_temp(id.with("fwd"), evs));
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta<'a>(label: &'a str, group: &'a str) -> Meta<'a> {
        Meta { label, group, ..Default::default() }
    }

    fn fonts() -> Vec<Meta<'static>> {
        vec![
            meta("Inter", "Embedded"),
            meta("IBM Plex Sans", "Embedded"),
            meta("Noto Sans", "Embedded"),
            meta("Arial", "System fonts"),
            meta("Segoe UI", "System fonts"),
            meta("Segoe UI Variable", "System fonts"),
            Meta { label: "Webdings", group: "System fonts", disabled: true, ..Default::default() },
        ]
    }

    #[test]
    fn groups_get_headers_in_first_appearance_order() {
        let m = fonts();
        let rows = filter_rows(&m, "", false);
        assert_eq!(rows[0], Row::Header(0));
        assert_eq!(&rows[1..4], &[Row::Item(0), Row::Item(1), Row::Item(2)]);
        assert_eq!(rows[4], Row::Header(3));
        assert_eq!(rows.len(), 2 + 7);
        // Ungrouped items get no header; custom goes last.
        let plain = [meta("a", ""), meta("b", "")];
        assert_eq!(filter_rows(&plain, "", true), vec![Row::Item(0), Row::Item(1), Row::Custom]);
    }

    #[test]
    fn filtering_drops_empty_sections_and_ranks_best_first() {
        let m = fonts();
        let rows = filter_rows(&m, "segoe", false);
        assert_eq!(rows, vec![Row::Header(4), Row::Item(4), Row::Item(5)]);
        // Prefix match beats a scattered one within a section.
        let m = [meta("Source Code Pro", ""), meta("Cascadia Mono", ""), meta("Consolas", "")];
        let rows = filter_rows(&m, "cons", false);
        assert_eq!(rows.first(), Some(&Row::Item(2)));
        // Nothing found: only the custom entry stays.
        assert_eq!(filter_rows(&fonts(), "zzzz", true), vec![Row::Custom]);
    }

    #[test]
    fn substring_matches_hide_scattered_fuzzy_ones() {
        let m = [meta("Bodoni MT Condensed", ""), meta("Consolas", ""), meta("Lucida Console", "")];
        assert_eq!(filter_rows(&m, "cons", false), vec![Row::Item(1), Row::Item(2)]);
        // No substring anywhere: fuzzy matching finds abbreviations.
        assert_eq!(filter_rows(&m, "lcon", false), vec![Row::Item(2)]);
        assert_eq!(match_indices("Lucida Console", "cons"), vec![7, 8, 9, 10]);
        assert_eq!(match_indices("İstanbul", "ist"), vec![0, 1, 2]);
        assert!(match_indices("Arial", "").is_empty());
    }

    #[test]
    fn secondary_text_and_keywords_are_searched() {
        let m = [
            Meta { label: "pwsh", secondary: "PowerShell 7", ..Default::default() },
            Meta { label: "cmd", keywords: "command prompt", ..Default::default() },
        ];
        assert_eq!(filter_rows(&m, "powershell", false), vec![Row::Item(0)]);
        assert_eq!(filter_rows(&m, "prompt", false), vec![Row::Item(1)]);
    }

    #[test]
    fn turkish_case_folding() {
        let m = [meta("İngilizce", ""), meta("Türkçe", ""), meta("Işık", "")];
        assert_eq!(filter_rows(&m, "ingi", false), vec![Row::Item(0)]);
        assert_eq!(filter_rows(&m, "TÜRK", false), vec![Row::Item(1)]);
        assert_eq!(filter_rows(&m, "isik", false), vec![Row::Item(2)]);
    }

    #[test]
    fn navigation_skips_headers_and_disabled_items() {
        let m = fonts();
        let rows = filter_rows(&m, "", true);
        // rows: H, I0, I1, I2, H, I3, I4, I5, I6(disabled), Custom
        assert_eq!(navigate(&rows, &m, None, Nav::Down), Some(1));
        assert_eq!(navigate(&rows, &m, Some(3), Nav::Down), Some(5), "skips the header");
        assert_eq!(navigate(&rows, &m, Some(5), Nav::Up), Some(3));
        assert_eq!(navigate(&rows, &m, Some(7), Nav::Down), Some(9), "skips the disabled item");
        assert_eq!(navigate(&rows, &m, Some(9), Nav::Down), Some(9), "stops at the end");
        assert_eq!(navigate(&rows, &m, Some(1), Nav::Up), Some(1), "stops at the start");
        assert_eq!(navigate(&rows, &m, Some(5), Nav::Home), Some(1));
        assert_eq!(navigate(&rows, &m, Some(1), Nav::End), Some(9));
        assert_eq!(navigate(&rows, &m, None, Nav::Up), Some(9));
    }

    #[test]
    fn paging() {
        let labels: Vec<String> = (0..30).map(|i| format!("item {i}")).collect();
        let m: Vec<Meta<'_>> = labels.iter().map(|l| meta(l, "")).collect();
        let rows = filter_rows(&m, "", false);
        assert_eq!(navigate(&rows, &m, Some(0), Nav::PageDown(10)), Some(10));
        assert_eq!(navigate(&rows, &m, Some(25), Nav::PageDown(10)), Some(29));
        assert_eq!(navigate(&rows, &m, Some(29), Nav::PageDown(10)), Some(29));
        assert_eq!(navigate(&rows, &m, Some(15), Nav::PageUp(10)), Some(5));
        assert_eq!(navigate(&rows, &m, Some(3), Nav::PageUp(10)), Some(0));
        assert_eq!(navigate(&[], &[], None, Nav::Down), None);
    }

    #[test]
    fn accesskit_combobox_and_listbox() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_fonts(crate::theme::fonts::definitions());
        let items = vec![Item::new(1, "Dark"), Item::new(2, "Light").disabled(Some("no".into()))];
        let mut v = 1;
        let mut run = |open: bool| {
            if open {
                request_open(&ctx, "t", "");
            }
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                Dropdown::new("t").name("Mode").show(ui, &mut v, &items);
            });
            out.textures_delta.clear();
            out.platform_output.accesskit_update.take().map(|u| u.nodes).unwrap_or_default()
        };
        run(false);
        let nodes = run(true);
        let nodes = if nodes.iter().any(|(_, n)| n.role() == Role::ListBoxOption) { nodes } else { run(false) };
        let combo = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::ComboBox).expect("combobox");
        assert_eq!(combo.label(), Some("Mode"));
        assert_eq!(combo.value(), Some("Dark"));
        let opts: Vec<_> = nodes.iter().map(|(_, n)| n).filter(|n| n.role() == Role::ListBoxOption).collect();
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].is_selected(), Some(true));
        assert!(opts[1].is_disabled());
    }

    #[test]
    fn type_ahead_prefers_prefixes() {
        let m = [meta("Dark", ""), meta("Light", ""), meta("System (light at day)", "")];
        let rows = filter_rows(&m, "", false);
        assert_eq!(type_ahead(&rows, &m, "l"), Some(1));
        assert_eq!(type_ahead(&rows, &m, "sy"), Some(2));
        assert_eq!(type_ahead(&rows, &m, "day"), Some(2), "falls back to contains");
        assert_eq!(type_ahead(&rows, &m, "x"), None);
    }
}
