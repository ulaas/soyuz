//! btop's boxes: a frame in the box's own hue with the title set into the top line,
//! `╭─┐CPU┌──────────┐28%┌─╮`, and an optional note in the bottom line.
//!
//! The lines follow `[tui] borders` (rounded, plain, ASCII). Without lines (`none`), in
//! screen-reader mode or when the area is too small for a frame, a box is its title row
//! (the note at the right) above the content.

use soyuz_core::config::TuiBorders;

use super::Cx;
use super::canvas::Area;
use crate::theme::{RAMP_STEPS, Ramp, Role};
use crate::view::text;

/// The lines of a box.
struct Lines {
    tl: &'static str,
    tr: &'static str,
    bl: &'static str,
    br: &'static str,
    h: &'static str,
    v: &'static str,
    /// Around a title in the top line: `┐` and `┌`.
    open: &'static str,
    close: &'static str,
    /// Around a note in the bottom line: `┘` and `└`.
    open_b: &'static str,
    close_b: &'static str,
}

const ROUNDED: Lines =
    Lines {
        tl: "╭", tr: "╮", bl: "╰", br: "╯", h: "─", v: "│", open: "┐", close: "┌", open_b: "┘", close_b: "└"
    };
const PLAIN: Lines =
    Lines {
        tl: "┌", tr: "┐", bl: "└", br: "┘", h: "─", v: "│", open: "┐", close: "┌", open_b: "┘", close_b: "└"
    };
const ASCII: Lines =
    Lines { tl: "+", tr: "+", bl: "+", br: "+", h: "-", v: "|", open: "[", close: "]", open_b: "[", close_b: "]" };

/// What a box says in its frame.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Frame<'a> {
    /// Set into the top line at the left.
    pub title: &'a str,
    /// Set into the top line at the right (a value).
    pub info: &'a str,
    /// Set into the bottom line at the right.
    pub foot: &'a str,
}

impl Cx<'_> {
    fn lines(&self) -> Option<&'static Lines> {
        let lines = match self.config.tui.borders {
            _ if self.look.sr => None,
            TuiBorders::Rounded => Some(&ROUNDED),
            TuiBorders::Plain => Some(&PLAIN),
            TuiBorders::Ascii => Some(&ASCII),
            TuiBorders::None => None,
        };
        if self.look.ascii && lines.is_some() { Some(&ASCII) } else { lines }
    }

    /// The rows a box's frame takes: two lines, or one title row without lines.
    pub(super) fn frame_rows(&self) -> u16 {
        if self.lines().is_some() { 2 } else { 1 }
    }

    /// A box over `a` in the hue `hue` (`Ramp::Hue`); returns the area inside it.
    pub(super) fn boxed(&mut self, a: Area, f: Frame<'_>, hue: Ramp) -> Area {
        match self.lines() {
            Some(l) if a.h >= 3 && a.w >= 8 => self.frame(a, f, hue, l),
            _ => self.title_row(a, f),
        }
    }

    /// The box as a title row (no lines).
    fn title_row(&mut self, a: Area, f: Frame<'_>) -> Area {
        if a.h == 0 || f.title.is_empty() && f.info.is_empty() {
            return a;
        }
        let iw = text::width(f.info) as u16;
        let tw = if iw > 0 && iw + 3 < a.w { a.w - iw - 2 } else { a.w };
        self.c.bold(a.x, a.y, tw, f.title, Role::GroupHeader);
        if tw < a.w && !self.look.sr {
            self.c.right(a.x, a.y, a.w, f.info, Role::Text);
        }
        a.below(1)
    }

    fn frame(&mut self, a: Area, f: Frame<'_>, hue: Ramp, l: &Lines) -> Area {
        let line = Role::Ramp(hue, RAMP_STEPS * 2 / 3);
        let (x0, x1) = (a.x, a.x + a.w - 1);
        let (top, bottom) = (a.y, a.y + i32::from(a.h) - 1);
        let rule = |n: u16| l.h.repeat(usize::from(n));
        // The top line: corner, a dash, the title, a run of dashes, the info, a dash, corner.
        self.c.text(x0, top, a.w, &format!("{}{}", l.tl, rule(a.w - 2)), line);
        self.c.text(x1, top, 1, l.tr, line);
        // Room inside the top line, after the corner and one dash on each side.
        let room = a.w.saturating_sub(4);
        let iw = text::width(f.info) as u16;
        let show_info = iw > 0 && iw + 2 + 8 <= room;
        let info_room = if show_info { iw + 3 } else { 0 };
        let title_w = (text::width(f.title) as u16).min(room.saturating_sub(info_room + 2));
        if !f.title.is_empty() && title_w > 0 {
            let x = x0 + 2;
            self.c.text(x, top, 1, l.open, line);
            self.c.bold(x + 1, top, title_w, f.title, Role::Title);
            self.c.text(x + 1 + title_w, top, 1, l.close, line);
        }
        if show_info {
            let x = x1 - 2 - iw;
            self.c.text(x - 1, top, 1, l.open, line);
            self.c.text(x, top, iw, f.info, Role::Text);
            self.c.text(x + iw, top, 1, l.close, line);
        }
        for y in top + 1..bottom {
            self.c.text(x0, y, 1, l.v, line);
            self.c.text(x1, y, 1, l.v, line);
        }
        self.c.text(x0, bottom, a.w, &format!("{}{}", l.bl, rule(a.w - 2)), line);
        self.c.text(x1, bottom, 1, l.br, line);
        // The note is cut with "…" to fit after the corner and two dashes.
        let fw = (text::width(f.foot) as u16).min(a.w.saturating_sub(6));
        if fw >= 4 {
            let x = x1 - 2 - fw;
            self.c.text(x - 1, bottom, 1, l.open_b, line);
            self.c.text(x, bottom, fw, f.foot, Role::Muted);
            self.c.text(x + fw, bottom, 1, l.close_b, line);
        }
        Area { x: a.x + 1, y: a.y + 1, w: a.w - 2, h: a.h - 2 }
    }
}

/// The hue of a page's boxes (`Ramp::Hue` order: accent, info, code, audio, image,
/// archive, success, warning), like btop's per-box colours.
pub(super) mod hue {
    use crate::theme::Ramp;

    pub const CPU: Ramp = Ramp::Hue(6);
    pub const MEMORY: Ramp = Ramp::Hue(7);
    pub const DISKS: Ramp = Ramp::Hue(2);
    pub const NETWORK: Ramp = Ramp::Hue(0);
    pub const GPU: Ramp = Ramp::Hue(4);
    pub const SWAP: Ramp = Ramp::Hue(5);
    pub const SENSORS: Ramp = Ramp::Hue(3);
    pub const PROCESSES: Ramp = Ramp::Hue(5);
    pub const LISTS: Ramp = Ramp::Hue(1);
}

#[cfg(test)]
mod tests {
    use soyuz_core::config::Config;

    use super::*;
    use crate::input::monitor::PaneNav;
    use crate::view::monitor::Look;
    use crate::view::monitor::canvas::{Canvas, Op};

    fn rows(borders: TuiBorders, sr: bool, w: u16, h: u16) -> (Vec<String>, Area) {
        let mut config = Config::default();
        config.tui.borders = borders;
        let mut nav = PaneNav::default();
        let mut c = Canvas::new(w, h);
        let mut cx = Cx {
            c: &mut c,
            config: &config,
            nav: &mut nav,
            look: Look { sr, ..Look::default() },
            focused: false,
            announce: String::new(),
            rows: 0,
            matches: 0,
            filter_error: None,
        };
        let f = Frame { title: "CPU", info: "28%", foot: "4 cores" };
        let inner = cx.boxed(Area { x: 0, y: 0, w, h }, f, hue::CPU);
        let mut grid = vec![vec![' '; usize::from(w)]; usize::from(h)];
        for op in &c.ops {
            if let Op::Text { x, y, text, .. } = op {
                for (i, ch) in text.chars().enumerate() {
                    grid[usize::from(*y)][usize::from(*x) + i] = ch;
                }
            }
        }
        (grid.into_iter().map(|r| r.into_iter().collect()).collect(), inner)
    }

    #[test]
    fn titles_sit_in_the_frame() {
        let (r, inner) = rows(TuiBorders::Rounded, false, 30, 4);
        assert_eq!(r[0], "╭─┐CPU┌────────────────┐28%┌─╮");
        assert_eq!(r[1], format!("│{}│", " ".repeat(28)));
        assert_eq!(r[3], "╰──────────────────┘4 cores└─╯");
        assert_eq!(inner, Area { x: 1, y: 1, w: 28, h: 2 });
        let (r, _) = rows(TuiBorders::Ascii, false, 30, 4);
        assert!(r[0].starts_with("+-[CPU]") && r[0].is_ascii(), "{}", r[0]);
    }

    #[test]
    fn without_lines_a_box_is_its_title_row() {
        for (b, sr) in [(TuiBorders::None, false), (TuiBorders::Rounded, true)] {
            let (r, inner) = rows(b, sr, 30, 4);
            assert!(r[0].starts_with("CPU"), "{}", r[0]);
            assert_eq!(inner, Area { x: 0, y: 1, w: 30, h: 3 });
        }
        // Too small for a frame.
        let (_, inner) = rows(TuiBorders::Rounded, false, 30, 2);
        assert_eq!(inner.y, 1);
    }
}
