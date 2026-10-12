//! Popup menus: the context menu (Shift+F10, right click), submenus, and any list of
//! commands that opens at a point.
//!
//! A [`Menu`] holds [`MenuItem`]s: entries with a label, an optional chord label shown on
//! the right (`chord_label` of the action's binding), an optional letter accelerator, a
//! check mark, a disabled flag and either a payload `T` (what activating it means to the
//! caller, typically an `Action` or a `Cmd`) or a submenu; and separators.
//!
//! Keys ([`Menu::apply`], the deepest open submenu first): Up / Down move between enabled
//! entries (wrapping), Home / End, Enter / Space activate (or open a submenu), Right opens
//! a submenu, Left / Esc close the submenu (Esc on the root menu closes the menu), a
//! letter jumps to the entries with that accelerator (or, without accelerators, the first
//! letter of the label): one match activates, several cycle. Mouse ([`Menu::click`]):
//! click targets `MenuItem { depth, index }` recorded by [`Menu::draw`].
//!
//! Placement ([`Menu::layout`]): below-right of the anchor cell, flipped up / left when it
//! does not fit; submenus open beside their entry. Taller than the screen, the menu
//! scrolls with the cursor.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::Clear;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_core::config::{KeyName, Mods};

use crate::borders;
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// What an entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind<T> {
    /// Activating it returns this payload.
    Run(T),
    /// It opens these items.
    Submenu(Vec<MenuItem<T>>),
}

/// One entry of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry<T> {
    pub label: String,
    /// The key that runs it anyway (shown on the right).
    pub chord: Option<String>,
    /// The letter that picks it in the open menu (lower case, drawn underlined when the
    /// label contains it).
    pub accel: Option<char>,
    /// A check mark (`Some(true)` on, `Some(false)` an empty box).
    pub check: Option<bool>,
    pub enabled: bool,
    pub kind: EntryKind<T>,
}

/// A menu row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem<T> {
    Entry(MenuEntry<T>),
    Separator,
}

impl<T> MenuItem<T> {
    /// An enabled entry running `payload`.
    pub fn run(label: impl Into<String>, payload: T) -> MenuItem<T> {
        MenuItem::Entry(MenuEntry {
            label: label.into(),
            chord: None,
            accel: None,
            check: None,
            enabled: true,
            kind: EntryKind::Run(payload),
        })
    }

    /// An enabled entry opening `items`.
    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem<T>>) -> MenuItem<T> {
        MenuItem::Entry(MenuEntry {
            label: label.into(),
            chord: None,
            accel: None,
            check: None,
            enabled: true,
            kind: EntryKind::Submenu(items),
        })
    }

    /// With a chord label.
    pub fn chord(mut self, chord: impl Into<String>) -> Self {
        if let MenuItem::Entry(e) = &mut self {
            let c = chord.into();
            e.chord = (!c.is_empty()).then_some(c);
        }
        self
    }

    /// With a letter accelerator.
    pub fn accel(mut self, c: char) -> Self {
        if let MenuItem::Entry(e) = &mut self {
            e.accel = Some(c.to_ascii_lowercase());
        }
        self
    }

    /// With a check mark.
    pub fn checked(mut self, on: bool) -> Self {
        if let MenuItem::Entry(e) = &mut self {
            e.check = Some(on);
        }
        self
    }

    /// Enabled or not.
    pub fn enabled(mut self, on: bool) -> Self {
        if let MenuItem::Entry(e) = &mut self {
            e.enabled = on;
        }
        self
    }

    fn entry(&self) -> Option<&MenuEntry<T>> {
        match self {
            MenuItem::Entry(e) => Some(e),
            MenuItem::Separator => None,
        }
    }

    fn selectable(&self) -> bool {
        self.entry().is_some_and(|e| e.enabled)
    }
}

/// What a key or click did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOutcome<T> {
    /// Moved, opened or closed a submenu.
    Consumed,
    /// Not a menu key.
    Ignored,
    /// An entry was activated: close the menu and run this.
    Activated(T),
    /// Esc on the root menu (or a click outside): close it.
    Close,
}

/// An open menu (and its open submenu, if any).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu<T> {
    pub items: Vec<MenuItem<T>>,
    /// The cell it opens at (below-right of it).
    pub anchor: (u16, u16),
    /// The highlighted item (an enabled entry), if any.
    pub cursor: Option<usize>,
    /// First item shown when the menu is taller than the screen.
    pub top: usize,
    /// The open submenu (of the item at `cursor`).
    pub sub: Option<Box<Menu<T>>>,
}

/// Columns around the label: a border and a space on the left (plus the check mark),
/// a space, the submenu arrow and a border on the right.
const PAD: u16 = 4;

impl<T: Clone> Menu<T> {
    /// A menu at `anchor` with the cursor on the first enabled entry.
    pub fn new(items: Vec<MenuItem<T>>, anchor: (u16, u16)) -> Menu<T> {
        let cursor = items.iter().position(MenuItem::selectable);
        Menu { items, anchor, cursor, top: 0, sub: None }
    }

    /// How deep the open submenus go (0: only this menu).
    pub fn depth(&self) -> usize {
        self.sub.as_ref().map_or(0, |s| 1 + s.depth())
    }

    /// The menu at `depth` (0: this one).
    pub fn level(&self, depth: usize) -> Option<&Menu<T>> {
        if depth == 0 { Some(self) } else { self.sub.as_ref()?.level(depth - 1) }
    }

    fn level_mut(&mut self, depth: usize) -> Option<&mut Menu<T>> {
        if depth == 0 { Some(self) } else { self.sub.as_mut()?.level_mut(depth - 1) }
    }

    fn step(&mut self, forward: bool) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        let start = self.cursor.unwrap_or(if forward { n - 1 } else { 0 });
        for k in 1..=n {
            let i = if forward { (start + k) % n } else { (start + n - k % n) % n };
            if self.items[i].selectable() {
                self.cursor = Some(i);
                return;
            }
        }
    }

    fn edge(&mut self, first: bool) {
        let mut it: Box<dyn Iterator<Item = (usize, &MenuItem<T>)>> =
            if first { Box::new(self.items.iter().enumerate()) } else { Box::new(self.items.iter().enumerate().rev()) };
        self.cursor = it.find(|(_, m)| m.selectable()).map(|(i, _)| i);
    }

    /// Activate the item at the cursor: its payload, or open its submenu.
    fn activate(&mut self, at_x: u16) -> MenuOutcome<T> {
        let Some(i) = self.cursor else { return MenuOutcome::Consumed };
        let Some(e) = self.items.get(i).and_then(MenuItem::entry).filter(|e| e.enabled) else {
            return MenuOutcome::Consumed;
        };
        match &e.kind {
            EntryKind::Run(t) => MenuOutcome::Activated(t.clone()),
            EntryKind::Submenu(items) => {
                let y = self.anchor.1 + 1 + (i.saturating_sub(self.top)) as u16;
                self.sub = Some(Box::new(Menu::new(items.clone(), (at_x, y))));
                MenuOutcome::Consumed
            }
        }
    }

    /// The accelerator of entry `e`: its own, else the first letter of the label.
    fn letter(e: &MenuEntry<T>) -> Option<char> {
        e.accel.or_else(|| e.label.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_lowercase().next().unwrap_or(c)))
    }

    fn by_letter(&mut self, c: char, at_x: u16) -> MenuOutcome<T> {
        let c = c.to_lowercase().next().unwrap_or(c);
        let matches: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, m)| m.entry().is_some_and(|e| e.enabled && Self::letter(e) == Some(c)))
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [] => MenuOutcome::Consumed,
            [one] => {
                self.cursor = Some(*one);
                self.activate(at_x)
            }
            many => {
                // The next match after the cursor, wrapping.
                let next = many.iter().copied().find(|&i| self.cursor.is_some_and(|c| i > c)).unwrap_or(many[0]);
                self.cursor = Some(next);
                MenuOutcome::Consumed
            }
        }
    }

    /// Apply one key event (see the module docs). `screen`: the area the menus fit into
    /// (submenus open beside their parent).
    pub fn apply(&mut self, ev: &InputEvent, screen: Rect) -> MenuOutcome<T> {
        let depth = self.depth();
        let rects = self.layout(screen);
        let at_x = rects.get(depth).map_or(0, |r| r.right());
        let Some(m) = self.level_mut(depth) else { return MenuOutcome::Ignored };
        match ev {
            InputEvent::Text(s) => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_control() && c != ' ' => m.by_letter(c, at_x),
                    _ => MenuOutcome::Ignored,
                }
            }
            InputEvent::Chord(c) => {
                if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) {
                    return MenuOutcome::Ignored;
                }
                match c.key {
                    KeyName::ArrowDown => m.step(true),
                    KeyName::ArrowUp => m.step(false),
                    KeyName::Tab if c.mods.contains(Mods::SHIFT) => m.step(false),
                    KeyName::Tab => m.step(true),
                    KeyName::Home | KeyName::PageUp => m.edge(true),
                    KeyName::End | KeyName::PageDown => m.edge(false),
                    KeyName::Enter | KeyName::Space => return m.activate(at_x),
                    KeyName::ArrowRight => {
                        let opens = m
                            .cursor
                            .and_then(|i| m.items.get(i))
                            .and_then(MenuItem::entry)
                            .is_some_and(|e| matches!(e.kind, EntryKind::Submenu(_)));
                        return if opens { m.activate(at_x) } else { MenuOutcome::Ignored };
                    }
                    KeyName::ArrowLeft | KeyName::Escape => {
                        if depth == 0 {
                            return if c.key == KeyName::Escape { MenuOutcome::Close } else { MenuOutcome::Ignored };
                        }
                        if let Some(parent) = self.level_mut(depth - 1) {
                            parent.sub = None;
                        }
                    }
                    // A typed letter arrives as text right after its chord.
                    _ => return MenuOutcome::Ignored,
                }
                MenuOutcome::Consumed
            }
            InputEvent::Other => MenuOutcome::Ignored,
        }
    }

    /// A click on item `index` of the menu at `depth` (`MenuItem` hit target).
    pub fn click(&mut self, depth: usize, index: usize, screen: Rect) -> MenuOutcome<T> {
        let rects = self.layout(screen);
        let at_x = rects.get(depth).map_or(0, |r| r.right());
        let Some(m) = self.level_mut(depth) else { return MenuOutcome::Ignored };
        if !m.items.get(index).is_some_and(MenuItem::selectable) {
            return MenuOutcome::Consumed;
        }
        m.cursor = Some(index);
        m.sub = None;
        m.activate(at_x)
    }

    /// Width of the box (borders included).
    fn width(&self) -> u16 {
        let checks = self.items.iter().any(|m| m.entry().is_some_and(|e| e.check.is_some()));
        let w = self
            .items
            .iter()
            .filter_map(MenuItem::entry)
            .map(|e| text::width(&e.label) + e.chord.as_deref().map_or(0, |c| text::width(c) + 2))
            .max()
            .unwrap_or(0);
        (w as u16).saturating_add(PAD + if checks { 2 } else { 0 })
    }

    /// The box of this menu and of each open submenu (index = depth), inside `screen`.
    pub fn layout(&self, screen: Rect) -> Vec<Rect> {
        let mut out = Vec::new();
        let mut m = Some(self);
        let mut anchor = self.anchor;
        let mut parent: Option<Rect> = None;
        while let Some(menu) = m {
            let w = menu.width().min(screen.width);
            let h = (menu.items.len() as u16).saturating_add(2).min(screen.height);
            let fits_right = anchor.0.saturating_add(w) <= screen.right();
            let x = match parent {
                // Submenus: right of the parent, else left of it.
                Some(p) if !fits_right => p.x.saturating_sub(w).max(screen.x),
                _ if !fits_right => screen.right().saturating_sub(w),
                _ => anchor.0.max(screen.x),
            };
            let y = if anchor.1.saturating_add(h) <= screen.bottom() {
                anchor.1.max(screen.y)
            } else if parent.is_none() && anchor.1 >= screen.y + h {
                // Above the anchor.
                anchor.1 - h
            } else {
                screen.bottom().saturating_sub(h).max(screen.y)
            };
            let r = Rect::new(x, y, w, h).intersection(screen);
            out.push(r);
            parent = Some(r);
            m = menu.sub.as_deref();
            if let Some(s) = m {
                anchor = (r.right(), s.anchor.1.max(r.y));
            }
        }
        out
    }

    /// Draw the menu and its open submenus over `screen`.
    pub fn draw(&mut self, f: &mut Frame<'_>, screen: Rect, theme: &TuiTheme, hits: &mut Hits) {
        let screen = screen.intersection(f.area());
        let rects = self.layout(screen);
        let mut m: Option<&mut Menu<T>> = Some(self);
        for (depth, r) in rects.into_iter().enumerate() {
            let Some(menu) = m else { break };
            menu.draw_one(f, r, depth, theme, hits);
            m = menu.sub.as_deref_mut();
        }
    }

    fn draw_one(&mut self, f: &mut Frame<'_>, r: Rect, depth: usize, theme: &TuiTheme, hits: &mut Hits) {
        if r.width < 3 || r.height < 3 {
            return;
        }
        f.render_widget(Clear, r);
        let block = borders::overlay_block(theme.borders(), theme.style(Role::Border));
        let inner = block.inner(r);
        f.render_widget(block, r);
        let rows = usize::from(inner.height);
        if let Some(c) = self.cursor {
            if c < self.top {
                self.top = c;
            } else if c >= self.top + rows {
                self.top = c + 1 - rows;
            }
        }
        self.top = self.top.min(self.items.len().saturating_sub(rows));
        let checks = self.items.iter().any(|m| m.entry().is_some_and(|e| e.check.is_some()));
        let w = usize::from(inner.width);
        let buf = f.buffer_mut();
        for (y, i) in (inner.y..inner.bottom()).zip(self.top..self.items.len()) {
            let row = Rect::new(inner.x, y, inner.width, 1);
            match &self.items[i] {
                MenuItem::Separator => {
                    let line = borders::lines(theme.borders()).horizontal.repeat(w);
                    buf.set_stringn(inner.x, y, line, w, theme.style(Role::Faint));
                }
                MenuItem::Entry(e) => {
                    let mut style = theme.style(if e.enabled { Role::Text } else { Role::Disabled });
                    if self.cursor == Some(i) {
                        style = style.patch(theme.style(Role::PaletteSelected));
                    }
                    buf.set_style(row, style);
                    let mut x = inner.x + 1;
                    if checks {
                        let mark = match e.check {
                            Some(true) => "✓",
                            _ => " ",
                        };
                        buf.set_stringn(x, y, mark, 1, style);
                        x += 2;
                    }
                    let arrow = matches!(e.kind, EntryKind::Submenu(_));
                    let chord = e.chord.as_deref().unwrap_or("");
                    let right_w = if arrow { 1 } else { text::width(chord) };
                    let used = usize::from(x - inner.x);
                    let label_w = w.saturating_sub(used + right_w + 2);
                    let label = text::truncate_end(&e.label, label_w);
                    buf.set_stringn(x, y, &label, label_w, style);
                    // Underline the accelerator in the label.
                    if let Some(a) = e.accel {
                        let mut cx = x;
                        for ch in label.chars() {
                            if ch.to_lowercase().next() == Some(a) {
                                if let Some(cell) = buf.cell_mut((cx, y)) {
                                    cell.modifier.insert(Modifier::UNDERLINED);
                                }
                                break;
                            }
                            cx += text::width(ch.encode_utf8(&mut [0; 4])) as u16;
                        }
                    }
                    if right_w > 0 && used + right_w + 1 < w {
                        let rx = inner.right() - 1 - right_w as u16;
                        let s = if arrow { "▸" } else { chord };
                        let rs = if arrow { style } else { style.patch(theme.style(Role::Muted)) };
                        buf.set_stringn(rx, y, s, right_w, rs);
                    }
                    hits.push(row, HitTarget::MenuItem { depth: depth as u8, index: i });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn txt(s: &str) -> InputEvent {
        InputEvent::Text(s.into())
    }

    fn sample() -> Menu<u32> {
        Menu::new(
            vec![
                MenuItem::run("Open", 1).chord("Enter").accel('o'),
                MenuItem::run("Disabled", 2).enabled(false),
                MenuItem::Separator,
                MenuItem::submenu("Sort by", vec![MenuItem::run("Name", 10), MenuItem::run("Size", 11).checked(true)]),
                MenuItem::run("Copy", 3).chord("Ctrl+C"),
                MenuItem::run("Cut", 4),
            ],
            (2, 1),
        )
    }

    const SCREEN: Rect = Rect::new(0, 0, 60, 20);

    #[test]
    fn keys_skip_disabled_and_separators() {
        let mut m = sample();
        assert_eq!(m.cursor, Some(0));
        assert_eq!(m.apply(&chord("Down"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.cursor, Some(3), "skips the disabled entry and the separator");
        m.apply(&chord("Up"), SCREEN);
        assert_eq!(m.cursor, Some(0));
        m.apply(&chord("Up"), SCREEN);
        assert_eq!(m.cursor, Some(5), "wraps");
        m.apply(&chord("Home"), SCREEN);
        assert_eq!(m.cursor, Some(0));
        assert_eq!(m.apply(&chord("Enter"), SCREEN), MenuOutcome::Activated(1));
        assert_eq!(m.apply(&chord("Ctrl+Q"), SCREEN), MenuOutcome::Ignored);
        assert_eq!(m.apply(&chord("Esc"), SCREEN), MenuOutcome::Close);
    }

    #[test]
    fn submenus_open_and_close() {
        let mut m = sample();
        m.apply(&chord("Down"), SCREEN);
        assert_eq!(m.apply(&chord("Right"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.depth(), 1);
        m.apply(&chord("Down"), SCREEN);
        assert_eq!(m.level(1).and_then(|s| s.cursor), Some(1));
        assert_eq!(m.apply(&chord("Enter"), SCREEN), MenuOutcome::Activated(11));
        assert_eq!(m.apply(&chord("Left"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.depth(), 0);
        assert_eq!(m.apply(&chord("Left"), SCREEN), MenuOutcome::Ignored);
        // Esc closes the submenu first.
        m.apply(&chord("Enter"), SCREEN);
        assert_eq!(m.depth(), 1);
        assert_eq!(m.apply(&chord("Esc"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.depth(), 0);
        let rects = m.clone().tap_open().layout(SCREEN);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[1].x, rects[0].right(), "the submenu opens to the right");
    }

    #[test]
    fn letters_pick_entries() {
        let mut m = sample();
        assert_eq!(m.apply(&txt("o"), SCREEN), MenuOutcome::Activated(1));
        // "Copy" and "Cut" share their first letter: cycle.
        assert_eq!(m.apply(&txt("c"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.cursor, Some(4));
        assert_eq!(m.apply(&txt("C"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.cursor, Some(5));
        assert_eq!(m.apply(&txt("c"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.cursor, Some(4));
        // Disabled entries never match; unknown letters do nothing.
        assert_eq!(m.apply(&txt("d"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.cursor, Some(4));
        assert_eq!(m.apply(&txt("s"), SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.depth(), 1, "a letter on a submenu opens it");
    }

    #[test]
    fn clicks_and_placement() {
        let mut m = sample();
        assert_eq!(m.click(0, 4, SCREEN), MenuOutcome::Activated(3));
        assert_eq!(m.click(0, 1, SCREEN), MenuOutcome::Consumed, "disabled");
        assert_eq!(m.click(0, 3, SCREEN), MenuOutcome::Consumed);
        assert_eq!(m.click(1, 0, SCREEN), MenuOutcome::Activated(10));
        // Near the bottom-right corner it flips up and left.
        let m = Menu::new(sample().items, (58, 18));
        let r = m.layout(SCREEN)[0];
        assert!(r.right() <= 60 && r.bottom() <= 18, "{r:?}");
        assert_eq!(r.height, 8);
    }

    #[test]
    fn draws_with_targets_and_never_panics() {
        let mut t = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut hits = Hits::default();
        let mut m = sample();
        t.draw(|f| m.draw(f, f.area(), &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer();
        let line = |y: u16| (0..40).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(line(2).contains("Open") && line(2).contains("Enter"), "{:?}", line(2));
        assert!(line(5).contains("Sort by") && line(5).contains('▸'), "{:?}", line(5));
        assert!(buf[(4, 2)].modifier.contains(Modifier::UNDERLINED), "the accelerator is underlined");
        assert_eq!(hits.at(5, 2), Some(HitTarget::MenuItem { depth: 0, index: 0 }));
        assert_eq!(hits.at(5, 4), None, "separators are no target");
        for w in 0..=20 {
            for h in 0..=6 {
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut m = sample();
                m.apply(&chord("Down"), Rect::new(0, 0, w, h));
                m.apply(&chord("Enter"), Rect::new(0, 0, w, h));
                t.draw(|f| m.draw(f, f.area(), &TuiTheme::plain(), &mut hits)).unwrap();
            }
        }
    }

    impl Menu<u32> {
        fn tap_open(mut self) -> Self {
            self.cursor = Some(3);
            let _ = self.activate(10);
            self
        }
    }
}
