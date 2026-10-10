//! "Where was I?" — the per-tab history graph popup. Every place the tab's panes
//! visited, as a tree that keeps the branches back/forward history forgets, with the time
//! of the last visit. Click a place to go back there.

use egui::{Align2, CornerRadius, FontId, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};

use super::WsCmd;
use crate::cmd::Cmd;
use crate::state::crumbs::location_title;
use crate::state::pane::PaneId;
use crate::state::workspace::TabId;
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dialogs::{Dialog, DialogCtx};

const ROW_H: f32 = 30.0;
const INDENT: f32 = 16.0;

pub struct HistoryDialog {
    tab: TabId,
    pane: PaneId,
    scrolled: bool,
}

impl HistoryDialog {
    pub fn new(tab: TabId, pane: PaneId) -> Self {
        HistoryDialog { tab, pane, scrolled: false }
    }
}

/// `14:05` today, `Mon 14:05` this week, else `12 Oct`.
pub fn when_label(ts: u64, now: u64) -> String {
    use chrono::{Datelike, TimeZone};
    let Some(t) = chrono::Local.timestamp_opt(ts as i64, 0).single() else { return String::new() };
    let age = now.saturating_sub(ts);
    let l = soyuz_core::locale::current();
    if age < 20 * 3600 {
        t.format("%H:%M").to_string()
    } else if age < 6 * 86_400 {
        let day = soyuz_core::locale::weekday_abbr(l)[t.weekday().num_days_from_monday() as usize];
        format!("{day} {}", t.format("%H:%M"))
    } else {
        format!("{:02} {}", t.day(), soyuz_core::locale::month_abbr(l)[t.month0() as usize])
    }
}

impl Dialog for HistoryDialog {
    fn title(&self) -> &str {
        tr!("workspaces-history-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::GIT_FORK)
    }

    fn width(&self) -> f32 {
        560.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let Some(tab) = ctx.ws.tabs.iter_mut().find(|t| t.id == self.tab) else { return false };
        // Panes that haven't moved yet (e.g. restored tabs) still show where they are.
        let now = crate::services::recent::unix_now();
        let shown = tab.shown();
        for i in 0..shown {
            let (id, loc) = (tab.panes[i].id, tab.panes[i].location.clone());
            if tab.visits.cursor(id).is_none() {
                tab.visits.visit(id, &loc, now, crate::state::workspace_visits::VisitKind::New);
            }
        }
        let rows = tab.visits.rows(Some(self.pane));
        ui.label(RichText::new(tr!("workspaces-history-intro")).color(p.text_secondary));
        ui.add_space(10.0);
        if rows.is_empty() {
            ui.label(RichText::new(tr!("workspaces-history-empty")).color(p.text_tertiary));
        }
        let mut go = None;
        ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
            let width = ui.available_width();
            for (k, row) in rows.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
                let painter = ui.painter_at(rect);
                if row.current {
                    painter.rect_filled(rect, CornerRadius::same(radius::ROW), p.accent_subtle);
                    if !self.scrolled {
                        resp.scroll_to_me(Some(egui::Align::Center));
                        self.scrolled = true;
                    }
                } else if resp.hovered() {
                    painter.rect_filled(rect, CornerRadius::same(radius::ROW), p.bg_surface2);
                }
                // Tree lines: a vertical guide per level and an elbow into the row.
                let x0 = rect.left() + 10.0;
                let line = Stroke::new(1.0, p.border_strong);
                for d in 0..row.depth {
                    let x = x0 + d as f32 * INDENT + 6.0;
                    let next_deeper = rows.get(k + 1).is_some_and(|r| r.depth > d);
                    let bottom = if d + 1 == row.depth && !next_deeper { rect.center().y } else { rect.bottom() };
                    painter.line_segment([pos2(x, rect.top()), pos2(x, bottom)], line);
                }
                let ix = x0 + row.depth as f32 * INDENT;
                if row.depth > 0 {
                    let x = ix - INDENT + 6.0;
                    painter.line_segment([pos2(x, rect.center().y), pos2(ix + 2.0, rect.center().y)], line);
                }
                let glyph = if row.branches { ph::GIT_FORK } else { ph::FOLDER };
                let gcolor = if row.current { p.accent } else { p.glyph_folder };
                painter.text(
                    pos2(ix + 10.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    glyph,
                    icons::font_fill(14.0),
                    gcolor,
                );
                let title = location_title(&row.location);
                let tx = ix + 22.0;
                let right = rect.right() - 110.0;
                let g = crate::ui::widgets::trunc_galley(
                    &painter,
                    &title,
                    FontId::proportional(12.5),
                    p.text_primary,
                    (right - tx).max(40.0),
                );
                let title_w = g.size().x;
                painter.galley(pos2(tx, rect.center().y - g.size().y / 2.0), g, p.text_primary);
                let detail = row.location.display();
                let dx = tx + title_w + 8.0;
                if right - dx > 40.0 {
                    let g = crate::ui::widgets::trunc_galley(
                        &painter,
                        &detail,
                        FontId::proportional(11.0),
                        p.text_tertiary,
                        right - dx,
                    );
                    painter.galley(pos2(dx, rect.center().y - g.size().y / 2.0), g, p.text_tertiary);
                }
                let when = when_label(row.last, now);
                let count = if row.count > 1 { format!("  ×{}", row.count) } else { String::new() };
                painter.text(
                    pos2(rect.right() - 8.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    format!("{when}{count}"),
                    FontId::proportional(11.5),
                    p.text_secondary,
                );
                let resp = resp.on_hover_text(detail);
                crate::ui::a11y::button(&resp, &title);
                if resp.clicked() && !row.current {
                    go = Some(row.location.clone());
                }
            }
        });
        if let Some(location) = go {
            ctx.cmds.push(Cmd::Workspace(WsCmd::Visit { pane: self.pane, location }));
            return false;
        }
        !crate::ui::dialogs::close_footer(ui, tr!("common-close"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_labels() {
        let now = 1_791_385_200u64;
        assert_eq!(when_label(now - 60, now).len(), 5, "HH:MM today");
        assert!(when_label(now - 3 * 86_400, now).contains(':'), "weekday + time this week");
        assert!(!when_label(now - 30 * 86_400, now).contains(':'), "date only");
    }
}
