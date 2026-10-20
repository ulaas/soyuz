//! The command palette overlay (step T2.7): the core's palette state
//! (`soyuz_appcore::palette`), drawn like the GUI's (`soyuz-app/src/ui/palette`) in cells.
//!
//! A bordered box centred over the panes, near the top: the input line (mode glyph, mode
//! chip for the prefixed and folder modes, the text with the terminal cursor, an `Esc`
//! hint), a separator, the ranked results in sections (headers, the icon tinted by the
//! item's [`Tint`], the title with its fuzzy matches in bold, the detail when there is
//! room, the tag and key chord right-aligned) and the key hints and the prefix legend on
//! the bottom border. The list scrolls so the selected row stays visible.
//!
//! What the view keeps between frames (the list's scroll and the input cursor, which the
//! `LineEdit` in `crate::input::palette` owns) is a [`Kept`] in the TUI's input state
//! (`crate::input::palette::State::view`): [`prepare`] scrolls it, the input route sets
//! the cursor, [`PaletteVm::from_core`] reads both.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::palette::query::Mode;
use soyuz_appcore::palette::{self, Item, Tint};
use soyuz_appcore::tr;
use unicode_segmentation::UnicodeSegmentation;

use super::text;
use crate::borders;
use crate::dialog::safe;
use crate::icons::{self, IconSet};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::widgets::LineEdit;

/// Most list lines the box shows (headers included).
pub const MAX_LIST_LINES: u16 = 18;
/// Widest box, in cells.
pub const MAX_WIDTH: u16 = 84;

/// State of the drawn palette that is not in the core.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    /// The input cursor (byte offset in `palette.input`), set by the input route (`None`:
    /// at the end).
    pub cursor: Option<usize>,
    /// First list line shown.
    pub top: usize,
}

/// One result row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    /// Index in `palette.results`.
    pub index: usize,
    pub icon: &'static str,
    /// The icon's colour role (from the item's tint).
    pub tint: Role,
    /// The title in runs, `true` for the fuzzy-matched ones.
    pub title: Vec<(String, bool)>,
    pub detail: String,
    pub tag: Option<String>,
    pub chord: Option<String>,
    pub selected: bool,
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteLine {
    /// A section header (already upper-case).
    Header(String),
    Item(PaletteRow),
}

/// The palette's input and results.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaletteVm {
    pub open: bool,
    /// The mode's glyph.
    pub glyph: &'static str,
    /// The mode chip (prefixed and folder modes; `None` for commands).
    pub chip: Option<String>,
    /// The input text, terminal-safe.
    pub input: String,
    /// The cursor as a byte offset in `input`.
    pub cursor: usize,
    /// Shown when the input is empty.
    pub placeholder: String,
    /// Sections and rows, in order.
    pub lines: Vec<PaletteLine>,
    /// First line shown.
    pub top: usize,
    /// Shown instead of the list when there are no results.
    pub empty: String,
    /// Key hints on the bottom border.
    pub hints: String,
    /// The prefix legend (`: path`, …) with the active one marked.
    pub legend: Vec<(String, bool)>,
}

impl PaletteVm {
    /// The model of the open palette; `kept`: what the view keeps between frames.
    pub fn from_core(core: &AppCore, kept: &Kept) -> PaletteVm {
        let p = &core.palette;
        if !p.open {
            return PaletteVm::default();
        }
        let set = IconSet::from_config(&core.config.tui);
        let mode = p.mode();
        let raw_cursor = match kept.cursor {
            Some(c) if !p.cursor_to_end && c <= p.input.len() && p.input.is_char_boundary(c) => c,
            _ => p.input.len(),
        };
        let lines = lines(&p.results, p.selected, set);
        let mut hints = format!("↑↓ {} · Enter {}", tr!("actions-palette-hint-move"), tr!("actions-palette-hint-run"));
        if matches!(mode, Mode::Path | Mode::Shell | Mode::Folders | Mode::Bookmarks) {
            hints.push_str(&format!(" · Tab {}", tr!("actions-palette-hint-complete")));
        }
        PaletteVm {
            open: true,
            glyph: icons::glyph(mode_icon(mode), set),
            chip: (mode != Mode::Commands).then(|| safe(mode.label())),
            input: safe(&p.input),
            cursor: safe(&p.input[..raw_cursor]).len(),
            placeholder: safe(mode.placeholder()),
            top: kept.top.min(lines.len().saturating_sub(1)),
            lines,
            empty: tr!("actions-palette-no-matches").to_string(),
            hints,
            legend: Mode::PREFIXED
                .iter()
                .map(|m| (format!("{} {}", m.prefix().unwrap_or(' '), m.legend()), *m == mode))
                .collect(),
        }
    }

    /// The row of result `index`.
    pub fn row(&self, index: usize) -> Option<&PaletteRow> {
        self.lines.iter().find_map(|l| match l {
            PaletteLine::Item(r) if r.index == index => Some(r),
            _ => None,
        })
    }
}

/// The input line's icon per mode (the GUI's).
fn mode_icon(mode: Mode) -> soyuz_appcore::icon::IconId {
    use soyuz_appcore::icon::IconId;
    match mode {
        Mode::Commands => IconId::Command,
        Mode::Folders | Mode::Path => IconId::FolderOpen,
        Mode::Bookmarks => IconId::BookmarkSimple,
        Mode::Search => IconId::MagnifyingGlass,
        Mode::Shell => IconId::Terminal,
        Mode::Tabs => IconId::Browsers,
        Mode::Clipboard => IconId::Clipboard,
        Mode::Ask => IconId::Sparkle,
    }
}

fn tint_role(t: Tint) -> Role {
    match t {
        Tint::Accent => Role::Accent,
        Tint::Folder => Role::Folder,
        Tint::Neutral => Role::Muted,
        Tint::Success => Role::Success,
        Tint::Warning => Role::Warning,
        Tint::Info => Role::Info,
    }
}

/// `title` in runs of matched / unmatched characters (`hl`: sorted char indices). Each
/// character is made terminal-safe on its own, so the indices stay valid.
fn title_runs(title: &str, hl: &[u32]) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for (i, c) in title.chars().enumerate() {
        let h = hl.binary_search(&(i as u32)).is_ok();
        let s = safe(c.encode_utf8(&mut [0; 4]));
        match out.last_mut() {
            Some((run, rh)) if *rh == h => run.push_str(&s),
            _ => out.push((s, h)),
        }
    }
    out
}

/// The list lines: a header before every section change, then the rows.
fn lines(results: &[Item], selected: usize, set: IconSet) -> Vec<PaletteLine> {
    let mut out = Vec::with_capacity(results.len() + 4);
    let mut last: Option<&str> = None;
    for (i, it) in results.iter().enumerate() {
        if last != Some(it.section) {
            out.push(PaletteLine::Header(safe(&palette::upper(it.section))));
            last = Some(it.section);
        }
        out.push(PaletteLine::Item(PaletteRow {
            index: i,
            icon: icons::glyph(it.icon, set),
            tint: tint_role(it.tint),
            title: title_runs(&it.title, &it.hl),
            detail: safe(&it.detail),
            tag: it.tag.as_deref().map(safe),
            chord: it.chord.as_deref().map(safe),
            selected: i == selected,
        }));
    }
    out
}

/// Number of list lines and the line of result `selected` (with its section's header
/// when it is the first of its section: that header is scrolled in with it).
fn line_of(results: &[Item], selected: usize) -> (usize, usize, bool) {
    let mut n = 0;
    let mut sel = (0, false);
    let mut last: Option<&str> = None;
    for (i, it) in results.iter().enumerate() {
        let first = last != Some(it.section);
        if first {
            n += 1;
            last = Some(it.section);
        }
        if i == selected {
            sel = (n, first);
        }
        n += 1;
    }
    (n, sel.0, sel.1)
}

/// Where the palette goes in `area`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Geom {
    /// The whole box (cleared).
    outer: Rect,
    bordered: bool,
    /// The input line.
    input: Rect,
    /// The list (or the "no matches" line).
    list: Rect,
}

/// The box for `n_lines` list lines.
fn geometry(area: Rect, n_lines: usize) -> Geom {
    geometry_in(area, n_lines, false)
}

/// [`geometry`], as wide as `area` when `full` (screen-reader mode: no text of the screen
/// below shares a row with the palette's).
fn geometry_in(area: Rect, n_lines: usize, full: bool) -> Geom {
    if area.is_empty() {
        return Geom::default();
    }
    let bordered = area.width >= 24 && area.height >= 6;
    let (chrome, margin) = if bordered { (4, if area.width >= 50 { 2 } else { 0 }) } else { (1, 0) };
    let width = if bordered && !full { (area.width - 2 * margin).min(MAX_WIDTH) } else { area.width };
    let space = area.height.saturating_sub(chrome);
    let offset = if bordered { (area.height * 13 / 100).min(space.saturating_sub(1)) } else { 0 };
    let wanted = (n_lines.max(1)).min(usize::from(MAX_LIST_LINES)) as u16;
    let list_h = wanted.min(space - offset);
    let outer = Rect { x: area.x + (area.width - width) / 2, y: area.y + offset, width, height: chrome + list_h };
    let inner = if bordered { Block::new().borders(Borders::ALL).inner(outer) } else { outer };
    let input = Rect { height: inner.height.min(1), ..inner };
    let skip = if bordered { 2 } else { 1 };
    let list = Rect { y: inner.y + skip.min(inner.height), height: inner.height.saturating_sub(skip), ..inner };
    Geom { outer, bordered, input, list }
}

/// Before the model is built: refresh the results (the palette may have been opened by
/// this frame's command) and scroll the list so the selection stays visible.
pub fn prepare(core: &mut AppCore, kept: &mut Kept, regions: &Regions) {
    if !core.palette.open {
        *kept = Kept::default();
        return;
    }
    palette::refresh_results(core);
    core.palette.scroll_to_selected = false;
    let (n, sel, first) = line_of(&core.palette.results, core.palette.selected);
    let h = usize::from(geometry(regions.area, n).list.height).max(1);
    let mut top = kept.top;
    let want = if first { sel.saturating_sub(1) } else { sel };
    if want < top {
        top = want;
    } else if sel >= top + h {
        top = sel + 1 - h;
    }
    kept.top = top.min(n.saturating_sub(h));
}

/// `runs` fitted into `max` cells (cut at the end with an ellipsis), with the width used.
fn fit_runs(runs: &[(String, bool)], max: usize, normal: Style, strong: Style) -> (Vec<Span<'static>>, usize) {
    let total: usize = runs.iter().map(|(s, _)| text::width(s)).sum();
    let (limit, cut) = if total <= max { (max, false) } else { (max.saturating_sub(1), true) };
    let mut out = Vec::new();
    let mut used = 0;
    'runs: for (s, h) in runs {
        let mut part = String::new();
        for g in s.graphemes(true) {
            let w = text::width(g);
            if used + w > limit {
                if !part.is_empty() {
                    out.push(Span::styled(part, if *h { strong } else { normal }));
                }
                break 'runs;
            }
            used += w;
            part.push_str(g);
        }
        if !part.is_empty() {
            out.push(Span::styled(part, if *h { strong } else { normal }));
        }
    }
    if cut && max > 0 {
        out.push(Span::styled(text::ELLIPSIS, normal));
        used += 1;
    }
    (out, used)
}

/// One result row, `w` cells wide.
fn row_line(r: &PaletteRow, w: usize, theme: &TuiTheme) -> Line<'static> {
    let base = if r.selected { theme.style(Role::PaletteSelected) } else { theme.style(Role::Text) };
    // Screen-reader mode (T8): no icons.
    let icon = if theme.screen_reader() { " ".to_string() } else { format!(" {} ", r.icon) };
    let left_w = text::width(&icon);
    // The right side drops the tag first, then the chord, to leave the title room.
    let mut tag = r.tag.clone();
    let mut chord = r.chord.clone();
    let right_w = |tag: &Option<String>, chord: &Option<String>| {
        tag.as_deref().map_or(0, |t| text::width(t) + 2) + chord.as_deref().map_or(0, |c| text::width(c) + 2)
    };
    if w < left_w + right_w(&tag, &chord) + 12 {
        tag = None;
    }
    if w < left_w + right_w(&tag, &chord) + 12 {
        chord = None;
    }
    let rw = right_w(&tag, &chord);
    let room = w.saturating_sub(left_w + rw);
    let mut spans = vec![Span::styled(icon, base.patch(theme.style(r.tint)))];
    let strong = base.patch(theme.style(Role::Accent)).add_modifier(Modifier::BOLD);
    let (title, mut used) = fit_runs(&r.title, room, base, strong);
    spans.extend(title);
    if !r.detail.is_empty() && room > used + 8 {
        let d = text::truncate_end(&r.detail, room - used - 2);
        used += 2 + text::width(&d);
        spans.push(Span::styled(format!("  {d}"), base.patch(theme.style(Role::Muted))));
    }
    spans.push(Span::styled(" ".repeat(room.saturating_sub(used)), base));
    if let Some(t) = tag {
        spans.push(Span::styled(format!("{t}  "), base.patch(theme.style(Role::Muted))));
    }
    if let Some(c) = chord {
        spans.push(Span::styled(c, base.patch(theme.style(Role::KeyHint))));
        spans.push(Span::styled("  ", base));
    }
    Line::from(spans).style(base)
}

/// Draw the palette over `area` (the whole screen) when it is open.
pub fn draw_palette(f: &mut Frame<'_>, area: Rect, m: &PaletteVm, theme: &TuiTheme, hits: &mut Hits) {
    if !m.open {
        return;
    }
    let g = geometry_in(area, m.lines.len(), theme.screen_reader());
    if g.outer.is_empty() {
        return;
    }
    f.render_widget(Clear, g.outer);
    if g.bordered {
        let border = theme.style(Role::BorderFocused);
        let mut block = borders::overlay_block(theme.borders(), border);
        let inner_w = usize::from(g.outer.width.saturating_sub(2));
        let hints = format!(" {} ", m.hints);
        let legend: Vec<Span<'_>> = m
            .legend
            .iter()
            .flat_map(|(s, active)| {
                let style = if *active {
                    theme.style(Role::Accent).add_modifier(Modifier::BOLD)
                } else {
                    theme.style(Role::Muted)
                };
                [Span::styled(s.as_str(), style), Span::raw(" ")]
            })
            .collect();
        let legend_w: usize = legend.iter().map(|s| text::width(&s.content)).sum();
        if text::width(&hints) + 2 <= inner_w {
            block = block.title_bottom(Line::from(Span::styled(hints.clone(), theme.style(Role::KeyHint))));
            if text::width(&hints) + legend_w + 3 <= inner_w {
                let mut l = vec![Span::raw(" ")];
                l.extend(legend);
                block = block.title_bottom(Line::from(l).right_aligned());
            }
        }
        f.render_widget(block, g.outer);
        // The separator under the input, joined to the border.
        if g.outer.height > 3 {
            let y = g.input.y + 1;
            let l = borders::lines(theme.borders());
            let sep = format!("{}{}{}", l.tee_left, l.horizontal.repeat(inner_w), l.tee_right);
            f.render_widget(
                Paragraph::new(sep).style(border),
                Rect { x: g.outer.x, y, width: g.outer.width, height: 1 },
            );
        }
    }
    draw_input(f, g.input, m, theme, hits);
    draw_list(f, g.list, m, theme, hits);
}

fn draw_input(f: &mut Frame<'_>, r: Rect, m: &PaletteVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    hits.push(r, HitTarget::PaletteInput);
    let w = usize::from(r.width);
    // Screen-reader mode (T8): no mode glyph (the chip names the mode).
    let lead = if theme.screen_reader() { " ".to_string() } else { format!(" {} ", m.glyph) };
    let mut spans = vec![Span::styled(lead, theme.style(Role::Accent))];
    let mut used = text::width(&spans[0].content);
    if let Some(chip) = &m.chip
        && used + text::width(chip) + 3 < w
    {
        let s = format!("[{chip}]");
        used += text::width(&s) + 1;
        spans.push(Span::styled(s, theme.style(Role::Accent).add_modifier(Modifier::BOLD)));
        spans.push(Span::raw(" "));
    }
    let esc = " Esc ";
    let esc_w = if w >= used + text::width(esc) + 8 { text::width(esc) } else { 0 };
    let field = w.saturating_sub(used + esc_w);
    let field_x = r.x + used as u16;
    let mut caret = None;
    if m.input.is_empty() {
        let ph = text::truncate_end(&m.placeholder, field.saturating_sub(1));
        spans.push(Span::styled(format!("{ph:<field$}", field = field), theme.style(Role::Faint)));
        caret = Some(0);
    } else if field > 0 {
        let mut edit = LineEdit::new();
        edit.set_cursor(&m.input, m.cursor);
        let (shown, col) = edit.view(&m.input, field as u16);
        let pad = field.saturating_sub(text::width(&shown));
        spans.push(Span::styled(shown, theme.style(Role::Input)));
        spans.push(Span::styled(" ".repeat(pad), theme.style(Role::Input)));
        caret = Some(col);
    }
    if esc_w > 0 {
        spans.push(Span::styled(esc, theme.style(Role::KeyHint)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), r);
    if let Some(col) = caret
        && field > 0
    {
        let at = Position { x: field_x + col.min(field as u16 - 1), y: r.y };
        f.set_cursor_position(at);
        hits.set_caret(at);
    }
}

fn draw_list(f: &mut Frame<'_>, r: Rect, m: &PaletteVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let w = usize::from(r.width);
    if m.lines.is_empty() {
        let s = text::truncate_end(&m.empty, w);
        let line = Line::from(Span::styled(s, theme.style(Role::Muted))).centered();
        f.render_widget(Paragraph::new(line), Rect { height: 1, ..r });
        return;
    }
    for (i, l) in m.lines.iter().skip(m.top).take(usize::from(r.height)).enumerate() {
        let lr = Rect { y: r.y + i as u16, height: 1, ..r };
        match l {
            PaletteLine::Header(h) => {
                let s = format!(" {}", text::truncate_end(h, w.saturating_sub(1)));
                f.render_widget(Paragraph::new(Span::styled(s, theme.style(Role::GroupHeader))), lr);
            }
            PaletteLine::Item(row) => {
                f.render_widget(Paragraph::new(row_line(row, w, theme)), lr);
                hits.push(lr, HitTarget::PaletteRow(row.index));
                // The highlighted result wins over the query's caret (screen-reader mode).
                if row.selected {
                    hits.set_caret(Position { x: lr.x, y: lr.y });
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

    fn row(index: usize, title: &str, hl: &[u32], selected: bool) -> PaletteLine {
        PaletteLine::Item(PaletteRow {
            index,
            icon: "*",
            tint: Role::Accent,
            title: title_runs(title, hl),
            detail: "Show or hide hidden files".into(),
            tag: Some("View".into()),
            chord: Some("Ctrl+H".into()),
            selected,
        })
    }

    fn model() -> PaletteVm {
        PaletteVm {
            open: true,
            glyph: "*",
            chip: None,
            input: "hid".into(),
            cursor: 3,
            placeholder: "Type a command".into(),
            lines: vec![
                PaletteLine::Header("VIEW".into()),
                row(0, "Toggle hidden files", &[7, 8, 9], true),
                row(1, "Hide sidebar", &[0, 1, 2], false),
                PaletteLine::Header("FILE".into()),
                row(2, "Hidden attribute", &[0, 1, 2], false),
            ],
            top: 0,
            empty: "No matches".into(),
            hints: "↑↓ move · Enter run".into(),
            legend: vec![(": path".into(), false), ("@ bookmarks".into(), false)],
        }
    }

    fn draw(m: &PaletteVm, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer, Hits, Option<Position>) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_palette(f, f.area(), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let cursor = t.get_cursor_position().ok();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, buf, hits, Some(cursor.unwrap_or_default()))
    }

    #[test]
    fn draws_input_sections_rows_and_hits() {
        let m = model();
        let (lines, buf, hits, cursor) = draw(&m, 80, 20);
        let all = lines.join("\n");
        // 80 columns: a 76-column box at x = 2, 13 % down (row 2).
        assert!(lines[2].starts_with("  ╭"), "{all}");
        assert!(lines[3].contains(" * hid") && lines[3].trim_end().ends_with("Esc │"), "{all}");
        assert!(lines[4].starts_with("  ├─"), "{all}");
        assert!(lines[5].contains(" VIEW"), "{all}");
        let toggle = &lines[6];
        assert!(
            toggle.contains("Toggle hidden files") && toggle.contains("View") && toggle.contains("Ctrl+H"),
            "{all}"
        );
        // The chord is right-aligned against the border.
        assert!(toggle.trim_end().ends_with("Ctrl+H  │"), "{toggle:?}");
        assert!(lines[8].contains(" FILE"), "{all}");
        assert!(lines[10].contains("↑↓ move · Enter run") && lines[10].contains("@ bookmarks"), "{all}");
        // The selected row is reversed; matches are bold.
        let x = lines[6].find("Toggle").map(|b| lines[6][..b].chars().count()).unwrap() as u16;
        assert!(buf[(x, 6)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(x, 6)].modifier.contains(Modifier::BOLD));
        assert!(buf[(x + 7, 6)].modifier.contains(Modifier::BOLD), "the 'h' of hidden matched");
        assert!(!buf[(x, 7)].modifier.contains(Modifier::REVERSED));
        // Click targets.
        assert_eq!(hits.at(10, 3), Some(HitTarget::PaletteInput));
        assert_eq!(hits.at(10, 6), Some(HitTarget::PaletteRow(0)));
        assert_eq!(hits.at(10, 7), Some(HitTarget::PaletteRow(1)));
        assert_eq!(hits.at(10, 9), Some(HitTarget::PaletteRow(2)));
        assert_eq!(hits.at(10, 5), None, "headers are not rows");
        // The terminal cursor sits after "hid".
        let col = lines[3].find("hid").map(|b| lines[3][..b].chars().count()).unwrap() as u16;
        assert_eq!(cursor, Some(Position { x: col + 3, y: 3 }));
    }

    #[test]
    fn mode_chip_placeholder_and_no_matches() {
        let mut m = model();
        m.chip = Some("Path".into());
        m.input.clear();
        m.cursor = 0;
        m.lines.clear();
        let (lines, _, _, _) = draw(&m, 60, 12);
        let all = lines.join("\n");
        assert!(all.contains("[Path] Type a command"), "{all}");
        assert!(all.contains("No matches"), "{all}");
    }

    #[test]
    fn scrolled_list_and_narrow_rows() {
        let mut m = model();
        m.top = 3;
        let (lines, _, hits, _) = draw(&m, 30, 12);
        let all = lines.join("\n");
        assert!(!all.contains("VIEW") && all.contains("FILE"), "{all}");
        // Narrow: the tag goes first.
        assert!(!all.contains("View") && all.contains("Hidden attri"), "{all}");
        assert!(hits.iter().all(|(_, t)| *t != HitTarget::PaletteRow(0)));
    }

    #[test]
    fn fit_runs_cuts_with_an_ellipsis() {
        let runs = title_runs("日本語 files", &[0, 4]);
        let (spans, used) = fit_runs(&runs, 6, Style::new(), Style::new().add_modifier(Modifier::BOLD));
        let s: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!((s.as_str(), used), ("日本…", 5));
        let (spans, used) = fit_runs(&runs, 40, Style::new(), Style::new());
        assert_eq!(used, 12);
        assert_eq!(spans.len(), 4);
        // Control characters never reach the terminal, and indices stay aligned.
        let runs = title_runs("a\u{1b}b", &[2]);
        assert_eq!(runs.last().map(|(s, h)| (s.as_str(), *h)), Some(("b", true)));
    }

    #[test]
    fn line_of_counts_headers() {
        assert_eq!(line_of(&[], 0), (0, 0, false));
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let mut m = model();
        for top in [0, 4] {
            m.top = top;
            for w in 0..=30 {
                for h in 0..=8 {
                    let _ = draw(&m, w, h);
                }
            }
        }
    }
}
