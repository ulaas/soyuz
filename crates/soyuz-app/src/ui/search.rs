//! Search bar (GUI): Ctrl+Shift+F / F3 opens a bar under the toolbar with
//! the query field and option pills (name mode, case, contents, size, date, kind,
//! hidden, .gitignore). Results stream into the focused pane as `Location::Search`
//! (sortable, actionable, Path column), with a status strip showing progress, the result
//! count and a cancel button. Searches can be saved (sidebar "Searches" section). The
//! state and the logic live in `models::search`; this module draws them.

use std::collections::BTreeSet;

use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Margin, Panel, Popup, Rect, RectAlign, Response, Sense,
    Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use soyuz_core::SearchId;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::models::search as model;
pub use crate::models::search::{
    SavedEntry, SearchState as SearchUi, date_label, folder_sizes, kind_label, mode_label, open, remove_saved, run,
    run_query, run_saved, toggle_flat_view,
};
use crate::services::search::{self as svc, DateRange, KindFilter, NameMode};
use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::widgets::{
    NavRow, collapsible_header, menu_check, menu_item, menu_separator, pill_button, spinner, trunc_galley,
};

const ROW_H: f32 = 44.0;
const STATUS_H: f32 = 28.0;

/// Close the bar and drop the keyboard focus of its fields.
fn close(app: &mut SoyuzApp, ctx: &egui::Context) {
    model::close(&mut app.core);
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
}

fn pill(ui: &mut Ui, r: Rect, id: Id, icon: Option<&str>, label: &str, active: bool, tip: &str) -> Response {
    let p = palette(ui.ctx());
    let resp = ui.interact(r, id, Sense::click());
    let fill = if active {
        p.accent_subtle
    } else if resp.hovered() {
        p.bg_surface2
    } else {
        p.bg_surface1
    };
    ui.painter().rect_filled(r, CornerRadius::same(13), fill);
    let stroke = if active { p.accent.gamma_multiply(0.55) } else { p.border_subtle };
    ui.painter().rect_stroke(r, CornerRadius::same(13), Stroke::new(1.0, stroke), StrokeKind::Inside);
    let c = if active { p.accent } else { p.text_secondary };
    let mut x = r.left() + 10.0;
    if let Some(g) = icon {
        ui.painter().text(pos2(x + 6.0, r.center().y), Align2::CENTER_CENTER, g, icons::font(13.0), c);
        x += 17.0;
    }
    ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), c);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

fn pill_width(ui: &Ui, icon: bool, label: &str) -> f32 {
    let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), Color32::WHITE).size().x;
    w + 20.0 + if icon { 17.0 } else { 0.0 }
}

pub fn bar(root: &mut Ui, app: &mut SoyuzApp) {
    let shown = model::shown_search(&app.core);
    if !app.core.ui.search.open && shown.is_none() {
        return;
    }
    let ctx = root.ctx().clone();
    let p = palette(&ctx);
    let height = if app.core.ui.search.open { ROW_H } else { 0.0 } + if shown.is_some() { STATUS_H } else { 0.0 };
    Panel::top("soyuz.search")
        .exact_size(height)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 6, right: 8, top: 0, bottom: 2 }))
        .show(root, |ui| {
            let full = ui.available_rect_before_wrap();
            let mut y = full.top();
            if app.core.ui.search.open {
                let row = Rect::from_min_size(pos2(full.left(), y), vec2(full.width(), ROW_H));
                controls(ui, row, app);
                y += ROW_H;
            }
            if let Some(id) = shown {
                let row = Rect::from_min_size(pos2(full.left(), y), vec2(full.width(), STATUS_H));
                status(ui, row, app, id);
            }
            ui.allocate_rect(full, Sense::hover());
        });
}

fn controls(ui: &mut Ui, row: Rect, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let card = row.shrink2(vec2(0.0, 4.0));
    ui.painter().rect_filled(card, CornerRadius::same(radius::MODAL), p.bg_surface1);
    ui.painter().rect_stroke(
        card,
        CornerRadius::same(radius::MODAL),
        Stroke::new(1.0, p.border_subtle),
        StrokeKind::Inside,
    );
    let cy = card.center().y;
    let running = model::shown_search(&app.core).is_some_and(svc::is_running);

    // Right side: close, save, run/stop.
    let mut rx = card.right() - 6.0;
    let close_r = Rect::from_center_size(pos2(rx - 13.0, cy), vec2(24.0, 24.0));
    rx = close_r.left() - 4.0;
    let cresp =
        ui.interact(close_r, Id::new("soyuz.search.close"), Sense::click()).on_hover_text(tr!("views-search-close"));
    if cresp.hovered() {
        ui.painter().rect_filled(close_r, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(close_r.center(), Align2::CENTER_CENTER, ph::X, icons::font(13.0), p.text_tertiary);
    if cresp.clicked() {
        close(app, &ctx);
        return;
    }
    let star_r = Rect::from_center_size(pos2(rx - 13.0, cy), vec2(24.0, 24.0));
    rx = star_r.left() - 6.0;
    let sresp =
        ui.interact(star_r, Id::new("soyuz.search.save"), Sense::click()).on_hover_text(tr!("views-search-save-tip"));
    if sresp.hovered() {
        ui.painter().rect_filled(star_r, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(star_r.center(), Align2::CENTER_CENTER, ph::BOOKMARK_SIMPLE, icons::font(14.0), p.text_secondary);
    if sresp.clicked() {
        model::begin_save(&mut app.core);
    }
    save_popup(ui, &sresp, app);
    let label = if running { tr!("views-search-stop") } else { tr!("views-search-run") };
    let bw = pill_width(ui, true, label) + 6.0;
    let br = Rect::from_center_size(pos2(rx - bw / 2.0, cy), vec2(bw, 26.0));
    rx = br.left() - 10.0;
    let mut bchild = ui.new_child(UiBuilder::new().max_rect(br));
    let run_clicked =
        pill_button(&mut bchild, Some(if running { ph::STOP } else { ph::MAGNIFYING_GLASS }), label, !running)
            .clicked();

    // Left: scope chip.
    let scope = soyuz_fs::strip_verbatim(&app.core.ui.search.form.root);
    let everywhere = app.core.ui.search.form.everywhere;
    let scope_name =
        if scope.as_os_str().is_empty() { "…".to_string() } else { soyuz_core::entry::display_name(&scope) };
    let scope_label = if everywhere {
        tr!("views-search-everywhere").to_string()
    } else {
        tr!("views-search-in", folder = scope_name)
    };
    let sw = pill_width(ui, true, &scope_label).min(180.0);
    let scope_r = Rect::from_min_size(pos2(card.left() + 6.0, cy - 13.0), vec2(sw, 26.0));
    let tip = if everywhere {
        tr!("views-search-everywhere-tip").to_string()
    } else {
        tr!("views-search-in-tip", path = scope.display().to_string())
    };
    let scope_icon = if everywhere { ph::DATABASE } else { ph::FOLDER_SIMPLE };
    let sresp =
        pill(ui, scope_r, Id::new("soyuz.search.scope"), Some(scope_icon), &trunc(&scope_label, 22), everywhere, &tip);
    let here = model::current_dir(&app.core);
    let index_on = crate::services::index::is_enabled();
    let mut rerun = false;
    crate::ui::native::popup::Menu::menu(&sresp).align(RectAlign::BOTTOM_START).show(|ui| {
        ui.set_width(250.0);
        let form = &mut app.core.ui.search.form;
        let label = match &here {
            Some(d) => tr!("views-search-this-folder-named", folder = soyuz_core::entry::display_name(d)),
            None => tr!("views-search-this-folder").to_string(),
        };
        if menu_check(ui, !form.everywhere, &label, None).clicked() {
            form.everywhere = false;
            if let Some(d) = &here {
                form.root = d.clone();
            }
            rerun = true;
            ui.close();
        }
        let r = menu_check(ui, form.everywhere, tr!("views-search-all-indexed"), None).on_hover_text(if index_on {
            tr!("views-search-all-indexed-tip")
        } else {
            tr!("views-search-all-indexed-off-tip")
        });
        if r.clicked() {
            form.everywhere = true;
            rerun = true;
            ui.close();
        }
    });

    // Pills from the right edge leftwards.
    let f = app.core.ui.search.form.clone();
    let mut pills: Vec<(&'static str, Option<&'static str>, String, bool, &'static str)> = vec![
        ("gitignore", None, ".gitignore".into(), f.respect_gitignore, tr!("views-search-gitignore-tip")),
        ("hidden", Some(ph::EYE), tr!("views-search-hidden").into(), f.include_hidden, tr!("views-search-hidden-tip")),
        (
            "kind",
            None,
            if f.kind == KindFilter::Any { tr!("views-search-kind").into() } else { kind_label(f.kind).into() },
            f.kind != KindFilter::Any,
            tr!("views-search-kind-tip"),
        ),
        (
            "date",
            Some(ph::CALENDAR),
            if f.date == DateRange::Any { tr!("views-search-date").into() } else { date_label(f.date).into() },
            f.date != DateRange::Any,
            tr!("views-search-date-tip"),
        ),
        (
            "size",
            Some(ph::RULER),
            model::size_label(&f),
            !f.min_size.trim().is_empty() || !f.max_size.trim().is_empty(),
            tr!("views-search-size-tip"),
        ),
        (
            "content",
            Some(ph::FILE_TEXT),
            tr!("views-search-contents").into(),
            f.content,
            tr!("views-search-contents-tip"),
        ),
        ("case", None, "Aa".into(), f.case_sensitive, tr!("views-search-case-tip")),
    ];
    if !f.content {
        pills.push((
            "mode",
            None,
            mode_label(f.mode).into(),
            f.mode != NameMode::Substring,
            tr!("views-search-mode-tip"),
        ));
    } else {
        pills.push(("regex", None, ".*".into(), f.content_regex, tr!("views-search-regex-tip")));
    }
    let mut x = rx;
    let min_text = 180.0;
    let left_limit = scope_r.right() + 8.0 + min_text;
    let mut clicked: Vec<(&'static str, Response)> = Vec::new();
    for (key, icon, label, active, tip) in pills {
        let w = pill_width(ui, icon.is_some(), &label);
        if x - w < left_limit {
            continue; // too narrow: drop the least important pills first
        }
        let r = Rect::from_min_size(pos2(x - w, cy - 13.0), vec2(w, 26.0));
        x -= w + 5.0;
        let resp = pill(ui, r, Id::new(("soyuz.search.pill", key)), icon, &label, active, tip);
        clicked.push((key, resp));
    }
    let mut changed = false;
    for (key, resp) in &clicked {
        let form = &mut app.core.ui.search.form;
        match *key {
            "gitignore" if resp.clicked() => {
                form.respect_gitignore = !form.respect_gitignore;
                changed = true;
            }
            "hidden" if resp.clicked() => {
                form.include_hidden = !form.include_hidden;
                changed = true;
            }
            "content" if resp.clicked() => {
                form.content = !form.content;
                app.core.ui.search.focus = true;
            }
            "case" if resp.clicked() => {
                form.case_sensitive = !form.case_sensitive;
                changed = true;
            }
            "regex" if resp.clicked() => {
                form.content_regex = !form.content_regex;
                changed = true;
            }
            "mode" => {
                crate::ui::native::popup::Menu::menu(resp).align(RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_width(200.0);
                    for m in NameMode::ALL {
                        let hint = match m {
                            NameMode::Substring => tr!("views-search-mode-contains-tip"),
                            NameMode::Glob => "*.rs, IMG_????.jpg",
                            NameMode::Regex => "^report-\\d+\\.pdf$",
                            NameMode::Fuzzy => tr!("views-search-mode-fuzzy-tip"),
                        };
                        if menu_check(ui, form.mode == m, mode_label(m), None).on_hover_text(hint).clicked() {
                            form.mode = m;
                            changed = true;
                            ui.close();
                        }
                    }
                });
            }
            "kind" => {
                crate::ui::native::popup::Menu::menu(resp).align(RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_width(180.0);
                    for k in KindFilter::ALL {
                        if menu_check(ui, form.kind == k, kind_label(k), None).clicked() {
                            form.kind = k;
                            changed = true;
                            ui.close();
                        }
                    }
                });
            }
            "date" => {
                crate::ui::native::popup::Menu::menu(resp).align(RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_width(190.0);
                    for d in DateRange::ALL {
                        if menu_check(ui, form.date == d, date_label(d), None).clicked() {
                            form.date = d;
                            changed = true;
                            ui.close();
                        }
                    }
                });
            }
            "size" => {
                crate::ui::native::popup::Menu::menu(resp)
                    .align(RectAlign::BOTTOM_START)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_width(230.0);
                        ui.label(
                            egui::RichText::new(tr!("views-search-size-caption")).size(10.5).color(p.text_tertiary),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(tr!("views-search-size-min")).color(p.text_secondary));
                            ui.add(
                                TextEdit::singleline(&mut form.min_size)
                                    .hint_text(tr!("views-search-size-min-hint"))
                                    .desired_width(70.0),
                            );
                            ui.label(egui::RichText::new(tr!("views-search-size-max")).color(p.text_secondary));
                            ui.add(
                                TextEdit::singleline(&mut form.max_size)
                                    .hint_text(tr!("views-search-size-max-hint"))
                                    .desired_width(70.0),
                            );
                        });
                        ui.add_space(4.0);
                        menu_separator(ui);
                        for (label, min, max) in [
                            (tr!("views-search-size-tiny"), "", "16k"),
                            (tr!("views-search-size-large"), "100M", ""),
                            (tr!("views-search-size-huge"), "1G", ""),
                            (tr!("views-search-size-any"), "", ""),
                        ] {
                            if menu_item(ui, None, label, None, true).clicked() {
                                form.min_size = min.into();
                                form.max_size = max.into();
                                changed = true;
                                ui.close();
                            }
                        }
                    });
            }
            _ => {}
        }
    }

    // Query field fills the middle.
    let field = Rect::from_min_max(
        pos2(scope_r.right() + 8.0, cy - 13.0),
        pos2(x.max(scope_r.right() + 8.0 + 60.0), cy + 13.0),
    );
    let fid = Id::new("soyuz.search.text");
    let focused = ui.memory(|m| m.has_focus(fid));
    ui.painter().rect_filled(field, CornerRadius::same(radius::ROW), p.bg_surface2);
    if focused {
        ui.painter().rect_stroke(
            field,
            CornerRadius::same(radius::ROW),
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        pos2(field.left() + 14.0, cy),
        Align2::CENTER_CENTER,
        if app.core.ui.search.form.content { ph::FILE_TEXT } else { ph::MAGNIFYING_GLASS },
        icons::font(13.5),
        if focused { p.accent } else { p.text_tertiary },
    );
    let content = app.core.ui.search.form.content;
    let glob_w = if content { (field.width() * 0.32).clamp(70.0, 140.0) } else { 0.0 };
    let inner = Rect::from_min_max(pos2(field.left() + 28.0, cy - 10.0), pos2(field.right() - 8.0 - glob_w, cy + 10.0));
    let esc = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let indexed = crate::services::index::uses_query_language(&app.core.ui.search.form)
        && (app.core.ui.search.form.everywhere || crate::services::index::covers(&app.core.ui.search.form.root));
    let hint = if content {
        tr!("views-search-hint-content")
    } else if indexed {
        tr!("views-search-hint-index")
    } else {
        tr!("views-search-hint-names")
    };
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    let resp = child.add(
        TextEdit::singleline(&mut app.core.ui.search.form.text)
            .id(fid)
            .hint_text(hint)
            .frame(Frame::NONE)
            .font(FontId::proportional(13.0))
            .vertical_align(egui::Align::Center)
            .desired_width(inner.width()),
    );
    if app.core.ui.search.focus {
        resp.request_focus();
        app.core.ui.search.focus = false;
    }
    let mut submit = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if content {
        let gr = Rect::from_min_max(pos2(inner.right() + 6.0, cy - 10.0), pos2(field.right() - 6.0, cy + 10.0));
        ui.painter().line_segment(
            [pos2(gr.left() - 3.0, gr.top()), pos2(gr.left() - 3.0, gr.bottom())],
            Stroke::new(1.0, p.border_strong),
        );
        let mut gchild = ui.new_child(UiBuilder::new().max_rect(gr.shrink2(vec2(4.0, 0.0))));
        let g = gchild.add(
            TextEdit::singleline(&mut app.core.ui.search.form.name_glob)
                .hint_text(tr!("views-search-hint-glob"))
                .frame(Frame::NONE)
                .font(FontId::proportional(12.0))
                .vertical_align(egui::Align::Center)
                .desired_width(gr.width() - 8.0),
        );
        submit |= g.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    }
    if let Some(err) = app.core.ui.search.error.clone() {
        let eg = trunc_galley(ui.painter(), &err, FontId::proportional(11.0), p.danger, inner.width() * 0.6);
        let pos = pos2(inner.right() - eg.size().x, cy - eg.size().y / 2.0);
        if app.core.ui.search.form.text.is_empty() || !focused {
            ui.painter().galley(pos, eg, p.danger);
        }
    }
    if esc {
        close(app, &ctx);
        return;
    }
    model::submit(&mut app.core, run_clicked && running, run_clicked || submit, changed || rerun);
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n { s.chars().take(n - 1).collect::<String>() + "…" } else { s.to_string() }
}

fn save_popup(ui: &mut Ui, anchor: &Response, app: &mut SoyuzApp) {
    if app.core.ui.search.saving.is_none() {
        return;
    }
    let p = palette(ui.ctx());
    let id = Id::new("soyuz.search.save.popup");
    Popup::open_id(ui.ctx(), id);
    let mut done: Option<bool> = None;
    crate::ui::native::popup::Menu::new(id, ui.ctx().clone(), anchor.rect, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .open_memory(None)
        .align(RectAlign::BOTTOM_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(260.0);
            ui.label(egui::RichText::new(tr!("views-search-save-caption")).size(10.5).color(p.text_tertiary));
            ui.add_space(4.0);
            if let Some(name) = app.core.ui.search.saving.as_mut() {
                let r = ui.add(TextEdit::singleline(name).desired_width(f32::INFINITY).margin(Margin::symmetric(8, 5)));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    done = Some(true);
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    done = Some(false);
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if pill_button(ui, None, tr!("common-save"), true).clicked() {
                    done = Some(true);
                }
                if pill_button(ui, None, tr!("common-cancel"), false).clicked() {
                    done = Some(false);
                }
            });
        });
    if !Popup::is_id_open(ui.ctx(), id) && done.is_none() {
        done = Some(false);
    }
    if let Some(save) = done {
        Popup::close_id(ui.ctx(), id);
        model::finish_save(&mut app.core, save);
    }
}

fn status(ui: &mut Ui, row: Rect, app: &mut SoyuzApp, id: SearchId) {
    let p = palette(ui.ctx());
    // The text and the buttons come from the core (`models::search::status_line`), shared
    // with the terminal UI.
    let Some(st) = model::status_line(&app.core, id) else { return };
    let cy = row.center().y;
    let mut x = row.left() + 14.0;
    if st.done {
        let (g, c) = if st.cancelled { (ph::STOP_CIRCLE, p.warning) } else { (ph::CHECK_CIRCLE, p.success) };
        ui.painter().text(pos2(x, cy), Align2::CENTER_CENTER, g, icons::font_fill(14.0), c);
    } else {
        spinner(ui, pos2(x, cy), 6.0, p.accent);
    }
    x += 14.0;
    let hg = trunc_galley(
        ui.painter(),
        &st.head,
        crate::ui::views::flow::semibold(12.0),
        p.text_primary,
        row.width() * 0.45,
    );
    let hw = hg.size().x;
    ui.painter().galley(pos2(x, cy - hg.size().y / 2.0), hg, p.text_primary);
    x += hw + 12.0;
    // Right side controls.
    let mut act = None;
    let mut rx = row.right() - 8.0;
    if st.can_cancel {
        let w = pill_width(ui, true, tr!("common-cancel"));
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.cancel"),
            Some(ph::X),
            tr!("common-cancel"),
            false,
            tr!("views-search-cancel-tip"),
        )
        .clicked()
        {
            act = Some(model::StatusAct::Cancel);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if st.fuzzy {
        let w = pill_width(ui, true, tr!("views-search-relevance"));
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.rank"),
            Some(ph::SPARKLE),
            tr!("views-search-relevance"),
            st.ranked,
            tr!("views-search-relevance-tip"),
        )
        .clicked()
        {
            act = Some(model::StatusAct::Relevance);
        }
    }
    if st.smart.is_some() {
        let w = pill_width(ui, true, tr!("views-search-edit"));
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.smart.edit"),
            Some(ph::PENCIL_SIMPLE),
            tr!("views-search-edit"),
            false,
            tr!("views-search-edit-smart-tip"),
        )
        .clicked()
        {
            act = Some(model::StatusAct::EditSmart);
        }
    } else if st.can_make_smart {
        let w = pill_width(ui, true, tr!("views-search-smart-folder"));
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.smart.new"),
            Some(ph::FOLDER_SIMPLE_DOTTED),
            tr!("views-search-smart-folder"),
            false,
            tr!("views-search-smart-folder-tip"),
        )
        .clicked()
        {
            act = Some(model::StatusAct::MakeSmart);
        }
    }
    if st.can_refine {
        let w = pill_width(ui, true, tr!("views-search-refine"));
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.refine"),
            Some(ph::SLIDERS_HORIZONTAL),
            tr!("views-search-refine"),
            false,
            tr!("views-search-refine-tip"),
        )
        .clicked()
        {
            act = Some(model::StatusAct::Refine);
        }
    }
    let dg =
        trunc_galley(ui.painter(), &st.detail, FontId::proportional(11.5), p.text_tertiary, (rx - x - 8.0).max(0.0));
    ui.painter().galley(pos2(x, cy - dg.size().y / 2.0), dg, p.text_tertiary);
    if let Some(act) = act {
        model::status_act(&mut app.core, id, act);
    }
}

/// Sidebar section with saved searches (called from the sidebar).
pub fn sidebar_section(
    ui: &mut Ui,
    names: &[String],
    collapsed: &mut BTreeSet<String>,
    cmds: &mut Vec<Cmd>,
    n_config: usize,
) {
    if names.is_empty() {
        return;
    }
    let mut open = !collapsed.contains("Searches");
    // The collapsed-set key stays "Searches" (persisted); the header is translated.
    collapsible_header(ui, tr!("views-search-sidebar"), &mut open);
    if open {
        collapsed.remove("Searches");
    } else {
        collapsed.insert("Searches".into());
        return;
    }
    for (i, name) in names.iter().enumerate() {
        let r =
            NavRow { icon: ph::MAGNIFYING_GLASS, label: name, active: false, icon_color: None, usage: None }.show(ui);
        if r.clicked() {
            cmds.push(Cmd::RunSavedSearch(i));
        }
        crate::ui::native::popup::context_menu(&r, |ui| {
            ui.set_width(200.0);
            if menu_item(ui, Some(ph::MAGNIFYING_GLASS), tr!("views-search-run-saved"), None, true).clicked() {
                cmds.push(Cmd::RunSavedSearch(i));
                ui.close();
            }
            if menu_item(ui, Some(ph::X), tr!("views-search-remove"), None, i >= n_config)
                .on_hover_text(if i < n_config { tr!("views-search-defined-in-config") } else { "" })
                .clicked()
            {
                cmds.push(Cmd::RemoveSavedSearch(i));
                ui.close();
            }
        });
    }
}
