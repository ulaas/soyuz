//! Custom window chrome (`[appearance] custom_titlebar`, on by default).
//!
//! * Windows / Linux: an undecorated window. Soyuz draws the title bar (app logo +
//!   wordmark above the sidebar, the tab strip, free space to drag), the window controls
//!   (minimize, maximize / restore, close), a 1 px border and invisible resize zones at
//!   the outer edges. Moving and resizing are handed to the OS (`StartDrag` /
//!   `BeginResize`), so Aero snap and the native resize loop keep working.
//! * macOS: the native window keeps its traffic lights; the title bar is transparent with
//!   the content drawn underneath (`fullsize_content_view`), and the bar leaves room for
//!   the lights. Changing the setting takes effect at the next start there.
//!
//! The layout math ([`layout`], [`control_rects`], [`resize_hit`]) is pure and tested.

use egui::accesskit::Role;
use egui::viewport::ResizeDirection;
use egui::{
    Area, Color32, Context, CornerRadius, CursorIcon, Frame, Id, LayerId, Order, Painter, Panel, PointerButton, Popup,
    Pos2, Rect, RectAlign, Sense, Stroke, StrokeKind, TextureHandle, Ui, ViewportBuilder, ViewportCommand, pos2, vec2,
};
use soyuz_core::config::{Action, TabStripMode};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::theme::tokens::radius;
use crate::theme::{icons::ph, palette};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{focus_ring, menu_item, menu_separator};

/// Title bar height (the tab strip lives inside it).
pub const HEIGHT: f32 = 40.0;
/// Thickness of the invisible resize zones along the outer window edge.
pub const RESIZE_EDGE: f32 = 5.0;
/// Length of the corner (diagonal) resize zones along each edge.
pub const RESIZE_CORNER: f32 = 14.0;
/// Width of one window control button (Windows 11 metrics).
pub const CONTROL_W: f32 = 46.0;
/// Space kept for the macOS traffic lights.
pub const MAC_TRAFFIC_LIGHTS: f32 = 78.0;
/// Width of the logo + wordmark block when there is no sidebar column to align with.
pub const BRAND_MIN: f32 = 104.0;
/// Free (draggable) space always left between the tabs and the window controls.
pub const MIN_DRAG: f32 = 56.0;
/// Logo size in points.
const LOGO: f32 = 20.0;

/// How the window frame is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Chrome {
    /// System title bar and borders.
    #[default]
    Native,
    /// Undecorated window with Soyuz's own title bar, controls, border and resize zones.
    Custom,
    /// macOS: native traffic lights over Soyuz's title bar (transparent system title bar).
    MacOverlay,
}

impl Chrome {
    /// The chrome for `[appearance] custom_titlebar` on this platform.
    pub fn for_config(custom: bool) -> Chrome {
        if !custom {
            Chrome::Native
        } else if cfg!(target_os = "macos") {
            Chrome::MacOverlay
        } else {
            Chrome::Custom
        }
    }

    /// Soyuz draws a title bar row (logo, tabs).
    pub fn has_titlebar(self) -> bool {
        self != Chrome::Native
    }

    /// Soyuz draws the window controls, the border and handles edge resizing.
    pub fn own_frame(self) -> bool {
        self == Chrome::Custom
    }

    /// Whether the chrome can be switched without restarting (macOS can't toggle the
    /// full-size content view at runtime).
    pub fn live_switch() -> bool {
        !cfg!(target_os = "macos")
    }
}

/// Window attributes for the chrome chosen at startup.
pub fn configure_viewport(builder: ViewportBuilder, chrome: Chrome) -> ViewportBuilder {
    match chrome {
        Chrome::Native => builder,
        // Transparency is not needed: the window keeps square corners (Windows 11 rounds
        // them itself) and egui-winit adds the undecorated drop shadow on Windows.
        Chrome::Custom => builder.with_decorations(false),
        Chrome::MacOverlay => {
            builder.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
        }
    }
}

/// Apply a changed `custom_titlebar` setting (live where the platform allows it).
pub fn sync(ctx: &Context, app: &mut SoyuzApp) {
    let want = Chrome::for_config(app.config.appearance.custom_titlebar);
    if want == app.ui.chrome || !Chrome::live_switch() {
        return;
    }
    ctx.send_viewport_cmd(ViewportCommand::Decorations(want == Chrome::Native));
    app.ui.chrome = want;
}

/// Rects of the title bar parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarLayout {
    /// Logo + wordmark column (aligned with the sidebar when it is visible).
    pub brand: Rect,
    /// Tabs and the new-tab button.
    pub tabs: Rect,
    /// Window controls (empty width on macOS, where the system draws them on the left).
    pub controls: Rect,
}

/// Split the title bar. `column`: sidebar width to align the brand with (None = compact);
/// `leading`: space reserved on the left (macOS traffic lights); `controls`: draw the
/// three window buttons on the right.
pub fn layout(bar: Rect, column: Option<f32>, leading: f32, controls: bool) -> BarLayout {
    let left = (bar.left() + leading).min(bar.right());
    let controls_w = if controls { 3.0 * CONTROL_W } else { 8.0 };
    let controls = Rect::from_min_max(pos2((bar.right() - controls_w).max(left), bar.top()), bar.right_bottom());
    let want = column.map_or(left + BRAND_MIN, |w| (bar.left() + w).max(left + BRAND_MIN));
    let brand_right = want.min(controls.left());
    let brand = Rect::from_min_max(pos2(left, bar.top()), pos2(brand_right, bar.bottom()));
    let tabs_right = (controls.left() - MIN_DRAG).max(brand_right);
    let tabs = Rect::from_min_max(pos2(brand_right, bar.top()), pos2(tabs_right, bar.bottom()));
    BarLayout { brand, tabs, controls }
}

/// The minimize / maximize / close button rects inside `controls` (left to right).
pub fn control_rects(controls: Rect) -> [Rect; 3] {
    let w = controls.width() / 3.0;
    std::array::from_fn(|i| {
        Rect::from_min_max(
            pos2(controls.left() + w * i as f32, controls.top()),
            pos2(controls.left() + w * (i + 1) as f32, controls.bottom()),
        )
    })
}

/// Which resize handle (if any) is under `pos`: within `edge` of a window side, with
/// the `corner`-long ends of each side resizing diagonally.
pub fn resize_hit(window: Rect, pos: Pos2, edge: f32, corner: f32) -> Option<ResizeDirection> {
    if !window.contains(pos) {
        return None;
    }
    let (dl, dr) = (pos.x - window.left(), window.right() - pos.x);
    let (dt, db) = (pos.y - window.top(), window.bottom() - pos.y);
    let (l, r, t, b) = (dl < edge, dr < edge, dt < edge, db < edge);
    let (cl, cr, ct, cb) = (dl < corner, dr < corner, dt < corner, db < corner);
    use ResizeDirection as D;
    Some(if (t && cl) || (l && ct) {
        D::NorthWest
    } else if (t && cr) || (r && ct) {
        D::NorthEast
    } else if (b && cl) || (l && cb) {
        D::SouthWest
    } else if (b && cr) || (r && cb) {
        D::SouthEast
    } else if t {
        D::North
    } else if b {
        D::South
    } else if l {
        D::West
    } else if r {
        D::East
    } else {
        return None;
    })
}

/// The mouse cursor for a resize handle.
pub fn resize_cursor(d: ResizeDirection) -> CursorIcon {
    match d {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct WindowState {
    maximized: bool,
    fullscreen: bool,
    focused: bool,
}

fn window_state(ctx: &Context) -> WindowState {
    ctx.input(|i| {
        let v = i.viewport();
        WindowState {
            maximized: v.maximized.unwrap_or(false),
            fullscreen: v.fullscreen.unwrap_or(false),
            focused: v.focused.unwrap_or(true),
        }
    })
}

/// Width of the sidebar panel last frame (the title bar is laid out before it).
fn sidebar_column(ctx: &Context) -> Option<f32> {
    let w = egui::containers::panel::PanelState::load(ctx, Id::new("soyuz.sidebar"))?.outer_rect.width();
    // Re-layout once more when the sidebar width settles after a resize.
    let key = Id::new("soyuz.titlebar.column");
    let last = ctx.data(|d| d.get_temp::<f32>(key));
    if last != Some(w) {
        ctx.data_mut(|d| d.insert_temp(key, w));
        ctx.request_repaint();
    }
    Some(w)
}

/// The title bar row (call first in the shell so it spans the full window width).
pub fn show(root: &mut Ui, app: &mut SoyuzApp) {
    let ctx = root.ctx().clone();
    let p = palette(&ctx);
    let chrome = app.ui.chrome;
    let ws = window_state(&ctx);
    let lay = Panel::top("soyuz.titlebar")
        .exact_size(HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window))
        .show(root, |ui| {
            let bar = ui.max_rect();
            let column = if app.ui.sidebar_visible { sidebar_column(&ctx) } else { None };
            let leading = if chrome == Chrome::MacOverlay && !ws.fullscreen { MAC_TRAFFIC_LIGHTS } else { 0.0 };
            let lay = layout(bar, column, leading, chrome.own_frame());

            // Empty space: drag to move, double-click to maximize, right-click for the
            // window menu. Registered first so everything drawn on top takes precedence.
            let bg = ui.interact(bar, Id::new("soyuz.titlebar.bg"), Sense::click_and_drag());
            crate::ui::a11y::container(&ctx, bg.id, Role::TitleBar, "Soyuz");
            if bg.double_clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!ws.maximized));
            } else if bg.drag_started_by(PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            bg.context_menu(|ui| window_menu(ui, &app.keymap, ws.maximized));

            brand(ui, lay.brand, leading > 0.0, ws.focused, app);
            if app.config.layout.tab_strip == TabStripMode::Always || app.ws.tabs.len() > 1 {
                let tabs = Rect::from_min_max(
                    pos2(lay.tabs.left() + 6.0, bar.top() + 6.0),
                    pos2(lay.tabs.right(), bar.top() + 38.0),
                );
                if tabs.width() > 60.0 {
                    crate::ui::tabstrip::strip(ui, tabs, app, true);
                }
            }
            lay
        })
        .inner;
    if chrome.own_frame() {
        controls(&ctx, lay.controls, ws);
    } else if ctx.viewport_id() == egui::ViewportId::ROOT {
        crate::ui::native_chrome::max_button(&ctx, None, false);
    }
}

/// Corner radius for the close button: its top-right corner follows the Windows 11
/// rounded window corner while the window is floating (else the red hover fill pokes
/// out of the DWM clip).
pub fn close_corner(win11: bool, ws_floating: bool) -> CornerRadius {
    if win11 && ws_floating {
        CornerRadius { ne: crate::ui::native_chrome::DWM_CORNER_RADIUS, ..CornerRadius::ZERO }
    } else {
        CornerRadius::ZERO
    }
}

/// App logo + wordmark (click: app menu).
fn brand(ui: &mut Ui, col: Rect, compact_pad: bool, focused: bool, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    // Line the logo up with the sidebar icons (rows start 8 px in, icons 10 px further).
    let logo_left = col.left() + if compact_pad { 6.0 } else { 16.0 };
    let font = semibold(14.0);
    let word = ui.painter().layout_no_wrap("Soyuz".into(), font, p.text_primary);
    let btn = Rect::from_min_max(
        pos2(logo_left - 6.0, col.center().y - 14.0),
        pos2(logo_left + LOGO + 8.0 + word.size().x + 10.0, col.center().y + 14.0),
    );
    if btn.right() > col.right() {
        return; // no room (tiny window): leave the space to dragging
    }
    let resp = ui.interact(btn, Id::new("soyuz.titlebar.brand"), Sense::click());
    crate::ui::a11y::button(&resp, "Soyuz menu");
    let hover = ctx.animate_bool_with_time(resp.id.with("h"), resp.hovered(), crate::theme::anim(&ctx, 0.10));
    let open = Popup::is_id_open(&ctx, Popup::default_response_id(&resp));
    let painter = ui.painter();
    if open || resp.is_pointer_button_down_on() {
        painter.rect_filled(btn, CornerRadius::same(radius::ROW), p.bg_surface3);
    } else if hover > 0.0 {
        painter.rect_filled(btn, CornerRadius::same(radius::ROW), p.bg_surface2.gamma_multiply(hover));
    }
    let ppp = ctx.pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let logo = Rect::from_min_size(pos2(snap(logo_left), snap(col.center().y - LOGO / 2.0)), vec2(LOGO, LOGO));
    let tex = logo_texture(&ctx, (LOGO * ppp).round().max(8.0) as u32);
    let tint = if focused { Color32::WHITE } else { Color32::from_white_alpha(200) };
    painter.image(tex.id(), logo, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), tint);
    let color = if focused { p.text_primary } else { p.text_secondary };
    painter.galley(pos2(logo.right() + 8.0, col.center().y - word.size().y / 2.0), word, color);
    if resp.has_focus() {
        focus_ring(ui, btn);
    }
    let resp = resp.on_hover_text("Soyuz menu");
    Popup::menu(&resp).align(RectAlign::BOTTOM_START).show(|ui| app_menu(ui, app));
}

/// The app icon rendered at exactly `px` pixels (crisp at any scale), cached per size.
fn logo_texture(ctx: &Context, px: u32) -> TextureHandle {
    let id = Id::new(("soyuz.titlebar.logo", px));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let tex = ctx.load_texture("soyuz.titlebar.logo", crate::app_icon::color_image(px), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

fn app_menu(ui: &mut Ui, app: &mut SoyuzApp) {
    ui.set_width(230.0);
    let km = app.keymap.clone();
    let mut cmds = Vec::new();
    let item = |ui: &mut Ui, cmds: &mut Vec<Cmd>, icon: &str, label: &str, action: Action| {
        if menu_item(ui, Some(icon), label, chord_label(&km, &action).as_deref(), true).clicked() {
            cmds.push(Cmd::Action(action));
            ui.close();
        }
    };
    item(ui, &mut cmds, ph::APP_WINDOW, "New window", Action::NewWindow);
    crate::ui::workspaces::workspaces_menu(ui, app, &mut cmds);
    menu_separator(ui);
    item(ui, &mut cmds, ph::GEAR_SIX, "Settings…", Action::Settings);
    item(ui, &mut cmds, ph::KEYBOARD, "Keyboard shortcuts", Action::ShowKeybindings);
    item(ui, &mut cmds, ph::INFO, "About Soyuz", Action::About);
    menu_separator(ui);
    item(ui, &mut cmds, ph::SIGN_OUT, "Quit", Action::Quit);
    app.cmds.extend(cmds);
}

fn window_menu(ui: &mut Ui, km: &soyuz_core::config::Keymap, maximized: bool) {
    ui.set_width(200.0);
    let ctx = ui.ctx().clone();
    if menu_item(ui, Some(ph::ARROWS_IN_SIMPLE), "Restore", None, maximized).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
        ui.close();
    }
    if menu_item(ui, Some(ph::MINUS), "Minimize", None, true).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROWS_OUT_SIMPLE), "Maximize", None, !maximized).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
        ui.close();
    }
    menu_separator(ui);
    let close_key = chord_label(km, &Action::CloseWindow).or_else(|| cfg!(windows).then(|| "Alt+F4".to_string()));
    if menu_item(ui, Some(ph::X), "Close", close_key.as_deref(), true).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
        ui.close();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Minimize / maximize-restore / close, in their own top layer so they stay usable
/// above modal dialogs (like native caption buttons).
fn controls(ctx: &Context, rect: Rect, ws: WindowState) {
    let p = palette(ctx);
    Area::new(Id::new("soyuz.titlebar.controls"))
        .order(Order::Tooltip)
        .fixed_pos(rect.min)
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            let [rmin, rmax, rclose] = control_rects(rect);
            // Snap Layouts: the OS owns the maximize button's hit test, so its hover /
            // press / click arrive from the window subclass instead of egui.
            // (Only the main window has the Snap Layouts window subclass.)
            let (os_hover, os_pressed, os_click) = if ctx.viewport_id() == egui::ViewportId::ROOT {
                crate::ui::native_chrome::max_button(ctx, Some(rmax), true)
            } else {
                (false, false, false)
            };

            let floating = !ws.maximized && !ws.fullscreen;
            let close_radius = close_corner(crate::ui::native_chrome::is_windows_11(), floating);
            let max_glyph = if ws.maximized { Glyph::Restore } else { Glyph::Maximize };
            let buttons = [
                (rmin, Glyph::Minimize, "Minimize"),
                (rmax, max_glyph, if ws.maximized { "Restore" } else { "Maximize" }),
                (rclose, Glyph::Close, "Close"),
            ];
            for (r, glyph, label) in buttons {
                let resp = ui.interact(r, Id::new(("soyuz.titlebar.ctl", label)), Sense::click());
                crate::ui::a11y::button(&resp, label);
                let is_max = matches!(glyph, Glyph::Maximize | Glyph::Restore);
                let hovered = resp.hovered() || (is_max && os_hover);
                let hover = ctx.animate_bool_with_time(resp.id.with("h"), hovered, crate::theme::anim(ctx, 0.10));
                let pressed = resp.is_pointer_button_down_on() || (is_max && os_pressed);
                let painter = ui.painter();
                let close = glyph == Glyph::Close;
                let red = Color32::from_rgb(0xC4, 0x2B, 0x1C);
                let mut fg = if ws.focused { p.text_primary } else { p.text_tertiary };
                if close {
                    if pressed {
                        painter.rect_filled(r, close_radius, red.gamma_multiply(0.85));
                        fg = Color32::WHITE;
                    } else if hover > 0.0 {
                        painter.rect_filled(r, close_radius, red.gamma_multiply(hover));
                        fg = lerp_color(fg, Color32::WHITE, hover);
                    }
                } else if pressed {
                    painter.rect_filled(r, CornerRadius::ZERO, p.bg_surface3);
                } else if hover > 0.0 {
                    painter.rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(hover));
                }
                paint_glyph(painter, glyph, r.center(), fg, ctx.pixels_per_point());
                if resp.has_focus() {
                    focus_ring(ui, r.shrink(4.0));
                }
                let resp = resp.on_hover_text(label);
                if resp.clicked() || (is_max && os_click) {
                    match glyph {
                        Glyph::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                        Glyph::Maximize => ctx.send_viewport_cmd(ViewportCommand::Maximized(true)),
                        Glyph::Restore => ctx.send_viewport_cmd(ViewportCommand::Maximized(false)),
                        Glyph::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
                    }
                }
            }
        });
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// Windows 11 style caption glyphs (10 pt), stroked at a whole number of device pixels
/// on pixel centers so they stay sharp at 100 % / 150 % / 200 %.
fn paint_glyph(painter: &Painter, glyph: Glyph, center: Pos2, color: Color32, ppp: f32) {
    let px = 1.0 / ppp;
    let w = ppp.round().max(1.0) * px;
    // Pixel-center snapping for odd-width strokes.
    let snap = |v: f32| ((v * ppp).floor() + 0.5) / ppp;
    let c = pos2(snap(center.x), snap(center.y));
    let s = 5.0;
    let stroke = Stroke::new(w, color);
    match glyph {
        Glyph::Minimize => {
            painter.line_segment([pos2(c.x - s, c.y), pos2(c.x + s, c.y)], stroke);
        }
        Glyph::Maximize => {
            let r = Rect::from_center_size(c, vec2(2.0 * s, 2.0 * s));
            painter.rect_stroke(r, CornerRadius::same(1), stroke, StrokeKind::Middle);
        }
        Glyph::Restore => {
            let front = Rect::from_min_max(pos2(c.x - s, c.y - s + 2.0), pos2(c.x + s - 2.0, c.y + s));
            painter.rect_stroke(front, CornerRadius::same(1), stroke, StrokeKind::Middle);
            // Back window: only the top and right edges peek out.
            painter.line_segment([pos2(c.x - s + 2.0, c.y - s), pos2(c.x + s, c.y - s)], stroke);
            painter.line_segment([pos2(c.x + s, c.y - s), pos2(c.x + s, c.y + s - 2.0)], stroke);
        }
        Glyph::Close => {
            painter.line_segment([pos2(c.x - s, c.y - s), pos2(c.x + s, c.y + s)], stroke);
            painter.line_segment([pos2(c.x - s, c.y + s), pos2(c.x + s, c.y - s)], stroke);
        }
    }
}

/// Window border and edge resize zones (call last in the frame). Only for
/// [`Chrome::Custom`] while the window is neither maximized nor fullscreen.
pub fn frame(ctx: &Context, app: &SoyuzApp) {
    if !app.ui.chrome.own_frame() {
        return;
    }
    let ws = window_state(ctx);
    if ws.maximized || ws.fullscreen {
        return;
    }
    let p = palette(ctx);
    let screen = ctx.content_rect();
    // A subtle 1 px edge so the borderless window stands out on similar backgrounds
    // (rounded like the DWM clip on Windows 11).
    let corner = if crate::ui::native_chrome::is_windows_11() {
        CornerRadius::same(crate::ui::native_chrome::DWM_CORNER_RADIUS)
    } else {
        CornerRadius::ZERO
    };
    ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("soyuz.window.border"))).rect_stroke(
        screen,
        corner,
        Stroke::new(1.0, if ws.focused { p.border_strong } else { p.border_subtle }),
        StrokeKind::Inside,
    );
    // Thin strips along the very edge only, so scrollbars and splitters stay reachable.
    let e = RESIZE_EDGE;
    let strips = [
        ("top", Rect::from_min_max(screen.min, pos2(screen.right(), screen.top() + e))),
        ("bottom", Rect::from_min_max(pos2(screen.left(), screen.bottom() - e), screen.max)),
        (
            "left",
            Rect::from_min_max(pos2(screen.left(), screen.top() + e), pos2(screen.left() + e, screen.bottom() - e)),
        ),
        (
            "right",
            Rect::from_min_max(pos2(screen.right() - e, screen.top() + e), pos2(screen.right(), screen.bottom() - e)),
        ),
    ];
    let pressed = ctx.input(|i| i.pointer.primary_pressed());
    for (name, strip) in strips {
        Area::new(Id::new(("soyuz.window.resize", name)))
            .order(Order::Tooltip)
            .fixed_pos(strip.min)
            .constrain(false)
            .interactable(true)
            .show(ctx, |ui| {
                let (_, resp) = ui.allocate_exact_size(strip.size(), Sense::click_and_drag());
                if !resp.hovered() && !resp.is_pointer_button_down_on() {
                    return;
                }
                let Some(pos) = resp.hover_pos().or_else(|| ctx.pointer_latest_pos()) else { return };
                let Some(dir) = resize_hit(screen, pos, RESIZE_EDGE, RESIZE_CORNER) else { return };
                ctx.set_cursor_icon(resize_cursor(dir));
                if pressed && resp.hovered() {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0))
    }

    #[test]
    fn resize_edges_and_corners() {
        let w = win();
        let hit = |x: f32, y: f32| resize_hit(w, pos2(x, y), RESIZE_EDGE, RESIZE_CORNER);
        use ResizeDirection as D;
        assert_eq!(hit(500.0, 300.0), None, "interior");
        assert_eq!(hit(500.0, 6.0), None, "just inside the edge zone");
        assert_eq!(hit(-1.0, 300.0), None, "outside the window");
        assert_eq!(hit(500.0, 0.0), Some(D::North));
        assert_eq!(hit(500.0, 599.0), Some(D::South));
        assert_eq!(hit(1.0, 300.0), Some(D::West));
        assert_eq!(hit(998.0, 300.0), Some(D::East));
        assert_eq!(hit(2.0, 2.0), Some(D::NorthWest));
        assert_eq!(hit(10.0, 1.0), Some(D::NorthWest), "corner zone runs along the top edge");
        assert_eq!(hit(1.0, 10.0), Some(D::NorthWest), "and along the left edge");
        assert_eq!(hit(20.0, 1.0), Some(D::North), "past the corner length");
        assert_eq!(hit(995.0, 3.0), Some(D::NorthEast));
        assert_eq!(hit(3.0, 590.0), Some(D::SouthWest));
        assert_eq!(hit(1000.0, 600.0), Some(D::SouthEast), "max corner is inclusive");
        // The edge zone never reaches a scrollbar 8 px in from the right.
        assert_eq!(hit(1000.0 - 8.0, 300.0), None);
    }

    #[test]
    fn cursors_match_directions() {
        assert_eq!(resize_cursor(ResizeDirection::NorthEast), CursorIcon::ResizeNorthEast);
        assert_eq!(resize_cursor(ResizeDirection::West), CursorIcon::ResizeWest);
    }

    #[test]
    fn layout_aligns_brand_with_sidebar() {
        let bar = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, HEIGHT));
        let l = layout(bar, Some(220.0), 0.0, true);
        assert_eq!(l.brand.left(), 0.0);
        assert_eq!(l.brand.right(), 220.0);
        assert_eq!(l.tabs.left(), 220.0);
        assert_eq!(l.controls.width(), 3.0 * CONTROL_W);
        assert_eq!(l.controls.right(), 1280.0);
        assert_eq!(l.tabs.right(), l.controls.left() - MIN_DRAG, "drag space before the controls");
        assert!(l.brand.height() == HEIGHT && l.tabs.height() == HEIGHT);
    }

    #[test]
    fn layout_compact_and_mac() {
        let bar = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, HEIGHT));
        // No sidebar: compact brand.
        let l = layout(bar, None, 0.0, true);
        assert_eq!(l.brand.width(), BRAND_MIN);
        // A sidebar narrower than the brand still leaves room for the logo + wordmark.
        assert_eq!(layout(bar, Some(60.0), 0.0, true).brand.right(), BRAND_MIN);
        // macOS: traffic lights on the left, no controls on the right.
        let m = layout(bar, Some(220.0), MAC_TRAFFIC_LIGHTS, false);
        assert_eq!(m.brand.left(), MAC_TRAFFIC_LIGHTS);
        assert_eq!(m.brand.right(), 220.0);
        assert!(m.controls.width() < CONTROL_W);
    }

    #[test]
    fn layout_degrades_on_tiny_windows() {
        let bar = Rect::from_min_size(Pos2::ZERO, vec2(200.0, HEIGHT));
        let l = layout(bar, Some(220.0), 0.0, true);
        assert!(l.brand.right() <= l.controls.left());
        assert!(l.tabs.width() >= 0.0 && l.tabs.left() == l.brand.right());
        assert_eq!(l.controls.right(), 200.0);
    }

    #[test]
    fn control_rects_split_evenly() {
        let r = Rect::from_min_max(pos2(100.0, 0.0), pos2(238.0, HEIGHT));
        let [a, b, c] = control_rects(r);
        assert_eq!(a.left(), 100.0);
        assert_eq!(a.right(), b.left());
        assert_eq!(b.right(), c.left());
        assert_eq!(c.right(), 238.0);
        assert!((a.width() - 46.0).abs() < 1e-4);
    }

    #[test]
    fn close_button_follows_the_window_corner() {
        assert_eq!(close_corner(true, true).ne, crate::ui::native_chrome::DWM_CORNER_RADIUS);
        assert_eq!(close_corner(true, true).nw, 0);
        assert_eq!(close_corner(true, false), CornerRadius::ZERO, "maximized: square");
        assert_eq!(close_corner(false, true), CornerRadius::ZERO, "Windows 10 / Linux: square");
    }

    #[test]
    fn chrome_for_config() {
        assert_eq!(Chrome::for_config(false), Chrome::Native);
        let on = Chrome::for_config(true);
        assert!(on.has_titlebar());
        assert_eq!(on.own_frame(), !cfg!(target_os = "macos"));
        assert!(!Chrome::Native.has_titlebar() && !Chrome::Native.own_frame());
    }
}
