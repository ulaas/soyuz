//! The tab strip and the chrome of multi-pane layouts (step T2.4).
//!
//! # Tab strip
//!
//! One row above the location line, shown while the window has more than one tab ([`shown`]):
//! a terminal row is worth more than the GUI's always-on strip, so a single tab keeps the
//! Phase T1 screen. In strip order:
//! - tab groups open with a chip in their tag colour (`Role::Tag(color)`, reversed) with the
//!   group's name; a collapsed group shows `name n` and hides its tabs (like the GUI, the
//!   active tab's group is never collapsed); grouped tabs carry the group's colour;
//! - each tab shows its focused pane's location title (`state::crumbs::location_title`),
//!   the active one in `Role::TabActive`, the others in `Role::TabInactive`. Pinned tabs
//!   are marked with [`PIN`] and kept short; unpinned tabs get a [`CLOSE`] mark while there
//!   is more than one tab;
//! - the new-tab target ` + ` follows the last tab.
//!
//! Titles shrink from [`MAX_TITLE`] down to [`MIN_TITLE`] cells to fit; when even that is
//! too wide the strip scrolls so the active tab stays visible, with `‹` / `›` where tabs
//! are cut off (clicking one selects the nearest hidden tab on that side).
//!
//! Click targets: [`HitTarget::Tab`], [`HitTarget::TabClose`], [`HitTarget::NewTab`].
//!
//! # Pane chrome
//!
//! [`draw_pane_chrome`] draws what `Regions` leaves between and above the panes of a dual
//! or quad layout: the splitters (`│` / `─`, `┼` where they cross, `Role::Border`,
//! [`HitTarget::Splitter`]) and one title line per pane with its location, the focused
//! pane's in `Role::BorderFocused` (bold), the others in `Role::Border`.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::state::crumbs::location_title;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::state::workspace::GroupId;

use super::text;
use crate::dialog::safe;
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};

/// Widest tab title (cells).
pub const MAX_TITLE: usize = 24;
/// Titles shrink down to this before the strip scrolls.
pub const MIN_TITLE: usize = 6;
/// Widest title of a pinned tab.
pub const PINNED_TITLE: usize = 8;
/// Marks a pinned tab.
pub const PIN: &str = "•";
/// The close mark of a tab.
pub const CLOSE: &str = "×";
/// The new-tab target.
pub const NEW_TAB: &str = " + ";
/// Tabs cut off on the left / right.
pub const MORE_LEFT: &str = "‹";
pub const MORE_RIGHT: &str = "›";

/// A tab in the strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabItem {
    /// Index in `Workspace::tabs`.
    pub index: usize,
    /// Its focused pane's location title.
    pub title: String,
    pub pinned: bool,
    /// Tag colour of its group.
    pub color: Option<u8>,
    /// Shows a close mark (unpinned, more than one tab).
    pub closable: bool,
}

/// The chip that opens a tab group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupChip {
    /// Name (plus the tab count when collapsed).
    pub label: String,
    /// Tag colour index.
    pub color: u8,
    pub collapsed: bool,
}

/// One entry of the strip, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StripItem {
    Group(GroupChip),
    Tab(TabItem),
}

/// The title line of a drawn pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneTitle {
    /// Index in `Tab::panes`.
    pub index: usize,
    pub id: PaneId,
    /// The pane's location (full path).
    pub title: String,
    pub focused: bool,
}

/// The tabs of the window and the panes of the active tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabsVm {
    pub items: Vec<StripItem>,
    /// Index of the active tab in `Workspace::tabs`.
    pub active: usize,
    /// The active tab's shown panes.
    pub panes: Vec<PaneTitle>,
}

impl TabsVm {
    pub fn from_core(core: &AppCore) -> TabsVm {
        let ws = &core.ws;
        let n = ws.tabs.len();
        let mut items = Vec::with_capacity(n + ws.groups.len());
        let mut prev: Option<GroupId> = None;
        for (i, t) in ws.tabs.iter().enumerate() {
            let group = t.group.and_then(|g| ws.group(g));
            if let Some(g) = group
                && prev != Some(g.id)
            {
                let label = chip_label(&g.name, g.collapsed, ws.group_tab_count(g.id));
                items.push(StripItem::Group(GroupChip { label: safe(&label), color: g.color, collapsed: g.collapsed }));
            }
            prev = group.map(|g| g.id);
            if group.is_some_and(|g| g.collapsed) && i != ws.active {
                continue;
            }
            items.push(StripItem::Tab(TabItem {
                index: i,
                title: safe(&location_title(&t.active().location)),
                pinned: t.pinned,
                color: group.map(|g| g.color),
                closable: n > 1 && !t.pinned,
            }));
        }
        let tab = ws.tab();
        let panes = if tab.shown() > 1 {
            tab.panes
                .iter()
                .take(tab.shown())
                .enumerate()
                .map(|(i, p)| PaneTitle {
                    index: i,
                    id: p.id,
                    title: safe(&p.location.display()),
                    focused: i == tab.focused,
                })
                .collect()
        } else {
            Vec::new()
        };
        TabsVm { items, active: ws.active, panes }
    }

    /// The tabs shown in the strip (not those of collapsed groups).
    pub fn tabs(&self) -> impl Iterator<Item = &TabItem> {
        self.items.iter().filter_map(|i| match i {
            StripItem::Tab(t) => Some(t),
            StripItem::Group(_) => None,
        })
    }
}

/// The label of a group chip (the GUI's rule): the name; collapsed, the name and the
/// number of tabs.
fn chip_label(name: &str, collapsed: bool, count: usize) -> String {
    let name = name.trim();
    match (collapsed, name.is_empty()) {
        (true, true) => format!("{count}"),
        (true, false) => format!("{name} {count}"),
        (false, _) => name.to_string(),
    }
}

/// Whether the tab strip takes a row (`layout::Facts::tab_strip`): more than one tab.
pub fn shown(core: &AppCore) -> bool {
    core.ws.tabs.len() > 1
}

/// A run of text in the strip.
struct Piece {
    text: String,
    style: Style,
    hit: Option<HitTarget>,
}

/// The pieces of `item` with titles cut to `cap` cells.
fn pieces(item: &StripItem, cap: usize, active: usize, theme: &TuiTheme) -> Vec<Piece> {
    match item {
        StripItem::Group(g) => {
            let style = theme.style(Role::Tag(g.color)).add_modifier(Modifier::REVERSED | Modifier::BOLD);
            vec![Piece { text: format!(" {} ", text::truncate_end(&g.label, cap)), style, hit: None }]
        }
        StripItem::Tab(t) => {
            let mut style = theme.style(if t.index == active { Role::TabActive } else { Role::TabInactive });
            if let Some(c) = t.color.filter(|_| t.index != active) {
                style.fg = theme.style(Role::Tag(c)).fg.or(style.fg);
            }
            let tab = Some(HitTarget::Tab(t.index));
            let title = if t.pinned {
                format!(" {PIN}{} ", text::truncate_end(&t.title, cap.min(PINNED_TITLE)))
            } else {
                format!(" {} ", text::truncate_end(&t.title, cap))
            };
            let mut out = vec![Piece { text: title, style, hit: tab }];
            if t.closable {
                out.push(Piece { text: CLOSE.into(), style, hit: Some(HitTarget::TabClose(t.index)) });
                out.push(Piece { text: " ".into(), style, hit: tab });
            }
            out
        }
    }
}

/// The first tab of `items`, as a click target.
fn nearest<'a>(mut items: impl Iterator<Item = &'a StripItem>) -> Option<HitTarget> {
    items.find_map(|i| match i {
        StripItem::Tab(t) => Some(HitTarget::Tab(t.index)),
        StripItem::Group(_) => None,
    })
}

fn width(p: &[Piece]) -> usize {
    p.iter().map(|p| text::width(&p.text)).sum()
}

/// Write `p` at `*x` on row `y`, stopping before `limit`; record its hit over what was written.
fn put(buf: &mut Buffer, x: &mut u16, y: u16, limit: u16, p: &Piece, hits: &mut Hits) {
    if *x >= limit {
        return;
    }
    let (nx, _) = buf.set_stringn(*x, y, &p.text, usize::from(limit - *x), p.style);
    if let Some(h) = p.hit {
        hits.push(Rect::new(*x, y, nx.saturating_sub(*x), 1), h);
    }
    *x = nx;
}

/// Draw the tab strip into `rect` (one row; nothing when empty).
pub fn draw_tabs(f: &mut Frame<'_>, rect: Rect, m: &TabsVm, theme: &TuiTheme, hits: &mut Hits) {
    if rect.is_empty() || m.items.is_empty() {
        return;
    }
    let buf = f.buffer_mut();
    let rect = rect.intersection(buf.area);
    if rect.is_empty() {
        return;
    }
    buf.set_style(rect, theme.style(Role::TabInactive));
    let w = usize::from(rect.width);
    let plus_w = text::width(NEW_TAB);
    let plus = Piece { text: NEW_TAB.into(), style: theme.style(Role::Accent), hit: Some(HitTarget::NewTab) };
    let (y, right) = (rect.y, rect.right());

    // Everything fits with some title width: draw it all.
    for cap in (MIN_TITLE..=MAX_TITLE).rev() {
        let all: Vec<Vec<Piece>> = m.items.iter().map(|i| pieces(i, cap, m.active, theme)).collect();
        if all.iter().map(|p| width(p)).sum::<usize>() + plus_w <= w {
            let mut x = rect.x;
            for p in all.iter().flatten() {
                put(buf, &mut x, y, right, p, hits);
            }
            put(buf, &mut x, y, right, &plus, hits);
            return;
        }
    }

    // Overflow: a window of items around the active tab, arrows for what is cut off.
    let all: Vec<Vec<Piece>> = m.items.iter().map(|i| pieces(i, MIN_TITLE, m.active, theme)).collect();
    let widths: Vec<usize> = all.iter().map(|p| width(p)).collect();
    let avail = w.saturating_sub(plus_w + 2);
    let a = m.items.iter().position(|i| matches!(i, StripItem::Tab(t) if t.index == m.active)).unwrap_or(0);
    let (mut lo, mut hi, mut used) = (a, a + 1, widths[a]);
    loop {
        let mut grew = false;
        if hi < widths.len() && used + widths[hi] <= avail {
            used += widths[hi];
            hi += 1;
            grew = true;
        }
        if lo > 0 && used + widths[lo - 1] <= avail {
            lo -= 1;
            used += widths[lo];
            grew = true;
        }
        if !grew {
            break;
        }
    }
    let faint = theme.style(Role::Muted);
    let mut x = rect.x;
    let left = nearest(m.items[..lo].iter().rev());
    let arrow = |text: &str, hit: Option<HitTarget>| Piece {
        text: if hit.is_some() { text.into() } else { " ".into() },
        style: faint,
        hit,
    };
    put(buf, &mut x, y, right, &arrow(MORE_LEFT, left), hits);
    // The items stop before the right arrow and the new-tab target.
    let stop = right.saturating_sub(plus_w as u16 + 1).max(x);
    for p in all[lo..hi].iter().flatten() {
        put(buf, &mut x, y, stop, p, hits);
    }
    let mut x = stop;
    let next = nearest(m.items[hi..].iter());
    put(buf, &mut x, y, right, &arrow(MORE_RIGHT, next), hits);
    put(buf, &mut x, y, right, &plus, hits);
}

/// Draw the splitters and pane title lines of `regions` (dual and quad layouts).
pub fn draw_pane_chrome(f: &mut Frame<'_>, regions: &Regions, m: &TabsVm, theme: &TuiTheme, hits: &mut Hits) {
    let buf = f.buffer_mut();
    let area = buf.area;
    let border = theme.style(Role::Border);
    for (r, vertical) in &regions.splitters {
        let r = r.intersection(area);
        let line = if *vertical { "│" } else { "─" };
        for p in r.positions() {
            buf[p].set_symbol(line).set_style(border);
        }
        hits.push(r, HitTarget::Splitter { vertical: *vertical });
    }
    // Where a vertical and a horizontal splitter cross.
    for (v, _) in regions.splitters.iter().filter(|s| s.1) {
        for (h, _) in regions.splitters.iter().filter(|s| !s.1) {
            let c = Position { x: v.x, y: h.y };
            if v.contains(c) && h.contains(c) && area.contains(c) {
                buf[c].set_symbol("┼");
            }
        }
    }
    for t in &regions.titles {
        let r = t.rect.intersection(area);
        if r.is_empty() {
            continue;
        }
        let pane = m.panes.iter().find(|p| p.index == t.index);
        let focused = pane.is_some_and(|p| p.focused);
        let style = theme.style(if focused { Role::BorderFocused } else { Role::Border });
        for p in r.positions() {
            buf[p].set_symbol("─").set_style(style);
        }
        if let Some(p) = pane {
            let w = usize::from(r.width);
            if w >= 5 {
                let title = format!(" {} ", text::truncate_middle(&p.title, w - 4));
                let text_style = if focused { style.add_modifier(Modifier::BOLD) } else { style };
                buf.set_stringn(r.x + 1, r.y, &title, w - 1, text_style);
            }
            hits.push(r, HitTarget::PaneBackground { pane: p.id });
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use soyuz_core::config::TuiSection;

    use super::*;
    use crate::layout::Facts;

    fn tab(index: usize, title: &str) -> StripItem {
        StripItem::Tab(TabItem { index, title: title.into(), pinned: false, color: None, closable: true })
    }

    fn strip(m: &TabsVm, w: u16) -> (String, Buffer, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, 1)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_tabs(f, Rect::new(0, 0, w, 1), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let line = (0..w).map(|x| buf[(x, 0)].symbol().to_string()).collect::<String>().trim_end().to_string();
        (line, buf, hits)
    }

    /// The column where `s` starts in `line` (in cells; the tests use one-cell glyphs).
    fn col(line: &str, s: &str) -> u16 {
        line[..line.find(s).unwrap_or_else(|| panic!("{s:?} in {line:?}"))].chars().count() as u16
    }

    #[test]
    fn tabs_close_marks_new_tab_and_active() {
        let m = TabsVm { items: vec![tab(0, "data"), tab(1, "docs")], active: 1, panes: Vec::new() };
        let (line, buf, hits) = strip(&m, 40);
        assert_eq!(line, " data ×  docs ×  +");
        // Only the active tab is reversed (the plain theme's TabActive).
        assert!(!buf[(1, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(9, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(hits.at(col(&line, "data"), 0), Some(HitTarget::Tab(0)));
        assert_eq!(hits.at(col(&line, "×"), 0), Some(HitTarget::TabClose(0)));
        assert_eq!(hits.at(col(&line, "docs") + 5, 0), Some(HitTarget::TabClose(1)));
        assert_eq!(hits.at(col(&line, "+"), 0), Some(HitTarget::NewTab));
        assert_eq!(hits.at(30, 0), None);
    }

    #[test]
    fn pinned_tabs_and_groups() {
        let mut pinned = tab(0, "pinned-folder-title");
        if let StripItem::Tab(t) = &mut pinned {
            t.pinned = true;
            t.closable = false;
        }
        let grouped =
            StripItem::Tab(TabItem { index: 1, title: "src".into(), pinned: false, color: Some(2), closable: true });
        let m = TabsVm {
            items: vec![
                pinned,
                StripItem::Group(GroupChip { label: "work".into(), color: 2, collapsed: false }),
                grouped,
                StripItem::Group(GroupChip { label: "old 3".into(), color: 5, collapsed: true }),
            ],
            active: 0,
            panes: Vec::new(),
        };
        let (line, buf, hits) = strip(&m, 60);
        assert_eq!(line, " •pinned-…  work  src ×  old 3  +");
        // The chips are reversed, in their tag colour, and not click targets of a tab.
        let chip = col(&line, "work");
        assert!(buf[(chip, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(hits.at(chip, 0), None);
        assert_eq!(hits.at(col(&line, "src"), 0), Some(HitTarget::Tab(1)));
        assert_eq!(hits.at(col(&line, "•"), 0), Some(HitTarget::Tab(0)));
        assert!(!hits.iter().any(|(_, t)| *t == HitTarget::TabClose(0)), "pinned tabs have no close mark");
    }

    #[test]
    fn titles_shrink_then_the_strip_scrolls_to_the_active_tab() {
        let items: Vec<StripItem> = (0..12).map(|i| tab(i, &format!("folder-number-{i:02}"))).collect();
        // Wide enough with shorter titles: everything shows, cut with an ellipsis.
        let m = TabsVm { items: items[..3].to_vec(), active: 0, panes: Vec::new() };
        let (line, _, _) = strip(&m, 60);
        assert!(!line.contains(MORE_LEFT) && !line.contains(MORE_RIGHT) && line.contains('…'), "{line}");
        assert_eq!(line.matches('×').count(), 3);
        // Twelve tabs in 60 cells: a window around the active one.
        for active in [0, 6, 11] {
            let m = TabsVm { items: items.clone(), active, panes: Vec::new() };
            let (line, _, hits) = strip(&m, 60);
            assert!(hits.iter().any(|(_, t)| *t == HitTarget::Tab(active)), "active {active} shown: {line}");
            assert_eq!(line.starts_with(MORE_LEFT), active > 0, "{line}");
            assert_eq!(line.contains(MORE_RIGHT), active < 11, "{line}");
            assert!(line.ends_with('+'), "{line}");
            // The arrows select the nearest hidden tab.
            if active == 6 {
                let Some(HitTarget::Tab(l)) = hits.at(0, 0) else { panic!("left arrow: {line}") };
                assert!(l < 6 && !hits.iter().any(|(r, t)| *t == HitTarget::Tab(l) && r.x > 0));
                let r = col(&line, MORE_RIGHT);
                let Some(HitTarget::Tab(n)) = hits.at(r, 0) else { panic!("right arrow: {line}") };
                assert!(n > 6);
            }
        }
    }

    #[test]
    fn tiny_strips_never_panic() {
        let items: Vec<StripItem> = (0..5).map(|i| tab(i, "日本語のフォルダ名")).collect();
        let m = TabsVm { items, active: 4, panes: Vec::new() };
        for w in 0..=30 {
            let (_, _, hits) = strip(&m, w);
            assert!(hits.iter().all(|(r, _)| r.right() <= w));
        }
        let (_, _, hits) = strip(&TabsVm::default(), 20);
        assert!(hits.is_empty());
    }

    fn chrome(facts: &Facts, m: &TabsVm, w: u16, h: u16) -> (Vec<String>, Buffer, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let regions = Regions::compute(Rect::new(0, 0, w, h), &TuiSection::default(), facts);
        let mut hits = Hits::default();
        t.draw(|f| draw_pane_chrome(f, &regions, m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect();
        (lines, buf, hits)
    }

    fn titles(n: usize, focused: usize) -> TabsVm {
        let panes = (0..n)
            .map(|i| PaneTitle { index: i, id: PaneId(10 + i as u64), title: format!("/p{i}"), focused: i == focused })
            .collect();
        TabsVm { panes, ..TabsVm::default() }
    }

    #[test]
    fn dual_titles_and_splitter() {
        let facts = Facts { shown: 2, focused: 1, ..Facts::default() };
        let (lines, buf, hits) = chrome(&facts, &titles(2, 1), 81, 8);
        // Row 0 is the location line, 7 the status line; the splitter runs down column 40.
        let dash = |n| "─".repeat(n);
        assert_eq!(lines[1], format!("─ /p0 {}│─ /p1 {}", dash(34), dash(34)));
        assert!(lines[2..7].iter().all(|l| l.chars().nth(40) == Some('│')), "{lines:?}");
        assert!(buf[(43, 1)].modifier.contains(Modifier::BOLD), "the focused pane's title is bold");
        assert!(!buf[(3, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(hits.at(40, 3), Some(HitTarget::Splitter { vertical: true }));
        assert_eq!(hits.at(60, 1), Some(HitTarget::PaneBackground { pane: PaneId(11) }));
    }

    #[test]
    fn quad_titles_and_crossing_splitters() {
        let facts = Facts { shown: 4, focused: 3, ..Facts::default() };
        let (lines, _, hits) = chrome(&facts, &titles(4, 3), 61, 12);
        let all = lines.join("\n");
        for p in ["/p0", "/p1", "/p2", "/p3"] {
            assert_eq!(all.matches(p).count(), 1, "{all}");
        }
        assert_eq!(all.matches('┼').count(), 1, "{all}");
        assert!(hits.iter().any(|(_, t)| *t == HitTarget::Splitter { vertical: false }));
        // Stacked into one column on a narrow terminal: three horizontal splitters.
        let (lines, _, _) = chrome(&facts, &titles(4, 3), 40, 20);
        assert_eq!(lines.join("\n").matches("/p").count(), 4, "{lines:?}");
        // Tiny: nothing breaks.
        for w in 0..=12 {
            for h in 0..=8 {
                let _ = chrome(&facts, &titles(4, 0), w, h);
            }
        }
    }
}
