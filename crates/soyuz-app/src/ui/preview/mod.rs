//! Right-side preview pane (PLAN M9, Alt+P): previews the focused pane's cursor item
//! (or the current folder when nothing is selected) with the shared renderers in
//! [`render`], plus an info section. Everything loads off the UI thread; changing the
//! selection makes in-flight requests stale.

pub mod hex;
pub mod markdown;
pub mod render;

use egui::{
    Align2, CornerRadius, FontId, Frame, Margin, Panel, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};
use soyuz_core::{EntryInfo, EntryKind, Location};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::services::preview::Content;
use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::views::flow::semibold;
use crate::ui::views::icons as file_icons;
use crate::ui::widgets::{icon_toggle, trunc_galley};
use render::{PreviewAction, PreviewState, info_grid, info_rows};

/// Preview pane state (lives in `UiState`).
pub struct PreviewPane {
    pub open: bool,
    pub width: f32,
    state: Option<PreviewState>,
}

impl Default for PreviewPane {
    fn default() -> Self {
        PreviewPane { open: false, width: 340.0, state: None }
    }
}

impl PreviewPane {
    pub fn new(open: bool) -> Self {
        PreviewPane { open, ..Default::default() }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open
            && let Some(s) = &mut self.state
        {
            s.clear();
        }
    }

    pub fn toggle_hex(&mut self) {
        if let Some(s) = &mut self.state {
            s.force_hex = !s.force_hex;
            s.invalidate();
        }
    }
}

/// What the preview should show for the focused pane: the cursor entry, else the first
/// selected entry, else the folder itself.
pub fn target_entry(app: &SoyuzApp) -> Option<EntryInfo> {
    let pane = app.ws.pane();
    if let Some(e) = pane.cursor.as_deref().and_then(|c| pane.entry(c)) {
        return Some(e.clone());
    }
    if let Some(e) = pane.selected_entries().first() {
        return Some((*e).clone());
    }
    match &pane.location {
        Location::Dir(p) => Some(EntryInfo::minimal(p.clone(), EntryKind::Dir)),
        Location::Archive { archive, inner } if !inner.is_empty() => {
            Some(EntryInfo::minimal(soyuz_core::join_vpath(archive, inner), EntryKind::Dir))
        }
        Location::Archive { archive, .. } => Some(EntryInfo::minimal(archive.clone(), EntryKind::File)),
        _ => None,
    }
}

pub fn show(root: &mut Ui, app: &mut SoyuzApp) {
    if !app.ui.preview.open {
        return;
    }
    let ctx = root.ctx().clone();
    let p = palette(&ctx);
    let target = target_entry(app);
    let pane_id = app.ws.pane().id;
    let st = app.ui.preview.state.get_or_insert_with(|| PreviewState::new(&ctx, false));
    let limits = st.limits(&app.config, &p);
    st.set_target(target.as_ref(), limits);
    st.poll(&ctx);
    let mut action = None;
    let mut close = false;
    let resp = Panel::right("soyuz.preview")
        .resizable(true)
        .default_size(app.ui.preview.width)
        .size_range(240.0..=760.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 0, right: 8, top: 2, bottom: 2 }))
        .show(root, |ui| {
            let card = ui.available_rect_before_wrap();
            ui.painter().rect_filled(card, CornerRadius::same(radius::MODAL), p.bg_surface1);
            ui.painter().rect_stroke(
                card,
                CornerRadius::same(radius::MODAL),
                Stroke::new(1.0, p.border_subtle),
                egui::StrokeKind::Inside,
            );
            ui.allocate_rect(card, Sense::hover());
            let Some(st) = app.ui.preview.state.as_mut() else { return };

            // Header: glyph, name, kind; hex toggle and close.
            let header = Rect::from_min_size(card.min, vec2(card.width(), 48.0));
            let (glyph, color, name, sub) = match &st.entry {
                Some(e) => {
                    let kind = st.loaded.as_ref().map(|l| l.kind.label().to_string()).unwrap_or_default();
                    (file_icons::glyph(e), file_icons::color(e, &p), e.name.clone(), kind)
                }
                None => (ph::EYE, p.text_tertiary, "Preview".to_string(), String::new()),
            };
            let name = if name.is_empty() { "Preview".into() } else { name };
            ui.painter().text(
                pos2(header.left() + 22.0, header.center().y),
                Align2::CENTER_CENTER,
                glyph,
                icons::font_fill(18.0),
                color,
            );
            let max_w = header.width() - 40.0 - 72.0;
            let tg = trunc_galley(ui.painter(), &name, semibold(13.0), p.text_primary, max_w);
            ui.painter().galley(pos2(header.left() + 40.0, header.center().y - 15.0), tg, p.text_primary);
            ui.painter().text(
                pos2(header.left() + 40.0, header.center().y + 9.0),
                Align2::LEFT_CENTER,
                sub,
                FontId::proportional(11.0),
                p.text_tertiary,
            );
            let hex_r = Rect::from_min_size(pos2(header.right() - 66.0, header.center().y - 14.0), vec2(28.0, 28.0));
            let can_hex = st.entry.as_ref().is_some_and(|e| !e.is_dir_like());
            let hex_on = st.force_hex;
            let hresp = ui
                .scope_builder(UiBuilder::new().max_rect(hex_r), |ui| {
                    icon_toggle(ui, ph::BINARY, "Show as hex", can_hex, hex_on)
                })
                .inner;
            if hresp.clicked() {
                action = Some(PreviewAction::ToggleHex);
            }
            let close_r = Rect::from_min_size(pos2(header.right() - 34.0, header.center().y - 14.0), vec2(28.0, 28.0));
            let cresp = ui
                .scope_builder(UiBuilder::new().max_rect(close_r), |ui| {
                    icon_toggle(ui, ph::X, "Close preview  (Alt+P)", true, false)
                })
                .inner;
            close = cresp.clicked();
            ui.painter().line_segment(
                [pos2(card.left() + 4.0, header.bottom()), pos2(card.right() - 4.0, header.bottom())],
                Stroke::new(1.0, p.border_subtle),
            );

            // Content vs info split: visual kinds get a compact viewer and a roomy info
            // section; text-like kinds get most of the height.
            let body = Rect::from_min_max(pos2(card.left() + 1.0, header.bottom() + 1.0), card.max - vec2(1.0, 1.0));
            let rows = match &st.entry {
                Some(e) => info_rows(e, st.loaded.as_ref(), st.folder_size(), &app.config),
                None => Vec::new(),
            };
            let textual = matches!(
                st.loaded.as_ref().map(|l| &l.content),
                Some(Content::Text(_) | Content::Markdown { .. } | Content::Hex(_) | Content::Archive(_))
            );
            let info_h = if rows.is_empty() {
                0.0
            } else if textual {
                (rows.len() as f32 * 21.0 + 40.0).min(body.height() * 0.32).min(170.0)
            } else {
                (rows.len() as f32 * 21.0 + 40.0).min(body.height() * 0.5)
            };
            let content_r = Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - info_h));
            let mut child = ui.new_child(UiBuilder::new().max_rect(content_r));
            child.set_clip_rect(content_r);
            if let Some(a) = render::content(&mut child, content_r, st, false) {
                action = Some(a);
            }
            if info_h > 0.0 {
                let ir = Rect::from_min_max(pos2(body.left(), content_r.bottom()), body.max);
                ui.painter().line_segment(
                    [pos2(ir.left() + 4.0, ir.top()), pos2(ir.right() - 4.0, ir.top())],
                    Stroke::new(1.0, p.border_subtle),
                );
                let mut ichild = ui.new_child(UiBuilder::new().max_rect(ir.shrink2(vec2(14.0, 10.0))));
                ScrollArea::vertical().id_salt("preview.info").auto_shrink([false, false]).show(&mut ichild, |ui| {
                    ui.label(egui::RichText::new("INFO").size(10.5).color(p.text_tertiary).strong());
                    ui.add_space(4.0);
                    info_grid(ui, &rows, 78.0);
                });
            }
        });
    app.ui.preview.width = resp.response.rect.width();
    if close {
        app.ui.preview.toggle();
    }
    match action {
        Some(PreviewAction::ToggleHex) => app.ui.preview.toggle_hex(),
        Some(PreviewAction::Open(path)) => {
            app.cmds.push(Cmd::Navigate {
                pane: pane_id,
                location: Location::Archive { archive: path, inner: String::new() },
                select: None,
            });
        }
        None => {}
    }
}
