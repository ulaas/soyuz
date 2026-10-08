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

const WIDTH: f32 = 340.0;
const ANIM: f64 = 0.2;
const MAX_VISIBLE: usize = 5;

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
    pub fn show(&mut self, ctx: &egui::Context, area: Rect, reduced_motion: bool) -> Vec<Action> {
        let mut clicked = Vec::new();
        if self.live.is_empty() {
            return clicked;
        }
        let now = ctx.input(|i| i.time);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let p = palette(ctx);
        let mut y = area.bottom() - 12.0;
        let start = self.live.len().saturating_sub(MAX_VISIBLE);
        let mut any_animating = false;
        for l in self.live[start..].iter_mut().rev() {
            if l.born.is_nan() {
                l.born = now;
            }
            if l.closing.is_some_and(f64::is_nan) {
                l.closing = Some(now);
            }
            let appear = if reduced_motion { 1.0 } else { ((now - l.born) / ANIM).clamp(0.0, 1.0) as f32 };
            let vanish = match l.closing {
                Some(t) if !reduced_motion => 1.0 - ((now - t) / ANIM).clamp(0.0, 1.0) as f32,
                Some(_) => 0.0,
                None => 1.0,
            };
            let ease = |x: f32| 1.0 - (1.0 - x).powi(3);
            let a = ease(appear) * ease(vanish);
            if appear < 1.0 || l.closing.is_some() {
                any_animating = true;
            }

            let body_galley = l.toast.body.as_ref().map(|b| {
                let mut job = egui::text::LayoutJob::simple(
                    b.clone(),
                    FontId::proportional(12.0),
                    p.text_secondary,
                    WIDTH - 64.0,
                );
                job.wrap.max_rows = 4;
                ctx.layer_painter(egui::LayerId::background()).layout_job(job)
            });
            let body_h = body_galley.as_ref().map_or(0.0, |g| g.size().y + 4.0);
            let action_h = if l.toast.action.is_some() { 30.0 } else { 0.0 };
            let h = 46.0 + body_h + action_h;
            let slide = (1.0 - a) * 16.0;
            let rect = Rect::from_min_size(pos2(area.right() - WIDTH - 12.0, y - h + slide), vec2(WIDTH, h));
            y -= (h + 8.0) * a.max(0.001);

            let id = Id::new("soyuz.toast").with(l.id);
            let mut close = false;
            Area::new(id).order(Order::Tooltip).fixed_pos(rect.min).interactable(true).show(ctx, |ui| {
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
                let painter = ui.painter();
                let shadow = egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha((p.shadow_alpha * 255.0 * a) as u8),
                };
                painter.add(shadow.as_shape(r, CornerRadius::same(radius::MODAL)));
                painter.rect_filled(r, CornerRadius::same(radius::MODAL), p.bg_surface3.gamma_multiply(a));
                painter.rect_stroke(
                    r,
                    CornerRadius::same(radius::MODAL),
                    Stroke::new(1.0, p.border_strong.gamma_multiply(a)),
                    StrokeKind::Inside,
                );
                let (glyph, color) = match l.toast.kind {
                    ToastKind::Info => (ph::INFO, p.info),
                    ToastKind::Success => (ph::CHECK_CIRCLE, p.success),
                    ToastKind::Warning => (ph::WARNING, p.warning),
                    ToastKind::Error => (ph::WARNING_CIRCLE, p.danger),
                };
                // Accent stripe.
                let stripe = Rect::from_min_size(r.min + vec2(0.0, 10.0), vec2(3.0, r.height() - 20.0));
                painter.rect_filled(stripe, CornerRadius::same(2), color.gamma_multiply(a));
                painter.text(
                    pos2(r.left() + 22.0, r.top() + 23.0),
                    Align2::CENTER_CENTER,
                    glyph,
                    icons::font_fill(18.0),
                    color.gamma_multiply(a),
                );
                let title = trunc_galley(
                    painter,
                    &l.toast.title,
                    FontId::new(13.0, egui::FontFamily::Name(crate::theme::fonts::SEMIBOLD.into())),
                    p.text_primary.gamma_multiply(a),
                    WIDTH - 80.0,
                );
                painter.galley(pos2(r.left() + 40.0, r.top() + 23.0 - title.size().y / 2.0), title, p.text_primary);
                if let Some(g) = body_galley {
                    painter.galley(pos2(r.left() + 40.0, r.top() + 38.0), g, p.text_secondary.gamma_multiply(a));
                }
                // Close button.
                let cr = Rect::from_center_size(pos2(r.right() - 20.0, r.top() + 23.0), vec2(22.0, 22.0));
                let cresp = ui.interact(cr, id.with("x"), Sense::click());
                crate::ui::a11y::button(&cresp, tr!("app-toast-dismiss"));
                if cresp.hovered() {
                    painter.rect_filled(cr, CornerRadius::same(radius::CONTROL), p.bg_surface2);
                }
                painter.text(
                    cr.center(),
                    Align2::CENTER_CENTER,
                    ph::X,
                    icons::font(13.0),
                    p.text_tertiary.gamma_multiply(a),
                );
                if cresp.clicked() {
                    close = true;
                }
                if let Some((label, action)) = &l.toast.action {
                    let ar = Rect::from_min_size(pos2(r.left() + 40.0, r.bottom() - 34.0), vec2(0.0, 24.0));
                    let g = ui.painter().layout_no_wrap(label.clone(), FontId::proportional(12.5), p.accent);
                    let ar = Rect::from_min_size(ar.min, vec2(g.size().x + 20.0, 24.0));
                    let aresp = ui.interact(ar, id.with("a"), Sense::click());
                    crate::ui::a11y::button(&aresp, label);
                    let fill =
                        if aresp.hovered() { p.accent.gamma_multiply(0.22) } else { p.accent.gamma_multiply(0.12) };
                    painter.rect_filled(ar, CornerRadius::same(radius::ROW), fill.gamma_multiply(a));
                    painter.galley(pos2(ar.left() + 10.0, ar.center().y - g.size().y / 2.0), g, p.accent);
                    if aresp.clicked() {
                        clicked.push(action.clone());
                        close = true;
                    }
                }
                let hovered = resp.hovered() || ui.rect_contains_pointer(r);
                if !hovered
                    && l.closing.is_none()
                    && let Some(rem) = l.remaining.as_mut()
                {
                    *rem -= dt;
                    if *rem <= 0.0 {
                        close = true;
                    }
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
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
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
}
