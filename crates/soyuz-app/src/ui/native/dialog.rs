//! A modal dialog in its own OS window: the app's own title bar (icon, title, close
//! button; drag to move), body below it, centered over the window it was opened from
//! (or where the user last left a dialog of the same kind this session), owned by that
//! window, which is disabled and dimmed while the dialog is open.
//!
//! Callers keep their in-window [`egui::Modal`] path for when [`super::active`] is false.

use std::cell::RefCell;
use std::collections::HashMap;

use egui::{Align2, Color32, Context, Id, Key, Order, Rect, Sense, Ui, UiBuilder, Vec2, ViewportCommand, pos2, vec2};

use super::{Kind, Placement, geometry};
use crate::theme::{icons, palette};

/// Height of the dialog title bar (points).
pub const TITLE_H: f32 = 40.0;

/// Remembered per dialog kind for this session.
#[derive(Clone, Copy, Default)]
struct Memory {
    /// Last outer top-left (physical pixels) once the user saw it.
    pos: Option<egui::Pos2>,
    /// User-resized size (points), resizable dialogs only.
    size: Option<Vec2>,
    /// Measured body height (points), auto-sized dialogs.
    body_h: f32,
}

thread_local! {
    static MEM: RefCell<HashMap<Id, Memory>> = RefCell::new(HashMap::new());
}

fn mem(kind: Id) -> Memory {
    MEM.with(|m| m.borrow().get(&kind).copied().unwrap_or_default())
}

fn set_mem(kind: Id, f: impl FnOnce(&mut Memory)) {
    MEM.with(|m| f(m.borrow_mut().entry(kind).or_default()));
}

/// Outcome of one frame of a native dialog.
pub struct Shown<R> {
    /// `None`: the window did not draw this frame (still being created).
    pub inner: Option<R>,
    /// Close button, Esc, or Alt+F4.
    pub close: bool,
}

/// A native dialog window; configure, then [`show`](Self::show) every frame.
pub struct DialogWindow<'a> {
    /// Identity of the window (viewport) — unique among open dialogs.
    pub id: Id,
    /// Kind of dialog (remembered position / size) — e.g. the Rust type name.
    pub kind: Id,
    pub title: &'a str,
    pub icon: Option<&'static str>,
    pub alert: bool,
    /// Body width (points) of an auto-sized dialog.
    pub width: f32,
    /// `Some(default size)`: a resizable dialog whose body fills the window.
    pub resizable: Option<Vec2>,
    /// Body margin (points).
    pub margin: f32,
    pub fill: Color32,
}

impl<'a> DialogWindow<'a> {
    pub fn new(id: Id, kind: Id, title: &'a str) -> Self {
        DialogWindow {
            id,
            kind,
            title,
            icon: None,
            alert: false,
            width: 420.0,
            resizable: None,
            margin: 20.0,
            fill: Color32::TRANSPARENT,
        }
    }

    pub fn show<R>(self, ctx: &Context, body: impl FnOnce(&mut Ui) -> R) -> Shown<R> {
        let p = palette(ctx);
        let fill = if self.fill == Color32::TRANSPARENT { p.bg_surface3 } else { self.fill };
        let owner_vp = ctx.viewport_id();
        let vp = super::viewport_id(Kind::Dialog, self.id);
        let m = mem(self.kind);
        let ppp = if super::is_shown(vp) { super::ppp_of(ctx, vp) } else { super::ppp_of(ctx, owner_vp) };
        // Size in points.
        let size = match self.resizable {
            Some(default) => m.size.unwrap_or(default),
            None => vec2(self.width + self.margin * 2.0, TITLE_H + m.body_h.max(60.0) + self.margin),
        };
        let measured_h = m.body_h;
        let size_px = size * ppp;
        // Position (physical pixels): remembered, else centered over the owner.
        let owner = super::outer_px(ctx, owner_vp).unwrap_or(Rect::from_min_size(egui::Pos2::ZERO, size_px));
        let rect_px = match m.pos {
            Some(pos) => {
                let work = super::work_area_at(ctx, owner_vp, pos);
                geometry::keep_reachable(Rect::from_min_size(pos, size_px), work, TITLE_H * ppp)
            }
            None => {
                let work = super::work_area_at(ctx, owner_vp, owner.center());
                geometry::center_over(owner, geometry::fit_size(size_px, work), work)
            }
        };
        let place = Placement {
            kind: Kind::Dialog,
            title: self.title.to_string(),
            rect_px,
            keep_size: self.resizable.is_none(),
            resizable: self.resizable.is_some(),
            min_size: if self.resizable.is_some() { vec2(480.0, 320.0) } else { vec2(200.0, 80.0) },
            modal: true,
        };

        // Owner: dimmed, clicks go to the dialog (the OS also refuses input to the
        // disabled owner and flashes the dialog).
        dim_owner(ctx, self.id, vp);

        let (title, icon, alert, margin, width, resizable, kind) =
            (self.title, self.icon, self.alert, self.margin, self.width, self.resizable.is_some(), self.kind);
        let out = super::show(ctx, vp, &place, |ui| {
            let ctx = ui.ctx().clone();
            let full = ctx.content_rect();
            let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
            ui.painter().rect_filled(full, 0.0, fill);
            crate::ui::a11y::dialog(&ctx, ui.unique_id(), title, alert, true);

            // Title bar: drag to move, close button.
            let bar = Rect::from_min_size(full.min, vec2(full.width(), TITLE_H));
            let close_r = Rect::from_min_max(pos2(bar.right() - crate::ui::titlebar::CONTROL_W, bar.top()), bar.max);
            let drag = ui.interact(
                Rect::from_min_max(bar.min, pos2(close_r.left(), bar.bottom())),
                Id::new("soyuz.dialog.drag"),
                Sense::click_and_drag(),
            );
            if drag.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            let mut x = bar.left() + margin;
            if let Some(g) = icon {
                ui.painter().text(pos2(x, bar.center().y), Align2::LEFT_CENTER, g, icons::font(17.0), p.accent);
                x += 26.0;
            }
            let fg = if focused { p.text_primary } else { p.text_secondary };
            let galley = crate::ui::widgets::trunc_galley(
                ui.painter(),
                title,
                crate::ui::views::flow::semibold(15.0),
                fg,
                (close_r.left() - x - 8.0).max(20.0),
            );
            ui.painter().galley(pos2(x, bar.center().y - galley.size().y / 2.0), galley, fg);
            let mut close =
                crate::ui::titlebar::caption_close(ui, close_r, Id::new("soyuz.dialog.close"), focused, true).clicked();

            // Body.
            let body_rect = if resizable {
                Rect::from_min_max(
                    pos2(full.left() + margin, bar.bottom()),
                    pos2(full.right() - margin, full.bottom() - margin),
                )
            } else {
                Rect::from_min_size(
                    pos2(full.left() + margin, bar.bottom() + 2.0),
                    vec2(width, if measured_h > 0.0 { measured_h } else { 300.0 }),
                )
            };
            // Like an egui area: an invisible sizing pass first, then the measured height.
            let mut b = UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min));
            if !resizable && measured_h <= 0.0 {
                b = b.sizing_pass().invisible();
            }
            let mut child = ui.new_child(b);
            if !resizable {
                child.set_width(width);
            }
            let inner = body(&mut child);
            let body_h = child.min_rect().height().clamp(1.0, 4000.0);

            crate::ui::titlebar::border_and_resize(&ctx, focused, resizable);

            // Esc (unless the body used it), Alt+F4 / OS close.
            let esc = ctx.input(|i| i.key_pressed(Key::Escape)) && !crate::ui::widgets::dropdown::any_open(&ctx);
            if ctx.input(|i| i.viewport().close_requested()) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                close = true;
            }
            close |= esc;

            // Remember where it is (and the user's size for resizable dialogs).
            let (inner_rect, vppp) = ctx.input(|i| (i.viewport().inner_rect, i.pixels_per_point));
            let shown = super::is_shown(ctx.viewport_id());
            set_mem(kind, |m| {
                if !resizable && (m.body_h - body_h).abs() > 0.5 {
                    m.body_h = body_h;
                    ctx.request_repaint();
                }
                if shown && let Some(r) = inner_rect {
                    m.pos = Some((r.min.to_vec2() * vppp).to_pos2());
                    if resizable {
                        m.size = Some(r.size());
                    }
                }
            });
            (inner, close)
        });
        match out {
            Some((inner, close)) => Shown { inner: Some(inner), close },
            None => Shown { inner: None, close: false },
        }
    }
}

/// Dim the owner window and send clicks on it to the dialog.
fn dim_owner(ctx: &Context, id: Id, vp: egui::ViewportId) {
    let p = palette(ctx);
    let t =
        ctx.animate_bool_with_time(id.with("dim"), true, crate::theme::anim(ctx, crate::theme::tokens::motion::MODAL));
    let screen = ctx.content_rect();
    egui::Area::new(id.with("native.dim")).order(Order::Foreground).fixed_pos(screen.min).constrain(false).show(
        ctx,
        |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha((if p.dark { 90.0 } else { 50.0 } * t) as u8));
            if resp.clicked() || resp.drag_started() {
                ctx.send_viewport_cmd_to(vp, ViewportCommand::Focus);
            }
        },
    );
}
