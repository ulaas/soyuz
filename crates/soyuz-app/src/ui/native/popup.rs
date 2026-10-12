//! Popup menus that leave the window when they do not fit inside it.
//!
//! [`Menu`] mirrors the [`egui::Popup`] builder the app uses (`menu`, `context_menu`,
//! `from_toggle_button_response`, `new` + `align` / `width` / `close_behavior` / …).
//! While the popup fits the window it *is* an `egui::Popup` (cheap, identical to before).
//! egui measures a popup invisibly on the frame it opens; when no alignment of that size
//! fits the window, the popup switches to a borderless native window
//! ([`super::show`], kind [`Kind::Popup`]) placed on the monitor's work area (flipped /
//! shifted by [`geometry::place`]). It closes like the in-window one: item activation
//! (`ui.close()`), Esc, a click elsewhere (per [`PopupCloseBehavior`]), plus focus loss
//! and the owner window moving or minimizing.
//!
//! [`submenu`] does the same for submenus (always native inside a native popup).

use std::cell::RefCell;
use std::collections::HashMap;

use egui::containers::menu::{MenuConfig, MenuState, SubMenu, menu_style};
use egui::{
    Context, Frame, Id, InnerResponse, Key, LayerId, Layout, Popup, PopupAnchor, PopupCloseBehavior, PopupKind, Pos2,
    Rect, RectAlign, Response, ScrollArea, SetOpenCommand, Ui, UiBuilder, UiStackInfo, Vec2, ViewportId, vec2,
};

use super::geometry::{self, Side};
use super::{Kind, Placement};

/// Seconds without focus on the popup (or its owner) before it closes.
const FOCUS_GRACE: f64 = 0.3;
/// Seconds the pointer rests on a submenu row before its native submenu opens, and on
/// another entry before an open one closes: passing over a row must not create and
/// destroy a window (a visible flash), and a diagonal move towards the submenu may
/// cross a neighbouring entry.
const SUBMENU_OPEN_DELAY: f64 = 0.2;
const SUBMENU_CLOSE_DELAY: f64 = 0.3;

#[derive(Clone, Copy)]
enum Anchor {
    Rect(Rect),
    PointerFixed,
    Position(Pos2),
}

/// Per native popup (keyed by popup id).
struct PopState {
    vp: ViewportId,
    owner_vp: ViewportId,
    /// Anchor in physical screen pixels.
    anchor_px: Rect,
    side: Side,
    gap: f32,
    /// Window size in points (content + frame); `None` until measured.
    size: Option<Vec2>,
    opened_at: f64,
    owner_rect: Option<Rect>,
    unfocused_since: Option<f64>,
    touched: u64,
    activate: bool,
}

#[derive(Default)]
struct State {
    pops: HashMap<Id, PopState>,
    /// Open native submenu (row id) per parent menu.
    subs: HashMap<Id, Id>,
    /// Native popups being drawn (innermost last): submenus find their parent here.
    stack: Vec<Id>,
    /// A submenu row was clicked in this native popup this frame (no click-close).
    sub_clicked: Vec<Id>,
    /// Per parent menu: the submenu row under the pointer, since when, and the frame it
    /// was last seen there.
    hover: HashMap<Id, (Id, f64, u64)>,
    /// Per open native submenu: since when the pointer is on another entry of its parent.
    away: HashMap<Id, f64>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

/// Forget native popups that were not drawn this frame (end of the main frame).
pub fn end_frame() {
    let now = super::frame();
    with(|s| {
        s.pops.retain(|_, p| p.touched == now);
        let live: Vec<Id> = s.pops.keys().copied().collect();
        s.subs.retain(|_, row| live.contains(&SubMenu::id_from_widget_id(*row).with("native")));
        s.away.retain(|key, _| live.contains(key));
        s.hover.retain(|_, h| h.2 == now);
        s.stack.clear();
        s.sub_clicked.clear();
    });
}

/// A popup / menu that becomes a native window when it does not fit. See the module
/// docs; the builder methods match [`egui::Popup`]'s.
#[must_use = "call `.show()`"]
pub struct Menu {
    id: Id,
    ctx: Context,
    anchor: Anchor,
    layer: LayerId,
    align: RectAlign,
    set: Option<SetOpenCommand>,
    close_behavior: PopupCloseBehavior,
    kind: PopupKind,
    width: Option<f32>,
    gap: f32,
    frame: Option<Frame>,
    layout: Layout,
    menu_style: bool,
}

fn anchor_rect(resp: &Response) -> Rect {
    match PopupAnchor::from(resp) {
        PopupAnchor::ParentRect(r) => r,
        _ => resp.rect,
    }
}

impl Menu {
    /// Like [`Popup::new`] (always open; drive it with [`open_memory`](Self::open_memory)).
    pub fn new(id: Id, ctx: Context, anchor: Rect, layer: LayerId) -> Self {
        Menu {
            id,
            ctx,
            anchor: Anchor::Rect(anchor),
            layer,
            align: RectAlign::BOTTOM_START,
            set: None,
            close_behavior: PopupCloseBehavior::CloseOnClick,
            kind: PopupKind::Popup,
            width: None,
            gap: 0.0,
            frame: None,
            layout: Layout::default(),
            menu_style: false,
        }
    }

    /// Like [`Popup::from_toggle_button_response`].
    pub fn from_toggle_button_response(resp: &Response) -> Self {
        let mut m = Menu::new(Popup::default_response_id(resp), resp.ctx.clone(), anchor_rect(resp), resp.layer_id);
        m.set = resp.clicked().then_some(SetOpenCommand::Toggle);
        m
    }

    /// Like [`Popup::menu`].
    #[allow(clippy::self_named_constructors)] // mirrors `egui::Popup::menu`
    pub fn menu(resp: &Response) -> Self {
        let mut m = Menu::from_toggle_button_response(resp);
        m.kind = PopupKind::Menu;
        m.layout = Layout::top_down_justified(egui::Align::Min);
        m.menu_style = true;
        m
    }

    /// Like [`Popup::context_menu`] (opens at the pointer on a secondary click).
    pub fn context_menu(resp: &Response) -> Self {
        let mut m = Menu::menu(resp);
        m.set = if resp.secondary_clicked() {
            Some(SetOpenCommand::Bool(true))
        } else if resp.clicked() {
            Some(SetOpenCommand::Bool(false))
        } else {
            None
        };
        m.anchor = Anchor::PointerFixed;
        m
    }

    pub fn kind(mut self, kind: PopupKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn open_memory(mut self, set: impl Into<Option<SetOpenCommand>>) -> Self {
        self.set = set.into();
        self
    }

    pub fn align(mut self, align: RectAlign) -> Self {
        self.align = align;
        self
    }

    pub fn close_behavior(mut self, b: PopupCloseBehavior) -> Self {
        self.close_behavior = b;
        self
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn id(mut self, id: Id) -> Self {
        self.id = id;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn frame(mut self, frame: Frame) -> Self {
        self.frame = Some(frame);
        self
    }

    pub fn layout(mut self, layout: Layout) -> Self {
        self.layout = layout;
        self
    }

    pub fn at_position(mut self, pos: Pos2) -> Self {
        self.anchor = Anchor::Position(pos);
        self
    }

    fn egui_popup(&self) -> Popup<'static> {
        let mut p = Popup::new(self.id, self.ctx.clone(), Rect::NOTHING, self.layer)
            .kind(self.kind)
            .open_memory(self.set)
            .align(self.align)
            .close_behavior(self.close_behavior)
            .gap(self.gap)
            .layout(self.layout);
        p = match self.anchor {
            Anchor::Rect(r) => p.anchor(r),
            Anchor::PointerFixed => p.at_pointer_fixed(),
            Anchor::Position(pos) => p.at_position(pos),
        };
        if self.menu_style {
            p = p.style(menu_style);
        }
        if let Some(w) = self.width {
            p = p.width(w);
        }
        if let Some(f) = self.frame {
            p = p.frame(f);
        }
        p
    }

    fn frame_or_default(&self) -> Frame {
        self.frame.unwrap_or_else(|| {
            let mut style = (*self.ctx.global_style()).clone();
            if self.menu_style {
                menu_style(&mut style);
            }
            Frame::popup(&style)
        })
    }

    /// The anchor rect (points) as egui would use it.
    fn anchor_points(&self) -> Option<Rect> {
        match self.anchor {
            Anchor::Rect(r) => Some(r),
            Anchor::PointerFixed => Popup::position_of_id(&self.ctx, self.id).map(Rect::from_pos),
            Anchor::Position(p) => Some(Rect::from_pos(p)),
        }
    }

    /// Would egui find an alignment of `size` that fits the window?
    fn fits_in_window(&self, size: Vec2) -> bool {
        let Some(anchor) = self.anchor_points() else { return true };
        let content = self.ctx.content_rect();
        std::iter::once(self.align)
            .chain(self.align.symmetries().iter().copied())
            .chain(RectAlign::MENU_ALIGNS.iter().copied())
            .any(|a| content.contains_rect(a.align_rect(&anchor, size, self.gap)))
    }

    fn side(&self) -> Side {
        match self.anchor {
            Anchor::PointerFixed | Anchor::Position(_) => Side::Point,
            Anchor::Rect(_) if self.align == RectAlign::BOTTOM_END => Side::BelowEnd,
            Anchor::Rect(_) if self.align == RectAlign::RIGHT_START => Side::Right,
            Anchor::Rect(_) => Side::Below,
        }
    }

    pub fn show<R>(self, content: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
        let ctx = self.ctx.clone();
        let id = self.id;
        let native = with(|s| s.pops.contains_key(&id));
        if !super::active(&ctx) {
            if native {
                with(|s| s.pops.remove(&id));
            }
            return self.egui_popup().show(content);
        }
        if native {
            // Commands while native: the toggle button / a click on the widget closes;
            // a new right-click reopens in-window at the new pointer position.
            match self.set {
                Some(SetOpenCommand::Toggle) | Some(SetOpenCommand::Bool(false)) => {
                    close(&ctx, id);
                    return None;
                }
                Some(SetOpenCommand::Bool(true)) => {
                    with(|s| s.pops.remove(&id));
                    return self.egui_popup().show(content);
                }
                None => {}
            }
            if !Popup::is_id_open(&ctx, id) {
                // Closed by someone else (Popup::close_id / close_all / another popup).
                with(|s| s.pops.remove(&id));
                return None;
            }
            return self.show_native(content);
        }
        // In-window. After egui's invisible sizing frame we know the size: switch to a
        // native window when no alignment fits.
        let open_now = Popup::is_id_open(&ctx, id);
        let opening = matches!(self.set, Some(SetOpenCommand::Toggle) | Some(SetOpenCommand::Bool(_)));
        let shown_last = ctx.read_response(id).is_some();
        if open_now
            && !opening
            && shown_last
            && let Some(size) = egui::AreaState::load(&ctx, id).and_then(|a| a.size)
            && !self.fits_in_window(size)
            && let Some(anchor) = self.anchor_points()
        {
            let owner_vp = ctx.viewport_id();
            let ppp = super::ppp_of(&ctx, owner_vp);
            let Some(inner) = super::inner_px(&ctx, owner_vp) else { return self.egui_popup().show(content) };
            let anchor_px = Rect::from_min_size(inner.min + anchor.min.to_vec2() * ppp, anchor.size() * ppp);
            let now = ctx.input(|i| i.time);
            with(|s| {
                s.pops.insert(
                    id,
                    PopState {
                        vp: super::viewport_id(Kind::Popup, id),
                        owner_vp,
                        anchor_px,
                        side: self.side(),
                        gap: self.gap * ppp,
                        size: Some(size),
                        opened_at: now,
                        owner_rect: super::outer_px(&ctx, owner_vp),
                        unfocused_since: None,
                        touched: super::frame(),
                        activate: true,
                    },
                )
            });
            log::debug!("popup {id:?} ({size:?}) does not fit the window: native");
            return self.show_native(content);
        }
        self.egui_popup().show(content)
    }

    fn show_native<R>(self, content: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
        let ctx = self.ctx.clone();
        let id = self.id;
        // Keep egui's open state (buttons highlight with `Popup::is_id_open`).
        Popup::open_id(&ctx, id);
        // A click in the owner window (not the one that opened it) closes.
        let owner_click = ctx.input(|i| i.pointer.any_click());
        let r = render(
            &ctx,
            id,
            &self.frame_or_default(),
            self.layout,
            self.menu_style,
            self.kind,
            self.close_behavior,
            content,
        );
        let close_now =
            r.close_all || r.esc || (owner_click && self.close_behavior != PopupCloseBehavior::IgnoreClicks) || r.lost;
        if close_now {
            close(&ctx, id);
        }
        r.inner.map(|(inner, response)| InnerResponse { inner, response })
    }
}

fn close(ctx: &Context, id: Id) {
    Popup::close_id(ctx, id);
    with(|s| {
        s.pops.remove(&id);
        s.subs.remove(&id);
    });
}

/// Result of one native popup frame.
struct Rendered<R> {
    inner: Option<(R, Response)>,
    /// `ui.close()` or a click inside (CloseOnClick).
    close_all: bool,
    esc: bool,
    /// Focus went elsewhere, or the owner moved / minimized.
    lost: bool,
}

/// Draw the native popup `id` (its [`PopState`] must exist).
#[allow(clippy::too_many_arguments)]
fn render<R>(
    ctx: &Context,
    id: Id,
    frame: &Frame,
    layout: Layout,
    use_menu_style: bool,
    kind: PopupKind,
    close_behavior: PopupCloseBehavior,
    content: impl FnOnce(&mut Ui) -> R,
) -> Rendered<R> {
    let now_frame = super::frame();
    let Some((vp, owner_vp, anchor_px, side, gap, size, opened_at, owner_rect, activate)) = with(|s| {
        s.pops.get_mut(&id).map(|p| {
            p.touched = now_frame;
            (p.vp, p.owner_vp, p.anchor_px, p.side, p.gap, p.size, p.opened_at, p.owner_rect, p.activate)
        })
    }) else {
        return Rendered { inner: None, close_all: true, esc: false, lost: false };
    };
    let now = ctx.input(|i| i.time);
    let ppp = if super::is_shown(vp) { super::ppp_of(ctx, vp) } else { super::ppp_of(ctx, owner_vp) };
    let margin = frame.total_margin();
    let sizing = size.is_none();
    let size_pts = size.unwrap_or(vec2(ctx.global_style().spacing.default_area_size.x, 200.0));
    let work = super::work_area_at(ctx, owner_vp, anchor_px.center());
    let rect_px = geometry::place(anchor_px, size_pts * ppp, work, side, gap);
    let capped = rect_px.height() + 0.5 < size_pts.y * ppp;
    let place = Placement {
        kind: Kind::Popup,
        title: String::new(),
        rect_px,
        keep_size: true,
        resizable: false,
        min_size: vec2(8.0, 8.0),
        modal: false,
    };
    super::set_activate(vp, activate);
    let style = (*ctx.global_style()).clone();
    let out = super::show(ctx, vp, &place, |ui| {
        let ctx = ui.ctx().clone();
        let full = ctx.content_rect();
        ui.painter().rect_filled(full, 0.0, frame.fill);
        if !crate::ui::native_chrome::is_windows_11() {
            ui.painter().rect_stroke(full, 0.0, frame.stroke, egui::StrokeKind::Inside);
        }
        let inner_rect = Rect::from_min_max(full.min + margin.left_top(), full.max - margin.right_bottom());
        let mut cfg_style = style.clone();
        if use_menu_style {
            menu_style(&mut cfg_style);
        }
        let info = UiStackInfo::new(kind.into()).with_tag_value(
            MenuConfig::MENU_CONFIG_TAG,
            MenuConfig::new().close_behavior(close_behavior).style(if use_menu_style {
                egui::style::StyleModifier::from(menu_style as fn(&mut egui::Style))
            } else {
                egui::style::StyleModifier::default()
            }),
        );
        let max_rect = if sizing {
            Rect::from_min_size(inner_rect.min, vec2(size_pts.x - margin.sum().x, f32::INFINITY))
        } else {
            inner_rect
        };
        let mut b = UiBuilder::new().max_rect(max_rect).layout(layout).ui_stack_info(info).closable();
        if sizing {
            b = b.sizing_pass().invisible();
        }
        let mut child = ui.new_child(b);
        *child.style_mut() = cfg_style;
        with(|s| s.stack.push(id));
        let (inner, content_size) = if capped {
            let o = ScrollArea::vertical()
                .max_height(inner_rect.height())
                .auto_shrink([false, true])
                .show(&mut child, content);
            (o.inner, vec2(o.content_size.x, o.content_size.y))
        } else {
            let r = content(&mut child);
            (r, child.min_rect().size())
        };
        with(|s| s.stack.pop());
        let sub_clicked = with(|s| {
            let k = s.sub_clicked.iter().position(|x| *x == id);
            k.map(|k| s.sub_clicked.remove(k)).is_some()
        });
        let measured =
            vec2(if capped { size_pts.x } else { content_size.x + margin.sum().x }, content_size.y + margin.sum().y);
        let esc = ctx.input(|i| i.key_pressed(Key::Escape));
        let click_inside = ctx.input(|i| i.pointer.any_click()) && !sub_clicked;
        let should_close = child.should_close();
        let response = child.response();
        (inner, response, measured, esc, click_inside, should_close)
    });
    let Some((inner, response, measured, esc, click_inside, should_close)) = out else {
        return Rendered { inner: None, close_all: false, esc: false, lost: false };
    };
    with(|s| {
        if let Some(p) = s.pops.get_mut(&id)
            && p.size.is_none_or(|old| (old - measured).length() > 0.5)
        {
            p.size = Some(measured);
            ctx.request_repaint();
        }
    });
    let settled = now - opened_at > 0.05 && super::is_shown(vp);
    let close_all = should_close || (settled && click_inside && close_behavior == PopupCloseBehavior::CloseOnClick);

    // Focus: the popup, a submenu of it, or the owner window must have it.
    let mut lost = false;
    if settled && !super::quiet() {
        let focused = |v: ViewportId| ctx.input_for(v, |i| i.viewport().focused) == Some(true);
        let chain_focused = focused(vp)
            || focused(owner_vp)
            || super::popup_focused_for(ctx, vp)
            || super::popup_focused_for(ctx, owner_vp);
        let since = with(|s| {
            let p = s.pops.get_mut(&id)?;
            if chain_focused {
                p.unfocused_since = None;
            } else {
                p.unfocused_since.get_or_insert(now);
            }
            p.unfocused_since
        });
        if let Some(t) = since {
            if now - t > FOCUS_GRACE {
                lost = true;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(FOCUS_GRACE));
            }
        }
    }
    // The owner moved or minimized.
    if let (Some(h), Some(before)) = (super::hwnd_of(ctx, owner_vp), owner_rect) {
        let moved = soyuz_fs::chrome::owned::window_rect(h)
            .map(geometry::ltrb)
            .is_some_and(|r| (r.min - before.min).length() > 2.0);
        if moved || soyuz_fs::chrome::owned::is_minimized(h) {
            lost = true;
        }
    }
    Rendered { inner: Some((inner, response)), close_all, esc, lost }
}

/// The parent menu key of a submenu row drawn in `ui`: the native popup being drawn,
/// else the in-window egui menu.
fn parent_key(ui: &Ui) -> Option<(Id, bool)> {
    if let Some(top) = with(|s| s.stack.last().copied())
        && super::is_native(ui.ctx().viewport_id())
    {
        return Some((top, true));
    }
    egui::containers::menu::is_in_menu(ui).then(|| (MenuState::from_ui(ui, |_, stack| stack.id), false))
}

/// A submenu for the menu row `row` (see [`crate::ui::widgets::submenu`]): native inside
/// a native popup, or when the in-window submenu would not fit the window.
pub fn submenu<R>(ui: &mut Ui, row: &Response, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let ctx = ui.ctx().clone();
    let Some((parent, parent_native)) = parent_key(ui).filter(|_| super::active(&ctx)) else {
        return SubMenu::new().show(ui, row, content).map(|r| r.inner);
    };
    let frame = Frame::menu(ui.style());
    let egui_id = SubMenu::id_from_widget_id(row.id);
    let key = egui_id.with("native");
    let gap = frame.total_margin().sum().x / 2.0 + 2.0;
    let expand = vec2(0.0, frame.total_margin().sum().y / 2.0);
    let anchor = row.interact_rect.expand2(expand);
    let is_open = with(|s| s.subs.get(&parent) == Some(&row.id));
    if !parent_native && !is_open {
        // In-window parent: egui's submenu unless it would not fit.
        let fits = egui::AreaState::load(&ctx, egui_id).and_then(|a| a.size).is_none_or(|size| {
            let content = ctx.content_rect();
            std::iter::once(RectAlign::RIGHT_START)
                .chain(RectAlign::RIGHT_START.symmetries().iter().copied())
                .chain(RectAlign::MENU_ALIGNS.iter().copied())
                .any(|a| content.contains_rect(a.align_rect(&anchor, size, gap)))
        });
        if fits {
            return SubMenu::new().show(ui, row, content).map(|r| r.inner);
        }
        // Measured and too big for the window: a native submenu, never egui's (it would
        // be drawn clipped while the native one waits for its delay).
        MenuState::from_ui(ui, |st, _| {
            if st.open_item == Some(egui_id) {
                st.open_item = None;
            }
        });
    }
    let now = ctx.input(|i| i.time);
    let hover = ctx.pointer_hover_pos();
    let row_zone = row.rect.expand2(ui.style().spacing.item_spacing / 2.0);
    let on_row = hover.is_some_and(|p| row_zone.contains(p));
    let rested = with(|s| {
        if on_row {
            let frame = super::frame();
            let h = s.hover.entry(parent).or_insert((row.id, now, frame));
            if h.0 != row.id {
                *h = (row.id, now, frame);
            }
            h.2 = frame;
            now - h.1
        } else {
            if s.hover.get(&parent).is_some_and(|h| h.0 == row.id) {
                s.hover.remove(&parent);
            }
            0.0
        }
    });
    let keyboard_open = row.has_focus() && ui.input(|i| i.key_pressed(Key::ArrowRight));
    let clicked = row.clicked();
    if clicked {
        with(|s| s.sub_clicked.extend(s.stack.last().copied()));
    }
    let is_open = with(|s| s.subs.get(&parent) == Some(&row.id));
    let hover_open = on_row && rested >= SUBMENU_OPEN_DELAY;
    if !is_open && ui.is_enabled() && on_row && !hover_open {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SUBMENU_OPEN_DELAY - rested));
    }
    if !is_open && ui.is_enabled() && (hover_open || clicked || keyboard_open) {
        let by_keyboard = keyboard_open || (clicked && !row.clicked_by(egui::PointerButton::Primary));
        let size = if parent_native { None } else { egui::AreaState::load(&ctx, egui_id).and_then(|a| a.size) };
        open_sub(&ctx, ui, parent, row, key, anchor, gap, size, by_keyboard);
    }
    if !with(|s| s.subs.get(&parent) == Some(&row.id)) {
        return None;
    }
    // Hovering another entry of the parent menu closes this submenu.
    let parent_rect = if parent_native {
        Some(ctx.content_rect())
    } else {
        ctx.read_response(parent).map(|r| r.rect - frame.total_margin())
    };
    if hover.is_some_and(|p| parent_rect.is_some_and(|r| r.contains(p))) && !on_row {
        let since = with(|s| *s.away.entry(key).or_insert(now));
        if now - since >= SUBMENU_CLOSE_DELAY {
            close_sub(&ctx, parent, key);
            return None;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SUBMENU_CLOSE_DELAY - (now - since)));
    } else {
        with(|s| s.away.remove(&key));
    }
    // Follow the row (the parent may scroll or re-layout).
    let owner_vp = ctx.viewport_id();
    let ppp = super::ppp_of(&ctx, owner_vp);
    if let Some(inner) = super::inner_px(&ctx, owner_vp) {
        let anchor_px = Rect::from_min_size(inner.min + anchor.min.to_vec2() * ppp, anchor.size() * ppp);
        with(|s| {
            if let Some(p) = s.pops.get_mut(&key) {
                p.anchor_px = anchor_px;
            }
        });
    }
    let r = render(
        &ctx,
        key,
        &frame,
        Layout::top_down_justified(egui::Align::Min),
        true,
        PopupKind::Menu,
        PopupCloseBehavior::CloseOnClick,
        |ui| {
            // ← closes the submenu from the keyboard.
            if ui.input(|i| i.key_pressed(Key::ArrowLeft)) {
                ui.data_mut(|d| d.insert_temp(Id::new("soyuz.submenu.left"), true));
            }
            content(ui)
        },
    );
    let left = ctx.data_mut(|d| d.remove_temp::<bool>(Id::new("soyuz.submenu.left"))).unwrap_or(false);
    if r.close_all {
        // An item was chosen: close the whole menu.
        close_sub(&ctx, parent, key);
        ui.close();
    } else if r.esc || left || r.lost {
        close_sub(&ctx, parent, key);
        if r.esc || left {
            ctx.send_viewport_cmd_to(owner_vp, egui::ViewportCommand::Focus);
        }
    }
    r.inner.map(|(inner, _)| inner)
}

#[allow(clippy::too_many_arguments)]
fn open_sub(
    ctx: &Context,
    ui: &Ui,
    parent: Id,
    row: &Response,
    key: Id,
    anchor: Rect,
    gap: f32,
    size: Option<Vec2>,
    activate: bool,
) {
    let owner_vp = ctx.viewport_id();
    let ppp = super::ppp_of(ctx, owner_vp);
    let Some(inner) = super::inner_px(ctx, owner_vp) else { return };
    let anchor_px = Rect::from_min_size(inner.min + anchor.min.to_vec2() * ppp, anchor.size() * ppp);
    let now = ctx.input(|i| i.time);
    let _ = ui;
    log::debug!("native submenu {key:?} opens (keyboard: {activate})");
    with(|s| {
        if let Some(old) = s.subs.insert(parent, row.id)
            && old != row.id
        {
            s.pops.remove(&SubMenu::id_from_widget_id(old).with("native"));
        }
        s.pops.insert(
            key,
            PopState {
                vp: super::viewport_id(Kind::Popup, key),
                owner_vp,
                anchor_px,
                side: Side::Right,
                gap: gap * ppp,
                size,
                opened_at: now,
                owner_rect: super::outer_px(ctx, owner_vp),
                unfocused_since: None,
                touched: super::frame(),
                activate,
            },
        );
    });
}

fn close_sub(_ctx: &Context, parent: Id, key: Id) {
    log::debug!("native submenu {key:?} closes");
    with(|s| {
        s.subs.remove(&parent);
        s.pops.remove(&key);
        s.away.remove(&key);
    });
}

/// `crate::ui::native::popup::context_menu(&resp, f)` that can leave the window.
pub fn context_menu<R>(resp: &Response, content: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
    Menu::context_menu(resp).show(content)
}

/// Result of [`native_area`].
pub struct AreaOut<R> {
    pub inner: Option<R>,
    /// Focus went to another app / window, or the owner moved: close it.
    pub lost: bool,
}

/// A caller-managed popup (the dropdown list) in a native window of `size` points next
/// to `anchor` (points in the current window), drawn with `frame`. Open / close state
/// stays with the caller: stop calling this to close it.
pub fn native_area<R>(
    ctx: &Context,
    key: Id,
    anchor: Rect,
    side: Side,
    size: Vec2,
    frame: &Frame,
    content: impl FnOnce(&mut Ui) -> R,
) -> AreaOut<R> {
    let owner_vp = ctx.viewport_id();
    let ppp = super::ppp_of(ctx, owner_vp);
    let Some(inner) = super::inner_px(ctx, owner_vp) else { return AreaOut { inner: None, lost: false } };
    let anchor_px = Rect::from_min_size(inner.min + anchor.min.to_vec2() * ppp, anchor.size() * ppp);
    let now = ctx.input(|i| i.time);
    with(|s| {
        let p = s.pops.entry(key).or_insert_with(|| PopState {
            vp: super::viewport_id(Kind::Popup, key),
            owner_vp,
            anchor_px,
            side,
            gap: 4.0 * ppp,
            size: Some(size),
            opened_at: now,
            owner_rect: super::outer_px(ctx, owner_vp),
            unfocused_since: None,
            touched: super::frame(),
            activate: true,
        });
        p.anchor_px = anchor_px;
        p.size = Some(size);
    });
    let r = render(
        ctx,
        key,
        frame,
        Layout::top_down(egui::Align::Min),
        false,
        PopupKind::Popup,
        PopupCloseBehavior::IgnoreClicks,
        content,
    );
    with(|s| {
        if let Some(p) = s.pops.get_mut(&key) {
            p.size = Some(size);
        }
    });
    AreaOut { inner: r.inner.map(|(i, _)| i), lost: r.lost }
}

/// Is the caller-managed native popup `key` open (drawn last frame)?
pub fn is_native_open(key: Id) -> bool {
    with(|s| s.pops.contains_key(&key))
}
