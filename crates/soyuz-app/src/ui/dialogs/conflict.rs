//! Job prompts as dialogs: the conflict dialog (side-by-side source vs destination with
//! "newer" / "larger" highlights, Replace / Skip / Keep both / Merge / Rename… / Replace if
//! newer or larger, apply-to-all) plus the other prompt kinds drawn by `error.rs`.
//!
//! The dialog never blocks anything: the job thread waits on its own channel and the UI
//! keeps running. Several jobs may wait at once; `ops::fileops` shows one prompt at a time
//! and passes the count. Esc / clicking outside defers the prompt (the job's activity card
//! offers "Resolve…").
//!
//! The state and the answers are the core's [`PromptModel`]; this file only draws it.

use std::time::SystemTime;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind,
    TextEdit, Ui, pos2, vec2,
};
use soyuz_core::{ConflictResolution, EntryInfo, SizeStyle, format_size};
use soyuz_jobs::{ConflictInfo, ConflictKind, JobPrompt};

use super::DialogCtx;
pub use crate::models::dialogs::PromptShared;
use crate::models::dialogs::{PromptAnswer as Answer, PromptModel};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons as ticons, icons::ph, palette};
use crate::ui::views::flow::semibold;
use crate::ui::views::icons;
use crate::ui::widgets::trunc_galley;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Btn {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// Dialog button in the design-system style.
pub(crate) fn button(ui: &mut Ui, label: &str, kind: Btn) -> egui::Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::WHITE);
    let size = vec2((galley.size().x + 26.0).max(72.0), 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    crate::ui::a11y::button(&resp, label);
    let hover = resp.hovered();
    let (bg, fg, stroke) = match kind {
        Btn::Primary => (if hover { p.accent_hover } else { p.accent }, Color32::WHITE, None),
        Btn::Danger => (if hover { p.danger } else { p.danger.gamma_multiply(0.88) }, Color32::WHITE, None),
        Btn::Secondary => (if hover { p.bg_surface3 } else { p.bg_surface2 }, p.text_primary, Some(p.border_subtle)),
        Btn::Ghost => (if hover { p.bg_surface2 } else { Color32::TRANSPARENT }, p.text_secondary, None),
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), bg);
    if let Some(s) = stroke {
        ui.painter().rect_stroke(rect, CornerRadius::same(radius::ROW), Stroke::new(1.0, s), StrokeKind::Inside);
    }
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), fg);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, fg);
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, rect);
    }
    resp
}

/// Small rounded tag ("Newer", "Larger").
pub(crate) fn tag(ui: &Ui, pos: egui::Pos2, text: &str, color: Color32) -> f32 {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(10.5), color);
    let r = Rect::from_min_size(pos2(pos.x, pos.y - 8.0), vec2(g.size().x + 12.0, 16.0));
    ui.painter().rect_filled(r, CornerRadius::same(8), color.gamma_multiply(0.16));
    ui.painter().galley(pos2(r.left() + 6.0, r.center().y - g.size().y / 2.0), g, color);
    r.width()
}

/// "1 of 3 waiting" badge in the top-right corner of the body.
fn count_badge(ui: &Ui, total: usize) {
    if total <= 1 {
        return;
    }
    let p = palette(ui.ctx());
    let text = tr!("dialogs-prompt-waiting", total = total);
    let g = ui.painter().layout_no_wrap(text, FontId::proportional(11.0), p.warning);
    let top_right = ui.max_rect().right_top();
    let r =
        Rect::from_min_size(pos2(top_right.x - g.size().x - 16.0, top_right.y - 34.0), vec2(g.size().x + 16.0, 20.0));
    ui.painter().rect_filled(r, CornerRadius::same(10), p.warning.gamma_multiply(0.15));
    ui.painter().galley(r.center() - g.size() / 2.0, g, p.warning);
}

/// Glyph of a job prompt.
pub(crate) fn icon(m: &PromptModel) -> &'static str {
    match &m.prompt {
        JobPrompt::Conflict(_) => ph::COPY,
        JobPrompt::Error { .. } => ph::WARNING_CIRCLE,
        JobPrompt::NotEnoughSpace { .. } => ph::HARD_DRIVE,
        JobPrompt::TrashUnsupported { .. } => ph::TRASH,
        JobPrompt::ReadOnlyItem { .. } => ph::LOCK_SIMPLE,
        JobPrompt::Password { .. } => ph::KEY,
    }
}

pub(crate) fn width(m: &PromptModel) -> f32 {
    match m.prompt {
        JobPrompt::Conflict(_) => 600.0,
        _ => 460.0,
    }
}

/// Draw a job prompt. Returns `false` to close.
pub(crate) fn show(ui: &mut Ui, m: &mut PromptModel, ctx: &mut DialogCtx<'_>) -> bool {
    if !m.poll() {
        return false;
    }
    let p = palette(ui.ctx());
    count_badge(ui, m.total());
    ui.label(RichText::new(&m.job_title).font(FontId::proportional(11.5)).color(p.text_tertiary));
    ui.add_space(8.0);
    let answer = match m.prompt.clone() {
        JobPrompt::Conflict(info) => conflict(ui, m, &info, &p),
        other => super::error::body(ui, &other, m, &p),
    };
    match answer {
        Some(a) => {
            ctx.cmds.push(m.answer(a));
            false
        }
        None => true,
    }
}

fn conflict(ui: &mut Ui, m: &mut PromptModel, c: &ConflictInfo, p: &Palette) -> Option<Answer> {
    let name = crate::ui::widgets::shown_name(&c.dst.name);
    let msg = match c.kind {
        ConflictKind::FileExists => tr!("dialogs-conflict-file-exists", name = &name),
        ConflictKind::DirExists => tr!("dialogs-conflict-dir-exists", name = &name),
        ConflictKind::FileVsDir => tr!("dialogs-conflict-kind-differs", name = &name),
        ConflictKind::SameFile => tr!("dialogs-conflict-same").to_string(),
        ConflictKind::MoveIntoSelf => {
            tr!("dialogs-conflict-into-self", name = crate::ui::widgets::shown_name(&c.src.name))
        }
        ConflictKind::CaseOnly => tr!("dialogs-conflict-case", name = &name),
    };
    ui.label(RichText::new(msg).color(p.text_secondary));
    crate::ui::widgets::hidden_note(ui, &c.dst.name);
    if c.src.name != c.dst.name {
        crate::ui::widgets::hidden_note(ui, &c.src.name);
    }
    ui.add_space(12.0);

    // Side-by-side cards.
    let w = ui.available_width();
    let card_w = (w - 12.0) / 2.0;
    let (row, _) = ui.allocate_exact_size(vec2(w, 132.0), Sense::hover());
    let left = Rect::from_min_size(row.min, vec2(card_w, row.height()));
    let right = Rect::from_min_size(pos2(row.left() + card_w + 12.0, row.top()), vec2(card_w, row.height()));
    let newer = cmp_time(c.src.modified, c.dst.modified);
    let larger = (!c.src.is_dir_like() && !c.dst.is_dir_like()).then(|| c.src.size.cmp(&c.dst.size));
    entry_card(
        ui,
        left,
        tr!("dialogs-conflict-source"),
        &c.src,
        newer == Some(std::cmp::Ordering::Greater),
        larger == Some(std::cmp::Ordering::Greater),
        p,
    );
    entry_card(
        ui,
        right,
        tr!("dialogs-conflict-destination"),
        &c.dst,
        newer == Some(std::cmp::Ordering::Less),
        larger == Some(std::cmp::Ordering::Less),
        p,
    );
    ui.add_space(10.0);

    let blocked = matches!(c.kind, ConflictKind::MoveIntoSelf);
    if m.rename.is_some() {
        ui.label(
            RichText::new(tr!("dialogs-conflict-new-name")).font(FontId::proportional(11.5)).color(p.text_secondary),
        );
        let focus = std::mem::take(&mut m.focus_rename);
        let text = m.rename.get_or_insert_with(String::new);
        let r = ui.add(TextEdit::singleline(text).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 6)));
        if focus {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && m.rename_differs() {
            return m.commit_rename();
        }
        ui.add_space(6.0);
    }
    if !blocked {
        ui.checkbox(&mut m.apply_all, tr!("dialogs-conflict-apply-all"));
    }
    ui.add_space(14.0);

    let renaming = m.rename.is_some();
    let mut pick: Option<ConflictResolution> = None;
    let (mut cancel_job, mut commit, mut back, mut start_rename) = (false, false, false, false);
    ui.horizontal(|ui| {
        if button(ui, tr!("dialogs-prompt-cancel-op"), Btn::Ghost).clicked() {
            cancel_job = true;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if renaming {
                if button(ui, tr!("common-rename"), Btn::Primary).clicked() {
                    commit = true;
                }
                if button(ui, tr!("dialogs-conflict-back"), Btn::Secondary).clicked() {
                    back = true;
                }
                return;
            }
            match c.kind {
                ConflictKind::MoveIntoSelf => {
                    if button(ui, tr!("common-skip"), Btn::Primary).clicked() {
                        pick = Some(ConflictResolution::Skip);
                    }
                }
                ConflictKind::SameFile => {
                    if button(ui, tr!("dialogs-conflict-keep-both"), Btn::Primary).clicked() {
                        pick = Some(ConflictResolution::KeepBoth);
                    }
                    if button(ui, tr!("common-skip"), Btn::Secondary).clicked() {
                        pick = Some(ConflictResolution::Skip);
                    }
                }
                _ => {
                    let dir = c.kind == ConflictKind::DirExists;
                    let (primary, res) = if dir {
                        (tr!("dialogs-conflict-merge"), ConflictResolution::MergeDir)
                    } else {
                        (tr!("dialogs-conflict-replace"), ConflictResolution::Overwrite)
                    };
                    if button(ui, primary, Btn::Primary).clicked() {
                        pick = Some(res);
                    }
                    if button(ui, tr!("dialogs-conflict-keep-both"), Btn::Secondary).clicked() {
                        pick = Some(ConflictResolution::KeepBoth);
                    }
                    if button(ui, tr!("common-skip"), Btn::Secondary).clicked() {
                        pick = Some(ConflictResolution::Skip);
                    }
                    let more = button(ui, tr!("dialogs-conflict-more"), Btn::Ghost);
                    crate::ui::native::popup::Menu::menu(&more).show(|ui| {
                        ui.set_min_width(220.0);
                        use crate::ui::widgets::menu_item;
                        if dir
                            && menu_item(ui, Some(ph::SWAP), tr!("dialogs-conflict-replace-folder"), None, true)
                                .clicked()
                        {
                            pick = Some(ConflictResolution::Overwrite);
                        }
                        if !dir {
                            if menu_item(
                                ui,
                                Some(ph::CLOCK_CLOCKWISE),
                                tr!("dialogs-conflict-replace-newer"),
                                None,
                                true,
                            )
                            .clicked()
                            {
                                pick = Some(ConflictResolution::OverwriteIfNewer);
                            }
                            if menu_item(
                                ui,
                                Some(ph::ARROWS_OUT_SIMPLE),
                                tr!("dialogs-conflict-replace-larger"),
                                None,
                                true,
                            )
                            .clicked()
                            {
                                pick = Some(ConflictResolution::OverwriteIfLarger);
                            }
                            if menu_item(ui, Some(ph::SCALES), tr!("dialogs-conflict-replace-different"), None, true)
                                .clicked()
                            {
                                pick = Some(ConflictResolution::OverwriteIfDifferent);
                            }
                        }
                        if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("dialogs-conflict-rename"), None, true).clicked()
                        {
                            start_rename = true;
                        }
                    });
                }
            }
        });
    });
    // Later widgets win, as when each click overwrote the answer in turn.
    let mut out = if cancel_job { Some(Answer::CancelJob) } else { None };
    if commit && let Some(a) = m.commit_rename() {
        out = Some(a);
    }
    if back {
        m.cancel_rename();
    }
    if let Some(r) = pick {
        out = Some(m.resolve(r));
    }
    if start_rename {
        m.start_rename();
    }
    // Keyboard: Enter = primary for the plain file case.
    if out.is_none() && ui.input(|i| i.key_pressed(Key::Enter)) {
        out = m.default_answer();
    }
    out
}

fn cmp_time(a: Option<SystemTime>, b: Option<SystemTime>) -> Option<std::cmp::Ordering> {
    let (a, b) = (a?, b?);
    // Within two seconds counts as equal (FAT timestamps, copies).
    let d = match a.duration_since(b) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    };
    Some(if d > 2.0 {
        std::cmp::Ordering::Greater
    } else if d < -2.0 {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Equal
    })
}

/// Thumbnail for small images (decoded by egui's image loaders), else the type glyph.
fn preview(ui: &mut Ui, r: Rect, e: &EntryInfo, p: &Palette) {
    ui.painter().rect_filled(r, CornerRadius::same(8), p.bg_sunken);
    let img = !e.is_dir_like()
        && e.size > 0
        && e.size < 12 * 1024 * 1024
        && crate::services::thumbs::is_thumbnailable(&e.path);
    if img {
        let uri = format!("file://{}", soyuz_fs::strip_verbatim(&e.path).display());
        egui::Image::new(uri).fit_to_exact_size(r.size() - vec2(6.0, 6.0)).corner_radius(6).paint_at(ui, r.shrink(3.0));
        return;
    }
    let font = if e.is_dir_like() { ticons::font_fill(30.0) } else { ticons::font(30.0) };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::glyph(e), font, icons::color(e, p));
}

fn entry_card(ui: &mut Ui, r: Rect, caption: &str, e: &EntryInfo, newer: bool, larger: bool, p: &Palette) {
    let highlight = newer || larger;
    ui.painter().rect_filled(r, CornerRadius::same(radius::CARD), p.bg_surface2);
    ui.painter().rect_stroke(
        r,
        CornerRadius::same(radius::CARD),
        Stroke::new(1.0, if highlight { p.accent.gamma_multiply(0.6) } else { p.border_subtle }),
        StrokeKind::Inside,
    );
    ui.painter().text(
        pos2(r.left() + 12.0, r.top() + 14.0),
        Align2::LEFT_CENTER,
        super::caption_upper(caption),
        FontId::proportional(10.0),
        p.text_tertiary,
    );
    let thumb = Rect::from_min_size(pos2(r.left() + 12.0, r.top() + 28.0), vec2(64.0, 64.0));
    preview(ui, thumb, e, p);
    let x = thumb.right() + 12.0;
    let tw = r.right() - x - 10.0;
    let badge = if soyuz_core::naming::has_hidden_controls(&e.name) { 14.0 } else { 0.0 };
    let g = trunc_galley(ui.painter(), &e.name, semibold(13.0), p.text_primary, tw - badge);
    let (gw, gh) = (g.size().x, g.size().y);
    ui.painter().galley(pos2(x, thumb.top() + 2.0), g, p.text_primary);
    crate::ui::widgets::paint_hidden_badge(ui.painter(), p, pos2(x + gw, thumb.top() + 2.0 + gh / 2.0), &e.name);
    let dir = e.path.parent().map(|d| soyuz_fs::strip_verbatim(d).to_string_lossy().into_owned()).unwrap_or_default();
    let g = trunc_galley(ui.painter(), &dir, FontId::proportional(11.0), p.text_tertiary, tw);
    ui.painter().galley(pos2(x, thumb.top() + 20.0), g, p.text_tertiary);
    let size = if e.is_dir_like() {
        tr!("dialogs-conflict-folder").to_string()
    } else {
        format_size(e.size, SizeStyle::Binary)
    };
    let sy = thumb.top() + 46.0;
    let sg = ui.painter().layout_no_wrap(size, FontId::proportional(12.0), p.text_secondary);
    let sw = sg.size().x;
    ui.painter().galley(pos2(x, sy - sg.size().y / 2.0), sg, p.text_secondary);
    if larger {
        tag(ui, pos2(x + sw + 8.0, sy), tr!("dialogs-conflict-larger"), p.accent);
    }
    let date = e
        .modified
        .map(|t| soyuz_core::format_time(t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Short)))
        .unwrap_or_else(|| "\u{2014}".into());
    let dy = thumb.top() + 64.0;
    let dg = ui.painter().layout_no_wrap(date, FontId::proportional(12.0), p.text_secondary);
    let dw = dg.size().x;
    ui.painter().galley(pos2(x, dy - dg.size().y / 2.0), dg, p.text_secondary);
    if newer {
        tag(ui, pos2(x + dw + 8.0, dy), tr!("dialogs-conflict-newer"), p.success);
    }
}
