//! Drag and drop, the egui side. Internal drags carry a [`DragPayload`] through egui's
//! `DragAndDrop` (panes, tabs, sidebar favorites/devices/trash and breadcrumb segments
//! are drop targets); external drops come from `raw.dropped_files` and always copy.
//! Effects, valid targets and the external drop target are decided in
//! [`crate::ops::fileops::drop`] (modifier semantics there).
//!
//! Spring-loaded folders: resting on a folder item, breadcrumb segment, tab or
//! sidebar row for `[behaviour] spring_load_delay_ms` opens it, so a drag can go deeper.

pub mod dragout;
pub mod linux_dnd;

use std::path::{Path, PathBuf};

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, DragAndDrop, FontId, Id, Order, Rect, Response, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use soyuz_core::config::DragDefault;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::fileops::drop::{self, is_noop};
pub use crate::ops::fileops::drop::{DndFrame, DragPayload, DropEffect, DropMods, valid_target};
use crate::ops::fileops::{self, FileOpCmd};
use crate::state::pane::{Pane, PaneId};
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::geom;

/// Drop modifiers from egui's (Cmd counts as Ctrl).
pub fn drop_mods(m: egui::Modifiers) -> DropMods {
    DropMods { ctrl: m.command || m.ctrl, shift: m.shift, alt: m.alt }
}

/// Effect for the current modifiers, dropping `payload` into `dest`.
pub fn effect_for(ctx: &egui::Context, payload: &DragPayload, dest: &Path) -> Option<DropEffect> {
    drop::effect_for(drop_mods(ctx.input(|i| i.modifiers)), payload, dest)
}

fn effect_glyph(e: DropEffect) -> &'static str {
    match e {
        DropEffect::Copy => ph::COPY,
        DropEffect::Move => ph::ARROW_BEND_DOWN_RIGHT,
        DropEffect::Link => ph::LINK,
    }
}

/// Start an internal drag with the pane's selection.
pub fn begin(ctx: &egui::Context, pane: &Pane, default: DragDefault) {
    let paths: Vec<PathBuf> = pane.selected_entries().iter().map(|e| e.path.clone()).collect();
    let draggable = matches!(pane.location, soyuz_core::Location::Dir(_))
        || crate::ui::tools::tags_ui::is_virtual_listing(&pane.location);
    if paths.is_empty() || !draggable {
        return;
    }
    drop::reset_volume_cache();
    DragAndDrop::set_payload(ctx, DragPayload { paths, source_pane: Some(pane.id), default });
}

/// The active internal drag, if any.
pub fn payload(ctx: &egui::Context) -> Option<std::sync::Arc<DragPayload>> {
    DragAndDrop::payload::<DragPayload>(ctx)
}

/// What the badge says about the current target.
#[derive(Clone, Debug, Default)]
struct HoverInfo {
    label: String,
    effect: Option<DropEffect>,
}

fn hover_id() -> Id {
    Id::new("soyuz.dnd.hover")
}

fn set_hover(ctx: &egui::Context, label: String, effect: Option<DropEffect>) {
    ctx.data_mut(|d| d.insert_temp(hover_id(), HoverInfo { label, effect }));
}

/// Paint the drop highlight (2 px accent inset stroke + faint fill).
pub fn highlight(ui: &Ui, rect: Rect, rounding: u8) {
    let rect = rect.intersect(ui.clip_rect());
    if !rect.is_positive() {
        return;
    }
    let p = palette(ui.ctx());
    let painter = ui.ctx().layer_painter(egui::LayerId::new(Order::Foreground, Id::new("soyuz.dnd.hl")));
    painter.rect_filled(rect, CornerRadius::same(rounding), p.accent.gamma_multiply(0.08));
    painter.rect_stroke(rect, CornerRadius::same(rounding), Stroke::new(2.0, p.accent), StrokeKind::Inside);
}

/// Handle hover + release of an internal drag over `rect` targeting `dest`. Returns
/// `true` while this target is hovered.
pub fn target_rect(ui: &Ui, rect: Rect, dest: &Path, rounding: u8, cmds: &mut Vec<Cmd>) -> bool {
    target_rect_spring(ui, rect, dest, rounding, cmds, None)
}

fn spring_id() -> Id {
    Id::new("soyuz.dnd.spring")
}

/// Spring-loading: run `open` once the pointer rested on `dest` long enough; paints a
/// filling hint meanwhile.
fn spring(ui: &Ui, rect: Rect, dest: &Path, rounding: u8, open: Cmd, cmds: &mut Vec<Cmd>) {
    let Some(delay) = crate::ui::looks::spring_delay() else { return };
    let ctx = ui.ctx();
    let now = ctx.input(|i| i.time);
    let mut timer: crate::ui::motion::HoverTimer = ctx.data(|d| d.get_temp(spring_id())).unwrap_or_default();
    let fire = timer.update(dest, now, delay);
    let progress = timer.progress(now, delay);
    let fired = timer.fired;
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(spring_id(), timer);
        d.insert_temp(spring_id().with("pass"), pass);
    });
    if fire {
        cmds.push(open);
    } else if !fired {
        ctx.request_repaint();
        // A fill that grows from the left edge while the timer runs.
        if progress > 0.15 && !crate::theme::reduced_motion(ctx) {
            let p = palette(ctx);
            let r = rect.intersect(ui.clip_rect());
            let w = r.width() * crate::ui::motion::ease_out((progress - 0.15) / 0.85);
            let fill = Rect::from_min_size(r.min, vec2(w, r.height()));
            let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("soyuz.dnd.hl")));
            painter.rect_filled(fill, CornerRadius::same(rounding), p.accent.gamma_multiply(0.10));
        }
    }
}

/// [`target_rect`] that also spring-loads: `open` runs after the hover delay.
pub fn target_rect_spring(
    ui: &Ui,
    rect: Rect,
    dest: &Path,
    rounding: u8,
    cmds: &mut Vec<Cmd>,
    open: Option<Cmd>,
) -> bool {
    let ctx = ui.ctx();
    let Some(payload) = payload(ctx) else { return false };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return false };
    if !rect.contains(pos) || !ui.clip_rect().contains(pos) || !valid_target(&payload.paths, dest) {
        return false;
    }
    let released = ctx.input(|i| i.pointer.any_released());
    // Modifiers are latched while hovering: a modifier key let go in the release frame
    // must not change the effect the badge promised.
    let key = Id::new(("soyuz.dnd.effect", dest));
    let effect = match ctx.data(|d| d.get_temp::<Option<DropEffect>>(key)) {
        Some(e) if released => e,
        _ => effect_for(ctx, &payload, dest),
    };
    if is_noop(&payload.paths, dest, effect) {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(key, effect));
    highlight(ui, rect, rounding);
    if !released && let Some(open) = open {
        spring(ui, rect, dest, rounding, open, cmds);
    }
    set_hover(ctx, format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(dest)), effect);
    if released && let Some(p) = DragAndDrop::take_payload::<DragPayload>(ctx) {
        ctx.data_mut(|d| d.remove::<Option<DropEffect>>(key));
        let pos = geom::point(pos);
        cmds.push(Cmd::FileOp(FileOpCmd::Drop { paths: p.paths.clone(), dest: dest.to_path_buf(), effect, pos }));
    }
    true
}

/// Drop target on any response (sidebar rows, breadcrumb segments, tabs).
pub fn target(ui: &Ui, resp: &Response, dest: &Path, cmds: &mut Vec<Cmd>) -> bool {
    target_rect(ui, resp.rect, dest, radius::ROW, cmds)
}

/// [`target`] that spring-loads: hovering opens `dest` in `pane`.
pub fn target_open(ui: &Ui, resp: &Response, dest: &Path, pane: PaneId, cmds: &mut Vec<Cmd>) -> bool {
    let open = Cmd::Navigate { pane, location: soyuz_core::Location::Dir(dest.to_path_buf()), select: None };
    target_rect_spring(ui, resp.rect, dest, radius::ROW, cmds, Some(open))
}

/// [`target`] that spring-loads with a custom command (tabs: select the tab).
pub fn target_with(ui: &Ui, resp: &Response, dest: &Path, open: Cmd, cmds: &mut Vec<Cmd>) -> bool {
    target_rect_spring(ui, resp.rect, dest, radius::ROW, cmds, Some(open))
}

/// The sidebar Trash row: dropping moves the items to the trash.
pub fn trash_target(ui: &Ui, resp: &Response, cmds: &mut Vec<Cmd>) -> bool {
    let ctx = ui.ctx();
    let Some(_) = payload(ctx) else { return false };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return false };
    if !resp.rect.contains(pos) {
        return false;
    }
    highlight(ui, resp.rect, radius::ROW);
    set_hover(ctx, tr!("ops-dnd-trash").into(), None);
    ctx.data_mut(|d| d.insert_temp(Id::new("soyuz.dnd.trash"), true));
    if ctx.input(|i| i.pointer.any_released())
        && let Some(p) = DragAndDrop::take_payload::<DragPayload>(ctx)
    {
        cmds.push(Cmd::FileOp(FileOpCmd::TrashPaths(p.paths.clone())));
    }
    true
}

/// Badge for a drop target that pins the dragged folders instead of transferring them
/// (the sidebar Portal): `label` is what the badge says.
pub fn set_pin_hover(ctx: &egui::Context, label: String) {
    set_hover(ctx, label, None);
    ctx.data_mut(|d| d.insert_temp(Id::new("soyuz.dnd.pin"), true));
}

/// Pane-level target: drop into the pane's folder (background, empty folder state),
/// plus the external-drop highlight. Called once per drawn pane.
pub fn pane_zone(ui: &Ui, rect: Rect, pane: &Pane, frame: &mut DndFrame, cmds: &mut Vec<Cmd>) {
    let dir = match &pane.location {
        soyuz_core::Location::Dir(d) => Some(d.clone()),
        _ => None,
    };
    frame.panes.push((pane.id, geom::area(rect), dir.clone()));
    let Some(dir) = dir else { return };
    let ctx = ui.ctx();
    let external = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if external {
        let pos = ctx.input(|i| i.pointer.latest_pos());
        if pos.is_some_and(|p| rect.contains(p)) {
            highlight(ui, rect.shrink(2.0), radius::CARD);
        }
        return;
    }
    if frame.item_claimed {
        return;
    }
    target_rect(ui, rect.shrink(2.0), &dir, radius::CARD, cmds);
}

/// Folder items inside a view: `hit` is the item under the pointer and its rect.
pub fn item_target(ui: &Ui, pane: &Pane, hit: Option<(usize, Rect)>, frame: &mut DndFrame, cmds: &mut Vec<Cmd>) {
    let Some((i, rect)) = hit else { return };
    let Some(e) = pane.entry_at(i) else { return };
    if !e.is_dir_like() {
        return;
    }
    let path = e.path.clone();
    let open = Cmd::Navigate { pane: pane.id, location: soyuz_core::Location::Dir(path.clone()), select: None };
    if target_rect_spring(ui, rect, &path, radius::ROW, cmds, Some(open)) {
        frame.item_claimed = true;
    }
}

/// Floating badge next to the pointer: item count, effect and target.
pub fn paint_badge(ctx: &egui::Context) {
    let hover: Option<HoverInfo> = ctx.data_mut(|d| d.remove_temp(hover_id()));
    let pass = ctx.cumulative_pass_nr();
    if ctx.data(|d| d.get_temp::<u64>(spring_id().with("pass"))) != Some(pass) {
        ctx.data_mut(|d| d.remove::<crate::ui::motion::HoverTimer>(spring_id()));
    }
    let to_trash: bool = ctx.data_mut(|d| d.remove_temp::<bool>(Id::new("soyuz.dnd.trash"))).unwrap_or(false);
    let to_pin: bool = ctx.data_mut(|d| d.remove_temp::<bool>(Id::new("soyuz.dnd.pin"))).unwrap_or(false);
    let Some(payload) = payload(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    if !ctx.input(|i| i.pointer.primary_down()) {
        return;
    }
    let p = palette(ctx);
    let n = payload.paths.len();
    let what = if n == 1 { soyuz_core::entry::display_name(&payload.paths[0]) } else { tr!("common-items", count = n) };
    let (glyph, line2, accent) = match &hover {
        Some(h) if to_trash => (ph::TRASH, tr!("ops-dnd-badge-trash", target = &h.label), p.danger),
        Some(h) if to_pin => (ph::PUSH_PIN, h.label.clone(), p.accent),
        Some(h) => match h.effect {
            Some(e) => {
                let line = match e {
                    DropEffect::Copy => tr!("ops-dnd-badge-copy", target = &h.label),
                    DropEffect::Move => tr!("ops-dnd-badge-move", target = &h.label),
                    DropEffect::Link => tr!("ops-dnd-badge-link", target = &h.label),
                };
                (effect_glyph(e), line, p.accent)
            }
            None => (ph::QUESTION, tr!("ops-dnd-badge-drop", target = &h.label), p.accent),
        },
        None => (ph::FILES, String::new(), p.text_tertiary),
    };
    ctx.set_cursor_icon(match hover.as_ref().and_then(|h| h.effect) {
        _ if hover.is_none() => CursorIcon::NoDrop,
        _ if to_pin => CursorIcon::Alias,
        Some(DropEffect::Copy) => CursorIcon::Copy,
        Some(DropEffect::Link) => CursorIcon::Alias,
        _ => CursorIcon::Grabbing,
    });
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("soyuz.dnd.badge")));
    let title = crate::ui::widgets::trunc_galley(
        &painter,
        &what,
        crate::ui::views::flow::semibold(12.5),
        p.text_primary,
        220.0,
    );
    let sub = (!line2.is_empty())
        .then(|| crate::ui::widgets::trunc_galley(&painter, &line2, FontId::proportional(11.5), accent, 220.0));
    let w = 46.0 + title.size().x.max(sub.as_ref().map_or(0.0, |g| g.size().x)) + 14.0;
    let h = if sub.is_some() { 44.0 } else { 32.0 };
    let r = Rect::from_min_size(pos + vec2(16.0, 14.0), vec2(w, h));
    let shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
    };
    painter.add(shadow.as_shape(r, CornerRadius::same(radius::CARD)));
    painter.rect_filled(r, CornerRadius::same(radius::CARD), p.bg_surface3);
    painter.rect_stroke(r, CornerRadius::same(radius::CARD), Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    // Stacked-documents glyph with a count bubble.
    let gc = pos2(r.left() + 22.0, r.center().y);
    if n > 1 {
        // Two cards peeking out behind the glyph: "many items".
        for k in [2.0f32, 1.0] {
            let card = Rect::from_center_size(gc + vec2(k * 2.5, -k * 2.5), vec2(16.0, 19.0));
            painter.rect_filled(card, CornerRadius::same(3), p.bg_surface2);
            painter.rect_stroke(card, CornerRadius::same(3), Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        }
    }
    painter.text(gc, Align2::CENTER_CENTER, glyph, icons::font_fill(18.0), accent);
    // Effect chip (+ copy, ↗ link) at the glyph's bottom-right, like the OS cursors.
    let chip = match hover.as_ref().and_then(|h| h.effect) {
        Some(DropEffect::Copy) if !to_trash => Some((ph::PLUS, p.success)),
        Some(DropEffect::Link) if !to_trash => Some((ph::ARROW_UP_RIGHT, p.info)),
        _ => None,
    };
    if let Some((g, c)) = chip {
        let cc = gc + vec2(8.0, 8.0);
        painter.circle_filled(cc, 7.0, p.bg_surface3);
        painter.circle_filled(cc, 5.8, c);
        painter.text(cc, Align2::CENTER_CENTER, g, icons::font(9.0), Color32::WHITE);
    }
    if n > 1 {
        let b = pos2(r.left() + 32.0, r.top() + 9.0);
        let label = if n > 99 { "99+".to_string() } else { n.to_string() };
        let g = painter.layout_no_wrap(label, FontId::proportional(9.5), Color32::WHITE);
        let br = Rect::from_center_size(b, vec2((g.size().x + 8.0).max(15.0), 15.0));
        painter.rect_filled(br, CornerRadius::same(8), p.accent);
        painter.galley(br.center() - g.size() / 2.0, g, Color32::WHITE);
    }
    let tx = r.left() + 44.0;
    if let Some(sub) = sub {
        painter.galley(pos2(tx, r.top() + 7.0), title, p.text_primary);
        painter.galley(pos2(tx, r.top() + 24.0), sub, accent);
    } else {
        painter.galley(pos2(tx, r.center().y - title.size().y / 2.0), title, p.text_primary);
    }
    ctx.request_repaint();
}

/// End of frame: hand a drag that left the window to the OS, paint the drag badge, show
/// the "ask" drop menu.
pub fn end_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    dragout::end_frame(app, ctx);
    paint_badge(ctx);
    ask_menu(app, ctx);
}

/// "Move here / Copy here / Link here" at the drop point (`drag_default = "ask"`).
fn ask_menu(app: &mut SoyuzApp, ctx: &egui::Context) {
    let Some(menu) = app.fileops.dnd.ask.clone() else { return };
    let mut close = false;
    let mut chosen = None;
    let resp = egui::Area::new(Id::new("soyuz.dnd.ask"))
        .order(egui::Order::Foreground)
        .fixed_pos(geom::pos(menu.pos))
        .show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(200.0);
                use crate::ui::widgets::{menu_item, menu_separator};
                if menu_item(ui, Some(ph::ARROW_BEND_DOWN_RIGHT), tr!("ops-dnd-move-here"), None, true).clicked() {
                    chosen = Some(DropEffect::Move);
                }
                if menu_item(ui, Some(ph::COPY), tr!("ops-dnd-copy-here"), None, true).clicked() {
                    chosen = Some(DropEffect::Copy);
                }
                if menu_item(ui, Some(ph::LINK), tr!("ops-dnd-link-here"), None, true).clicked() {
                    chosen = Some(DropEffect::Link);
                }
                menu_separator(ui);
                if menu_item(ui, Some(ph::X), tr!("common-cancel"), None, true).clicked() {
                    close = true;
                }
            });
        });
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
        && !ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| resp.response.rect.contains(p));
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || clicked_outside {
        close = true;
    }
    if let Some(effect) = chosen {
        app.fileops.dnd.ask = None;
        fileops::transfer(app, menu.paths, menu.dest, effect);
    } else if close {
        app.fileops.dnd.ask = None;
    }
}

/// Files dropped onto the window from other apps this frame: copied into the pane under
/// the pointer (else the focused one).
pub fn external_drops(app: &mut SoyuzApp, ctx: &egui::Context) {
    let (dropped, pointer) = ctx.input(|i| {
        (
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect::<Vec<PathBuf>>(),
            i.pointer.latest_pos(),
        )
    });
    fileops::drop_external(app, dropped, pointer.map(geom::point));
}
