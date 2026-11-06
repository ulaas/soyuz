//! The charts of the system monitor: an anti-aliased 1.5 px line over a soft vertical
//! gradient (an egui mesh), a recessive three-line grid, axis labels in small tabular
//! figures, one y axis per chart with nice rounded scales, a crosshair with a tooltip on
//! hover and a legend with direct values for two or more series. Hues follow the entity
//! ([`Hue`]) and are derived from the theme's tokens, so every theme, light, dark and high
//! contrast, gets a matching set; status colours stay reserved for state.
//!
//! Nothing here repaints on its own: a chart is repainted when the hub wakes the window or
//! the pointer moves over it.

use egui::epaint::Mesh;
use egui::{Align2, Color32, Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::SizeStyle;
use soyuz_core::config::monitor::TempUnit;
use soyuz_sysmon::Pt;

use crate::models::monitor::labels;
use crate::theme::Palette;
use crate::theme::font;
use crate::theme::tokens::mix;

/// What a series measures (its colour follows it everywhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    Cpu,
    Memory,
    Swap,
    DiskRead,
    DiskWrite,
    NetRx,
    NetTx,
    Gpu,
    Temp,
    Power,
    /// Counts and other neutral quantities.
    Neutral,
}

/// The colour of a hue in `p`. Categorical hues come from the theme's tag colours, so
/// they stay distinct on every theme and keep the theme's saturation.
pub fn hue(p: &Palette, h: Hue) -> Color32 {
    match h {
        Hue::Cpu => p.accent,
        Hue::Memory => p.tags[6],
        Hue::Swap => mix(p.tags[6], p.text_secondary, 0.45),
        Hue::DiskRead => p.tags[4],
        Hue::DiskWrite => p.tags[1],
        Hue::NetRx => p.tags[3],
        Hue::NetTx => p.tags[2],
        Hue::Gpu => p.tags[5],
        Hue::Temp => mix(p.tags[0], p.tags[1], 0.5),
        Hue::Power => p.tags[2],
        Hue::Neutral => p.text_secondary,
    }
}

/// The fill of a heat cell at `t` (0..=1): one sequential hue over the surface.
pub fn heat(p: &Palette, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    p.accent.gamma_multiply(0.06 + 0.34 * t.sqrt())
}

/// The fill of a heat-grid tile at `t` (0..=1): from the surface to the accent.
pub fn heat_tile(p: &Palette, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    mix(p.bg_surface2, p.accent, 0.08 + 0.92 * t)
}

/// How values are written on axes and in tooltips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    Percent,
    Bytes,
    Rate,
    Count,
    Temp(TempUnit),
    Mhz,
    Watts,
    Rpm,
    Raw,
}

/// Number formatting the user chose.
#[derive(Clone, Copy, Debug)]
pub struct Fmt {
    pub style: SizeStyle,
}

/// A value in a unit, for tooltips and legends.
pub fn fmt_value(unit: Unit, v: f32, fmt: Fmt) -> String {
    if !v.is_finite() {
        return "–".to_string();
    }
    match unit {
        Unit::Percent => labels::percent(v),
        Unit::Bytes => soyuz_core::format_size(v.max(0.0) as u64, fmt.style),
        Unit::Rate => labels::rate(f64::from(v), fmt.style),
        Unit::Count => {
            if v < 10.0 && v.fract() != 0.0 {
                format!("{v:.1}")
            } else {
                labels::count(v.max(0.0).round() as u64)
            }
        }
        Unit::Temp(u) => labels::temperature(v, u),
        Unit::Mhz => {
            if v >= 1000.0 {
                format!("{:.2} GHz", v / 1000.0)
            } else {
                format!("{v:.0} MHz")
            }
        }
        Unit::Watts => format!("{v:.1} W"),
        Unit::Rpm => format!("{v:.0} RPM"),
        Unit::Raw => format!("{v:.1}"),
    }
}

/// A short axis label.
fn axis_label(unit: Unit, v: f32, fmt: Fmt) -> String {
    match unit {
        Unit::Percent => format!("{v:.0}%"),
        _ => fmt_value(unit, v, fmt),
    }
}

/// The smallest "round" value at or above `v`: 1, 2, 2.5, 5 times a power of ten (powers of
/// two times a binary unit for byte quantities).
pub fn nice_max(unit: Unit, v: f32) -> f32 {
    if !v.is_finite() || v <= 0.0 {
        return match unit {
            Unit::Percent => 100.0,
            Unit::Bytes | Unit::Rate => 1024.0,
            _ => 1.0,
        };
    }
    match unit {
        Unit::Bytes | Unit::Rate => {
            let mut unit_size = 1.0f32;
            while v / unit_size >= 1024.0 {
                unit_size *= 1024.0;
            }
            let x = v / unit_size;
            let steps = [
                1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0, 96.0, 128.0, 192.0, 256.0,
                384.0, 512.0, 768.0, 1024.0,
            ];
            steps.iter().copied().find(|s| *s >= x).unwrap_or(1024.0) * unit_size
        }
        _ => {
            let mag = 10f32.powf(v.log10().floor());
            let x = v / mag;
            let step = [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().find(|s| *s >= x - 1e-4).unwrap_or(10.0);
            step * mag
        }
    }
}

/// One line of a chart.
pub struct Series<'a> {
    pub label: String,
    pub color: Color32,
    pub pts: &'a [Pt],
    /// Gradient fill under the line.
    pub fill: bool,
    /// Draw the faint band between the average and the maximum (aggregated history).
    pub band: bool,
}

impl<'a> Series<'a> {
    pub fn new(label: impl Into<String>, color: Color32, pts: &'a [Pt]) -> Self {
        Series { label: label.into(), color, pts, fill: true, band: false }
    }

    pub fn line(mut self) -> Self {
        self.fill = false;
        self
    }

    pub fn band(mut self) -> Self {
        self.band = true;
        self
    }

    /// The newest finite value.
    pub fn last(&self) -> Option<f32> {
        self.pts.iter().rev().find(|p| p.avg.is_finite()).map(|p| p.avg)
    }
}

/// How the time axis is labelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeLabels {
    /// "-2 min" … "now".
    Relative,
    /// Wall-clock times.
    Absolute,
}

/// What the chart reported back.
pub struct ChartOut {
    pub response: Response,
    /// The moment under the pointer.
    pub hover_t: Option<i64>,
    /// The plot area (for callers that draw markers of their own).
    pub plot: Rect,
}

/// A chart; configure, then [`Chart::show`].
pub struct Chart {
    /// The time span shown, unix milliseconds.
    pub x: (i64, i64),
    /// A fixed top of the y axis (`None`: a nice value above the data).
    pub y_max: Option<f32>,
    pub unit: Unit,
    pub fmt: Fmt,
    pub axes: bool,
    pub grid: bool,
    pub legend: bool,
    pub crosshair: bool,
    /// The pointer can drag the chart (the caller reads `response.drag_delta()`).
    pub drag: bool,
    pub time_labels: TimeLabels,
    /// Draw a dot on the newest value of every series.
    pub live_dot: bool,
    /// The crosshair position from another chart sharing the time axis.
    pub shared_cursor: Option<i64>,
}

impl Chart {
    pub fn new(x: (i64, i64), unit: Unit, fmt: Fmt) -> Self {
        Chart {
            x,
            y_max: None,
            unit,
            fmt,
            axes: true,
            grid: true,
            legend: true,
            crosshair: true,
            drag: false,
            time_labels: TimeLabels::Relative,
            live_dot: true,
            shared_cursor: None,
        }
    }

    /// A sparkline-sized chart: no axes, grid or legend.
    pub fn mini(x: (i64, i64), unit: Unit, fmt: Fmt) -> Self {
        Chart { axes: false, grid: false, legend: false, live_dot: false, ..Chart::new(x, unit, fmt) }
    }

    pub fn y_max(mut self, v: f32) -> Self {
        self.y_max = Some(v);
        self
    }

    pub fn absolute_time(mut self) -> Self {
        self.time_labels = TimeLabels::Absolute;
        self
    }

    pub fn draggable(mut self) -> Self {
        self.drag = true;
        self
    }

    pub fn no_legend(mut self) -> Self {
        self.legend = false;
        self
    }

    pub fn cursor(mut self, t: Option<i64>) -> Self {
        self.shared_cursor = t;
        self
    }

    pub fn show(&self, ui: &mut Ui, rect: Rect, id: Id, p: &Palette, series: &[Series<'_>]) -> ChartOut {
        let sense = if self.drag { Sense::click_and_drag() } else { Sense::hover() };
        let response = ui.interact(rect, id, sense);
        let painter = ui.painter_at(rect);
        let (x0, x1) = self.x;
        let span = (x1 - x0).max(1) as f32;
        let data_max = series
            .iter()
            .flat_map(|s| s.pts.iter())
            .map(|pt| if pt.max.is_finite() { pt.max.max(pt.avg) } else { pt.avg })
            .filter(|v| v.is_finite())
            .fold(0.0f32, f32::max);
        let y_max = match self.y_max {
            Some(m) if data_max <= m * 1.001 => m,
            Some(_) | None => nice_max(self.unit, data_max * 1.05),
        };
        let y_max = y_max.max(f32::EPSILON);
        // The left gutter fits the widest axis label.
        let axis_font = font::numbers(10.0);
        let gutter_l = if self.axes {
            let w = [y_max, y_max / 2.0, 0.0]
                .into_iter()
                .map(|v| {
                    painter
                        .layout_no_wrap(axis_label(self.unit, v, self.fmt), axis_font.clone(), Color32::PLACEHOLDER)
                        .size()
                        .x
                })
                .fold(0.0f32, f32::max);
            (w + 10.0).max(font::grow(30.0))
        } else {
            0.0
        };
        let gutter_b = if self.axes { font::grow(16.0) } else { 0.0 };
        let plot =
            Rect::from_min_max(pos2(rect.left() + gutter_l, rect.top()), pos2(rect.right(), rect.bottom() - gutter_b));
        if plot.width() < 8.0 || plot.height() < 8.0 {
            return ChartOut { response, hover_t: None, plot };
        }
        let to_x = |t: i64| plot.left() + ((t - x0) as f32 / span) * plot.width();
        let to_y = |v: f32| plot.bottom() - (v / y_max).clamp(0.0, 1.0) * plot.height();

        // ---- grid ----
        if self.grid {
            let faint = Stroke::new(1.0, p.border_subtle.gamma_multiply(0.75));
            for k in 1..4 {
                let y = (plot.bottom() - plot.height() * k as f32 / 4.0).round() + 0.5;
                painter.hline(plot.x_range(), y, faint);
            }
            painter.hline(plot.x_range(), plot.bottom().round() - 0.5, Stroke::new(1.0, p.border_subtle));
        }
        // ---- axes ----
        if self.axes {
            let f = axis_font;
            let lx = plot.left() - 6.0;
            painter.text(
                pos2(lx, plot.top() + 1.0),
                Align2::RIGHT_TOP,
                axis_label(self.unit, y_max, self.fmt),
                f.clone(),
                p.text_tertiary,
            );
            painter.text(
                pos2(lx, plot.center().y),
                Align2::RIGHT_CENTER,
                axis_label(self.unit, y_max / 2.0, self.fmt),
                f.clone(),
                p.text_tertiary,
            );
            painter.text(
                pos2(lx, plot.bottom() - 1.0),
                Align2::RIGHT_BOTTOM,
                axis_label(self.unit, 0.0, self.fmt),
                f.clone(),
                p.text_tertiary,
            );
            let ty = rect.bottom() - 1.0;
            let label = |t: i64, end: bool| match self.time_labels {
                TimeLabels::Relative => {
                    if end {
                        tr!("monitor-gui-now").to_string()
                    } else {
                        let secs = ((x1 - t) / 1000).max(0) as u64;
                        format!("-{}", short_duration(secs))
                    }
                }
                TimeLabels::Absolute => clock(t, span > 36.0 * 3_600_000.0),
            };
            painter.text(pos2(plot.left(), ty), Align2::LEFT_BOTTOM, label(x0, false), f.clone(), p.text_tertiary);
            if plot.width() > 260.0 {
                painter.text(
                    pos2(plot.center().x, ty),
                    Align2::CENTER_BOTTOM,
                    label((x0 + x1) / 2, false),
                    f.clone(),
                    p.text_tertiary,
                );
            }
            painter.text(pos2(plot.right(), ty), Align2::RIGHT_BOTTOM, label(x1, true), f, p.text_tertiary);
        }

        // ---- series ----
        let base = plot.bottom();
        let inner = ui.painter_at(plot.expand2(vec2(0.0, 2.0)));
        for s in series {
            let mut run: Vec<Pos2> = Vec::new();
            let mut mesh = Mesh::default();
            let mut band = Mesh::default();
            let top_c = s.color.gamma_multiply(if p.dark { 0.30 } else { 0.24 });
            let band_c = s.color.gamma_multiply(0.11);
            let flush = |run: &mut Vec<Pos2>, inner: &egui::Painter| {
                if run.len() >= 2 {
                    inner.add(Shape::line(std::mem::take(run), Stroke::new(1.5, s.color)));
                } else if let Some(pt) = run.pop() {
                    inner.circle_filled(pt, 1.2, s.color);
                }
            };
            let mut prev: Option<(Pos2, f32)> = None;
            for pt in s.pts {
                if pt.is_break() || !pt.avg.is_finite() || pt.t < x0 - 1000 || pt.t > x1 + 1000 {
                    flush(&mut run, &inner);
                    prev = None;
                    continue;
                }
                let x = to_x(pt.t);
                let y = to_y(pt.avg);
                let pos = pos2(x, y);
                if s.fill
                    && let Some((a, _)) = prev
                {
                    let i = mesh.vertices.len() as u32;
                    mesh.colored_vertex(a, top_c);
                    mesh.colored_vertex(pos, top_c);
                    mesh.colored_vertex(pos2(x, base), Color32::TRANSPARENT);
                    mesh.colored_vertex(pos2(a.x, base), Color32::TRANSPARENT);
                    mesh.add_triangle(i, i + 1, i + 2);
                    mesh.add_triangle(i, i + 2, i + 3);
                }
                if s.band
                    && pt.max.is_finite()
                    && let Some((a, amax)) = prev
                {
                    let i = band.vertices.len() as u32;
                    band.colored_vertex(a, band_c);
                    band.colored_vertex(pos, band_c);
                    band.colored_vertex(pos2(x, to_y(pt.max)), band_c);
                    band.colored_vertex(pos2(a.x, to_y(amax)), band_c);
                    band.add_triangle(i, i + 1, i + 2);
                    band.add_triangle(i, i + 2, i + 3);
                }
                run.push(pos);
                prev = Some((pos, if pt.max.is_finite() { pt.max } else { pt.avg }));
            }
            if !mesh.is_empty() {
                inner.add(Shape::mesh(mesh));
            }
            if !band.is_empty() {
                inner.add(Shape::mesh(band));
            }
            let last = run.last().copied();
            flush(&mut run, &inner);
            if self.live_dot
                && let Some(l) = last
            {
                inner.circle_filled(l, 2.5, s.color);
            }
        }

        // ---- legend ----
        if self.legend && series.len() >= 2 {
            let f = font::ui(11.0);
            let mut x = plot.right() - 6.0;
            let y = plot.top() + 9.0;
            let wide = plot.width() > 320.0;
            for s in series.iter().rev() {
                let text = match (wide, s.last()) {
                    (true, Some(v)) => format!("{} {}", s.label, fmt_value(self.unit, v, self.fmt)),
                    _ => s.label.clone(),
                };
                let g = painter.layout_no_wrap(text, f.clone(), p.text_secondary);
                x -= g.size().x;
                painter.galley(pos2(x, y - g.size().y / 2.0), g, p.text_secondary);
                x -= 10.0;
                painter.circle_filled(pos2(x, y), 3.0, s.color);
                x -= 14.0;
            }
        }

        // ---- crosshair ----
        let mut hover_t = None;
        let pointer = response.hover_pos().filter(|pos| plot.contains(*pos));
        let cursor_t =
            pointer.map(|pos| x0 + ((pos.x - plot.left()) / plot.width() * span) as i64).or(self.shared_cursor);
        if self.crosshair
            && let Some(t) = cursor_t
            && (x0..=x1).contains(&t)
        {
            hover_t = Some(t);
            let x = to_x(t).round() + 0.5;
            painter.vline(x, plot.y_range(), Stroke::new(1.0, p.border_strong));
            let mut lines: Vec<(Color32, String, String)> = Vec::new();
            for s in series {
                if let Some(pt) = Pt::near(s.pts, t, (x1 - x0).max(1)) {
                    painter.circle_filled(pos2(to_x(pt.t), to_y(pt.avg)), 3.5, p.bg_surface1);
                    painter.circle_filled(pos2(to_x(pt.t), to_y(pt.avg)), 2.5, s.color);
                    let v = fmt_value(self.unit, pt.avg, self.fmt);
                    let v = if s.band && pt.max.is_finite() && (pt.max - pt.avg).abs() > f32::EPSILON {
                        tr!(
                            "monitor-gui-avg-max",
                            avg = v.as_str(),
                            max = fmt_value(self.unit, pt.max, self.fmt).as_str()
                        )
                    } else {
                        v
                    };
                    lines.push((s.color, s.label.clone(), v));
                }
            }
            if pointer.is_some() && !lines.is_empty() {
                tooltip(&painter, p, plot, pos2(x, pointer.map_or(plot.top(), |q| q.y)), &clock(t, false), &lines);
            }
        }
        ChartOut { response, hover_t, plot }
    }
}

/// The value tooltip of a crosshair: a small card beside the pointer, flipped to stay inside.
fn tooltip(
    painter: &egui::Painter,
    p: &Palette,
    plot: Rect,
    at: Pos2,
    title: &str,
    lines: &[(Color32, String, String)],
) {
    let f = font::ui(11.5);
    let fn_ = font::numbers(11.5);
    let title_g = painter.layout_no_wrap(title.to_string(), font::ui(11.0), p.text_tertiary);
    let rows: Vec<_> = lines
        .iter()
        .map(|(c, l, v)| {
            (
                *c,
                painter.layout_no_wrap(l.clone(), f.clone(), p.text_secondary),
                painter.layout_no_wrap(v.clone(), fn_.clone(), p.text_primary),
            )
        })
        .collect();
    let row_h = 17.0;
    let w = rows.iter().map(|(_, l, v)| 14.0 + l.size().x + 12.0 + v.size().x).fold(title_g.size().x, f32::max) + 20.0;
    let h = 10.0 + 16.0 + rows.len() as f32 * row_h + 4.0;
    let mut x = at.x + 14.0;
    if x + w > plot.right() {
        x = at.x - 14.0 - w;
    }
    let mut y = at.y - h / 2.0;
    y = y.clamp(plot.top(), (plot.bottom() - h).max(plot.top()));
    let r = Rect::from_min_size(pos2(x, y), vec2(w, h));
    painter.rect_filled(r, crate::theme::tokens::radius::same(crate::theme::tokens::radius::CARD), p.bg_surface3);
    painter.rect_stroke(
        r,
        crate::theme::tokens::radius::same(crate::theme::tokens::radius::CARD),
        Stroke::new(1.0, p.border_strong),
        StrokeKind::Inside,
    );
    painter.galley(pos2(r.left() + 10.0, r.top() + 7.0), title_g, p.text_tertiary);
    let mut ry = r.top() + 26.0;
    for (c, l, v) in rows {
        painter.circle_filled(pos2(r.left() + 14.0, ry + row_h / 2.0), 3.0, c);
        painter.galley(pos2(r.left() + 24.0, ry + (row_h - l.size().y) / 2.0), l, p.text_secondary);
        painter.galley(pos2(r.right() - 10.0 - v.size().x, ry + (row_h - v.size().y) / 2.0), v, p.text_primary);
        ry += row_h;
    }
}

/// `HH:MM:SS` (or `DD.MM HH:MM` when `with_date`) in local time.
pub fn clock(ms: i64, with_date: bool) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(t) => {
            if with_date {
                t.format("%d.%m %H:%M").to_string()
            } else {
                t.format("%H:%M:%S").to_string()
            }
        }
        _ => String::new(),
    }
}

/// `45 s`, `2 min`, `6 h`, `3 d`.
pub fn short_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} s")
    } else if secs < 3600 {
        format!("{} min", secs / 60)
    } else if secs < 86_400 {
        format!("{} h", secs / 3600)
    } else {
        format!("{} d", secs / 86_400)
    }
}

/// A horizontal meter: a track with a filled share and optional marks (a maximum, a
/// critical point) drawn as thin ticks, so a level is never told by colour alone.
pub fn meter(painter: &egui::Painter, p: &Palette, rect: Rect, frac: f32, color: Color32, marks: &[(f32, Color32)]) {
    let cr = crate::theme::tokens::radius::same(2);
    painter.rect_filled(rect, cr, p.border_subtle);
    let mut fill = rect;
    fill.set_width((rect.width() * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { 2.0 } else { 0.0 }));
    painter.rect_filled(fill, cr, color);
    for (m, c) in marks {
        let x = (rect.left() + rect.width() * m.clamp(0.0, 1.0)).round() + 0.5;
        painter.vline(x, (rect.top() - 2.0)..=(rect.bottom() + 2.0), Stroke::new(1.0, *c));
    }
}

/// A stacked composition bar: segments of `parts` (fraction, colour, label) left to right.
pub fn composition(painter: &egui::Painter, p: &Palette, rect: Rect, parts: &[(f32, Color32)]) {
    let cr = crate::theme::tokens::radius::same(3);
    painter.rect_filled(rect, cr, p.border_subtle);
    let mut x = rect.left();
    for (i, (frac, c)) in parts.iter().enumerate() {
        let w = rect.width() * frac.clamp(0.0, 1.0);
        if w < 0.5 {
            continue;
        }
        let r = Rect::from_min_max(pos2(x, rect.top()), pos2((x + w).min(rect.right()), rect.bottom()));
        painter.rect_filled(r, if i == 0 { cr } else { egui::CornerRadius::ZERO }, *c);
        x += w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_maxima() {
        assert_eq!(nice_max(Unit::Percent, 0.0), 100.0);
        assert_eq!(nice_max(Unit::Count, 73.0), 100.0);
        assert_eq!(nice_max(Unit::Count, 21.0), 25.0);
        assert_eq!(nice_max(Unit::Count, 4.1), 5.0);
        assert_eq!(nice_max(Unit::Rate, 1_500_000.0), 1.5 * 1024.0 * 1024.0);
        assert_eq!(nice_max(Unit::Rate, 1_700_000.0), 2.0 * 1024.0 * 1024.0);
        assert_eq!(nice_max(Unit::Bytes, 700.0), 768.0);
    }

    #[test]
    fn durations() {
        assert_eq!(short_duration(45), "45 s");
        assert_eq!(short_duration(120), "2 min");
        assert_eq!(short_duration(7200), "2 h");
        assert_eq!(short_duration(3 * 86_400), "3 d");
    }
}
