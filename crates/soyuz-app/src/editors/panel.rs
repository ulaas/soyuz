//! Editor panel chrome: the dockable card (right or bottom, or maximized over the file
//! panes) with one tab per document, the external-change banner, loading / error states
//! and the "unsaved changes" prompts.

use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Key, Layout, Margin, Panel, Rect, RichText, Sense, Stroke, Ui, pos2,
    vec2,
};
use soyuz_core::config::EditorDock;

use super::io::{Answer, Msg, Want};
use super::{DocKind, External};
use crate::app::SoyuzApp;
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dialogs::conflict::{Btn, button};
use crate::ui::dialogs::{Dialog, DialogCtx};

const HEADER_H: f32 = 38.0;

/// The docked panel (right or bottom).
pub fn docked(ui: &mut Ui, app: &mut SoyuzApp) {
    let p = palette(ui.ctx());
    let screen = ui.ctx().content_rect();
    match app.config.editor.dock {
        EditorDock::Right => {
            Panel::right("soyuz.editor")
                .resizable(true)
                .default_size((screen.width() * 0.46).max(420.0))
                .size_range(360.0..=(screen.width() * 0.8).max(400.0))
                .show_separator_line(false)
                .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 2, right: 8, top: 2, bottom: 6 }))
                .show(ui, |ui| {
                    let r = ui.available_rect_before_wrap();
                    ui.allocate_rect(r, Sense::hover());
                    card(ui, r, app);
                });
        }
        EditorDock::Bottom => {
            Panel::bottom("soyuz.editor.bottom")
                .resizable(true)
                .default_size((screen.height() * 0.42).max(260.0))
                .size_range(200.0..=(screen.height() * 0.8).max(240.0))
                .show_separator_line(false)
                .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 0, right: 8, top: 2, bottom: 6 }))
                .show(ui, |ui| {
                    let r = ui.available_rect_before_wrap();
                    ui.allocate_rect(r, Sense::hover());
                    card(ui, r, app);
                });
        }
    }
}

fn kind_glyph(k: &DocKind) -> &'static str {
    match k {
        DocKind::Text(_) => ph::FILE_TEXT,
        DocKind::Hex(_) => ph::BINARY,
        DocKind::Image(_) => ph::IMAGE,
        DocKind::Loading => ph::FILE,
        DocKind::Failed(_) => ph::WARNING,
    }
}

enum TabCmd {
    Select(usize),
    Save(usize),
    Close(usize),
    Reopen(usize, Want),
    Reload(usize),
    Reveal(usize),
}

/// The editor card in `rect`.
pub fn card(ui: &mut Ui, rect: Rect, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    ui.painter().rect_filled(rect, CornerRadius::same(radius::MODAL), p.bg_surface1);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius::MODAL),
        Stroke::new(1.0, p.border_subtle),
        egui::StrokeKind::Inside,
    );
    let mut focus: Vec<egui::Id> = Vec::new();
    // ---- header: tabs + panel buttons ----
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H));
    let buttons_w = 3.0 * 28.0 + 10.0;
    let tabs_right = header.right() - buttons_w;
    let mut cmds: Vec<TabCmd> = Vec::new();
    let n = app.editors.docs.len();
    let avail = (tabs_right - header.left() - 12.0).max(60.0);
    // Natural widths (name + icon + close), shrunk evenly when they don't fit.
    let natural: Vec<f32> = app
        .editors
        .docs
        .iter()
        .map(|d| {
            let g = ui.painter().layout_no_wrap(d.name.clone(), FontId::proportional(12.5), p.text_primary);
            (g.size().x + 54.0).clamp(84.0, 220.0)
        })
        .collect();
    let sum: f32 = natural.iter().sum::<f32>() + 4.0 * n as f32;
    let shrink = if sum > avail { avail / sum } else { 1.0 };
    let mut x = header.left() + 8.0;
    for (i, d) in app.editors.docs.iter().enumerate() {
        if x + 40.0 > tabs_right {
            break;
        }
        let w = (natural[i] * shrink).max(60.0).min(tabs_right - x);
        let r = Rect::from_min_size(pos2(x, header.top() + 6.0), vec2(w, 26.0));
        let resp = ui.interact(r, ui.id().with(("editor.tab", d.id)), Sense::click());
        let active = i == app.editors.active;
        crate::ui::a11y::tab(&resp, &d.name, active);
        if active {
            ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2);
            ui.painter().hline((r.left() + 8.0)..=(r.right() - 8.0), r.bottom() - 1.0, Stroke::new(2.0, p.accent));
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2.gamma_multiply(0.6));
        }
        let fg = if active { p.text_primary } else { p.text_secondary };
        let glyph_color = match &d.kind {
            DocKind::Image(_) => p.glyph_image,
            DocKind::Hex(_) => p.glyph_exe,
            DocKind::Text(_) => p.glyph_code,
            DocKind::Failed(_) => p.danger,
            DocKind::Loading => p.text_tertiary,
        };
        ui.painter().text(
            pos2(r.left() + 14.0, r.center().y),
            Align2::CENTER_CENTER,
            kind_glyph(&d.kind),
            icons::font(14.0),
            glyph_color,
        );
        let name_w = (w - 50.0).max(10.0);
        let g = crate::ui::widgets::trunc_galley(ui.painter(), &d.name, FontId::proportional(12.5), fg, name_w);
        ui.painter().galley(pos2(r.left() + 26.0, r.center().y - g.size().y / 2.0), g, fg);
        let xr = Rect::from_center_size(pos2(r.right() - 13.0, r.center().y), vec2(18.0, 18.0));
        let xresp = ui.interact(xr, ui.id().with(("editor.tab.x", d.id)), Sense::click());
        crate::ui::a11y::button(&xresp, tr!("common-close"));
        let dirty = d.is_dirty();
        if xresp.hovered() || (resp.hovered() && !dirty) || (active && !dirty) {
            if xresp.hovered() {
                ui.painter().rect_filled(xr, CornerRadius::same(4), p.bg_surface3);
            }
            ui.painter().text(xr.center(), Align2::CENTER_CENTER, ph::X, icons::font(11.0), p.text_tertiary);
        } else if dirty {
            ui.painter().circle_filled(xr.center(), 4.0, p.accent);
        }
        if d.saving {
            crate::ui::widgets::spinner(ui, xr.center(), 5.0, p.accent);
        }
        let full = d.path.display().to_string();
        let resp = resp.on_hover_text(if dirty { tr!("editors-tab-unsaved-tip", path = full) } else { full });
        if xresp.clicked() || resp.middle_clicked() {
            cmds.push(TabCmd::Close(i));
        } else if resp.clicked() {
            cmds.push(TabCmd::Select(i));
        }
        let is_text = matches!(d.kind, DocKind::Text(_));
        let is_hex = matches!(d.kind, DocKind::Hex(_));
        crate::ui::native::popup::context_menu(&resp, |ui| {
            ui.set_width(220.0);
            use crate::ui::widgets::{menu_item, menu_separator};
            if menu_item(ui, Some(ph::FLOPPY_DISK), tr!("common-save"), Some("Ctrl+S"), dirty).clicked() {
                cmds.push(TabCmd::Save(i));
                ui.close();
            }
            if menu_item(ui, Some(ph::ARROWS_CLOCKWISE), tr!("editors-tab-reload"), None, true).clicked() {
                cmds.push(TabCmd::Reload(i));
                ui.close();
            }
            menu_separator(ui);
            if !is_hex && menu_item(ui, Some(ph::BINARY), tr!("editors-tab-reopen-hex"), None, true).clicked() {
                cmds.push(TabCmd::Reopen(i, Want::Hex));
                ui.close();
            }
            if !is_text && menu_item(ui, Some(ph::FILE_TEXT), tr!("editors-tab-reopen-text"), None, true).clicked() {
                cmds.push(TabCmd::Reopen(i, Want::Text));
                ui.close();
            }
            if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("editors-tab-reveal"), None, true).clicked() {
                cmds.push(TabCmd::Reveal(i));
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::X), tr!("common-close"), Some("Ctrl+W"), true).clicked() {
                cmds.push(TabCmd::Close(i));
                ui.close();
            }
        });
        x += w + 4.0;
    }
    // Panel buttons.
    let dock = app.config.editor.dock;
    let maximized = app.editors.maximized;
    let br =
        |i: f32| Rect::from_center_size(pos2(header.right() - 20.0 - i * 28.0, header.top() + 19.0), vec2(24.0, 24.0));
    let id = ui.id().with("editor.panel");
    if super::widgets::icon_at(ui, br(0.0), id.with("close"), ph::X, tr!("editors-panel-hide"), true, &p) {
        app.editors.open = false;
        app.editors.maximized = false;
        app.ui.focus = crate::app::Focus::List;
    }
    let (mg, mt) = if maximized {
        (ph::CORNERS_IN, tr!("editors-panel-restore"))
    } else {
        (ph::CORNERS_OUT, tr!("editors-panel-maximize"))
    };
    if super::widgets::icon_at(ui, br(1.0), id.with("max"), mg, mt, true, &p) {
        app.editors.maximized = !maximized;
    }
    let (dg, dt) = match dock {
        EditorDock::Right => (ph::SQUARE_HALF_BOTTOM, tr!("editors-panel-dock-bottom")),
        EditorDock::Bottom => (ph::SQUARE_HALF, tr!("editors-panel-dock-right")),
    };
    if super::widgets::icon_at(ui, br(2.0), id.with("dock"), dg, dt, !maximized, &p) {
        let next = if dock == EditorDock::Right { EditorDock::Bottom } else { EditorDock::Right };
        crate::ui::settings::update_config(app, &ctx, |c| c.editor.dock = next);
    }
    ui.painter().hline(rect.x_range(), header.bottom(), Stroke::new(1.0, p.border_subtle));

    // ---- body ----
    let mut body = Rect::from_min_max(
        pos2(rect.left() + 1.0, header.bottom() + 1.0),
        pos2(rect.right() - 1.0, rect.bottom() - 1.0),
    );
    let idx = app.editors.active;
    let mut save_now = false;
    let mut toggle_wrap = false;
    let mut toggle_ws = false;
    let mut external_answer: Option<usize> = None;
    let cfg = app.config.editor.clone();
    if let Some(d) = app.editors.docs.get_mut(idx) {
        if let Some(ext) = d.external {
            let r = Rect::from_min_size(pos2(body.left() + 8.0, body.top() + 6.0), vec2(body.width() - 16.0, 30.0));
            let (text, buttons): (&str, Vec<&str>) = match ext {
                External::Changed => (
                    tr!("editors-external-changed"),
                    vec![tr!("editors-external-reload"), tr!("editors-external-keep")],
                ),
                External::Deleted => (tr!("editors-external-deleted"), vec![tr!("common-ok")]),
            };
            external_answer = super::widgets::banner(ui, r, &p, ph::WARNING, p.warning, text, &buttons);
            if ext == External::Deleted && external_answer.is_some() {
                d.external = None;
                external_answer = None;
            }
            body.min.y += 42.0;
        }
        let doc_id = egui::Id::new(("soyuz.editor.doc", d.id));
        match &mut d.kind {
            DocKind::Loading => {
                crate::ui::widgets::spinner(ui, body.center(), 12.0, p.accent);
            }
            DocKind::Failed(e) => {
                let msg = tr!("editors-open-failed", name = &d.name, error = &*e);
                ui.painter().text(
                    body.center(),
                    Align2::CENTER_CENTER,
                    msg,
                    FontId::proportional(13.0),
                    p.text_secondary,
                );
            }
            DocKind::Text(t) => {
                let mut env = super::text::TextEnv { cfg: &cfg, p: &p, focus: &mut focus, id: doc_id };
                let out = t.show(ui, body, &mut env);
                save_now = out.save;
                toggle_wrap = out.toggle_wrap;
                toggle_ws = out.toggle_whitespace;
            }
            DocKind::Hex(h) => {
                save_now = h.show(ui, body, &p, cfg.font_size, &mut focus, doc_id).save;
            }
            DocKind::Image(im) => {
                save_now = im.show(ui, body, &p, cfg.jpeg_quality, &mut focus, doc_id).save;
            }
        }
    } else {
        let c = body.center();
        ui.painter().text(
            c - vec2(0.0, 26.0),
            Align2::CENTER_CENTER,
            ph::NOTE_PENCIL,
            icons::font(30.0),
            p.text_tertiary,
        );
        ui.painter().text(
            c + vec2(0.0, 6.0),
            Align2::CENTER_CENTER,
            tr!("editors-empty"),
            FontId::proportional(14.0),
            p.text_secondary,
        );
        let key = crate::keys::chord_label(&app.keymap, &soyuz_core::config::Action::OpenInEditor)
            .unwrap_or_else(|| "F4".into());
        ui.painter().text(
            c + vec2(0.0, 28.0),
            Align2::CENTER_CENTER,
            tr!("editors-empty-hint", key = key),
            FontId::proportional(12.0),
            p.text_tertiary,
        );
    }
    app.editors.focus_ids = focus;
    if save_now {
        super::save(app, idx);
    }
    if toggle_wrap {
        crate::ui::settings::update_config(app, &ctx, |c| c.editor.wrap = !c.editor.wrap);
    }
    if toggle_ws {
        crate::ui::settings::update_config(app, &ctx, |c| c.editor.show_whitespace = !c.editor.show_whitespace);
    }
    match external_answer {
        Some(0) => super::reload(app, idx),
        Some(_) => {
            if let Some(d) = app.editors.docs.get_mut(idx) {
                d.external = None;
            }
        }
        None => {}
    }
    for c in cmds {
        match c {
            TabCmd::Select(i) => {
                app.editors.active = i;
                if let Some(d) = app.editors.docs.get_mut(i) {
                    d.request_focus();
                }
            }
            TabCmd::Save(i) => super::save(app, i),
            TabCmd::Close(i) => super::close(app, i, false),
            TabCmd::Reopen(i, want) => {
                if app.editors.docs.get(i).is_some_and(|d| d.is_dirty()) {
                    app.ui.toasts.push(crate::models::toasts::Toast::info(tr!("editors-reopen-dirty")));
                } else {
                    super::reopen_as(app, i, want);
                }
            }
            TabCmd::Reload(i) => super::reload(app, i),
            TabCmd::Reveal(i) => {
                if let Some(d) = app.editors.docs.get(i) {
                    app.cmds.push(crate::cmd::Cmd::RevealPath(d.path.clone()));
                }
            }
        }
    }
}

/// "Save changes?" for one document or for the whole window.
pub struct UnsavedDialog {
    title: String,
    body: String,
    target: Option<u64>,
    tx: crossbeam_channel::Sender<Msg>,
    answered: bool,
}

impl UnsavedDialog {
    pub fn close(name: &str, doc: u64, tx: crossbeam_channel::Sender<Msg>) -> Self {
        UnsavedDialog {
            title: tr!("editors-unsaved-title", name = name),
            body: tr!("editors-unsaved-body").into(),
            target: Some(doc),
            tx,
            answered: false,
        }
    }

    pub fn quit(names: &[String], tx: crossbeam_channel::Sender<Msg>) -> Self {
        let title = if names.len() == 1 {
            tr!("editors-unsaved-title", name = &names[0])
        } else {
            tr!("editors-unsaved-quit-title", count = names.len())
        };
        let body = if names.len() > 1 {
            let mut list = names.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
            if names.len() > 6 {
                list.push_str(", …");
            }
            tr!("editors-unsaved-quit-body-list", names = list)
        } else {
            tr!("editors-unsaved-quit-body").to_string()
        };
        UnsavedDialog { title, body, target: None, tx, answered: false }
    }

    fn answer(&mut self, a: Answer) {
        self.answered = true;
        let _ = self.tx.send(match self.target {
            Some(doc) => Msg::Close { doc, answer: a },
            None => Msg::Quit(a),
        });
    }
}

impl Drop for UnsavedDialog {
    fn drop(&mut self) {
        if !self.answered {
            self.answer(Answer::Cancel);
        }
    }
}

impl Dialog for UnsavedDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::FLOPPY_DISK)
    }

    fn width(&self) -> f32 {
        460.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(&self.body).color(p.text_secondary));
        ui.add_space(18.0);
        let (mut save, mut discard, mut cancel) = (false, false, false);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            save = button(
                ui,
                if self.target.is_some() { tr!("common-save") } else { tr!("editors-unsaved-save-all") },
                Btn::Primary,
            )
            .clicked();
            discard = button(ui, tr!("editors-unsaved-dont-save"), Btn::Danger).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
        });
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            save = true;
        }
        if save {
            self.answer(Answer::Save);
            return false;
        }
        if discard {
            self.answer(Answer::Discard);
            return false;
        }
        !cancel
    }
}
