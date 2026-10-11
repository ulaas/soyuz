//! Job prompts as dialogs: the conflict dialog (side-by-side source vs destination with
//! "newer" / "larger" highlights, Replace / Skip / Keep both / Merge / Rename… / Replace if
//! newer or larger, apply-to-all) plus the other prompt kinds drawn by `error.rs`.
//!
//! The dialog never blocks anything: the job thread waits on its own channel and the UI
//! keeps running. Several jobs may wait at once; `ops::fileops` shows one prompt at a time
//! and passes the count. Esc / clicking outside defers the prompt (the job's activity card
//! offers "Resolve…").

use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind,
    TextEdit, Ui, pos2, vec2,
};
use soyuz_core::{ConflictResolution, EntryInfo, SizeStyle, format_size};
use soyuz_jobs::{ConflictInfo, ConflictKind, JobId, JobPrompt, PromptResponse};

use super::{Dialog, DialogCtx};
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons as ticons, icons::ph, palette};
use crate::ui::views::flow::semibold;
use crate::ui::views::icons;
use crate::ui::widgets::trunc_galley;

/// State shared between the dialog and `ops::fileops` (which owns the prompt queue).
#[derive(Clone, Debug, Default)]
pub struct PromptShared {
    /// Still on the dialog stack.
    pub alive: bool,
    /// An answer (or cancel) was sent.
    pub answered: bool,
    /// The job no longer waits (cancelled / answered elsewhere): close immediately.
    pub stale: bool,
    /// Prompts waiting to be shown, this one included.
    pub total: usize,
}

/// What a prompt body decided.
pub(crate) enum Answer {
    Respond(PromptResponse),
    CancelJob,
}

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

pub struct PromptDialog {
    job: JobId,
    prompt: JobPrompt,
    job_title: String,
    title: String,
    shared: Rc<RefCell<PromptShared>>,
    pub(crate) apply_all: bool,
    rename: Option<String>,
    focus_rename: bool,
    pub(crate) password: String,
    pub(crate) focus_password: bool,
    /// Validate rename-on-conflict names with the Windows rules.
    windows_rules: bool,
}

impl PromptDialog {
    pub fn new(job: JobId, prompt: JobPrompt, job_title: String, shared: Rc<RefCell<PromptShared>>) -> Self {
        let title = match &prompt {
            JobPrompt::Conflict(c) => match c.kind {
                ConflictKind::FileExists => tr!("dialogs-conflict-title-file"),
                ConflictKind::DirExists => tr!("dialogs-conflict-title-dir"),
                ConflictKind::FileVsDir => tr!("dialogs-conflict-title-kind"),
                ConflictKind::SameFile => tr!("dialogs-conflict-title-same"),
                ConflictKind::MoveIntoSelf => tr!("dialogs-conflict-title-into-self"),
                ConflictKind::CaseOnly => tr!("dialogs-conflict-title-case"),
            },
            JobPrompt::Error { error, .. } if error.op == soyuz_jobs::kinds::extract::ARCHIVE_LIMIT_OP => {
                tr!("dialogs-prompt-title-archive-limit")
            }
            JobPrompt::Error { .. } => tr!("dialogs-prompt-title-error"),
            JobPrompt::NotEnoughSpace { .. } => tr!("dialogs-prompt-title-space"),
            JobPrompt::TrashUnsupported { .. } => tr!("dialogs-prompt-title-trash"),
            JobPrompt::ReadOnlyItem { .. } => tr!("dialogs-prompt-title-read-only"),
            JobPrompt::Password { .. } => tr!("dialogs-prompt-title-password"),
        }
        .to_string();
        PromptDialog {
            job,
            prompt,
            job_title,
            title,
            shared,
            apply_all: false,
            rename: None,
            focus_rename: false,
            password: String::new(),
            focus_password: true,
            windows_rules: cfg!(windows),
        }
    }

    /// Validate rename-on-conflict names with the Windows rules (target on a
    /// Windows-family volume, or strict names).
    pub fn windows_rules(mut self, on: bool) -> Self {
        self.windows_rules = on;
        self
    }
}

impl Drop for PromptDialog {
    fn drop(&mut self) {
        self.shared.borrow_mut().alive = false;
    }
}

impl Dialog for PromptDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn alert(&self) -> bool {
        true
    }

    fn icon(&self) -> Option<&'static str> {
        Some(match &self.prompt {
            JobPrompt::Conflict(_) => ph::COPY,
            JobPrompt::Error { .. } => ph::WARNING_CIRCLE,
            JobPrompt::NotEnoughSpace { .. } => ph::HARD_DRIVE,
            JobPrompt::TrashUnsupported { .. } => ph::TRASH,
            JobPrompt::ReadOnlyItem { .. } => ph::LOCK_SIMPLE,
            JobPrompt::Password { .. } => ph::KEY,
        })
    }

    fn width(&self) -> f32 {
        match self.prompt {
            JobPrompt::Conflict(_) => 600.0,
            _ => 460.0,
        }
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let (stale, total) = {
            let s = self.shared.borrow();
            (s.stale, s.total)
        };
        if stale {
            self.shared.borrow_mut().answered = true;
            return false;
        }
        let p = palette(ui.ctx());
        count_badge(ui, total);
        ui.label(RichText::new(&self.job_title).font(FontId::proportional(11.5)).color(p.text_tertiary));
        ui.add_space(8.0);
        let answer = match self.prompt.clone() {
            JobPrompt::Conflict(info) => self.conflict(ui, &info, &p),
            other => super::error::body(ui, &other, self, &p),
        };
        match answer {
            Some(Answer::Respond(r)) => {
                ctx.cmds.push(Cmd::FileOp(FileOpCmd::Respond { job: self.job, response: r }));
                self.shared.borrow_mut().answered = true;
                false
            }
            Some(Answer::CancelJob) => {
                ctx.cmds.push(Cmd::FileOp(FileOpCmd::CancelJob(self.job)));
                self.shared.borrow_mut().answered = true;
                false
            }
            None => true,
        }
    }
}

impl PromptDialog {
    fn conflict(&mut self, ui: &mut Ui, c: &ConflictInfo, p: &Palette) -> Option<Answer> {
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
        if let Some(text) = self.rename.as_mut() {
            ui.label(
                RichText::new(tr!("dialogs-conflict-new-name"))
                    .font(FontId::proportional(11.5))
                    .color(p.text_secondary),
            );
            let r = ui.add(TextEdit::singleline(text).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 6)));
            if self.focus_rename {
                r.request_focus();
                self.focus_rename = false;
            }
            let valid =
                soyuz_core::validate_name_with(text.trim(), self.windows_rules).is_ok() && text.trim() != c.dst.name;
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && valid {
                return Some(resolve(ConflictResolution::Rename(text.trim().to_string()), false));
            }
            ui.add_space(6.0);
        }
        if !blocked {
            ui.checkbox(&mut self.apply_all, tr!("dialogs-conflict-apply-all"));
        }
        ui.add_space(14.0);

        let mut out = None;
        ui.horizontal(|ui| {
            if button(ui, tr!("dialogs-prompt-cancel-op"), Btn::Ghost).clicked() {
                out = Some(Answer::CancelJob);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let all = self.apply_all;
                if let Some(text) = self.rename.clone() {
                    let valid = soyuz_core::validate_name_with(text.trim(), self.windows_rules).is_ok();
                    if button(ui, tr!("common-rename"), Btn::Primary).clicked() && valid {
                        out = Some(resolve(ConflictResolution::Rename(text.trim().to_string()), false));
                    }
                    if button(ui, tr!("dialogs-conflict-back"), Btn::Secondary).clicked() {
                        self.rename = None;
                    }
                    return;
                }
                match c.kind {
                    ConflictKind::MoveIntoSelf => {
                        if button(ui, tr!("common-skip"), Btn::Primary).clicked() {
                            out = Some(resolve(ConflictResolution::Skip, false));
                        }
                    }
                    ConflictKind::SameFile => {
                        if button(ui, tr!("dialogs-conflict-keep-both"), Btn::Primary).clicked() {
                            out = Some(resolve(ConflictResolution::KeepBoth, all));
                        }
                        if button(ui, tr!("common-skip"), Btn::Secondary).clicked() {
                            out = Some(resolve(ConflictResolution::Skip, all));
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
                            out = Some(resolve(res, all));
                        }
                        if button(ui, tr!("dialogs-conflict-keep-both"), Btn::Secondary).clicked() {
                            out = Some(resolve(ConflictResolution::KeepBoth, all));
                        }
                        if button(ui, tr!("common-skip"), Btn::Secondary).clicked() {
                            out = Some(resolve(ConflictResolution::Skip, all));
                        }
                        let more = button(ui, tr!("dialogs-conflict-more"), Btn::Ghost);
                        crate::ui::native::popup::Menu::menu(&more).show(|ui| {
                            ui.set_min_width(220.0);
                            use crate::ui::widgets::menu_item;
                            if dir
                                && menu_item(ui, Some(ph::SWAP), tr!("dialogs-conflict-replace-folder"), None, true)
                                    .clicked()
                            {
                                out = Some(resolve(ConflictResolution::Overwrite, all));
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
                                    out = Some(resolve(ConflictResolution::OverwriteIfNewer, all));
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
                                    out = Some(resolve(ConflictResolution::OverwriteIfLarger, all));
                                }
                                if menu_item(
                                    ui,
                                    Some(ph::SCALES),
                                    tr!("dialogs-conflict-replace-different"),
                                    None,
                                    true,
                                )
                                .clicked()
                                {
                                    out = Some(resolve(ConflictResolution::OverwriteIfDifferent, all));
                                }
                            }
                            if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("dialogs-conflict-rename"), None, true)
                                .clicked()
                            {
                                self.rename = Some(c.suggested_name.clone());
                                self.focus_rename = true;
                            }
                        });
                    }
                }
            });
        });
        // Keyboard: Enter = primary for the plain file case.
        if out.is_none()
            && self.rename.is_none()
            && c.kind == ConflictKind::FileExists
            && ui.input(|i| i.key_pressed(Key::Enter))
        {
            out = Some(resolve(ConflictResolution::Overwrite, self.apply_all));
        }
        out
    }
}

fn resolve(resolution: ConflictResolution, apply_to_all: bool) -> Answer {
    Answer::Respond(PromptResponse::Conflict { resolution, apply_to_all })
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
