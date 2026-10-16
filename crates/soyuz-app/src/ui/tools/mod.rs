//! Power tools: disk usage treemap, folder compare + sync, duplicate finder,
//! file split / join, the drop shelf, tags UI and git decorations.
//!
//! Tools are non-modal windows drawn over the panes (the user can keep browsing while a
//! scan runs). Their state is UI-thread-local, like `services::search`; widgets that
//! live in shared modules (context menu, sidebar, status bar) reach them through
//! [`queue`] (applied next frame) or through actions routed by [`execute`]. All file
//! system work happens on worker threads or as jobs.
//!
//! Frame hooks: [`begin_frame`] (services, queued commands, OS drops onto the shelf) runs
//! before the file-ops frame; [`show_panels`] draws the shelf before the central panel;
//! [`show_windows`] draws the tool windows over everything.

pub mod compare;
pub mod decor;
pub mod dupes;
pub mod shelf;
pub mod split;
pub mod sync_ui;
pub mod tags_ui;
pub mod treemap;
pub mod versions;

use std::cell::RefCell;
use std::path::PathBuf;

use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Key, LayerId, Margin, Modifiers, Order, Rect, Sense, Stroke, Ui,
    Vec2, pos2, vec2,
};
use soyuz_core::Location;
use soyuz_core::config::{Action, DragDefault};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dnd;
use crate::ui::toasts::Toast;
use crate::ui::views::flow::semibold;
use crate::ui::widgets::icon_button;

pub use crate::models::tools::{
    ToolCmd, any_window_open, execute, handles, queue, restore_shelf, shelf_count, shelf_open, shelf_paths, wake,
};
/// Sync profiles + their scheduler and the two-way sync state (moved to the core).
pub use crate::models::{compare_profiles as sync_sched, compare_state as sync_state};

/// The open tool windows and the shelf panel's view state.
#[derive(Default)]
pub struct Tools {
    pub treemap: Option<treemap::TreemapWindow>,
    pub compare: Option<compare::CompareWindow>,
    pub dupes: Option<dupes::DupesWindow>,
    pub split: Option<split::SplitWindow>,
    pub shelf: shelf::Shelf,
    pub sync_profiles: Option<sync_ui::SyncProfilesWindow>,
    pub versions: Option<versions::VersionsWindow>,
}

impl Tools {
    fn window_open(&self) -> bool {
        self.treemap.is_some()
            || self.compare.is_some()
            || self.dupes.is_some()
            || self.split.is_some()
            || self.sync_profiles.is_some()
            || self.versions.is_some()
    }
}

thread_local! {
    static TOOLS: RefCell<Tools> = RefCell::new(Tools::default());
}

/// Start of every frame (before file-op handling, after the core's service pumps):
/// queued commands and OS file drops onto the shelf.
pub fn begin_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    versions::begin_frame(app);
    crate::models::tools::debug_open_once();
    let cmds = crate::models::tools::take_queue();
    TOOLS.with_borrow_mut(|t| crate::models::tools::with_shelf(|d| t.shelf.external_drops(ctx, d)));
    // Config edits, tags and version actions run outside the tools borrow (they touch
    // many modules); the rest open tool windows.
    let cmds = crate::models::tools::apply_model_cmds(&mut app.core, &crate::host::EguiHost::new(ctx), cmds);
    TOOLS.with_borrow_mut(|t| {
        for c in cmds {
            apply(t, app, c);
        }
        crate::models::tools::set_window_open(t.window_open());
    });
}

/// Open the tool window a [`ToolCmd`] asks for.
fn apply(t: &mut Tools, app: &mut SoyuzApp, c: ToolCmd) {
    match c {
        ToolCmd::Treemap(path) => match &mut t.treemap {
            Some(w) if w.root_path() == path.as_path() => w.open = true,
            _ => t.treemap = Some(treemap::TreemapWindow::new(path)),
        },
        ToolCmd::Compare(l, r) => {
            let profiles = sync_sched::profiles();
            t.compare = Some(compare::CompareWindow::new(l, r, profiles));
        }
        ToolCmd::CompareProfile(name) => {
            let profiles = sync_sched::profiles();
            match profiles.iter().find(|p| p.name == name).cloned() {
                Some(pr) => t.compare = Some(compare::CompareWindow::with_profile(&pr, profiles)),
                None => {
                    app.core.ui.toasts.push(Toast::info(tr!("tools-toast-no-profile", name = name)));
                }
            }
        }
        ToolCmd::SyncProfiles => match &mut t.sync_profiles {
            Some(w) => w.open = true,
            None => t.sync_profiles = Some(sync_ui::SyncProfilesWindow::new()),
        },
        ToolCmd::Versions(path, is_dir, current) => {
            t.versions = Some(versions::VersionsWindow::new(path, is_dir, current));
        }
        ToolCmd::Duplicates(roots) => t.dupes = Some(dupes::DupesWindow::new(roots)),
        ToolCmd::Split(p) => t.split = Some(split::SplitWindow::split(p)),
        ToolCmd::Join(p) => t.split = Some(split::SplitWindow::join(p)),
        // Applied by `models::tools::apply_model_cmds`.
        ToolCmd::ShelfAdd(_)
        | ToolCmd::ToggleTag { .. }
        | ToolCmd::ClearTags(_)
        | ToolCmd::RenameTag { .. }
        | ToolCmd::ForgetRecent(_)
        | ToolCmd::VersionAction(_) => {}
    }
}

/// Right-side panels (the shelf). Called before the central panel.
pub fn show_panels(ui: &mut Ui, app: &mut SoyuzApp) {
    let mut t = TOOLS.with_borrow_mut(std::mem::take);
    let mut d = crate::models::tools::take_shelf();
    t.shelf.show(ui, app, &mut d);
    crate::models::tools::put_shelf(d);
    TOOLS.with_borrow_mut(|slot| restore(slot, t));
}

fn window_layer(id: &str) -> LayerId {
    LayerId::new(Order::Middle, Id::unique(("soyuz.tool", id)))
}

const WINDOW_IDS: [&str; 6] = ["treemap", "compare", "dupes", "split", "sync_profiles", "versions"];

/// A dimmed, full-screen layer right under the tool windows: it takes every click, hover
/// and wheel event aimed at the panes, so they stay untouched while a tool is open.
/// While a drag is in flight (an internal payload or files from another app) it lets the
/// pointer through, so the drag can be dropped on a pane, folder, tab or sidebar row.
fn backdrop(ctx: &egui::Context) {
    let p = palette(ctx);
    let screen = ctx.content_rect();
    let dragging = dnd::payload(ctx).is_some() || ctx.input(|i| !i.raw.hovered_files.is_empty());
    let layer = egui::Area::new(Id::unique("soyuz.tools.backdrop"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .interactable(!dragging)
        .show(ctx, |ui| {
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(if p.dark { 110 } else { 60 }));
            if !dragging {
                ui.allocate_rect(screen, Sense::click_and_drag());
            }
        })
        .response
        .layer_id;
    // A click on the backdrop raises it over the windows: put them back on top.
    if ctx.memory(|m| m.areas().top_layer_id(Order::Middle)) == Some(layer) {
        for id in WINDOW_IDS {
            ctx.move_to_top(window_layer(id));
        }
    }
}

/// Tool windows (over the panes, under toasts).
pub fn show_windows(ctx: &egui::Context, app: &mut SoyuzApp) {
    if any_window_open() {
        backdrop(ctx);
    }
    let mut t = TOOLS.with_borrow_mut(std::mem::take);
    if let Some(w) = &mut t.treemap {
        w.show(ctx, app);
        if !w.open {
            t.treemap = None;
        }
    }
    if let Some(w) = &mut t.compare {
        w.show(ctx, app);
        if !w.open {
            t.compare = None;
        }
    }
    if let Some(w) = &mut t.dupes {
        w.show(ctx, app);
        if !w.open {
            t.dupes = None;
        }
    }
    if let Some(w) = &mut t.split {
        w.show(ctx, app);
        if !w.open {
            t.split = None;
        }
    }
    if let Some(w) = &mut t.sync_profiles {
        w.show(ctx, app);
        if !w.open {
            t.sync_profiles = None;
        }
    }
    if let Some(w) = &mut t.versions {
        w.show(ctx, app);
        if !w.open {
            t.versions = None;
        }
    }
    crate::models::tools::with_shelf(|d| t.shelf.drag_strip(ctx, d));
    TOOLS.with_borrow_mut(|slot| {
        restore(slot, t);
        crate::models::tools::set_window_open(slot.window_open());
    });
}

/// Put the taken state back, keeping anything created meanwhile (defensive: nothing
/// should touch the slot while it is taken).
fn restore(slot: &mut Tools, t: Tools) {
    let fresh = std::mem::replace(slot, t);
    if fresh.treemap.is_some() {
        slot.treemap = fresh.treemap;
    }
    if fresh.compare.is_some() {
        slot.compare = fresh.compare;
    }
    if fresh.dupes.is_some() {
        slot.dupes = fresh.dupes;
    }
    if fresh.split.is_some() {
        slot.split = fresh.split;
    }
    if fresh.sync_profiles.is_some() {
        slot.sync_profiles = fresh.sync_profiles;
    }
    if fresh.versions.is_some() {
        slot.versions = fresh.versions;
    }
}

// ---------------------------------------------------------------------------------------
// Shared chrome.

/// Let a tool's result row start an internal drag: it can be dropped on a pane, folder,
/// tab or sidebar row behind the tool window (the backdrop lets drags through), or out of
/// the window onto the desktop / another app (drag-out). `paths` runs only when the drag
/// starts. The row needs a drag sense (`Sense::click_and_drag()` / `Sense::drag()`).
pub fn drag_source(resp: &egui::Response, default: DragDefault, paths: impl FnOnce() -> Vec<PathBuf>) {
    if resp.drag_started() && dnd::payload(&resp.ctx).is_none() {
        let paths = paths();
        if !paths.is_empty() {
            egui::DragAndDrop::set_payload(&resp.ctx, dnd::DragPayload { paths, source_pane: None, default });
        }
    }
}

/// A tool window: rounded surface, custom header (icon, title, subtitle, close button),
/// resizable, constrained to the screen. Returns `false` when the user closed it.
pub fn tool_window(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    subtitle: &str,
    icon: &str,
    default_size: Vec2,
    add: impl FnOnce(&mut Ui),
) -> bool {
    let p = palette(ctx);
    let screen = ctx.content_rect();
    let size = default_size.min(screen.size() - vec2(60.0, 60.0)).max(vec2(320.0, 240.0));
    let frame = Frame::new()
        .fill(p.bg_surface1)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(radius::MODAL))
        .inner_margin(Margin::same(0))
        .shadow(egui::epaint::Shadow {
            offset: [0, 10],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
        });
    let mut keep = true;
    // Esc closes the frontmost tool window (a focused text field or an open popup takes
    // Esc first).
    let frontmost = ctx.memory(|m| m.areas().top_layer_id(Order::Middle)) == Some(window_layer(id));
    let text_focused = ctx.memory(|m| m.focused()).is_some_and(|f| egui::TextEdit::load_state(ctx, f).is_some());
    if frontmost
        && !text_focused
        && !ctx.any_popup_open()
        && !crate::ui::widgets::dropdown::any_open(ctx)
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        keep = false;
    }
    egui::Window::new(title)
        .id(Id::unique(("soyuz.tool", id)))
        .title_bar(false)
        .resizable(true)
        .collapsible(false)
        .frame(frame)
        .default_size(size)
        .min_size(vec2(420.0, 280.0))
        .default_pos(screen.center() - size / 2.0)
        .constrain_to(screen)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            // Header.
            let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
            let painter = ui.painter();
            painter.text(
                pos2(hr.left() + 18.0, hr.center().y),
                Align2::CENTER_CENTER,
                icon,
                icons::font(18.0),
                p.accent,
            );
            let tg = painter.layout_no_wrap(title.to_string(), semibold(14.5), p.text_primary);
            let tw = tg.size().x;
            painter.galley(pos2(hr.left() + 34.0, hr.center().y - tg.size().y / 2.0), tg, p.text_primary);
            if !subtitle.is_empty() {
                let sg = crate::ui::widgets::trunc_galley(
                    painter,
                    subtitle,
                    FontId::proportional(12.0),
                    p.text_tertiary,
                    hr.width() - tw - 34.0 - 70.0,
                );
                painter.galley(pos2(hr.left() + 44.0 + tw, hr.center().y - sg.size().y / 2.0), sg, p.text_tertiary);
            }
            let close = Rect::from_center_size(pos2(hr.right() - 22.0, hr.center().y), vec2(30.0, 30.0));
            let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(close));
            if icon_button(&mut cu, ph::X, tr!("common-close"), true).clicked() {
                keep = false;
            }
            ui.painter().line_segment(
                [pos2(hr.left(), hr.bottom()), pos2(hr.right(), hr.bottom())],
                Stroke::new(1.0, p.border_subtle),
            );
            // Body.
            let body =
                Rect::from_min_max(pos2(hr.left() + 14.0, hr.bottom() + 10.0), ui.max_rect().max - vec2(14.0, 12.0));
            let mut child =
                ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            add(&mut child);
            ui.allocate_rect(Rect::from_min_max(hr.min, ui.max_rect().max), Sense::hover());
        });
    keep
}

/// Small text button with an optional icon; `kind`: 0 normal, 1 primary, 2 danger.
pub fn button(ui: &mut Ui, icon: Option<&str>, label: &str, kind: u8, enabled: bool) -> egui::Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.5);
    // PLACEHOLDER so the galley takes `fg` below (secondary buttons are dark in light mode).
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 18.0 } else { 0.0 };
    let size = vec2(galley.size().x + 22.0 + icon_w, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    crate::ui::a11y::button(&resp, label);
    let hover = enabled && resp.hovered();
    let (bg, fg, stroke) = match (kind, enabled) {
        (_, false) => (p.bg_surface2, p.text_tertiary, Some(p.border_subtle)),
        (1, _) => (if hover { p.accent_hover } else { p.accent }, Color32::WHITE, None),
        (2, _) => (if hover { p.danger } else { p.danger.gamma_multiply(0.85) }, Color32::WHITE, None),
        _ => (if hover { p.bg_surface3 } else { p.bg_surface2 }, p.text_primary, Some(p.border_subtle)),
    };
    let cr = CornerRadius::same(radius::ROW);
    ui.painter().rect_filled(rect, cr, bg);
    if let Some(s) = stroke {
        ui.painter().rect_stroke(rect, cr, Stroke::new(1.0, s), egui::StrokeKind::Inside);
    }
    let mut x = rect.left() + 11.0;
    if let Some(g) = icon {
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, g, icons::font(13.5), fg);
        x += icon_w;
    }
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, rect);
    }
    resp
}

/// A thin progress bar (`frac` = None → indeterminate shimmer).
pub fn progress_bar(ui: &mut Ui, width: f32, frac: Option<f32>) {
    let p = palette(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(2), p.border_subtle);
    match frac {
        Some(f) => {
            let mut fill = r;
            fill.set_width((r.width() * f.clamp(0.0, 1.0)).max(3.0));
            ui.painter().rect_filled(fill, CornerRadius::same(2), p.accent);
        }
        None => {
            let t = ui.input(|i| i.time) as f32;
            let w = r.width() * 0.3;
            let x = r.left() + (t * 0.8).fract() * (r.width() + w) - w;
            let seg = Rect::from_min_max(pos2(x.max(r.left()), r.top()), pos2((x + w).min(r.right()), r.bottom()));
            if seg.width() > 0.0 {
                ui.painter().rect_filled(seg, CornerRadius::same(2), p.accent);
            }
            ui.ctx().request_repaint();
        }
    }
}

/// Small colored chip ("Only left").
pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(11.0), color);
    let size = vec2(galley.size().x + 14.0, 18.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(r, CornerRadius::same(9), color.gamma_multiply(0.16));
    ui.painter().galley(pos2(r.left() + 7.0, r.center().y - galley.size().y / 2.0), galley, color);
    resp
}

/// Toggle chip for filters: filled when on.
pub fn filter_chip(ui: &mut Ui, text: &str, count: usize, color: Color32, on: &mut bool) -> egui::Response {
    let p = palette(ui.ctx());
    let label = format!("{text}  {count}");
    let galley = ui.painter().layout_no_wrap(label, FontId::proportional(11.5), p.text_primary);
    let size = vec2(galley.size().x + 28.0, 24.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let cr = CornerRadius::same(12);
    if *on {
        ui.painter().rect_filled(r, cr, color.gamma_multiply(0.18));
        ui.painter().rect_stroke(r, cr, Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    } else {
        ui.painter().rect_filled(r, cr, if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 });
        ui.painter().rect_stroke(r, cr, Stroke::new(1.0, p.border_subtle), egui::StrokeKind::Inside);
    }
    ui.painter().circle_filled(
        pos2(r.left() + 12.0, r.center().y),
        4.0,
        if *on { color } else { color.gamma_multiply(0.5) },
    );
    let fg = if *on { p.text_primary } else { p.text_secondary };
    ui.painter().galley_with_override_text_color(
        pos2(r.left() + 21.0, r.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    resp
}

/// A non-counting toggle chip (method / preset / tab pickers).
pub fn toggle_chip(ui: &mut Ui, text: &str, on: &mut bool, color: Color32) -> egui::Response {
    let p = palette(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(11.5), p.text_primary);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 24.0), Sense::click());
    let cr = CornerRadius::same(12);
    if *on {
        ui.painter().rect_filled(r, cr, color.gamma_multiply(0.18));
        ui.painter().rect_stroke(r, cr, Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    } else {
        ui.painter().rect_filled(r, cr, if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 });
    }
    let fg = if *on { p.text_primary } else { p.text_secondary };
    ui.painter().galley_with_override_text_color(
        pos2(r.left() + 9.0, r.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    resp
}

/// Text field with a leading icon, styled like the omnibar.
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(width)
            .margin(Margin::symmetric(8, 5))
            .font(FontId::proportional(12.5)),
    )
}

/// Status-bar git chip: branch, ahead/behind, change count. Clicking it navigates the
/// pane to the repository root.
pub fn git_chip(ui: &mut Ui, dir: Option<&std::path::Path>, pane: crate::state::pane::PaneId, cmds: &mut Vec<Cmd>) {
    let Some(dir) = dir else { return };
    let Some(s) = crate::services::git::summary_for(dir) else { return };
    let p = palette(ui.ctx());
    let branch = s.branch.clone().unwrap_or_else(|| tr!("tools-git-detached").into());
    let mut text = branch.clone();
    if s.ahead > 0 {
        text.push_str(&format!("  ↑{}", s.ahead));
    }
    if s.behind > 0 {
        text.push_str(&format!("  ↓{}", s.behind));
    }
    let dirty = s.changed + s.untracked + s.conflicts;
    let font = FontId::proportional(11.5);
    let g = ui.painter().layout_no_wrap(text, font.clone(), p.text_secondary);
    let extra = if dirty > 0 { format!("{dirty}") } else { String::new() };
    let eg = ui.painter().layout_no_wrap(extra.clone(), font, p.warning);
    let w = 18.0 + g.size().x + if dirty > 0 { eg.size().x + 16.0 } else { 0.0 } + 12.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::click());
    let cr = CornerRadius::same(9);
    ui.painter().rect_filled(r, cr, if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 });
    let color = if s.conflicts > 0 {
        p.danger
    } else if dirty > 0 {
        p.warning
    } else {
        p.success
    };
    ui.painter().text(
        pos2(r.left() + 11.0, r.center().y),
        Align2::CENTER_CENTER,
        ph::GIT_BRANCH,
        icons::font(12.0),
        color,
    );
    ui.painter().galley(pos2(r.left() + 20.0, r.center().y - g.size().y / 2.0), g.clone(), p.text_secondary);
    if dirty > 0 {
        let x = r.left() + 20.0 + g.size().x + 8.0;
        ui.painter().circle_filled(pos2(x + 2.0, r.center().y), 2.5, color);
        ui.painter().galley(pos2(x + 8.0, r.center().y - eg.size().y / 2.0), eg, color);
    }
    if s.busy {
        crate::ui::widgets::spinner(ui, pos2(r.right() + 8.0, r.center().y), 4.0, p.text_tertiary);
    }
    let mut tip =
        tr!("tools-git-tip-repo", root = soyuz_fs::strip_verbatim(&s.root).display().to_string(), branch = &branch);
    if s.loaded {
        tip.push('\n');
        tip.push_str(&if s.conflicts > 0 {
            tr!(
                "tools-git-tip-counts-conflicts",
                changed = s.changed,
                staged = s.staged,
                untracked = s.untracked,
                conflicts = s.conflicts
            )
        } else {
            tr!("tools-git-tip-counts", changed = s.changed, staged = s.staged, untracked = s.untracked)
        });
    }
    if let Some(e) = &s.error {
        tip.push_str(&format!("\n{e}"));
    }
    tip.push('\n');
    tip.push_str(tr!("tools-git-tip-click"));
    if resp.on_hover_text(tip).clicked() {
        cmds.push(Cmd::Navigate { pane, location: Location::Dir(s.root.clone()), select: None });
    }
}

/// Status-bar shelf toggle with an item count.
pub fn shelf_toggle(ui: &mut Ui, cmds: &mut Vec<Cmd>) {
    let n = shelf_count();
    let open = shelf_open();
    let p = palette(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(if n > 0 { 38.0 } else { 22.0 }, 18.0), Sense::click());
    if open || resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface2);
    }
    let color = if open { p.accent } else { p.text_tertiary };
    ui.painter().text(pos2(r.left() + 11.0, r.center().y), Align2::CENTER_CENTER, ph::TRAY, icons::font(12.5), color);
    if n > 0 {
        ui.painter().text(
            pos2(r.left() + 22.0, r.center().y),
            Align2::LEFT_CENTER,
            n.to_string(),
            FontId::proportional(11.0),
            color,
        );
    }
    let tip = if n > 0 { tr!("tools-shelf-tip-count", count = n) } else { tr!("tools-shelf-tip-empty").into() };
    crate::ui::a11y::toggle(&resp, &tip, open);
    if resp.on_hover_text(tip).clicked() {
        cmds.push(Cmd::Action(Action::ToggleShelf));
    }
}
