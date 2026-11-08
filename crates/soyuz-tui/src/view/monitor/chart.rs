//! The monitor's charts, drawn like btop's: area graphs coloured by height along a
//! gradient ([`Ramp`]), one-row graphs and `■` meters coloured by position, in three
//! styles.
//!
//! - Braille (the default): two by four dots per cell.
//! - Blocks (`TERM=linux`, whose console font has no braille): eighth blocks, one value per
//!   cell.
//! - ASCII (`[tui] icons = "ascii"`): `#` for the area, `.` for a part of a cell.
//!
//! A chart has at most two series. The first is an area; the second is a line over it, or
//! with [`Chart::mirror`] a second area that grows down from the middle (download up,
//! upload down, like btop's network box), each half on its own scale. The two differ in
//! shape as well as hue and stay apart in 16 colours and in mono; the legend, written over
//! the graph's corner, names both next to their marks. A series is cut at `Pt::is_break`.

use std::sync::OnceLock;

use soyuz_sysmon::Pt;

use super::canvas::{Area, Canvas};
use crate::icons::IconSet;
use crate::theme::{Ramp, Role};
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

/// The gradients of the entities a page shows, in fixed order.
pub const HUES: [Ramp; 6] = [Ramp::Hue(0), Ramp::Hue(1), Ramp::Hue(2), Ramp::Hue(3), Ramp::Hue(4), Ramp::Hue(5)];

/// One series of a chart.
pub struct Series<'a> {
    pub label: String,
    pub pts: &'a [Pt],
    pub ramp: Ramp,
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
    /// Write the legend over the graph (from three rows up).
    pub legend: bool,
    /// The values are bytes: the axis rounds in powers of 1024.
    pub bytes: bool,
    /// The second series grows down from the middle, on its own scale.
    pub mirror: bool,
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

/// Dots `fill` rows high from the bottom of the cell.
fn dots(col: &[u8; 4], fill: i32) -> u8 {
    (0..fill.clamp(0, 4) as usize).fold(0, |b, i| b | col[i])
}

/// Dots `fill` rows deep from the top of the cell.
fn dots_down(col: &[u8; 4], fill: i32) -> u8 {
    (0..fill.clamp(0, 4) as usize).fold(0, |b, i| b | col[3 - i])
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

/// The top of an axis for `pts`: `y_max`, or a round number above the peak.
fn axis_top(y_max: Option<f32>, pts: &[&[Pt]], bytes: bool) -> f32 {
    let top = y_max.unwrap_or_else(|| {
        let max = pts.iter().map(|p| peak(p)).fold(0.0, f32::max) * 1.05;
        if bytes { nice_bytes(max) } else { nice_ceil(max) }
    });
    if top > 0.0 { top } else { 1.0 }
}

/// Draw `chart` into `a`: the graph fills the area; the legend and the scale are written
/// over its corners.
pub fn draw(c: &mut Canvas, a: Area, chart: &Chart<'_>, style: ChartStyle) {
    if a.w == 0 || a.h == 0 {
        return;
    }
    if chart.mirror && chart.series.len() >= 2 && a.h >= 2 {
        let up_h = a.h.div_ceil(2);
        let up = Area { h: up_h, ..a };
        let down = Area { y: a.y + i32::from(up_h), h: a.h - up_h, ..a };
        let tops = [0, 1].map(|i| axis_top(chart.y_max, &[chart.series[i].pts], chart.bytes));
        let halves = [(up, false), (down, true)];
        for (i, (half, flip)) in halves.into_iter().enumerate() {
            area(c, half, &chart.series[i], None, chart, tops[i], style, flip);
        }
        if a.h >= 3 {
            let (top_row, bottom_row) = (up.row(0), down.row(down.h - 1));
            scale(c, top_row, &(chart.fmt)(tops[0]));
            scale(c, bottom_row, &(chart.fmt)(tops[1]));
            if chart.legend {
                legend_row(c, top_row, &chart.series[0], 0, chart, style);
                legend_row(c, bottom_row, &chart.series[1], 1, chart, style);
            }
        }
    } else {
        let all: Vec<&[Pt]> = chart.series.iter().map(|s| s.pts).collect();
        let top = axis_top(chart.y_max, &all, chart.bytes);
        if let Some(main) = chart.series.first() {
            area(c, a, main, chart.series.get(1), chart, top, style, false);
        }
        if a.h >= 2 {
            scale(c, a.row(0), &(chart.fmt)(top));
        }
        if chart.legend && a.h >= 3 {
            for (i, s) in chart.series.iter().take(2).enumerate() {
                legend_row(c, a.row(i as u16), s, i, chart, style);
            }
        }
    }
    if let Some(t) = chart.cursor
        && t >= chart.from
        && t <= chart.to
        && chart.to > chart.from
    {
        let col = i128::from(t - chart.from) * i128::from(a.w - 1) / i128::from(chart.to - chart.from);
        c.paint(Area { x: a.x + col as u16, w: 1, ..a }, Role::Highlight);
    }
}

/// The axis's top value at the right end of `row`.
fn scale(c: &mut Canvas, row: Area, s: &str) {
    let w = text::width(s) as u16;
    if row.w >= w + 12 {
        c.right(row.x, row.y, row.w, s, Role::Muted);
    }
}

/// `■ CPU 12%` (an area), `· System 3%` (a line) or `▲ Download 2 MiB/s` (a mirrored half)
/// at the left of `row`, clear of the scale.
fn legend_row(c: &mut Canvas, row: Area, s: &Series<'_>, i: usize, chart: &Chart<'_>, style: ChartStyle) {
    let ascii = style == ChartStyle::Ascii;
    let mark = match (chart.mirror, i == 0, ascii) {
        (true, true, true) => "^",
        (true, false, true) => "v",
        (true, true, false) => "▲",
        (true, false, false) => "▼",
        (false, true, true) => "#",
        (false, false, true) => "*",
        (false, true, false) => "■",
        (false, false, false) => "·",
    };
    let value = latest(s.pts).map(chart.fmt).unwrap_or_else(|| "–".into());
    let room = Area { w: row.w.saturating_sub(12), ..row };
    c.spans(
        room,
        &[(mark, s.ramp.top()), (" ", Role::Text), (&s.label, Role::Muted), (" ", Role::Text), (&value, Role::Title)],
    );
}

/// The area of `main` in `a` (from the bottom, or from the top when `down`), each row
/// coloured by its distance from the axis along the series' ramp, `line` drawn over it.
#[allow(clippy::too_many_arguments)]
fn area(
    c: &mut Canvas,
    a: Area,
    main: &Series<'_>,
    line: Option<&Series<'_>>,
    chart: &Chart<'_>,
    top: f32,
    style: ChartStyle,
    down: bool,
) {
    let (w, h) = (usize::from(a.w), i32::from(a.h));
    if w == 0 || h == 0 {
        return;
    }
    let braille = style == ChartStyle::Braille;
    let per = if braille { 4 } else { 8 };
    let rows = h * per;
    let cols = if braille { w * 2 } else { w };
    let main_s = resample(main.pts, chart.from, chart.to, cols);
    let line_s = line.map_or_else(Vec::new, |s| resample(s.pts, chart.from, chart.to, cols));
    let line_role = line.map_or(Role::Info, |s| s.ramp.top());
    let at = |v: &[Sample], i: usize| v.get(i).copied().flatten().map_or(0, |s| level(s.0, top, rows));
    for r in 0..h {
        // Rows between this one and the axis.
        let from_axis = if down { r } else { h - 1 - r };
        let base = from_axis * per;
        let fill_role = main.ramp.at((from_axis as f32 + 0.5) / h as f32);
        let mut runs: Vec<(String, Role)> = Vec::new();
        for x in 0..w {
            let (ch, role) = match style {
                ChartStyle::Braille => {
                    let (l, r2) = (x * 2, x * 2 + 1);
                    let (fl, fr) = (at(&main_s, l) - base, at(&main_s, r2) - base);
                    let fill_bits = if down {
                        dots_down(&BRAILLE_L, fl) | dots_down(&BRAILLE_R, fr)
                    } else {
                        dots(&BRAILLE_L, fl) | dots(&BRAILLE_R, fr)
                    };
                    let line_bits =
                        line_dot(&BRAILLE_L, at(&line_s, l), base) | line_dot(&BRAILLE_R, at(&line_s, r2), base);
                    let role = if line_bits != 0 { line_role } else { fill_role };
                    (char::from_u32(0x2800 + u32::from(fill_bits | line_bits)).unwrap_or(' '), role)
                }
                ChartStyle::Blocks | ChartStyle::Ascii => {
                    let fill = at(&main_s, x) - base;
                    let l = at(&line_s, x);
                    let on_line = l > base && l <= base + 8;
                    if style == ChartStyle::Blocks {
                        if fill >= 8 || !on_line {
                            let ch = if down {
                                // The console font has the upper half and the full block only.
                                match fill {
                                    ..=0 => ' ',
                                    1..=5 => '▀',
                                    _ => '█',
                                }
                            } else {
                                EIGHTHS[fill.clamp(0, 8) as usize]
                            };
                            (ch, fill_role)
                        } else {
                            ('·', line_role)
                        }
                    } else if on_line {
                        ('*', line_role)
                    } else if fill >= 4 {
                        ('#', fill_role)
                    } else if fill > 0 {
                        ('.', fill_role)
                    } else {
                        (' ', fill_role)
                    }
                }
            };
            push_run(&mut runs, ch, role);
        }
        put_runs(c, a.x, a.y + r, runs);
    }
}

/// Add `ch` to the runs; blank cells (spaces, the empty braille cell) join a faint run.
fn push_run(runs: &mut Vec<(String, Role)>, ch: char, role: Role) {
    let blank = ch == ' ' || ch == '\u{2800}';
    let role = if blank { Role::Faint } else { role };
    let ch = if blank { ' ' } else { ch };
    match runs.last_mut() {
        Some((s, rr)) if *rr == role => s.push(ch),
        _ => runs.push((ch.to_string(), role)),
    }
}

/// Write runs of one role each, skipping blank ones.
fn put_runs(c: &mut Canvas, x0: u16, y: i32, runs: Vec<(String, Role)>) {
    let mut x = x0;
    for (s, role) in runs {
        let n = s.chars().count() as u16;
        if s.chars().any(|ch| ch != ' ') {
            c.text(x, y, n, &s, role);
        }
        x += n;
    }
}

/// A one-row graph of `pts`, scaled to `y_max` (`None`: to its own peak), like btop's
/// core graphs: braille with two values per cell on a dotted baseline, each cell coloured
/// by its height along `ramp`.
pub fn spark(
    c: &mut Canvas,
    a: Area,
    pts: &[Pt],
    window: (i64, i64),
    y_max: Option<f32>,
    ramp: Ramp,
    style: ChartStyle,
) {
    if a.w == 0 {
        return;
    }
    let top = y_max.unwrap_or_else(|| peak(pts)).max(f32::MIN_POSITIVE);
    let w = usize::from(a.w);
    let braille = style == ChartStyle::Braille;
    let samples = resample(pts, window.0, window.1, if braille { w * 2 } else { w });
    let lvl = |i: usize, rows: i32| samples.get(i).copied().flatten().map_or(0, |(avg, _)| level(avg, top, rows));
    let mut runs: Vec<(String, Role)> = Vec::new();
    for x in 0..w {
        let (ch, share) = match style {
            ChartStyle::Braille => {
                let (l, r) = (lvl(x * 2, 4), lvl(x * 2 + 1, 4));
                let bits = dots(&BRAILLE_L, l) | dots(&BRAILLE_R, r);
                // An empty cell keeps the baseline.
                let bits = if bits == 0 { BRAILLE_L[0] | BRAILLE_R[0] } else { bits };
                (char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' '), l.max(r) as f32 / 4.0)
            }
            ChartStyle::Blocks => {
                let l = lvl(x, 8);
                (EIGHTHS[l.clamp(0, 8) as usize], l as f32 / 8.0)
            }
            ChartStyle::Ascii => {
                let l = lvl(x, 8);
                let ch = match l {
                    0 => ' ',
                    1..=2 => '.',
                    3..=5 => ':',
                    _ => '#',
                };
                (ch, l as f32 / 8.0)
            }
        };
        let role = if share <= 0.0 { Role::Faint } else { ramp.at(share) };
        match runs.last_mut() {
            Some((s, rr)) if *rr == role => s.push(ch),
            _ => runs.push((ch.to_string(), role)),
        }
    }
    put_runs(c, a.x, a.y, runs);
}

/// A btop meter: `share` of the cells filled with `■`, each coloured by its place along
/// `ramp`, the rest a dim track of the same squares (`#` and `.` in ASCII).
pub fn bar(c: &mut Canvas, a: Area, share: f32, ramp: Ramp, style: ChartStyle) {
    if a.w == 0 {
        return;
    }
    let share = if share.is_finite() { share.clamp(0.0, 1.0) } else { 0.0 };
    let w = usize::from(a.w);
    let filled = (share * w as f32).round() as usize;
    let filled = if share > 0.0 { filled.max(1) } else { 0 }.min(w);
    let (full, rest) = if style == ChartStyle::Ascii { ('#', '.') } else { ('■', '■') };
    let mut runs: Vec<(String, Role)> = Vec::new();
    for i in 0..w {
        let (ch, role) = if i < filled {
            (full, ramp.at(if w > 1 { i as f32 / (w - 1) as f32 } else { share }))
        } else {
            (rest, Role::ProgressTrack)
        };
        match runs.last_mut() {
            Some((s, rr)) if *rr == role => s.push(ch),
            _ => runs.push((ch.to_string(), role)),
        }
    }
    let mut x = a.x;
    for (s, role) in runs {
        let n = s.chars().count() as u16;
        c.text(x, a.y, n, &s, role);
        x += n;
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

    fn roles(c: &Canvas) -> Vec<Role> {
        c.ops
            .iter()
            .filter_map(|o| match o {
                Op::Text { role, .. } => Some(*role),
                _ => None,
            })
            .collect()
    }

    fn sample_chart(style: ChartStyle, w: u16, h: u16, mirror: bool) -> Canvas {
        let pts: Vec<Pt> = (0..20).map(|i| pt(i, i as f32 * 5.0)).collect();
        let fmt = |v: f32| format!("{v:.0}%");
        let chart = Chart {
            series: vec![
                Series { label: "CPU".into(), pts: &pts, ramp: Ramp::Heat },
                Series { label: "Sys".into(), pts: &pts, ramp: Ramp::Hue(1) },
            ],
            from: 0,
            to: 19,
            y_max: Some(100.0),
            fmt: &fmt,
            cursor: None,
            legend: true,
            bytes: false,
            mirror,
        };
        let mut c = Canvas::new(w, h);
        draw(&mut c, Area { x: 0, y: 0, w, h }, &chart, style);
        c
    }

    #[test]
    fn styles_use_their_own_glyphs() {
        let braille = texts(&sample_chart(ChartStyle::Braille, 40, 6, false));
        assert!(braille.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)));
        assert!(texts(&sample_chart(ChartStyle::Blocks, 40, 6, false)).contains('█'));
        let ascii = texts(&sample_chart(ChartStyle::Ascii, 40, 6, false));
        assert!(ascii.contains('#') && ascii.is_ascii(), "{ascii}");
        assert!(ascii.contains("100%"), "the scale: {ascii}");
    }

    #[test]
    fn graphs_are_coloured_by_height() {
        let c = sample_chart(ChartStyle::Braille, 40, 8, false);
        let steps: Vec<u8> = roles(&c)
            .into_iter()
            .filter_map(|r| match r {
                Role::Ramp(Ramp::Heat, s) => Some(s),
                _ => None,
            })
            .collect();
        let (lo, hi) = (steps.iter().min().copied(), steps.iter().max().copied());
        assert!(lo < hi, "rows differ: {steps:?}");
    }

    #[test]
    fn a_mirrored_chart_marks_both_halves() {
        let t = texts(&sample_chart(ChartStyle::Braille, 40, 6, true));
        assert!(t.contains('▲') && t.contains('▼'), "{t}");
        assert!(t.contains("CPU") && t.contains("Sys"), "{t}");
        let a = texts(&sample_chart(ChartStyle::Ascii, 40, 6, true));
        assert!(a.contains('^') && a.contains('v') && a.is_ascii(), "{a}");
    }

    #[test]
    fn tiny_charts_do_not_panic() {
        for w in 0..12 {
            for h in 0..5 {
                for s in [ChartStyle::Braille, ChartStyle::Blocks, ChartStyle::Ascii] {
                    sample_chart(s, w, h, false);
                    sample_chart(s, w, h, true);
                }
            }
        }
    }

    #[test]
    fn a_meter_is_squares_on_a_track() {
        let mut c = Canvas::new(8, 1);
        bar(&mut c, Area { x: 0, y: 0, w: 8, h: 1 }, 0.5, Ramp::Heat, ChartStyle::Braille);
        assert_eq!(texts(&c), "■■■■■■■■");
        let r = roles(&c);
        assert_eq!(r.last(), Some(&Role::ProgressTrack));
        assert_eq!(r.first(), Some(&Role::Ramp(Ramp::Heat, 0)), "the meter starts at the low end");
        let mut c = Canvas::new(4, 1);
        bar(&mut c, Area { x: 0, y: 0, w: 4, h: 1 }, 0.5, Ramp::Heat, ChartStyle::Ascii);
        assert_eq!(texts(&c), "##..");
    }

    #[test]
    fn a_spark_keeps_a_baseline() {
        let mut c = Canvas::new(4, 1);
        spark(&mut c, Area { x: 0, y: 0, w: 4, h: 1 }, &[], (0, 10), Some(100.0), Ramp::Heat, ChartStyle::Braille);
        assert_eq!(texts(&c), "⣀⣀⣀⣀");
    }
}
