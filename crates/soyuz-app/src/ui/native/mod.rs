//! Native OS windows for dialogs and popups (`[appearance] native_dialogs`, default on).
//!
//! Dialogs (the [`crate::ui::dialogs`] host, settings, the path picker) and popup menus
//! / dropdowns that do not fit inside their window are drawn in egui *immediate*
//! viewports: real top-level windows that can leave the main window and move to other
//! monitors. Each one is
//!
//! * created hidden with a marker title, found by that title among this thread's windows
//!   ([`owned::find_thread_window`]) and made **owned** by the window it was opened from
//!   (no taskbar button, always above its owner, minimized with it); popups also get the
//!   tool-window style and small rounded DWM corners;
//! * placed in physical pixels on the work area of the monitor its anchor / owner is on
//!   ([`geometry`], unit-tested), and only shown once it has rendered a frame at its final
//!   size (no flicker);
//! * swept when its code stops drawing it (the viewport closes with it); a modal dialog's
//!   disabled owner is re-enabled *before* the window goes so Windows activates the owner.
//!
//! Off Windows, with the setting off, during the CPU-drawn first frames and in test
//! harnesses without real viewports everything falls back to the in-window rendering.

pub mod dialog;
pub mod geometry;
pub mod popup;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use egui::{Context, Pos2, Rect, Ui, Vec2, ViewportBuilder, ViewportClass, ViewportId, pos2, vec2};
use soyuz_fs::chrome::owned;

static ENABLED: AtomicBool = AtomicBool::new(true);
static FRAME: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(0);

/// `[appearance] native_dialogs` (startup and every config change).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Native windows are used this frame: the setting is on, we are on Windows and the
/// backend has real viewports.
pub fn active(ctx: &Context) -> bool {
    cfg!(windows) && ENABLED.load(Ordering::Relaxed) && !ctx.embed_viewports()
}

/// Scripted / off-screen sessions (`SOYUZ_WINDOW_POS`) never activate windows, so they
/// cannot take the keyboard from whatever the user is doing.
pub(crate) fn quiet() -> bool {
    static Q: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *Q.get_or_init(|| std::env::var_os("SOYUZ_WINDOW_POS").is_some())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dialog,
    Popup,
}

/// Bookkeeping for one native window.
struct Slot {
    kind: Kind,
    marker: String,
    owner_vp: ViewportId,
    hwnd: isize,
    owner_hwnd: isize,
    /// Builder values fixed at creation (egui would re-apply changed ones).
    create_pos: Pos2,
    create_size: Vec2,
    shown: bool,
    /// Frames rendered at the requested rect (shown after one).
    stable: u32,
    /// Frame number of the last [`show`] call.
    touched: u64,
    /// The owner we disabled for a modal dialog (re-enabled on sweep).
    disabled_owner: isize,
    /// Creation order (the newest window gets scripted keys).
    seq: u64,
    /// Synthetic input for the next frame (`SOYUZ_SCRIPT` `keys:`).
    events: Vec<egui::Event>,
    /// Show activated (keyboard focus).
    activate: bool,
}

#[derive(Default)]
struct Registry {
    slots: HashMap<ViewportId, Slot>,
    /// HWNDs of viewports we did not create (secondary in-process windows), matched by rect.
    known: HashMap<ViewportId, isize>,
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

fn with_reg<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REG.with(|r| f(&mut r.borrow_mut()))
}

pub(crate) fn frame() -> u64 {
    FRAME.load(Ordering::Relaxed)
}

/// Start of the main window's frame.
pub fn begin_frame() {
    FRAME.fetch_add(1, Ordering::Relaxed);
}

/// End of the main window's frame (after every window was drawn): forget windows that
/// were not drawn this frame (egui closes their viewports right after) and give a modal
/// dialog's owner its input back first.
pub fn end_frame(ctx: &Context) {
    let now = frame();
    let gone: Vec<Slot> = with_reg(|r| {
        let ids: Vec<ViewportId> = r.slots.iter().filter(|(_, s)| s.touched != now).map(|(id, _)| *id).collect();
        ids.into_iter().filter_map(|id| r.slots.remove(&id)).collect()
    });
    for s in gone {
        if s.disabled_owner != 0 && !any_modal_on(s.disabled_owner) {
            owned::enable_window(s.disabled_owner, true);
        }
        if s.hwnd != 0 {
            // Hide first so the owner is activated before the window is destroyed.
            owned::hide_window(s.hwnd);
        }
        log::debug!("native window closed: {:?} {}", s.kind, s.marker);
    }
    // A modal owner whose dialog vanished without a sweep (e.g. setting turned off).
    with_reg(|r| r.known.retain(|_, h| owned::is_window(*h)));
    let _ = ctx;
}

fn any_modal_on(owner: isize) -> bool {
    with_reg(|r| r.slots.values().any(|s| s.disabled_owner == owner))
}

/// Is `vp` one of our native windows (dialog or popup)?
pub fn is_native(vp: ViewportId) -> bool {
    with_reg(|r| r.slots.contains_key(&vp))
}

/// Kind of the native window `vp`.
pub fn kind_of(vp: ViewportId) -> Option<Kind> {
    with_reg(|r| r.slots.get(&vp).map(|s| s.kind))
}

/// One of the native popups owned (directly or through other popups) by `vp` has the
/// keyboard focus (the owner keeps its "active" look, like with OS menus).
pub fn popup_focused_for(ctx: &Context, vp: ViewportId) -> bool {
    let popups: Vec<(ViewportId, ViewportId)> =
        with_reg(|r| r.slots.iter().filter(|(_, s)| s.kind == Kind::Popup).map(|(id, s)| (*id, s.owner_vp)).collect());
    popups.iter().any(|(id, _)| {
        let mut owner = *id;
        let mut hops = 0;
        while let Some((_, o)) = popups.iter().find(|(p, _)| *p == owner) {
            owner = *o;
            hops += 1;
            if owner == vp || hops > 8 {
                break;
            }
        }
        owner == vp && ctx.input_for(*id, |i| i.viewport().focused) == Some(true)
    })
}

/// Pixels per point of viewport `vp` (its last input).
pub fn ppp_of(ctx: &Context, vp: ViewportId) -> f32 {
    ctx.input_for(vp, |i| i.pixels_per_point).max(0.25)
}

/// Inner (client) rect of viewport `vp` in physical screen pixels.
pub fn inner_px(ctx: &Context, vp: ViewportId) -> Option<Rect> {
    let (inner, ppp) = ctx.input_for(vp, |i| (i.viewport().inner_rect, i.pixels_per_point));
    inner.map(|r| Rect::from_min_max((r.min.to_vec2() * ppp).to_pos2(), (r.max.to_vec2() * ppp).to_pos2()))
}

/// Outer rect of viewport `vp` in physical screen pixels.
pub fn outer_px(ctx: &Context, vp: ViewportId) -> Option<Rect> {
    if let Some(h) = hwnd_of(ctx, vp)
        && let Some(r) = owned::window_rect(h)
    {
        return Some(geometry::ltrb(r));
    }
    let (outer, ppp) = ctx.input_for(vp, |i| (i.viewport().outer_rect, i.pixels_per_point));
    outer.map(|r| Rect::from_min_max((r.min.to_vec2() * ppp).to_pos2(), (r.max.to_vec2() * ppp).to_pos2()))
}

/// Work area (physical pixels) of the monitor at / nearest to `p` (pixels). Falls back to
/// the monitor egui reports for `vp`.
pub fn work_area_at(ctx: &Context, vp: ViewportId, p: Pos2) -> Rect {
    if quiet() {
        // Scripted off-screen sessions: a virtual area around the point, so nothing is
        // pulled onto the user's screen.
        return Rect::from_center_size(p, vec2(6000.0, 6000.0));
    }
    if let Some((work, _)) = owned::monitor_at(p.x.round() as i32, p.y.round() as i32) {
        return geometry::ltrb(work);
    }
    let (size, ppp) = ctx.input_for(vp, |i| (i.viewport().monitor_size, i.pixels_per_point));
    let size = size.unwrap_or(vec2(1920.0, 1080.0)) * ppp;
    Rect::from_min_size(Pos2::ZERO, size)
}

/// The native window handle of viewport `vp` (main window, one of ours, or a secondary
/// window matched by its rect).
pub fn hwnd_of(ctx: &Context, vp: ViewportId) -> Option<isize> {
    if vp == ViewportId::ROOT {
        let h = crate::ui::native_chrome::main_hwnd();
        if h != 0 {
            return Some(h);
        }
    }
    if let Some(h) = with_reg(|r| r.slots.get(&vp).map(|s| s.hwnd).filter(|h| *h != 0).or(r.known.get(&vp).copied())) {
        return Some(h);
    }
    // Match by outer rect among this thread's windows.
    let (outer, ppp) = ctx.input_for(vp, |i| (i.viewport().outer_rect, i.pixels_per_point));
    let outer = outer?;
    let want = [outer.left() * ppp, outer.top() * ppp, outer.right() * ppp, outer.bottom() * ppp];
    let ours: Vec<isize> = with_reg(|r| r.slots.values().map(|s| s.hwnd).chain(r.known.values().copied()).collect());
    let h = owned::thread_windows().into_iter().filter(|h| !ours.contains(h) && owned::is_visible(*h)).find(|h| {
        owned::window_rect(*h).is_some_and(|r| r.iter().zip(want).all(|(a, b)| (*a as f32 - b).abs() <= 3.0))
    })?;
    with_reg(|r| r.known.insert(vp, h));
    Some(h)
}

/// What [`show`] needs to place a native window.
pub struct Placement {
    pub kind: Kind,
    /// Title shown to the OS (task switcher, screen readers).
    pub title: String,
    /// Requested outer rect in physical pixels. Popups follow it every frame; dialogs use
    /// it until shown, and afterwards only its size (`keep_size = false` leaves the size
    /// to the user).
    pub rect_px: Rect,
    pub keep_size: bool,
    pub resizable: bool,
    /// Minimum inner size (points).
    pub min_size: Vec2,
    /// Disable the owner while shown (modal dialogs).
    pub modal: bool,
}

/// Viewport id of a native window keyed by `key`.
pub fn viewport_id(kind: Kind, key: egui::Id) -> ViewportId {
    ViewportId::from_hash_of((kind == Kind::Dialog, "soyuz.native", key))
}

fn app_icon() -> std::sync::Arc<egui::IconData> {
    static ICON: std::sync::OnceLock<std::sync::Arc<egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| std::sync::Arc::new(crate::app_icon::icon_data(64))).clone()
}

/// Draw `content` in the native window `vp` (created on first use). Call every frame
/// while it should stay open, from the owner window's frame. `None` when the window could
/// not draw this frame (not created yet).
pub fn show<R>(ctx: &Context, vp: ViewportId, place: &Placement, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let owner_vp = ctx.viewport_id();
    let now = frame();
    let owner_ppp = ppp_of(ctx, owner_vp);
    let (marker, claimed, create_pos, create_size) = with_reg(|r| {
        let s = r.slots.entry(vp).or_insert_with(|| Slot {
            kind: place.kind,
            marker: format!("soyuz-native-{:016x}", vp.0.value()),
            owner_vp,
            hwnd: 0,
            owner_hwnd: 0,
            create_pos: (place.rect_px.min.to_vec2() / owner_ppp).to_pos2(),
            create_size: place.rect_px.size() / owner_ppp,
            shown: false,
            stable: 0,
            touched: now,
            disabled_owner: 0,
            seq: SEQ.fetch_add(1, Ordering::Relaxed),
            events: Vec::new(),
            activate: PENDING_ACTIVATE.with(|p| p.borrow_mut().remove(&vp)).unwrap_or(true),
        });
        s.touched = now;
        s.owner_vp = owner_vp;
        (s.marker.clone(), s.hwnd != 0, s.create_pos, s.create_size)
    });
    let title = if claimed && place.kind == Kind::Dialog { place.title.clone() } else { marker.clone() };
    let builder = ViewportBuilder::default()
        .with_title(title)
        .with_app_id("soyuz")
        .with_icon(app_icon())
        .with_decorations(false)
        .with_resizable(place.resizable)
        .with_visible(false)
        .with_drag_and_drop(false)
        .with_position(create_pos)
        .with_inner_size(create_size)
        .with_min_inner_size(place.min_size);
    let mut content = Some(content);
    let out = ctx.show_viewport_immediate(vp, builder, |ui, class| {
        if class != ViewportClass::EmbeddedWindow {
            let evs = with_reg(|r| r.slots.get_mut(&vp).map(|s| std::mem::take(&mut s.events)).unwrap_or_default());
            if !evs.is_empty() {
                let ppp = ui.ctx().pixels_per_point();
                let options = ui.ctx().options(|o| o.input_options);
                ui.ctx().input_mut(|i| {
                    // `native:hover`: a scripted pointer must reach the pointer state,
                    // which egui built from the window's input before this frame.
                    let moves: Vec<_> =
                        evs.iter().filter(|e| matches!(e, egui::Event::PointerMoved(_))).cloned().collect();
                    if !moves.is_empty() {
                        let raw = egui::RawInput { events: moves, time: Some(i.time), ..Default::default() };
                        *i = std::mem::take(i).begin_pass(raw, false, ppp, options);
                    }
                    i.events.extend(evs);
                });
            }
        }
        content.take().map(|f| f(ui))
    });
    if out.is_none() {
        log::debug!("native window {vp:?}: no frame this time (window not ready)");
        ctx.request_repaint();
    }
    if !ctx.embed_viewports() {
        drive(ctx, vp, place);
    }
    out
}

/// Claim, own, place and show the window after its frame.
fn drive(ctx: &Context, vp: ViewportId, place: &Placement) {
    let (marker, mut hwnd, owner_vp, shown) =
        match with_reg(|r| r.slots.get(&vp).map(|s| (s.marker.clone(), s.hwnd, s.owner_vp, s.shown))) {
            Some(v) => v,
            None => return,
        };
    if hwnd == 0 {
        let Some(h) = owned::find_thread_window(&marker) else {
            ctx.request_repaint();
            return;
        };
        // The owner may be a native window created this same frame: wait for its handle.
        let Some(owner_hwnd) = hwnd_of(ctx, owner_vp) else {
            ctx.request_repaint();
            return;
        };
        hwnd = h;
        owned::set_owner(h, owner_hwnd);
        match place.kind {
            Kind::Popup => {
                owned::set_tool_window(h, true);
                owned::set_small_corners(h);
            }
            Kind::Dialog => {
                soyuz_fs::chrome::set_corner_preference(h, Some(true));
            }
        }
        soyuz_fs::chrome::set_dark_mode(h, ctx.theme() == egui::Theme::Dark);
        with_reg(|r| {
            if let Some(s) = r.slots.get_mut(&vp) {
                s.hwnd = h;
                s.owner_hwnd = owner_hwnd;
            }
        });
        log::debug!("native window {marker}: hwnd {h:#x}, owner {owner_hwnd:#x}");
    }
    let mut want = geometry::to_px(place.rect_px, 1.0);
    // Never ask the GPU for a surface beyond its texture limit.
    want[2] = want[2].clamp(1, 8000);
    want[3] = want[3].clamp(1, 8000);
    let actual = owned::window_rect(hwnd);
    let follow = place.kind == Kind::Popup || !shown;
    let mut stable = false;
    if let Some([l, t, r, b]) = actual {
        let (w, h) = (r - l, b - t);
        if follow {
            if (l - want[0]).abs() > 1 || (t - want[1]).abs() > 1 || (w - want[2]).abs() > 1 || (h - want[3]).abs() > 1
            {
                owned::set_window_rect(hwnd, want, true);
            } else {
                stable = true;
            }
        } else if place.keep_size && ((w - want[2]).abs() > 1 || (h - want[3]).abs() > 1) {
            // Content height changed: resize in place (kept on its monitor).
            let work = work_area_at(ctx, vp, pos2(l as f32, t as f32));
            let r = geometry::keep_reachable(
                Rect::from_min_size(pos2(l as f32, t as f32), vec2(want[2] as f32, want[3] as f32)),
                work,
                40.0,
            );
            owned::set_window_rect(hwnd, geometry::to_px(r, 1.0), true);
        } else {
            stable = true;
        }
    }
    let show_now = with_reg(|r| {
        let Some(s) = r.slots.get_mut(&vp) else { return false };
        s.stable = if stable { s.stable + 1 } else { 0 };
        !s.shown && s.stable >= 2
    });
    if show_now {
        let activate = with_reg(|r| r.slots.get(&vp).is_none_or(|s| s.activate));
        owned::show_window(hwnd, activate && !quiet());
        if place.modal {
            let owner = with_reg(|r| r.slots.get(&vp).map_or(0, |s| s.owner_hwnd));
            if owner != 0 {
                owned::enable_window(owner, false);
            }
            with_reg(|r| {
                if let Some(s) = r.slots.get_mut(&vp) {
                    s.disabled_owner = owner;
                }
            });
        }
        with_reg(|r| {
            if let Some(s) = r.slots.get_mut(&vp) {
                s.shown = true;
            }
        });
        log::debug!("native window {marker} shown at {want:?}");
    } else if !shown {
        ctx.request_repaint();
    }
}

/// Whether the native window `vp` is on screen yet.
pub fn is_shown(vp: ViewportId) -> bool {
    with_reg(|r| r.slots.get(&vp).is_some_and(|s| s.shown))
}

// ---------------------------------------------------------------------------
// SOYUZ_SCRIPT hooks (`native:<verb>`)
// ---------------------------------------------------------------------------

/// `native:list` / `native:size <w>,<h>` / `native:move <x>,<y>` (a dialog, physical
/// pixels) / `native:moveby <dx>,<dy>` / `native:hover <x>,<y>[,<n>]` (egui's pointer in
/// the `n`-th open popup, oldest first, in its points; no OS input).
pub fn devscript(ctx: &Context, arg: &str) {
    let (verb, rest) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    // Oldest first (bottom of the z-order first).
    let slots: Vec<(ViewportId, Kind, isize, String)> = with_reg(|r| {
        let mut v: Vec<_> = r.slots.iter().collect();
        v.sort_by_key(|(_, s)| s.seq);
        v.into_iter().map(|(id, s)| (*id, s.kind, s.hwnd, s.marker.clone())).collect()
    });
    match verb {
        "list" => {
            for (_, kind, h, marker) in &slots {
                log::info!("SOYUZ_SCRIPT: native {kind:?} {marker} hwnd {h} rect {:?}", owned::window_rect(*h));
            }
            if let Some(r) = hwnd_of(ctx, ViewportId::ROOT).and_then(owned::window_rect) {
                log::info!("SOYUZ_SCRIPT: native main hwnd {} rect {r:?}", hwnd_of(ctx, ViewportId::ROOT).unwrap_or(0));
            }
        }
        "size" => {
            let mut it = rest.split(',').filter_map(|v| v.trim().parse::<i32>().ok());
            let (Some(w), Some(h)) = (it.next(), it.next()) else { return };
            if let Some((_, _, hw, _)) = slots.iter().rfind(|s| s.1 == Kind::Dialog)
                && let Some(r) = owned::window_rect(*hw)
            {
                owned::set_window_rect(*hw, [r[0], r[1], w, h], true);
            }
        }
        "move" | "moveby" => {
            let mut it = rest.split(',').filter_map(|v| v.trim().parse::<i32>().ok());
            let (Some(x), Some(y)) = (it.next(), it.next()) else { return };
            if let Some((_, _, h, _)) = slots.iter().rfind(|s| s.1 == Kind::Dialog)
                && let Some(r) = owned::window_rect(*h)
            {
                let (nx, ny) = if verb == "move" { (x, y) } else { (r[0] + x, r[1] + y) };
                owned::set_window_rect(*h, [nx, ny, r[2] - r[0], r[3] - r[1]], false);
            }
        }
        "hover" => {
            let v: Vec<f32> = rest.split(',').filter_map(|v| v.trim().parse::<f32>().ok()).collect();
            let (Some(&x), Some(&y)) = (v.first(), v.get(1)) else { return };
            let n = v.get(2).map_or(0, |n| *n as usize);
            let Some((vp, ..)) = slots.iter().filter(|s| s.1 == Kind::Popup).nth(n) else { return };
            with_reg(|r| {
                if let Some(s) = r.slots.get_mut(vp) {
                    s.events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
                }
            });
            ctx.request_repaint_of(*vp);
        }
        _ => log::warn!("SOYUZ_SCRIPT: unknown native verb {verb}"),
    }
}

/// Whether the native window `vp` takes the keyboard focus when it appears (submenus
/// opened by hovering do not). Call before [`show`] on its first frame.
pub fn set_activate(vp: ViewportId, on: bool) {
    let exists = with_reg(|r| match r.slots.get_mut(&vp) {
        Some(s) => {
            s.activate = on;
            true
        }
        None => false,
    });
    if !exists {
        PENDING_ACTIVATE.with(|p| p.borrow_mut().insert(vp, on));
    }
}

thread_local! {
    static PENDING_ACTIVATE: RefCell<HashMap<ViewportId, bool>> = RefCell::new(HashMap::new());
}

/// `SOYUZ_SCRIPT` `keys:` while a native window is open: deliver `events` to the newest
/// one (it has the keyboard). Returns `false` when there is none.
pub fn inject(events: &[egui::Event]) -> bool {
    with_reg(|r| {
        let Some(s) = r.slots.values_mut().filter(|s| s.shown).max_by_key(|s| s.seq) else { return false };
        s.events.extend_from_slice(events);
        true
    })
}
