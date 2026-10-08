//! A pane card: rounded surface with an optional header (dual pane), loading / error /
//! empty states, and the active view.

use egui::{Align2, CornerRadius, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use soyuz_core::{Location, ViewMode};

use crate::cmd::Cmd;
use crate::state::crumbs::location_title;
use crate::state::pane::{ContextTarget, ListingStatus, Pane};
use crate::theme::tokens::radius;
use crate::theme::{icons::ph, palette};
use crate::ui::context_menu;
use crate::ui::empty_state::EmptyState;
use crate::ui::views::flow::{self, Style, semibold};
use crate::ui::views::{ViewCtx, computer, miller};
use crate::ui::widgets::{spinner, trunc_galley};

const HEADER_H: f32 = 32.0;
/// Inset that keeps square fills inside the card's rounded corners.
const INSET: f32 = 4.0;

pub fn show(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>, with_header: bool) {
    let p = palette(ui.ctx());
    let card = CornerRadius::same(radius::MODAL);
    ui.painter().rect_filled(rect, card, p.bg_surface1);
    let mut content = rect.shrink(INSET);
    if with_header {
        let hr = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H));
        header(ui, hr, pane, v);
        content.min.y = hr.bottom() + 2.0;
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(content).id_salt(("soyuz.pane", pane.id.0)));
    child.set_clip_rect(content);
    body(&mut child, content, pane, v);
    crate::ops::fileops::dnd::pane_zone(ui, content, pane, &mut v.fileops.dnd, v.cmds);
    let stroke = if with_header && v.focused {
        Stroke::new(1.0, p.accent.gamma_multiply(0.55))
    } else {
        Stroke::new(1.0, p.border_subtle)
    };
    ui.painter().rect_stroke(rect, card, stroke, StrokeKind::Inside);
}

fn header(ui: &mut Ui, r: Rect, pane: &Pane, v: &mut ViewCtx<'_>) {
    let p = palette(ui.ctx());
    let resp = ui.interact(r, Id::new(("soyuz.pane.header", pane.id.0)), Sense::click());
    if resp.clicked() {
        v.cmds.push(Cmd::FocusPane(pane.id));
    }
    let active = v.focused;
    let dot = pos2(r.left() + 16.0, r.center().y);
    ui.painter().circle_filled(dot, 3.5, if active { p.accent } else { p.border_strong });
    let title = location_title(&pane.location);
    let tg = trunc_galley(
        ui.painter(),
        &title,
        semibold(12.5),
        if active { p.text_primary } else { p.text_secondary },
        r.width() * 0.45,
    );
    let tw = tg.size().x;
    ui.painter().galley(pos2(r.left() + 28.0, r.center().y - tg.size().y / 2.0), tg, p.text_primary);
    let path = pane.location.display();
    let pg = trunc_galley(
        ui.painter(),
        &path,
        FontId::proportional(11.5),
        p.text_tertiary,
        r.right() - (r.left() + 40.0 + tw) - 12.0,
    );
    ui.painter().galley(pos2(r.left() + 38.0 + tw, r.center().y - pg.size().y / 2.0), pg, p.text_tertiary);
    ui.painter().line_segment(
        [pos2(r.left() + INSET, r.bottom() + 0.5), pos2(r.right() - INSET, r.bottom() + 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
}

fn body(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let p = palette(ui.ctx());
    match pane.status.clone() {
        ListingStatus::Virtual => {
            if matches!(pane.location, Location::Computer) {
                computer::show(ui, rect, pane, v);
            } else if matches!(pane.location, Location::Trash) {
                crate::ui::views::trash::show(ui, rect, pane, v);
            } else {
                let (glyph, title, sub) = match pane.location {
                    Location::Trash => (ph::TRASH, "Trash", "Browsing the trash arrives with the trash milestone."),
                    Location::Recent => (ph::CLOCK, "Recent", "Recent items are not tracked yet."),
                    Location::Search(_) => (ph::MAGNIFYING_GLASS, "Search", "Search results will appear here."),
                    Location::Archive { .. } => {
                        (ph::FILE_ZIP, "Archive", "Browsing archives arrives with the archive milestone.")
                    }
                    _ => (ph::TAG, "Tags", "Tags are not available yet."),
                };
                empty(ui, rect, pane, v, glyph, Some(p.text_secondary), title, sub, &[]);
            }
        }
        ListingStatus::Failed(e) => {
            let title = match e.kind {
                soyuz_core::FsErrorKind::PermissionDenied => "Access denied",
                soyuz_core::FsErrorKind::NotFound => "Folder not found",
                soyuz_core::FsErrorKind::NotDirectory => "Not a folder",
                _ => "Can't open this folder",
            };
            let sub = e.to_string();
            empty(
                ui,
                rect,
                pane,
                v,
                ph::WARNING_CIRCLE,
                Some(p.danger),
                title,
                &sub,
                &[("Alt+Up", "Go up"), ("Alt+Left", "Go back")],
            );
        }
        ListingStatus::Loading { since } if pane.entries.is_empty() => {
            // Avoid a flash for fast listings: show the spinner after a short delay.
            if since.elapsed().as_secs_f32() > 0.12 {
                spinner(ui, rect.center() - vec2(0.0, 20.0), 11.0, p.accent);
                ui.painter().text(
                    rect.center() + vec2(0.0, 10.0),
                    Align2::CENTER_TOP,
                    "Loading…",
                    FontId::proportional(12.5),
                    p.text_tertiary,
                );
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(130));
            }
        }
        _ => {
            if pane.visible.is_empty() {
                if pane.filter_active() {
                    let sub = format!("Nothing here matches “{}”.", pane.filter);
                    empty(
                        ui,
                        rect,
                        pane,
                        v,
                        ph::FUNNEL_X,
                        Some(p.text_secondary),
                        "No matches",
                        &sub,
                        &[("Esc", "Clear filter")],
                    );
                } else if !pane.entries.is_empty() {
                    empty(
                        ui,
                        rect,
                        pane,
                        v,
                        ph::EYE_SLASH,
                        Some(p.text_secondary),
                        "Only hidden items",
                        "This folder contains hidden items only.",
                        &[("Ctrl+H", "Show hidden files")],
                    );
                } else {
                    empty(
                        ui,
                        rect,
                        pane,
                        v,
                        ph::FOLDER_DASHED,
                        Some(p.glyph_folder),
                        "This folder is empty",
                        "Drop files here or create something new.",
                        &[],
                    );
                }
                return;
            }
            match pane.view.mode {
                ViewMode::Details => flow::show(ui, rect, pane, v, Style::Details),
                ViewMode::List => flow::show(ui, rect, pane, v, Style::List),
                ViewMode::Grid => flow::show(ui, rect, pane, v, Style::Grid),
                ViewMode::Gallery => flow::show(ui, rect, pane, v, Style::Gallery),
                ViewMode::Miller => miller::show(ui, rect, pane, v),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn empty(
    ui: &mut Ui,
    rect: Rect,
    pane: &mut Pane,
    v: &mut ViewCtx<'_>,
    glyph: &str,
    color: Option<egui::Color32>,
    title: &str,
    subtitle: &str,
    hints: &[(&str, &str)],
) {
    let bg = ui.interact(rect, Id::new(("soyuz.pane.empty", pane.id.0)), Sense::click());
    if bg.clicked() || bg.secondary_clicked() {
        v.cmds.push(Cmd::FocusPane(pane.id));
        pane.context = ContextTarget::Background;
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    EmptyState { glyph, glyph_color: color, title, subtitle, hints }.show(&mut child);
    bg.context_menu(|ui| context_menu::pane_menu(ui, pane, v));
}
