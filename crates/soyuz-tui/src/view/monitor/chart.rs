//! The monitor's charts: area charts, one-row sparklines and bars, in three styles.
//!
//! - Braille (the default): two by four dots per cell, an area with a faint band up to the
//!   peak of each decimated point.
//! - Blocks (`TERM=linux`, whose console font has no braille): eighth blocks, one value per
//!   cell.
//! - ASCII (`[tui] icons = "ascii"`): `#` for the area, `.` for a part of a cell.
//!
//! A chart has one y axis and at most two series. The first is an area, the second a line,
//! so the two differ in shape as well as hue and stay apart in 16 colours and in mono; the
//! legend names both next to their marks. A series is cut at `Pt::is_break`.

use std::sync::OnceLock;

use soyuz_sysmon::Pt;

use super::canvas::{Area, Canvas};
use crate::icons::IconSet;
use crate::theme::Role;
use crate::view::text;

/// How charts are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChartStyle {
    #[default]
    Braille,
    Blocks,
    Ascii,
}

impl ChartStyle {
    /// The style for the icon set; a Linux console gets blocks.
    pub fn of(icons: IconSet) -> ChartStyle {
        static CONSOLE: OnceLock<bool> = OnceLock::new();
        let console = *CONSOLE.get_or_init(|| std::env::var_os("TERM").is_some_and(|t| t == "linux"));
        match icons {
            IconSet::Ascii => ChartStyle::Ascii,
            _ if console => ChartStyle::Blocks,
            _ => ChartStyle::Braille,
        }
    }
}

/// The hues of the entities a page shows, in fixed order.
pub const HUES: [Role; 6] = [Role::Accent, Role::Info, Role::Code, Role::Audio, Role::Image, Role::Archive];

/// One series of a chart.
pub struct Series<'a> {
    pub label: String,
    pub pts: &'a [Pt],
    pub role: Role,
}

/// A chart.
pub struct Chart<'a> {
    pub series: Vec<Series<'a>>,
    pub from: i64,
    pub to: i64,
    /// The top of the y axis (`None`: from the data).
    pub y_max: Option<f32>,
    pub fmt: &'a dyn Fn(f32) -> String,
    /// The moment to mark.
    pub cursor: Option<i64>,
    /// Draw the legend row (from three rows up).
    pub legend: bool,
    /// The values are bytes: the axis rounds in powers of 1024.
    pub bytes: bool,
}

/// A point on the time axis: `(avg, max)`.
type Sample = Option<(f32, f32)>;

/// `pts` in `cols` columns between `from` and `to`: columns with several points are averaged
/// (and keep their peak), columns between two points hold the earlier one, a break leaves a gap.
pub fn resample(pts: &[Pt], from: i64, to: i64, cols: usize) -> Vec<Sample> {
    let mut out: Vec<Sample> = vec![None; cols];
    if cols == 0 || to <= from {
        return out;
    }
    let mut sum = vec![(0.0f32, 0u32, 0.0f32); cols];
    let span = i128::from(to - from);
    let mut last: Option<(usize, f32)> = None;
    for p in pts {
        if p.is_break() {
            last = None;
            continue;
        }
        if p.t < from || p.t > to {
            continue;
        }
        let col = (i128::from(p.t - from) * (cols as i128 - 1) / span) as usize;
        let s = &mut sum[col];
        s.0 += p.avg;
        s.1 += 1;
        s.2 = s.2.max(p.max.max(p.avg));
        if let Some((lc, lv)) = last {
            for slot in out.iter_mut().take(col).skip(lc + 1) {
                *slot = Some((lv, lv));
            }
        }
        last = Some((col, p.avg));
    }
    for (o, s) in out.iter_mut().zip(&sum) {
        if s.1 > 0 {
            *o = Some((s.0 / s.1 as f32, s.2));
        }
    }
    out
}

/// A round number at or above `v`.
pub fn nice_ceil(v: f32) -> f32 {
    if !(v.is_finite() && v > 0.0) {
        return 1.0;
    }
    let step = 10f32.powf(v.log10().floor());
    let m = v / step;
    let n = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    n * step
}

/// A round number of bytes at or above `v` (`512 KiB`, not `488 KiB`).
pub fn nice_bytes(v: f32) -> f32 {
    if !(v.is_finite() && v > 0.0) {
        return 1024.0;
    }
    let mut unit = 1.0f32;
    while v / unit >= 1024.0 {
        unit *= 1024.0;
    }
    nice_ceil(v / unit) * unit
}

/// The last value of `pts`.
pub fn latest(pts: &[Pt]) -> Option<f32> {
    pts.iter().rev().find(|p| !p.is_break()).map(|p| p.avg)
}

fn peak(pts: &[Pt]) -> f32 {
    pts.iter().filter(|p| !p.is_break()).map(|p| p.max.max(p.avg)).fold(0.0, f32::max)
}

const BRAILLE_L: [u8; 4] = [0x40, 0x04, 0x02, 0x01];
const BRAILLE_R: [u8; 4] = [0x80, 0x20, 0x10, 0x08];
const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Dots `level` rows high: `fill` from the bottom.
fn dots(col: &[u8; 4], fill: i32) -> u8 {
    (0..fill.clamp(0, 4) as usize).fold(0, |b, i| b | col[i])
}

/// The one dot of a line at `level` in this cell (`base`: the dots below the cell).
fn line_dot(col: &[u8; 4], level: i32, base: i32) -> u8 {
    let d = level - 1 - base;
    if level > 0 && (0..4).contains(&d) { col[d as usize] } else { 0 }
}

/// Height in sub-rows of `v` on an axis up to `y_max` with `rows` sub-rows; something above
/// zero shows at least one.
fn level(v: f32, y_max: f32, rows: i32) -> i32 {
    if v <= 0.0 || !v.is_finite() {
        return 0;
    }
    ((v / y_max * rows as f32).round() as i32).clamp(1, rows)
}

/// Draw `chart` into `a`: a legend row (from three rows up), then the y axis and the plot.
pub fn draw(c: &mut Canvas, mut a: Area, chart: &Chart<'_>, style: ChartStyle) {
    if a.w == 0 || a.h == 0 {
        return;
    }
    if a.h >= 3 && chart.legend {
        legend(c, a.take_top(1), chart, style);
    }
    let top = chart.y_max.unwrap_or_else(|| {
        let max = chart.series.iter().map(|s| peak(s.pts)).fold(0.0, f32::max) * 1.05;
        if chart.bytes { nice_bytes(max) } else { nice_ceil(max) }
    });
    let top = if top > 0.0 { top } else { 1.0 };
    let labels: Vec<(u16, String)> = {
        let mut l = vec![(0, (chart.fmt)(top)), (a.h.saturating_sub(1), (chart.fmt)(0.0))];
        if a.h >= 5 {
            l.push(((a.h - 1) / 2, (chart.fmt)(top / 2.0)));
        }
        l
    };
    let lw = labels.iter().map(|(_, s)| text::width(s) as u16).max().unwrap_or(0);
    let (px, pw) = if a.h >= 2 && a.w >= lw + 8 {
        for (row, s) in &labels {
            c.right(a.x, a.y + i32::from(*row), lw, s, Role::Muted);
        }
        let rule = if style == ChartStyle::Ascii { "|" } else { "│" };
        for r in 0..a.h {
            c.text(a.x + lw, a.y + i32::from(r), 1, rule, Role::Faint);
        }
        (a.x + lw + 1, a.w - lw - 1)
    } else {
        (a.x, a.w)
    };
    plot(c, Area { x: px, w: pw, ..a }, chart, top, style);
    if let Some(t) = chart.cursor
        && t >= chart.from
        && t <= chart.to
        && chart.to > chart.from
        && pw > 0
    {
        let col = i128::from(t - chart.from) * i128::from(pw - 1) / i128::from(chart.to - chart.from);
        c.paint(Area { x: px + col as u16, w: 1, ..a }, Role::Highlight);
    }
}

/// `■ CPU 12%   · System 3%`: the mark says area or line, the name says which.
fn legend(c: &mut Canvas, a: Area, chart: &Chart<'_>, style: ChartStyle) {
    let mut x = a.x;
    let end = a.x.saturating_add(a.w);
    for (i, s) in chart.series.iter().take(2).enumerate() {
        let mark = match (i, style) {
            (0, ChartStyle::Ascii) => "#",
            (_, ChartStyle::Ascii) => "*",
            (0, _) => "■",
            _ => "·",
        };
        let value = latest(s.pts).map(chart.fmt).unwrap_or_else(|| "–".into());
        let row = Area { x, w: end.saturating_sub(x), ..a };
        x = c.spans(
            row,
            &[
                (mark, s.role),
                (" ", Role::Text),
                (&s.label, Role::Muted),
                (" ", Role::Text),
                (&value, Role::Text),
                ("   ", Role::Text),
            ],
        );
    }
}

fn plot(c: &mut Canvas, a: Area, chart: &Chart<'_>, top: f32, style: ChartStyle) {
    let (w, h) = (usize::from(a.w), i32::from(a.h));
    if w == 0 || h == 0 {
        return;
    }
    let sub = if style == ChartStyle::Braille { 2 } else { 1 };
    let rows = h * if style == ChartStyle::Braille { 4 } else { 8 };
    let main = chart.series.first();
    let second = chart.series.get(1);
    let main_s = main.map_or_else(Vec::new, |s| resample(s.pts, chart.from, chart.to, w * sub));
    let second_s = second.map_or_else(Vec::new, |s| resample(s.pts, chart.from, chart.to, w * sub));
    let main_role = main.map_or(Role::Accent, |s| s.role);
    let second_role = second.map_or(Role::Info, |s| s.role);
    let at = |v: &[Sample], i: usize, f: fn((f32, f32)) -> f32| {
        v.get(i).copied().flatten().map_or(0, |s| level(f(s), top, rows))
    };
    for r in 0..h {
        let base = (h - 1 - r) * if style == ChartStyle::Braille { 4 } else { 8 };
        let mut runs: Vec<(String, Role)> = Vec::new();
        for x in 0..w {
            let (ch, role) = match style {
                ChartStyle::Braille => {
                    let (l, r2) = (x * 2, x * 2 + 1);
                    let fill_bits = dots(&BRAILLE_L, at(&main_s, l, |s| s.0) - base)
                        | dots(&BRAILLE_R, at(&main_s, r2, |s| s.0) - base);
                    let band_bits = dots(&BRAILLE_L, at(&main_s, l, |s| s.1) - base)
                        | dots(&BRAILLE_R, at(&main_s, r2, |s| s.1) - base);
                    let line_bits = line_dot(&BRAILLE_L, at(&second_s, l, |s| s.0), base)
                        | line_dot(&BRAILLE_R, at(&second_s, r2, |s| s.0), base);
                    let bits = fill_bits | band_bits | line_bits;
                    let role = if line_bits != 0 {
                        second_role
                    } else if fill_bits != 0 {
                        main_role
                    } else {
                        Role::Faint
                    };
                    (char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' '), role)
                }
                ChartStyle::Blocks | ChartStyle::Ascii => {
                    let fill = at(&main_s, x, |s| s.0) - base;
                    let line = at(&second_s, x, |s| s.0);
                    let on_line = line > base && line <= base + 8;
                    if style == ChartStyle::Blocks {
                        if fill >= 8 || !on_line {
                            (EIGHTHS[fill.clamp(0, 8) as usize], main_role)
                        } else {
                            ('·', second_role)
                        }
                    } else if on_line {
                        ('*', second_role)
                    } else if fill >= 4 {
                        ('#', main_role)
                    } else if fill > 0 {
                        ('.', main_role)
                    } else {
                        (' ', main_role)
                    }
                }
            };
            let blank = ch == ' ' || ch == '\u{2800}';
            let role = if blank { Role::Faint } else { role };
            let ch = if blank { ' ' } else { ch };
            match runs.last_mut() {
                Some((s, rr)) if *rr == role => s.push(ch),
                _ => runs.push((ch.to_string(), role)),
            }
        }
        let mut x = a.x;
        for (s, role) in runs {
            let n = s.chars().count() as u16;
            if s.chars().any(|ch| ch != ' ') {
                c.text(x, a.y + r, n, &s, role);
            }
            x += n;
        }
    }
}

/// A one-row chart of `pts`, scaled to `y_max` (`None`: to its own peak).
pub fn spark(
    c: &mut Canvas,
    a: Area,
    pts: &[Pt],
    window: (i64, i64),
    y_max: Option<f32>,
    role: Role,
    style: ChartStyle,
) {
    if a.w == 0 {
        return;
    }
    let top = y_max.unwrap_or_else(|| peak(pts)).max(f32::MIN_POSITIVE);
    let samples = resample(pts, window.0, window.1, usize::from(a.w));
    let mut s = String::with_capacity(samples.len() * 3);
    for v in samples {
        let l = v.map_or(0, |(avg, _)| level(avg, top, 8)).clamp(0, 8) as usize;
        s.push(match style {
            ChartStyle::Ascii => match l {
                0 => ' ',
                1..=2 => '.',
                3..=5 => ':',
                _ => '#',
            },
            _ => EIGHTHS[l],
        });
    }
    c.text(a.x, a.y, a.w, &s, role);
}

/// A horizontal bar: `share` of `w` cells in `role` on a track.
pub fn bar(c: &mut Canvas, a: Area, share: f32, role: Role, style: ChartStyle) {
    if a.w == 0 {
        return;
    }
    let share = if share.is_finite() { share.clamp(0.0, 1.0) } else { 0.0 };
    let w = usize::from(a.w);
    let (full, rest) = if style == ChartStyle::Ascii { ('#', '.') } else { ('█', '░') };
    let eighths = (share * (w * 8) as f32).round() as usize;
    let eighths = if share > 0.0 { eighths.max(1) } else { 0 };
    let cells = eighths / 8;
    let mut s: String = std::iter::repeat_n(full, cells).collect();
    if style != ChartStyle::Ascii && !eighths.is_multiple_of(8) {
        // Left-aligned partial cells: U+258F (one eighth) up to U+2589 (seven).
        s.push(char::from_u32(0x2590 - (eighths % 8) as u32).unwrap_or(full));
    } else if style == ChartStyle::Ascii && eighths % 8 >= 4 && cells < w {
        s.push('#');
    }
    let drawn = s.chars().count();
    c.text(a.x, a.y, a.w, &s, role);
    if drawn < w {
        let track: String = std::iter::repeat_n(rest, w - drawn).collect();
        c.text(a.x + drawn as u16, a.y, (w - drawn) as u16, &track, Role::ProgressTrack);
    }
}

/// A bar of several parts, each with its own glyph so the parts differ without colour.
pub fn stacked_bar(c: &mut Canvas, a: Area, parts: &[(f32, Role, char)]) {
    let w = usize::from(a.w);
    let mut x = 0usize;
    let mut sum = 0.0f32;
    for (share, role, glyph) in parts {
        sum += share.max(0.0);
        let end = ((sum * w as f32).round() as usize).min(w);
        if end > x {
            let s: String = std::iter::repeat_n(*glyph, end - x).collect();
            c.text(a.x + x as u16, a.y, (end - x) as u16, &s, *role);
            x = end;
        }
    }
}

/// How hot a load share looks.
pub fn heat_role(share: f32) -> Role {
    if share >= 0.85 {
        Role::Danger
    } else if share >= 0.6 {
        Role::Warning
    } else {
        Role::Progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::monitor::canvas::Op;

    fn pt(t: i64, v: f32) -> Pt {
        Pt { t, avg: v, max: v }
    }

    #[test]
    fn resample_holds_and_breaks() {
        let pts = [pt(0, 1.0), pt(4, 3.0), pt(5, f32::NAN), pt(8, 2.0)];
        let s = resample(&pts, 0, 8, 9);
        assert_eq!(s[0], Some((1.0, 1.0)));
        assert_eq!(s[2], Some((1.0, 1.0)), "held between two points");
        assert_eq!(s[4], Some((3.0, 3.0)));
        assert_eq!(s[6], None, "a break leaves a gap");
        assert_eq!(s[8], Some((2.0, 2.0)));
        assert!(resample(&pts, 5, 5, 3).iter().all(Option::is_none));
    }

    #[test]
    fn nice_numbers() {
        assert_eq!(nice_ceil(0.0), 1.0);
        assert_eq!(nice_ceil(3.0), 5.0);
        assert_eq!(nice_ceil(11.0), 20.0);
        assert_eq!(nice_ceil(900.0), 1000.0);
        assert_eq!(nice_bytes(90_000.0), 102_400.0);
        assert_eq!(nice_bytes(0.0), 1024.0);
    }

    fn texts(c: &Canvas) -> String {
        c.ops
            .iter()
            .filter_map(|o| match o {
                Op::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn sample_chart(style: ChartStyle, w: u16, h: u16) -> String {
        let pts: Vec<Pt> = (0..20).map(|i| pt(i, i as f32 * 5.0)).collect();
        let fmt = |v: f32| format!("{v:.0}%");
        let chart = Chart {
            series: vec![Series { label: "CPU".into(), pts: &pts, role: Role::Accent }],
            from: 0,
            to: 19,
            y_max: Some(100.0),
            fmt: &fmt,
            cursor: None,
            legend: true,
            bytes: false,
        };
        let mut c = Canvas::new(w, h);
        draw(&mut c, Area { x: 0, y: 0, w, h }, &chart, style);
        texts(&c)
    }

    #[test]
    fn styles_use_their_own_glyphs() {
        assert!(sample_chart(ChartStyle::Braille, 40, 6).chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)));
        assert!(sample_chart(ChartStyle::Blocks, 40, 6).contains('█'));
        let ascii = sample_chart(ChartStyle::Ascii, 40, 6);
        assert!(ascii.contains('#') && ascii.is_ascii(), "{ascii}");
        assert!(ascii.contains("100%"), "the axis: {ascii}");
    }

    #[test]
    fn tiny_charts_do_not_panic() {
        for w in 0..12 {
            for h in 0..5 {
                for s in [ChartStyle::Braille, ChartStyle::Blocks, ChartStyle::Ascii] {
                    sample_chart(s, w, h);
                }
            }
        }
    }

    #[test]
    fn a_bar_has_partial_cells() {
        let mut c = Canvas::new(8, 1);
        bar(&mut c, Area { x: 0, y: 0, w: 8, h: 1 }, 0.5625, Role::Progress, ChartStyle::Braille);
        assert_eq!(texts(&c), "████▌░░░");
        let mut c = Canvas::new(4, 1);
        bar(&mut c, Area { x: 0, y: 0, w: 4, h: 1 }, 0.5, Role::Progress, ChartStyle::Ascii);
        assert_eq!(texts(&c), "##..");
    }
}
