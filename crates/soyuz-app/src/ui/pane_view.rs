//! A pane card: rounded surface with an optional header (dual pane), loading / error /
//! empty states, and the active view.

use egui::{Align2, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use soyuz_core::config::Action;
use soyuz_core::{Location, ViewMode};

use crate::cmd::Cmd;
use crate::state::crumbs::location_title;
use crate::state::listing::EmptyKind;
use crate::state::pane::{ContextTarget, ListingStatus, Pane};
use crate::theme::font::semibold;
use crate::theme::tokens::radius;
use crate::theme::{icons::ph, palette};
use crate::ui::context_menu;
use crate::ui::empty_state::EmptyState;
use crate::ui::views::flow::{self, Style};
use crate::ui::views::{ViewCtx, computer, miller};
use crate::ui::widgets::{fit, spinner};

fn header_h() -> f32 {
    crate::theme::font::grow(32.0)
}
/// Inset that keeps square fills inside the card's rounded corners.
const INSET: f32 = 4.0;

pub fn show(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>, with_header: bool) {
    let p = palette(ui.ctx());
    let card = crate::theme::tokens::radius::same(radius::MODAL);
    ui.painter().rect_filled(rect, card, p.bg_surface1);
    let mut content = rect.shrink(INSET);
    if with_header {
        let hr = Rect::from_min_size(rect.min, vec2(rect.width(), header_h()));
        header(ui, hr, pane, v);
        content.min.y = hr.bottom() + 2.0;
    }
    let (dx, alpha) = crate::ui::looks::enter_transition(ui.ctx(), pane.id.0, &pane.location);
    let shifted = content.translate(vec2(dx, 0.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(shifted).id_salt(("soyuz.pane", pane.id.0)));
    child.set_clip_rect(content.intersect(ui.clip_rect()));
    if alpha < 1.0 {
        child.multiply_opacity(alpha);
    }
    body(&mut child, shifted, pane, v);
    crate::ui::dnd::pane_zone(ui, content, pane, &mut v.fileops.dnd, v.cmds);
    let stroke = if with_header && v.focused {
        Stroke::new(1.0, p.accent.gamma_multiply(0.55))
    } else {
        Stroke::new(1.0, p.border_subtle)
    };
    ui.painter().rect_stroke(rect, card, stroke, StrokeKind::Inside);
}

fn header(ui: &mut Ui, r: Rect, pane: &Pane, v: &mut ViewCtx<'_>) {
    let p = palette(ui.ctx());
    let resp = ui.interact(r, Id::unique(("soyuz.pane.header", pane.id.0)), Sense::click());
    if resp.clicked() {
        v.cmds.push(Cmd::FocusPane(pane.id));
    }
    let active = v.focused;
    if crate::ui::a11y::active(ui.ctx()) {
        let name = format!("{}, {}", location_title(&pane.location), pane.location.display());
        crate::ui::a11y::toggle(&resp, &name, active);
    }
    let dot = pos2(r.left() + 16.0, r.center().y);
    ui.painter().circle_filled(dot, 3.5, if active { p.accent } else { p.border_strong });
    let title = location_title(&pane.location);
    let tg = fit::ends(
        ui.painter(),
        &title,
        semibold(12.5),
        if active { p.text_primary } else { p.text_secondary },
        r.width() * 0.45,
    )
    .galley;
    let tw = tg.size().x;
    ui.painter().galley(pos2(r.left() + 28.0, r.center().y - tg.size().y / 2.0), tg, p.text_primary);
    let path = pane.location.display();
    let room = r.right() - (r.left() + 40.0 + tw) - 12.0;
    let pg = fit::ends(ui.painter(), &path, crate::theme::font::ui(11.5), p.text_tertiary, room).galley;
    ui.painter().galley(pos2(r.left() + 38.0 + tw, r.center().y - pg.size().y / 2.0), pg, p.text_tertiary);
    ui.painter().line_segment(
        [pos2(r.left() + INSET, r.bottom() + 0.5), pos2(r.right() - INSET, r.bottom() + 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
}

fn body(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let p = palette(ui.ctx());
    // The system monitor has no listing: its pages draw whatever the status says.
    if matches!(pane.location, Location::Monitor) {
        crate::ui::views::monitor::show(ui, rect, pane, v);
        return;
    }
    match pane.status.clone() {
        ListingStatus::Virtual => {
            if matches!(pane.location, Location::Computer) {
                computer::show(ui, rect, pane, v);
            } else if matches!(pane.location, Location::Trash) {
                crate::ui::views::trash::show(ui, rect, pane, v);
            } else {
                let (glyph, title, sub) = match pane.location {
                    Location::Trash => (ph::TRASH, tr!("views-trash-title"), tr!("views-pane-trash-pending")),
                    Location::Recent => (ph::CLOCK, tr!("views-pane-recent"), tr!("views-pane-recent-pending")),
                    Location::Search(_) => {
                        (ph::MAGNIFYING_GLASS, tr!("views-pane-search"), tr!("views-pane-search-pending"))
                    }
                    Location::Archive { .. } => {
                        (ph::FILE_ZIP, tr!("views-pane-archive"), tr!("views-pane-archive-pending"))
                    }
                    _ => (ph::TAG, tr!("views-pane-tags"), tr!("views-pane-tags-pending")),
                };
                empty(ui, rect, pane, v, glyph, Some(p.text_secondary), title, sub, &[]);
            }
        }
        ListingStatus::Failed(e) => {
            let title = match e.kind {
                soyuz_core::FsErrorKind::PermissionDenied => tr!("views-pane-access-denied"),
                soyuz_core::FsErrorKind::NotFound => tr!("views-pane-not-found"),
                soyuz_core::FsErrorKind::NotDirectory => tr!("views-pane-not-a-folder"),
                _ => tr!("views-pane-cant-open"),
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
                &[(Action::Up, tr!("views-pane-hint-go-up")), (Action::Back, tr!("views-pane-hint-go-back"))],
            );
        }
        ListingStatus::Loading { since } if pane.entries.is_empty() => {
            // Avoid a flash for fast listings: skeleton rows after a short delay.
            if since.elapsed().as_secs_f32() > 0.12 {
                let grid = matches!(pane.view.mode, ViewMode::Grid | ViewMode::Gallery);
                crate::ui::looks::paint_skeleton(
                    ui,
                    rect,
                    grid,
                    crate::theme::font::height(v.config.appearance.density.row_height(), 13.0),
                );
                if since.elapsed().as_secs_f32() > 2.5 {
                    // Slow folder (network, huge): say so.
                    spinner(ui, pos2(rect.right() - 24.0, rect.bottom() - 20.0), 6.0, p.accent);
                    ui.painter().text(
                        pos2(rect.right() - 36.0, rect.bottom() - 20.0),
                        Align2::RIGHT_CENTER,
                        tr!("common-loading"),
                        crate::theme::font::ui(12.0),
                        p.text_tertiary,
                    );
                }
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(130));
            }
        }
        _ => {
            let kind = pane.empty_kind(v.show_hidden, v.show_system);
            if kind != EmptyKind::NotEmpty {
                match kind {
                    EmptyKind::NoMatches => {
                        let sub = tr!("views-pane-no-matches-sub", filter = &pane.filter);
                        empty(
                            ui,
                            rect,
                            pane,
                            v,
                            ph::FUNNEL_X,
                            Some(p.text_secondary),
                            tr!("views-pane-no-matches"),
                            &sub,
                            &[(Action::SelectNone, tr!("views-pane-hint-clear-filter"))],
                        );
                    }
                    EmptyKind::OnlyHidden => empty(
                        ui,
                        rect,
                        pane,
                        v,
                        ph::EYE_SLASH,
                        Some(p.text_secondary),
                        tr!("views-pane-only-hidden"),
                        tr!("views-pane-only-hidden-sub"),
                        &[(Action::ToggleHidden, tr!("views-pane-hint-show-hidden"))],
                    ),
                    _ if matches!(pane.location, Location::Timeline) => empty(
                        ui,
                        rect,
                        pane,
                        v,
                        ph::CALENDAR_DOTS,
                        Some(p.text_secondary),
                        tr!("views-pane-timeline-empty"),
                        tr!("views-pane-timeline-empty-sub"),
                        &[],
                    ),
                    _ => empty(
                        ui,
                        rect,
                        pane,
                        v,
                        ph::FOLDER_DASHED,
                        Some(p.glyph_folder),
                        tr!("views-pane-empty"),
                        tr!("views-pane-empty-sub"),
                        &[],
                    ),
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
    hints: &[(Action, &str)],
) {
    let bg = ui.interact(rect, Id::unique(("soyuz.pane.empty", pane.id.0)), Sense::click());
    if bg.clicked() || bg.secondary_clicked() {
        v.cmds.push(Cmd::FocusPane(pane.id));
        pane.context = ContextTarget::Background;
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    // The keys as bound now; an unbound action has no hint.
    let bound: Vec<(String, &str)> =
        hints.iter().filter_map(|(a, what)| Some((crate::keys::chord_label(v.keymap, a)?, *what))).collect();
    let hints: Vec<(&str, &str)> = bound.iter().map(|(k, what)| (k.as_str(), *what)).collect();
    EmptyState { glyph, glyph_color: color, title, subtitle, hints: &hints }.show(&mut child);
    context_menu::show_pane_menu(ui, &bg, pane, v, |_| rect.left_top() + egui::vec2(16.0, 8.0));
}
