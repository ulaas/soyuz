//! Stacked, animated toast notifications (bottom-right). Toasts auto-dismiss (paused
//! while hovered) and may carry one action button whose [`Action`] is dispatched through
//! the normal action pipeline — e.g. `Action::Undo` for "Moved 3 items · Undo".

use egui::{Align2, Area, Color32, CornerRadius, FontId, Id, Order, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use soyuz_core::config::Action;

use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::trunc_galley;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: String,
    pub body: Option<String>,
    /// Button label and the action it dispatches.
    pub action: Option<(String, Action)>,
    /// Seconds before auto-dismiss (`None` = sticky).
    pub duration: Option<f32>,
}

impl Toast {
    pub fn new(kind: ToastKind, title: impl Into<String>) -> Self {
        let duration = match kind {
            ToastKind::Error => Some(8.0),
            ToastKind::Warning => Some(6.0),
            _ => Some(4.0),
        };
        Toast { kind, title: title.into(), body: None, action: None, duration }
    }

    pub fn info(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Info, title)
    }

    pub fn success(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Success, title)
    }

    pub fn warning(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Warning, title)
    }

    pub fn error(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Error, title)
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn action(mut self, label: impl Into<String>, action: Action) -> Self {
        self.action = Some((label.into(), action));
        self
    }

    pub fn sticky(mut self) -> Self {
        self.duration = None;
        self
    }
}

struct Live {
    id: u64,
    toast: Toast,
    /// Seconds of visible (unhovered) time left.
    remaining: Option<f32>,
    born: f64,
    closing: Option<f64>,
}

#[derive(Default)]
pub struct Toasts {
    live: Vec<Live>,
    next: u64,
}

/// Every toast has the same footprint: one title line and an optional body line.
const WIDTH: f32 = 360.0;
const HEIGHT: f32 = 56.0;
const GAP: f32 = 8.0;
const MARGIN: f32 = 12.0;
const ANIM: f64 = 0.2;
const MAX_VISIBLE: usize = 5;

/// Fraction of the auto-dismiss timer still left (`None` for sticky toasts).
fn timer_fraction(remaining: Option<f32>, duration: Option<f32>) -> Option<f32> {
    match (remaining, duration) {
        (Some(r), Some(d)) if d > 0.0 => Some((r / d).clamp(0.0, 1.0)),
        _ => None,
    }
}

/// Body text squeezed onto one line (the tooltip shows it as written).
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(crate::theme::fonts::SEMIBOLD.into()))
}

impl Toasts {
    pub fn push(&mut self, toast: Toast) -> u64 {
        self.next += 1;
        let id = self.next;
        // Collapse exact duplicates (e.g. repeated "coming soon").
        self.live.retain(|l| !(l.toast.title == toast.title && l.toast.body == toast.body && l.closing.is_none()));
        self.live.push(Live { id, remaining: toast.duration, toast, born: f64::NAN, closing: None });
        id
    }

    /// Push, replacing any live toast with the same title (status-like toasts such as
    /// the interface zoom level).
    pub fn push_replacing(&mut self, toast: Toast) -> u64 {
        self.live.retain(|l| l.toast.title != toast.title);
        self.push(toast)
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(l) = self.live.iter_mut().find(|l| l.id == id) {
            l.closing.get_or_insert(f64::NAN);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// Draw the stack anchored to the bottom-right of `area`. Returns actions clicked.
    ///
    /// Every toast has one fixed size: text that does not fit is cut with an ellipsis
    /// and shown whole in a tooltip. Hovering a toast pauses its timer (the progress line
    /// at the bottom stops) and reveals the close button. Older toasts glide up when a
    /// new one arrives and settle when one below them closes (reduced motion: jumps).
    pub fn show(&mut self, ctx: &egui::Context, area: Rect, reduced_motion: bool) -> Vec<Action> {
        let mut clicked = Vec::new();
        if self.live.is_empty() {
            return clicked;
        }
        let now = ctx.input(|i| i.time);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let p = palette(ctx);
        let width = WIDTH.min(area.width() - 2.0 * MARGIN).max(220.0);
        let mut bottom = area.bottom() - MARGIN;
        let start = self.live.len().saturating_sub(MAX_VISIBLE);
        let mut any_animating = false;
        let ease = |x: f32| 1.0 - (1.0 - x).powi(3);
        for l in self.live[start..].iter_mut().rev() {
            if l.born.is_nan() {
                l.born = now;
            }
            if l.closing.is_some_and(f64::is_nan) {
                l.closing = Some(now);
            }
            let appear = if reduced_motion { 1.0 } else { ease(((now - l.born) / ANIM).clamp(0.0, 1.0) as f32) };
            let vanish = match l.closing {
                Some(t) if !reduced_motion => ease(1.0 - ((now - t) / ANIM).clamp(0.0, 1.0) as f32),
                Some(_) => 0.0,
                None => 1.0,
            };
            let a = appear * vanish;
            if appear < 1.0 || l.closing.is_some() {
                any_animating = true;
            }

            let id = Id::new("soyuz.toast").with(l.id);
            // Stack slot: a closing toast hands its slot back as it fades.
            let target_top = bottom - HEIGHT;
            bottom -= (HEIGHT + GAP) * vanish;
            let top =
                if reduced_motion { target_top } else { crate::ui::motion::spring_to(ctx, id.with("y"), target_top) };
            // Enter: rise and fade in. Exit: drift right and fade out.
            let dy = (1.0 - appear) * 14.0;
            let dx = (1.0 - vanish) * 28.0;
            let rect = Rect::from_min_size(pos2(area.right() - width - MARGIN + dx, top + dy), vec2(width, HEIGHT));

            let mut close = false;
            Area::new(id).order(Order::Tooltip).fixed_pos(rect.min).interactable(true).show(ctx, |ui| {
                ui.multiply_opacity(a);
                let (r, resp) = ui.allocate_exact_size(rect.size(), Sense::hover());
                // Screen readers announce toasts as polite live alerts.
                let spoken = match &l.toast.body {
                    Some(b) => format!("{}. {b}", l.toast.title),
                    None => l.toast.title.clone(),
                };
                crate::ui::a11y::with(ui.ctx(), resp.id, |n| {
                    n.set_role(egui::accesskit::Role::Alert);
                    n.set_label(spoken);
                    n.set_live(egui::accesskit::Live::Polite);
                });
                let hovered = ui.rect_contains_pointer(r);
                let hover_t =
                    ui.ctx().animate_bool_with_time(id.with("hover"), hovered, if reduced_motion { 0.0 } else { 0.12 });
                let (glyph, color) = match l.toast.kind {
                    ToastKind::Info => (ph::INFO, p.info),
                    ToastKind::Success => (ph::CHECK_CIRCLE, p.success),
                    ToastKind::Warning => (ph::WARNING, p.warning),
                    ToastKind::Error => (ph::WARNING_CIRCLE, p.danger),
                };
                let card = CornerRadius::same(radius::CARD);
                let painter = ui.painter().clone();
                let shadow = egui::epaint::Shadow {
                    offset: [0, 4],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha((p.shadow_alpha * 200.0) as u8),
                };
                painter.add(shadow.as_shape(r, card));
                painter.rect_filled(r, card, p.bg_surface3);
                // Accent stripe along the leading edge.
                let stripe =
                    Rect::from_min_max(pos2(r.left() + 1.0, r.top() + 12.0), pos2(r.left() + 4.0, r.bottom() - 12.0));
                painter.rect_filled(stripe, CornerRadius::same(2), color);
                painter.rect_stroke(r, card, Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);

                // Kind icon in a tinted chip.
                let cy = r.center().y;
                let chip = pos2(r.left() + 27.0, cy);
                painter.circle_filled(chip, 13.0, color.gamma_multiply(if p.dark { 0.20 } else { 0.15 }));
                painter.text(chip, Align2::CENTER_CENTER, glyph, icons::font_fill(15.0), color);

                // Right side: the close slot is always reserved (nothing shifts on hover),
                // the action button sits left of it.
                let close_r = Rect::from_center_size(pos2(r.right() - 20.0, cy), vec2(24.0, 24.0));
                let mut text_right = close_r.left() - 4.0;
                if let Some((label, action)) = &l.toast.action {
                    let g = painter.layout_no_wrap(label.clone(), semibold(12.5), p.accent);
                    let w = (g.size().x + 22.0).max(56.0);
                    let ar = Rect::from_center_size(pos2(close_r.left() - 4.0 - w / 2.0, cy), vec2(w, 26.0));
                    let aresp = ui.interact(ar, id.with("a"), Sense::click());
                    crate::ui::a11y::button(&aresp, label);
                    let fill = p.accent.gamma_multiply(if aresp.hovered() { 0.24 } else { 0.13 });
                    painter.rect_filled(ar, CornerRadius::same(radius::ROW), fill);
                    if aresp.has_focus() {
                        painter.rect_stroke(
                            ar,
                            CornerRadius::same(radius::ROW),
                            Stroke::new(1.0, p.accent),
                            StrokeKind::Outside,
                        );
                    }
                    painter.galley(ar.center() - g.size() / 2.0, g, p.accent);
                    if aresp.clicked() {
                        clicked.push(action.clone());
                        close = true;
                    }
                    text_right = ar.left() - 10.0;
                }
                let cresp = ui.interact(close_r, id.with("x"), Sense::click());
                crate::ui::a11y::button(&cresp, tr!("app-toast-dismiss"));
                let show_x = if cresp.has_focus() { 1.0 } else { hover_t };
                if show_x > 0.0 {
                    if cresp.hovered() {
                        painter.rect_filled(close_r, CornerRadius::same(radius::CONTROL), p.bg_surface2);
                    }
                    let c = if cresp.hovered() { p.text_primary } else { p.text_tertiary };
                    painter.text(
                        close_r.center(),
                        Align2::CENTER_CENTER,
                        ph::X,
                        icons::font(13.0),
                        c.gamma_multiply(show_x),
                    );
                }
                if cresp.clicked() {
                    close = true;
                }

                // Title and body: one line each, cut with an ellipsis.
                let text_left = r.left() + 50.0;
                let max_w = (text_right - text_left).max(24.0);
                let title = trunc_galley(&painter, &l.toast.title, semibold(13.0), p.text_primary, max_w);
                let body = l.toast.body.as_deref().map(|b| {
                    let line = one_line(b);
                    let reflowed = line != b.trim();
                    (trunc_galley(&painter, &line, FontId::proportional(12.0), p.text_secondary, max_w), reflowed)
                });
                let clipped = title.elided || body.as_ref().is_some_and(|(g, reflowed)| g.elided || *reflowed);
                let title_y = if body.is_some() { cy - 9.0 } else { cy };
                painter.galley(pos2(text_left, title_y - title.size().y / 2.0), title, p.text_primary);
                if let Some((g, _)) = body {
                    painter.galley(pos2(text_left, cy + 9.0 - g.size().y / 2.0), g, p.text_secondary);
                }
                if clipped {
                    // The whole text on hover (the timer is paused while it is read).
                    let text_rect = Rect::from_min_max(pos2(r.left() + 12.0, r.top()), pos2(text_right, r.bottom()));
                    let tresp = ui.interact(text_rect, id.with("t"), Sense::hover());
                    let (t, b) = (&l.toast.title, &l.toast.body);
                    tresp.on_hover_ui(|ui| {
                        ui.set_max_width(420.0);
                        ui.label(egui::RichText::new(t).font(semibold(13.0)).color(p.text_primary));
                        if let Some(b) = b {
                            ui.label(egui::RichText::new(b).size(12.0).color(p.text_secondary));
                        }
                    });
                }

                // Auto-dismiss timer: paused while hovered, drawn as a thin line.
                if !hovered
                    && l.closing.is_none()
                    && let Some(rem) = l.remaining.as_mut()
                {
                    *rem -= dt;
                    if *rem <= 0.0 {
                        close = true;
                    }
                }
                if let Some(frac) = timer_fraction(l.remaining, l.toast.duration) {
                    let inset = f32::from(radius::CARD);
                    let track = Rect::from_min_max(
                        pos2(r.left() + inset, r.bottom() - 3.0),
                        pos2(r.right() - inset, r.bottom() - 1.0),
                    );
                    let mut bar = track;
                    bar.set_width(track.width() * frac);
                    let alpha = if hovered { 0.30 } else { 0.60 };
                    painter.rect_filled(bar, CornerRadius::same(1), color.gamma_multiply(alpha));
                }
            });
            if close && l.closing.is_none() {
                l.closing = Some(now);
                any_animating = true;
            }
        }
        // Drop finished toasts.
        self.live.retain(|l| l.closing.is_none_or(|t| t.is_nan() || now - t < ANIM && !reduced_motion));
        if any_animating {
            ctx.request_repaint();
        } else if self.live.iter().any(|l| l.remaining.is_some()) {
            // The progress line moves: ~30 fps is plenty for a 2 px bar.
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_collapse() {
        let mut t = Toasts::default();
        t.push(Toast::info("Coming soon"));
        t.push(Toast::info("Coming soon"));
        t.push(Toast::error("Failed").body("why"));
        assert_eq!(t.live.len(), 2);
        let id = t.push(Toast::success("Done").action("Undo", Action::Undo).sticky());
        assert!(t.live.last().unwrap().remaining.is_none());
        t.dismiss(id);
        assert!(t.live.last().unwrap().closing.is_some());
    }

    #[test]
    fn timer_fraction_tracks_remaining_time() {
        assert_eq!(timer_fraction(Some(2.0), Some(4.0)), Some(0.5));
        assert_eq!(timer_fraction(Some(-1.0), Some(4.0)), Some(0.0));
        assert_eq!(timer_fraction(None, None), None);
        assert_eq!(timer_fraction(Some(1.0), Some(0.0)), None);
    }

    #[test]
    fn bodies_collapse_to_one_line() {
        assert_eq!(one_line("a\n  b\tc "), "a b c");
    }
}
