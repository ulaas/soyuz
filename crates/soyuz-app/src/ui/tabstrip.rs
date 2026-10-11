//! Browser-style tab strip: pinned tabs (icon only) first, active tab as a raised pill,
//! close on hover, middle-click close, drag to reorder, double-click empty space or `+`
//! for a new tab, right-click for tab actions. While a tab is dragged the others make
//! room live, each sliding on a spring; hovering a tab during a file drag selects
//! it (spring-loading).
//!
//! Tab groups: a colored chip opens each group (click: collapse / expand, double
//! click: rename, right click: group menu) and the group's tabs carry its color. Drop a
//! tab between two tabs of a group (or on its chip) to add it. Drag a tab away from the
//! strip to tear it off into a new window, or onto another window to move it there.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, LayerId, Margin, Order, Panel, Pos2, Rect, Sense,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use soyuz_core::config::Action;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::state::crumbs::location_title;
use crate::state::workspace::{GroupId, TabId};
use crate::state::workspace_windows::{DropTarget, WindowGeom, WindowId, resolve_drop};
use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{menu_item, menu_separator, trunc_galley};
use crate::ui::workspaces::WsCmd;

const PINNED_W: f32 = 40.0;
const MAX_W: f32 = 220.0;
const MIN_W: f32 = 96.0;
const CHIP_H: f32 = 22.0;

#[derive(Clone, Copy, Debug)]
struct Drag {
    window: WindowId,
    tab: TabId,
    from: usize,
    /// Pointer offset inside the tab when the drag started.
    grab: Vec2,
    /// Last known pointer position (it is unknown while outside the window).
    last: Pos2,
}

pub fn show(root: &mut Ui, app: &mut SoyuzApp) {
    let p = palette(root.ctx());
    Panel::top("soyuz.tabstrip")
        .exact_size(size::TAB_STRIP)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 6, right: 8, top: 4, bottom: 0 }))
        .show(root, |ui| {
            let full = ui.available_rect_before_wrap();
            strip(ui, full, app, false);
        });
}

/// Where tabs and group chips go when laid out in `order`.
#[derive(Clone, Debug, PartialEq)]
pub struct Slots {
    /// Left edge of every tab (indexed by tab).
    pub lefts: Vec<f32>,
    /// `(group, left)` of each group chip, in strip order.
    pub chips: Vec<(GroupId, f32)>,
}

/// Lay out tabs in `order` with a chip in front of each group. `skip` (the dragged tab)
/// never opens a chip of its own. Zero-width tabs (collapsed groups) take no space.
pub fn strip_slots(
    order: &[usize],
    widths: &[f32],
    groups: &[Option<GroupId>],
    chip_w: &dyn Fn(GroupId) -> f32,
    skip: Option<usize>,
    start: f32,
) -> Slots {
    let mut lefts = vec![start; widths.len()];
    let mut chips = Vec::new();
    let mut x = start;
    let mut prev: Option<GroupId> = None;
    for &k in order {
        if Some(k) != skip {
            let g = groups[k];
            if let Some(gid) = g
                && prev != Some(gid)
            {
                chips.push((gid, x));
                x += chip_w(gid) + 4.0;
            }
            prev = g;
        }
        lefts[k] = x;
        if widths[k] > 0.0 {
            x += widths[k] + 2.0;
        }
    }
    Slots { lefts, chips }
}

/// The group a tab dropped between `left` and `right` (their groups) ends up in: inside a
/// group it joins; at a group's edge it stays only if it was a member already.
pub fn drop_group(left: Option<GroupId>, right: Option<GroupId>, own: Option<GroupId>) -> Option<GroupId> {
    match (left, right) {
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(a), _) if own == Some(a) => Some(a),
        (_, Some(b)) if own == Some(b) => Some(b),
        _ => None,
    }
}

fn chip_label(name: &str, collapsed: bool, count: usize) -> String {
    let name = if name.trim().is_empty() { "" } else { name.trim() };
    match (collapsed, name.is_empty()) {
        (true, true) => format!("{count}"),
        (true, false) => format!("{name}  {count}"),
        (false, _) => name.to_string(),
    }
}

/// The tabs and the new-tab button inside `full` (the tab strip panel, or the tab area
/// of the custom title bar). In the title bar, empty space is left to the title bar
/// (drag / double-click to maximize) instead of opening a new tab on double-click.
pub fn strip(ui: &mut Ui, full: Rect, app: &mut SoyuzApp, in_titlebar: bool) {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let n = app.ws.tabs.len();
    let window = app.workspaces.current;
    let plus_w = 32.0;
    let drag_id = Id::new("soyuz.tabdrag");
    let mut drag: Option<Drag> = ctx
        .data(|d| d.get_temp(drag_id))
        .filter(|d: &Drag| d.window == window && d.from < n && app.ws.tabs[d.from].id == d.tab);
    let pointer = ui.input(|i| i.pointer.latest_pos());
    if let (Some(d), Some(pp)) = (drag.as_mut(), pointer) {
        d.last = pp;
    }

    // Background: double-click empty space → new tab.
    if !in_titlebar {
        let bg = ui.interact(full, Id::new("soyuz.tabstrip.bg"), Sense::click());
        crate::ui::a11y::container(&ctx, bg.id, egui::accesskit::Role::TabList, tr!("app-tabs-list"));
        if bg.double_clicked() {
            app.cmds.push(Cmd::Action(Action::NewTab));
        }
    }

    // ---- widths ----
    let chip_font = FontId::proportional(11.5);
    let groups_of: Vec<Option<GroupId>> = app.ws.tabs.iter().map(|t| t.group).collect();
    let collapsed = |g: GroupId| app.ws.group(g).is_some_and(|x| x.collapsed);
    let hidden: Vec<bool> = groups_of.iter().map(|g| g.is_some_and(collapsed)).collect();
    let renaming = app.workspaces.renaming.clone().filter(|(g, _, _)| app.ws.group(*g).is_some());
    let chip_widths: Vec<(GroupId, f32)> = app
        .ws
        .groups
        .iter()
        .map(|g| {
            let label = chip_label(&g.name, g.collapsed, app.ws.group_tab_count(g.id));
            let w = if renaming.as_ref().is_some_and(|(r, _, _)| *r == g.id) {
                130.0
            } else if label.is_empty() {
                CHIP_H
            } else {
                ui.painter().layout_no_wrap(label, chip_font.clone(), Color32::WHITE).size().x + 20.0
            };
            (g.id, w.min(160.0))
        })
        .collect();
    let chip_w = |g: GroupId| chip_widths.iter().find(|(x, _)| *x == g).map_or(CHIP_H, |(_, w)| *w);
    let chips_total: f32 = app.ws.groups.iter().map(|g| chip_w(g.id) + 4.0).sum();
    let pinned = app.ws.tabs.iter().filter(|t| t.pinned).count();
    let avail = full.width() - plus_w - 4.0 - chips_total;
    let shown_unpinned = (0..n).filter(|&i| !app.ws.tabs[i].pinned && !hidden[i]).count().max(1) as f32;
    let w = ((avail - pinned as f32 * (PINNED_W + 2.0)) / shown_unpinned - 2.0).clamp(MIN_W, MAX_W);
    let widths: Vec<f32> = (0..n)
        .map(|i| {
            if hidden[i] {
                0.0
            } else if app.ws.tabs[i].pinned {
                PINNED_W
            } else {
                w
            }
        })
        .collect();

    // ---- layout: static (for hit testing / drop index) and live (while dragging) ----
    let identity: Vec<usize> = (0..n).collect();
    let base = strip_slots(&identity, &widths, &groups_of, &chip_w, None, full.left());
    let rects: Vec<Rect> = (0..n)
        .map(|i| Rect::from_min_size(pos2(base.lefts[i], full.top()), vec2(widths[i], full.height() - 4.0)))
        .collect();
    let clip = Rect::from_min_max(full.min, pos2(full.right() - plus_w, full.bottom()));

    // Record this window's geometry (global coordinates) for drops from other windows.
    let inner = ctx.input(|i| i.viewport().inner_rect);
    if let Some(inner) = inner {
        let off = inner.min.to_vec2();
        let tabs =
            (0..n).filter(|&i| !hidden[i]).map(|i| (i, crate::ui::geom::area(rects[i].translate(off)))).collect();
        app.workspaces.set_geom(WindowGeom {
            id: window,
            inner: crate::ui::geom::area(inner),
            strip: crate::ui::geom::area(full.translate(off)),
            tabs,
        });
    }

    let distance = app.config.workspace.tear_off_distance.clamp(8.0, 400.0);
    let can_give = n > 1 || !window.is_main();
    let tear_ok = app.config.workspace.tear_off && n > 1;
    // Out of the strip: a tear-off / move-to-window candidate.
    let far = drag.is_some_and(|d| {
        let pp = d.last;
        let outside = !ctx.content_rect().contains(pp);
        outside || pp.y > full.bottom() + distance || pp.y < full.top() - distance
    });
    let away_target = match (drag, far, inner) {
        (Some(d), true, Some(inner)) if can_give => Some(resolve_drop(
            window,
            crate::ui::geom::point(inner.min + d.last.to_vec2()),
            &app.workspaces.geoms,
            distance,
            tear_ok,
        )),
        _ => None,
    };

    let order = match drag {
        Some(d) if !far => live_order(n, d.from, drop_index(&rects, d.from, d.last.x - d.grab.x)),
        _ => identity.clone(),
    };
    let live = strip_slots(&order, &widths, &groups_of, &chip_w, drag.filter(|_| !far).map(|d| d.from), full.left());
    let painter = ui.painter_at(clip);

    // ---- group chips ----
    let mut hovered_chip: Option<(GroupId, Rect)> = None;
    for &(g, left) in &live.chips {
        let Some(group) = app.ws.group(g).cloned() else { continue };
        let cw = chip_w(g);
        let spring = Id::new(("soyuz.chip.x", g.0));
        let x = crate::ui::motion::spring_to(&ctx, spring, left);
        let r = Rect::from_min_size(pos2(x, full.center().y - CHIP_H / 2.0 - 2.0), vec2(cw, CHIP_H));
        let color = p.tags[(group.color as usize).min(7)];
        let resp = ui.interact(r, Id::new(("soyuz.chip", g.0)), Sense::click());
        let editing = renaming.as_ref().is_some_and(|(x, _, _)| *x == g);
        let fill = if group.collapsed { color.gamma_multiply(0.35) } else { color };
        if !editing {
            painter.rect_filled(r, CornerRadius::same(radius::CONTROL), fill);
            if resp.hovered() {
                painter.rect_stroke(
                    r,
                    CornerRadius::same(radius::CONTROL),
                    Stroke::new(1.0, p.text_primary.gamma_multiply(0.5)),
                    StrokeKind::Outside,
                );
            }
            let label = chip_label(&group.name, group.collapsed, app.ws.group_tab_count(g));
            let text_color = if group.collapsed { p.text_primary } else { Color32::from_gray(18) };
            let g2 = trunc_galley(&painter, &label, chip_font.clone(), text_color, cw - 14.0);
            painter.galley(pos2(r.left() + 7.0, r.center().y - g2.size().y / 2.0), g2, text_color);
        }
        // Group underline joining the chip with its tabs.
        if let Some(span) = app.ws.group_span(g).filter(|_| !group.collapsed) {
            let last = (span.start..span.end).rfind(|&i| !hidden[i]).unwrap_or(span.start);
            let tx =
                crate::ui::motion::spring_to(&ctx, Id::new(("soyuz.tab.x", app.ws.tabs[last].id.0)), live.lefts[last]);
            let right = tx + widths[last];
            let y = full.bottom() - 3.5;
            painter.line_segment(
                [pos2(r.center().x, y), pos2(right - 6.0, y)],
                Stroke::new(2.0, color.gamma_multiply(0.9)),
            );
        }
        let title = if group.name.is_empty() {
            tr!("app-tabs-group").to_string()
        } else {
            tr!("app-tabs-group-named", name = &group.name)
        };
        crate::ui::a11y::button(&resp, &title);
        if drag.is_some() && pointer.is_some_and(|pp| r.expand(4.0).contains(pp)) {
            hovered_chip = Some((g, r));
            painter.rect_stroke(
                r.expand(2.0),
                CornerRadius::same(radius::CONTROL),
                Stroke::new(1.5, p.accent),
                StrokeKind::Outside,
            );
        }
        if editing {
            group_rename(ui, app, g, r);
        } else {
            let resp = resp.on_hover_text(if group.collapsed {
                tr!("app-tabs-expand-group")
            } else {
                tr!("app-tabs-collapse-group")
            });
            if resp.double_clicked() {
                app.cmds.push(Cmd::Workspace(WsCmd::StartRenameGroup(g)));
            } else if resp.clicked() {
                app.cmds.push(Cmd::Workspace(WsCmd::ToggleGroup(g)));
            }
            let mut cmds = Vec::new();
            crate::ui::native::popup::context_menu(&resp, |ui| {
                crate::ui::workspaces::group_menu(ui, app, &mut cmds, g)
            });
            app.cmds.extend(cmds);
        }
    }

    // ---- tabs ----
    // Drawn rect and emphasis (1 = active / dragged, else hover) of every shown tab, for
    // the separators.
    let mut drawn: Vec<Option<(Rect, f32)>> = vec![None; n];
    for i in 0..n {
        if hidden[i] {
            continue;
        }
        let r = rects[i];
        let tab = &app.ws.tabs[i];
        let tab_id = tab.id;
        let active = i == app.ws.active;
        let resp = ui.interact(r, Id::new(("soyuz.tab", tab.id.0)), Sense::click_and_drag());
        let dragging = drag.is_some_and(|d| d.from == i);
        let spring_id = Id::new(("soyuz.tab.x", tab.id.0));
        let r = if dragging && !far {
            let px = d_left(drag, r.left());
            let x = px.clamp(full.left(), clip.right() - r.width());
            crate::ui::motion::spring_set(&ctx, spring_id, x);
            Rect::from_min_size(pos2(x, r.top()), r.size())
        } else {
            let x = crate::ui::motion::spring_to(&ctx, spring_id, live.lefts[i]);
            Rect::from_min_size(pos2(x, r.top()), r.size())
        };
        let ghosted = dragging && far;
        let hovered = resp.hovered() || dragging;
        let hover_t = ctx.animate_bool_with_time(resp.id.with("h"), hovered, crate::theme::anim(&ctx, 0.1));
        let tab_painter = if ghosted { faded(painter.clone()) } else { painter.clone() };
        paint_tab(ui, &tab_painter, app, i, r, active, dragging, hover_t);
        drawn[i] = Some((r, if active || dragging { 1.0 } else { hover_t }));
        let pane = app.ws.tabs[i].active();
        let title = location_title(&pane.location);
        crate::ui::a11y::tab(&resp, &title, active);
        let show_close = (active || hovered) && !app.ws.tabs[i].pinned;
        if show_close && n > 1 && !ghosted {
            let cr = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(20.0, 20.0));
            let cresp = ui.interact(cr, Id::new(("soyuz.tab.close", tab_id.0)), Sense::click());
            crate::ui::a11y::button(&cresp, &tr!("app-tabs-close-named", title = &title));
            if cresp.hovered() {
                painter.rect_filled(cr, CornerRadius::same(radius::CONTROL), p.bg_surface3);
            }
            painter.text(
                cr.center(),
                Align2::CENTER_CENTER,
                ph::X,
                icons::font(11.0),
                if cresp.hovered() { p.text_primary } else { p.text_tertiary },
            );
            if cresp.clicked() {
                app.cmds.push(Cmd::CloseTab(i));
            }
        }
        let resp = resp.on_hover_text(app.ws.tabs[i].active().location.display());
        if let soyuz_core::Location::Dir(d) = &app.ws.tabs[i].active().location {
            let d = d.clone();
            if active {
                crate::ops::fileops::dnd::target(ui, &resp, &d, &mut app.cmds);
            } else {
                crate::ops::fileops::dnd::target_with(ui, &resp, &d, Cmd::SelectTab(i), &mut app.cmds);
            }
        }
        if resp.clicked() {
            app.cmds.push(Cmd::SelectTab(i));
        }
        if resp.middle_clicked() {
            app.cmds.push(Cmd::CloseTab(i));
        }
        if resp.drag_started() {
            let pp = pointer.unwrap_or(rects[i].min);
            let d = Drag { window, tab: tab_id, from: i, grab: pp - rects[i].min, last: pp };
            ctx.data_mut(|dm| dm.insert_temp(drag_id, d));
            app.cmds.push(Cmd::SelectTab(i));
        }
        let km = app.keymap.clone();
        let pinned_now = app.ws.tabs[i].pinned;
        let mut cmds = Vec::new();
        crate::ui::native::popup::context_menu(&resp, |ui| {
            tab_menu(ui, &mut cmds, &km, i, pinned_now, n);
            crate::ui::workspaces::tab_menu_extra(ui, app, &mut cmds, i);
        });
        app.cmds.extend(cmds);
    }

    // ---- italic separators between neighbouring tabs (not next to the active / hovered
    // tab, nor where a group chip sits between the two) ----
    let shown: Vec<usize> = order.iter().copied().filter(|&k| drawn[k].is_some()).collect();
    for w in shown.windows(2) {
        let (a, b) = (w[0], w[1]);
        if groups_of[b].is_some() && groups_of[a] != groups_of[b] {
            continue;
        }
        let (Some((ra, ea)), Some((rb, eb))) = (drawn[a], drawn[b]) else { continue };
        let x = (ra.right() + rb.left()) / 2.0;
        paint_separator(&painter, &p, x, ra.center().y, 1.0 - ea.max(eb));
    }

    // ---- dragging ----
    if let Some(d) = drag {
        ctx.data_mut(|dm| dm.insert_temp(drag_id, d));
        ctx.set_cursor_icon(CursorIcon::Grabbing);
        if far {
            paint_ghost(&ctx, app, d, away_target.as_ref());
        }
        let down = ui.input(|i| i.pointer.primary_down());
        if !down {
            finish_drag(app, d, &rects, &groups_of, hovered_chip.map(|(g, _)| g), away_target, far);
            ctx.data_mut(|dm| dm.remove::<Drag>(drag_id));
        }
        ctx.request_repaint();
    }

    // New tab button.
    let last_right = (0..n).filter(|&i| !hidden[i]).map(|i| live.lefts[i] + widths[i]).fold(full.left(), f32::max);
    let chip_right = live.chips.last().map_or(full.left(), |(g, l)| l + chip_w(*g));
    let plus_x = (last_right.max(chip_right) + 4.0).min(full.right() - plus_w);
    let pr = Rect::from_min_size(pos2(plus_x, full.top() + 2.0), vec2(28.0, 28.0));
    let presp = ui.interact(pr, Id::new("soyuz.tab.new"), Sense::click());
    crate::ui::a11y::button(&presp, tr!("app-tabs-new"));
    if presp.hovered() {
        ui.painter().rect_filled(pr, CornerRadius::same(radius::ROW), p.bg_surface2);
    }
    ui.painter().text(
        pr.center(),
        Align2::CENTER_CENTER,
        ph::PLUS,
        icons::font(15.0),
        if presp.hovered() { p.text_primary } else { p.text_secondary },
    );
    let tip = match chord_label(&app.keymap, &Action::NewTab) {
        Some(k) => tr!("app-tabs-new-tip", key = k),
        None => tr!("app-tabs-new").into(),
    };
    if presp.on_hover_text(tip).clicked() {
        app.cmds.push(Cmd::Action(Action::NewTab));
    }
}

fn d_left(drag: Option<Drag>, fallback: f32) -> f32 {
    drag.map_or(fallback, |d| d.last.x - d.grab.x)
}

/// Dim a painter (the torn-off tab's empty slot).
fn faded(p: egui::Painter) -> egui::Painter {
    let mut p = p;
    p.set_opacity(0.35);
    p
}

/// Background, icon and title of one tab.
#[allow(clippy::too_many_arguments)]
fn paint_tab(
    ui: &Ui,
    painter: &egui::Painter,
    app: &SoyuzApp,
    i: usize,
    r: Rect,
    active: bool,
    dragging: bool,
    hover_t: f32,
) {
    let p = palette(ui.ctx());
    let tab = &app.ws.tabs[i];
    let (well, _, _) = p.tab_well();
    if active || dragging {
        if dragging {
            // Lifted while dragged: a soft drop shadow under the well.
            let shadow = egui::epaint::Shadow {
                offset: [0, 2],
                blur: 8,
                spread: 0,
                color: Color32::from_black_alpha(if p.dark { 70 } else { 24 }),
            };
            painter.add(shadow.as_shape(r, CornerRadius::same(radius::CARD)));
        }
        paint_well(painter, &p, r);
    } else if hover_t > 0.0 {
        painter.rect_filled(r, CornerRadius::same(radius::CARD), p.tab_hover().gamma_multiply(hover_t));
    }
    // Group color: a bar along the bottom edge.
    if let Some(g) = tab.group.and_then(|g| app.ws.group(g)) {
        let c = p.tags[(g.color as usize).min(7)];
        let bar = Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 4.5), pos2(r.right() - 6.0, r.bottom() - 2.5));
        painter.rect_filled(bar, CornerRadius::same(1), c);
    }
    let pane = tab.active();
    let title = location_title(&pane.location);
    let glyph = match tab.shown() {
        4 => ph::SQUARES_FOUR,
        2 => ph::SQUARE_SPLIT_HORIZONTAL,
        _ if matches!(pane.location, soyuz_core::Location::Timeline) => ph::CALENDAR_DOTS,
        _ if active => ph::FOLDER_OPEN,
        _ => ph::FOLDER,
    };
    let folder_color = match &pane.location {
        soyuz_core::Location::Dir(d) => crate::services::folder_colors::color_of(d, &p),
        _ => None,
    };
    let gcolor = match folder_color {
        Some(c) if active => c,
        Some(c) => c.gamma_multiply(0.8),
        None if active => p.glyph_folder,
        None => p.text_tertiary,
    };
    let loading = pane.is_loading();
    if tab.pinned {
        painter.text(r.center(), Align2::CENTER_CENTER, glyph, icons::font_fill(15.0), gcolor);
    } else {
        if loading && active {
            crate::ui::widgets::spinner(ui, pos2(r.left() + 18.0, r.center().y), 5.5, p.accent);
        } else {
            painter.text(
                pos2(r.left() + 18.0, r.center().y),
                Align2::CENTER_CENTER,
                glyph,
                icons::font_fill(15.0),
                gcolor,
            );
        }
        let hovered = hover_t > 0.5 || dragging;
        let show_close = active || hovered;
        let text_right = r.right() - if show_close { 30.0 } else { 12.0 };
        let color = if active { p.text_primary } else { p.text_secondary };
        let g = trunc_galley(painter, &title, FontId::proportional(12.5), color, text_right - (r.left() + 32.0));
        painter.galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, color);
    }
    if let Some(c) = folder_color
        && !(loading && active)
    {
        let at = if tab.pinned { r.center() } else { pos2(r.left() + 18.0, r.center().y) };
        crate::ui::looks::paint_dot(painter, at + vec2(6.0, 5.0), c, if active { well } else { p.bg_window });
    }
}

/// The active tab: a borderless, slightly recessed ("engraved") well. Layered fills
/// leave a 1 px inner shadow along the top and a 1 px highlight along the bottom that
/// follow the rounded corners. High contrast themes add an outline.
fn paint_well(painter: &egui::Painter, p: &crate::theme::Palette, r: Rect) {
    let (fill, shadow, highlight) = p.tab_well();
    let cr = CornerRadius::same(radius::CARD);
    painter.rect_filled(r, cr, highlight);
    painter.rect_filled(Rect::from_min_max(r.min, pos2(r.max.x, r.max.y - 1.0)), cr, shadow);
    painter.rect_filled(Rect::from_min_max(pos2(r.min.x, r.min.y + 1.0), pos2(r.max.x, r.max.y - 1.0)), cr, fill);
    // A softer second shadow row under the first one deepens the inset a touch.
    let soft = shadow.gamma_multiply(0.35);
    painter.rect_filled(Rect::from_min_max(pos2(r.min.x, r.min.y + 1.0), pos2(r.max.x, r.max.y - 1.0)), cr, soft);
    painter.rect_filled(Rect::from_min_max(pos2(r.min.x, r.min.y + 2.0), pos2(r.max.x, r.max.y - 1.0)), cr, fill);
    if p.high_contrast() {
        painter.rect_stroke(r, cr, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    }
}

/// The italic (slanted, like "/") separator between two inactive tabs, centered on `x`.
fn paint_separator(painter: &egui::Painter, p: &crate::theme::Palette, x: f32, cy: f32, alpha: f32) {
    if alpha <= 0.01 {
        return;
    }
    let (half_h, lean) = (7.0, 2.5);
    painter.line_segment(
        [pos2(x + lean, cy - half_h), pos2(x - lean, cy + half_h)],
        Stroke::new(1.0, p.tab_separator().gamma_multiply(alpha)),
    );
}

/// The dragged tab following the pointer once it left the strip, with what releasing does.
fn paint_ghost(ctx: &egui::Context, app: &SoyuzApp, d: Drag, target: Option<&DropTarget>) {
    if !ctx.content_rect().contains(d.last) {
        return; // outside the window: the OS cursor is all we can show
    }
    let p = palette(ctx);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("soyuz.tab.ghost")));
    let r = Rect::from_min_size(d.last - d.grab, vec2(200.0, 30.0));
    let shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(90) };
    painter.add(shadow.as_shape(r, CornerRadius::same(radius::CARD)));
    painter.rect_filled(r, CornerRadius::same(radius::CARD), p.bg_surface1);
    let title = app.ws.tabs.get(d.from).map(|t| location_title(&t.active().location)).unwrap_or_default();
    painter.text(
        pos2(r.left() + 18.0, r.center().y),
        Align2::CENTER_CENTER,
        ph::FOLDER_OPEN,
        icons::font_fill(15.0),
        p.glyph_folder,
    );
    let g = trunc_galley(&painter, &title, FontId::proportional(12.5), p.text_primary, r.width() - 44.0);
    painter.galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, p.text_primary);
    let hint = match target {
        Some(DropTarget::TearOff { .. }) => tr!("app-tabs-drop-new-window"),
        Some(DropTarget::Window { .. }) => tr!("app-tabs-drop-other-window"),
        Some(DropTarget::Nothing) | None => tr!("app-tabs-drop-keep-one"),
        Some(DropTarget::Reorder) => "",
    };
    if !hint.is_empty() {
        painter.text(
            pos2(r.left() + 4.0, r.bottom() + 8.0),
            Align2::LEFT_TOP,
            hint,
            FontId::proportional(11.5),
            p.text_secondary,
        );
    }
}

/// Mouse released: reorder (joining / leaving groups), or hand the tab to another window.
fn finish_drag(
    app: &mut SoyuzApp,
    d: Drag,
    rects: &[Rect],
    groups: &[Option<GroupId>],
    chip: Option<GroupId>,
    away: Option<DropTarget>,
    far: bool,
) {
    if far {
        if let Some(t) = away.filter(|t| !matches!(t, DropTarget::Reorder | DropTarget::Nothing)) {
            app.cmds.push(Cmd::Workspace(WsCmd::DropTab {
                tab: d.tab,
                target: t,
                grab: crate::ui::geom::size(d.grab),
            }));
        }
        return;
    }
    let n = rects.len();
    let others: Vec<usize> = (0..n).filter(|&k| k != d.from).collect();
    let own = groups[d.from];
    let (to, group) = match chip {
        Some(g) => (others.iter().position(|&k| groups[k] == Some(g)).unwrap_or(0), Some(g)),
        None => {
            let to = drop_index(rects, d.from, d.last.x - d.grab.x);
            let left = to.checked_sub(1).and_then(|j| others.get(j)).and_then(|&k| groups[k]);
            let right = others.get(to).and_then(|&k| groups[k]);
            (to, drop_group(left, right, own))
        }
    };
    if to != d.from || group != own {
        app.cmds.push(Cmd::Workspace(WsCmd::MoveTab { from: d.from, to, group }));
    }
}

/// Inline rename field over a group chip.
fn group_rename(ui: &mut Ui, app: &mut SoyuzApp, g: GroupId, r: Rect) {
    let Some((_, text, focus)) = app.workspaces.renaming.as_mut() else { return };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    let resp = child.add(
        egui::TextEdit::singleline(text)
            .hint_text(tr!("app-tabs-group-name"))
            .desired_width(r.width())
            .margin(Margin::symmetric(6, 2))
            .font(FontId::proportional(11.5)),
    );
    if *focus {
        resp.request_focus();
        *focus = false;
    }
    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if esc {
        app.workspaces.renaming = None;
    } else if resp.lost_focus() || enter {
        let name = text.clone();
        app.cmds.push(Cmd::Workspace(WsCmd::RenameGroup(g, name)));
    }
}

/// Where the tab `from`, dragged with its left edge at `left`, would be inserted (index
/// in the order without it, i.e. the `to` of `MoveTab`).
pub fn drop_index(rects: &[Rect], from: usize, left: f32) -> usize {
    let n = rects.len();
    let x = left + rects.get(from).map_or(0.0, |r| r.width() / 2.0);
    let to = rects.iter().position(|r| r.width() > 0.0 && x < r.center().x).unwrap_or(n).min(n);
    if to > from { to - 1 } else { to }
}

/// The tab order with `from` moved to `to`.
pub fn live_order(n: usize, from: usize, to: usize) -> Vec<usize> {
    let mut o: Vec<usize> = (0..n).filter(|&k| k != from).collect();
    o.insert(to.min(o.len()), from);
    o
}

/// Left edge of every tab (indexed by tab) when laid out in `order`.
pub fn slot_lefts(order: &[usize], widths: &[f32], start: f32) -> Vec<f32> {
    let groups = vec![None; widths.len()];
    strip_slots(order, widths, &groups, &|_| 0.0, None, start).lefts
}

fn tab_menu(ui: &mut Ui, cmds: &mut Vec<Cmd>, km: &soyuz_core::config::Keymap, i: usize, pinned: bool, n: usize) {
    ui.set_width(230.0);
    let sc = |a: &Action| chord_label(km, a);
    if menu_item(ui, Some(ph::PLUS), tr!("app-tabs-new"), sc(&Action::NewTab).as_deref(), true).clicked() {
        cmds.push(Cmd::NewTabAfter(i));
        ui.close();
    }
    if menu_item(ui, Some(ph::COPY_SIMPLE), tr!("app-tabs-duplicate"), sc(&Action::DuplicateTab).as_deref(), true)
        .clicked()
    {
        cmds.push(Cmd::DuplicateTab(i));
        ui.close();
    }
    let (pin_icon, pin_label) =
        if pinned { (ph::PUSH_PIN_SLASH, tr!("app-tabs-unpin")) } else { (ph::PUSH_PIN, tr!("app-tabs-pin")) };
    if menu_item(ui, Some(pin_icon), pin_label, sc(&Action::PinTab).as_deref(), true).clicked() {
        cmds.push(Cmd::PinTab(i, !pinned));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::X), tr!("app-tabs-close"), sc(&Action::CloseTab).as_deref(), n > 1).clicked() {
        cmds.push(Cmd::CloseTab(i));
        ui.close();
    }
    if menu_item(ui, Some(ph::X_CIRCLE), tr!("app-tabs-close-others"), None, n > 1).clicked() {
        cmds.push(Cmd::CloseOtherTabs(i));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROW_LINE_RIGHT), tr!("app-tabs-close-right"), None, i + 1 < n).clicked() {
        cmds.push(Cmd::CloseTabsToRight(i));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(
        ui,
        Some(ph::ARROW_COUNTER_CLOCKWISE),
        tr!("app-tabs-reopen-closed"),
        sc(&Action::ReopenClosedTab).as_deref(),
        true,
    )
    .clicked()
    {
        cmds.push(Cmd::Action(Action::ReopenClosedTab));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(ws: &[f32]) -> Vec<Rect> {
        let mut x = 0.0;
        ws.iter()
            .map(|w| {
                let r = Rect::from_min_size(pos2(x, 0.0), vec2(*w, 30.0));
                x += w + 2.0;
                r
            })
            .collect()
    }

    #[test]
    fn live_reorder_slots() {
        let r = rects(&[100.0, 100.0, 100.0, 100.0]);
        assert_eq!(drop_index(&r, 0, 0.0), 0, "not moved");
        assert_eq!(drop_index(&r, 0, 160.0), 1, "past the next tab's middle");
        assert_eq!(drop_index(&r, 0, 1000.0), 3, "to the end");
        assert_eq!(drop_index(&r, 3, -50.0), 0);
        assert_eq!(live_order(4, 0, 2), vec![1, 2, 0, 3]);
        assert_eq!(live_order(4, 3, 0), vec![3, 0, 1, 2]);
        let widths = [40.0, 100.0, 100.0];
        // Tab 0 (pinned, narrow) dragged to the end: the others shift left by 42.
        assert_eq!(slot_lefts(&live_order(3, 0, 2), &widths, 0.0), vec![204.0, 0.0, 102.0]);
        assert_eq!(slot_lefts(&[0, 1, 2], &widths, 10.0), vec![10.0, 52.0, 154.0]);
    }

    #[test]
    fn chips_open_groups() {
        let g = Some(GroupId(7));
        let groups = [None, g, g, None];
        let widths = [100.0, 100.0, 100.0, 100.0];
        let s = strip_slots(&[0, 1, 2, 3], &widths, &groups, &|_| 50.0, None, 0.0);
        assert_eq!(s.chips, vec![(GroupId(7), 102.0)]);
        assert_eq!(s.lefts, vec![0.0, 156.0, 258.0, 360.0]);
        // Collapsed: members take no room, the chip stays.
        let s = strip_slots(&[0, 1, 2, 3], &[100.0, 0.0, 0.0, 100.0], &groups, &|_| 50.0, None, 0.0);
        assert_eq!(s.lefts[3], 156.0);
        // The dragged member doesn't open a second chip when it moves inside the group.
        let s = strip_slots(&[0, 1, 3, 2], &widths, &groups, &|_| 50.0, Some(3), 0.0);
        assert_eq!(s.chips.len(), 1);
        // Hidden tabs are ignored by the drop index.
        let r = rects(&[100.0, 0.0, 100.0]);
        assert_eq!(drop_index(&r, 0, 60.0), 1, "lands after the hidden tab");
    }

    #[test]
    fn dropping_into_groups() {
        let (a, b) = (Some(GroupId(1)), Some(GroupId(2)));
        assert_eq!(drop_group(a, a, None), a, "between two members: joins");
        assert_eq!(drop_group(a, None, None), None, "after the group's end: stays out");
        assert_eq!(drop_group(a, None, a), a, "a member moved to the end stays in");
        assert_eq!(drop_group(None, b, b), b);
        assert_eq!(drop_group(a, b, None), None, "between two groups");
        assert_eq!(drop_group(None, None, a), None, "dragged out of its group");
        assert_eq!(chip_label("Work", true, 3), "Work  3");
        assert_eq!(chip_label("", true, 3), "3");
        assert_eq!(chip_label(" Work ", false, 3), "Work");
    }
}
