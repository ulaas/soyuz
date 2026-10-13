//! Previous versions: earlier copies of a file or folder from snapshots
//! (Windows Volume Shadow Copies via `soyuz_fs::snapshots`, no shell / VSS COM).
//!
//! [`VersionsPanel`] lists the versions (date, size, modified) with Open / Compare /
//! Restore / Copy to…; it is shared by the Properties dialog's "Previous versions" tab and
//! the "Restore previous version…" window. Enumeration runs on a worker thread. Restoring
//! is a normal `Copy` job onto the original location (the conflict prompt asks before
//! replacing; the replaced file goes through the job's undo journal), so it shows in the
//! activity panel and can be undone.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use soyuz_core::config::DragDefault;
use soyuz_core::{format_size, format_time};
use soyuz_fs::snapshots::PathVersion;

use super::{button, tool_window};
use crate::app::SoyuzApp;
use crate::models::versions::{
    Comparison, VersionsCfg, VersionsModel, denied_more_text, empty_text, intro_text, looking_text, snapshot_label,
    version_detail,
};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::trunc_galley;

pub use crate::models::versions::supported;

thread_local! {
    /// Settings the panel needs (refreshed every frame from the config by [`begin_frame`],
    /// so panels inside dialogs that have no app access still follow the config).
    static CFG: RefCell<VersionsCfg> = RefCell::new(VersionsCfg::default());
}

pub fn begin_frame(app: &SoyuzApp) {
    CFG.with_borrow_mut(|c| *c = VersionsCfg::from_config(&app.core.config));
}

/// Previous versions are on in the config and this platform has a snapshot provider
/// (`models::versions::supported`). Whether the volume of a given file has snapshots is
/// only known after the (blocking) scan, which the panel runs on a worker and explains
/// when it finds none.
pub fn enabled() -> bool {
    CFG.with_borrow(|c| c.enabled())
}

fn cfg() -> VersionsCfg {
    CFG.with_borrow(|c| c.clone())
}

pub use crate::models::tools::{VersionCmd, apply_version as apply};

/// The panel: the core's [`VersionsModel`] drawn with egui.
pub struct VersionsPanel {
    model: VersionsModel,
    /// Rows can be dragged out (copy by default: snapshots are read-only). Only in the
    /// tool window, which shares the main window; the Properties tab is its own OS window.
    drag: bool,
}

impl VersionsPanel {
    pub fn new(path: PathBuf, is_dir: bool, current: Option<(u64, Option<SystemTime>)>) -> Self {
        VersionsPanel { model: VersionsModel::new(path, is_dir, current, &cfg()), drag: false }
    }

    /// Draw the panel; returns a command to apply.
    pub fn show(&mut self, ui: &mut Ui) -> Option<VersionCmd> {
        self.model.poll();
        let p = palette(ui.ctx());
        let c = cfg();
        let mut out = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(intro_text()).font(FontId::proportional(11.5)).color(p.text_tertiary));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::ui::widgets::icon_button(
                    ui,
                    ph::ARROW_CLOCKWISE,
                    tr!("tools-versions-look-again"),
                    !self.model.looking(),
                )
                .clicked()
                {
                    self.model.refresh(&c);
                }
            });
        });
        ui.add_space(4.0);
        let report = match &self.model.report {
            None => {
                super::progress_bar(ui, ui.available_width(), None);
                ui.label(RichText::new(looking_text()).font(FontId::proportional(12.0)).color(p.text_secondary));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                return None;
            }
            Some(Err(e)) => {
                let e = e.clone();
                note(ui, ph::WARNING_CIRCLE, tr!("tools-versions-list-failed"), &e);
                return None;
            }
            Some(Ok(r)) => r.clone(),
        };
        if let Some(u) = &report.unsupported {
            note(ui, ph::INFO, tr!("tools-versions-unsupported"), u);
            return None;
        }
        if report.versions.is_empty() {
            let (title, body) = empty_text(&report, self.model.is_dir);
            note(ui, if report.denied > 0 { ph::SHIELD_WARNING } else { ph::CLOCK_COUNTER_CLOCKWISE }, &title, &body);
            // Elevation only helps on Windows; a root-only snapper folder needs snapper's
            // ALLOW_USERS instead (explained in the note).
            if report.denied > 0
                && cfg!(windows)
                && button(ui, Some(ph::SHIELD_CHECK), tr!("tools-versions-elevate"), 0, true)
                    .on_hover_text(tr!("tools-versions-elevate-tip"))
                    .clicked()
            {
                out = Some(VersionCmd::Elevate);
            }
            return out;
        }
        if report.denied > 0 {
            ui.label(RichText::new(denied_more_text(report.denied)).font(FontId::proportional(11.5)).color(p.warning));
        }
        // List.
        let row_h = 40.0;
        let list_h = (ui.available_height() - 120.0).clamp(80.0, 260.0);
        let mut clicked = None;
        ScrollArea::vertical()
            .id_salt(("versions", self.model.path.clone()))
            .max_height(list_h)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, report.versions.len(), |ui, range| {
                for i in range {
                    let v = &report.versions[i];
                    let sense = if self.drag { Sense::click_and_drag() } else { Sense::click() };
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), sense);
                    if self.drag {
                        super::drag_source(&resp, DragDefault::Copy, || vec![v.path.clone()]);
                    }
                    let sel = self.model.selected == Some(i);
                    if sel {
                        ui.painter().rect_filled(r, CornerRadius::same(6), p.accent.gamma_multiply(0.16));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface2);
                    }
                    let painter = ui.painter();
                    painter.text(
                        pos2(r.left() + 16.0, r.center().y),
                        Align2::CENTER_CENTER,
                        if v.is_dir { ph::FOLDER_SIMPLE } else { ph::CLOCK_COUNTER_CLOCKWISE },
                        icons::font(15.0),
                        if v.is_dir { p.glyph_folder } else { p.accent },
                    );
                    let when = snapshot_label(v, &c.date);
                    let g = trunc_galley(
                        painter,
                        &when,
                        crate::ui::views::flow::semibold(12.5),
                        p.text_primary,
                        r.width() - 50.0,
                    );
                    painter.galley(pos2(r.left() + 34.0, r.top() + 5.0), g, p.text_primary);
                    let sub = version_detail(v, &c.date, c.size);
                    let g = trunc_galley(painter, &sub, FontId::proportional(11.5), p.text_secondary, r.width() - 50.0);
                    painter.galley(pos2(r.left() + 34.0, r.top() + 22.0), g, p.text_secondary);
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                    if resp.double_clicked() {
                        out = Some(VersionCmd::Open { version: v.path.clone(), is_dir: v.is_dir, label: when.clone() });
                    }
                }
            });
        if let Some(i) = clicked {
            self.model.select(i);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let on = self.model.selection().is_some();
            if button(ui, Some(ph::ARROW_SQUARE_OUT), tr!("tools-open"), 0, on).clicked() {
                out = self.model.open_cmd(&c.date);
            }
            if !self.model.is_dir && button(ui, Some(ph::GIT_DIFF), tr!("tools-compare-start"), 0, on).clicked() {
                self.model.start_compare();
            }
            if button(ui, Some(ph::COPY), tr!("tools-versions-copy-to"), 0, on).clicked() {
                out = self.model.copy_to_cmd();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("tools-versions-restore"), 1, on)
                    .on_hover_text(tr!("tools-versions-restore-tip"))
                    .clicked()
                {
                    out = self.model.restore_cmd();
                }
            });
        });
        // Side-by-side comparison.
        if let Some(cmp) = &self.model.compare
            && let Some(v) = report.versions.get(cmp.index)
        {
            ui.add_space(8.0);
            self.compare_view(ui, v, cmp.result.as_ref(), &c);
        }
        out
    }

    fn compare_view(&self, ui: &mut Ui, v: &PathVersion, res: Option<&Comparison>, c: &VersionsCfg) {
        let p = palette(ui.ctx());
        let w = (ui.available_width() - 10.0) / 2.0;
        let cur = self.model.current;
        ui.horizontal(|ui| {
            for (title, size, modified, color) in [
                (tr!("tools-versions-current"), cur.map(|x| x.0), cur.and_then(|x| x.1), p.accent),
                (tr!("tools-versions-this"), Some(v.size), v.modified, p.warning),
            ] {
                let (r, _) = ui.allocate_exact_size(vec2(w, 62.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(8), p.bg_surface2);
                ui.painter().rect_filled(
                    Rect::from_min_size(r.min + vec2(0.0, 8.0), vec2(3.0, r.height() - 16.0)),
                    CornerRadius::same(1),
                    color,
                );
                let painter = ui.painter();
                painter.text(
                    r.min + vec2(12.0, 8.0),
                    Align2::LEFT_TOP,
                    title,
                    crate::ui::views::flow::semibold(12.0),
                    p.text_primary,
                );
                let s = match size {
                    Some(s) => format_size(s, c.size),
                    None => tr!("tools-versions-deleted").into(),
                };
                painter.text(
                    r.min + vec2(12.0, 26.0),
                    Align2::LEFT_TOP,
                    s,
                    FontId::proportional(11.5),
                    p.text_secondary,
                );
                let m = modified
                    .map(|t| tr!("tools-versions-modified", date = format_time(t, &c.date)))
                    .unwrap_or_default();
                painter.text(
                    r.min + vec2(12.0, 42.0),
                    Align2::LEFT_TOP,
                    m,
                    FontId::proportional(11.5),
                    p.text_secondary,
                );
            }
        });
        let col = match res {
            Some(Comparison { same_content: Some(true), .. }) => p.success,
            Some(Comparison { same_content: Some(false), .. }) => p.warning,
            _ => p.text_tertiary,
        };
        ui.label(RichText::new(Comparison::text(res)).font(FontId::proportional(11.5)).color(col));
    }
}

fn note(ui: &mut Ui, glyph: &str, title: &str, body: &str) {
    let p = palette(ui.ctx());
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(icons::icon(glyph, 26.0).color(p.text_tertiary));
        ui.vertical(|ui| {
            ui.label(RichText::new(title).font(crate::ui::views::flow::semibold(13.0)).color(p.text_primary));
            ui.add(
                egui::Label::new(RichText::new(body).font(FontId::proportional(12.0)).color(p.text_secondary)).wrap(),
            );
        });
    });
}

// ---------------------------------------------------------------------------------------
// Window ("Restore previous version…").

pub struct VersionsWindow {
    pub open: bool,
    title: String,
    panel: VersionsPanel,
}

impl VersionsWindow {
    pub fn new(path: PathBuf, is_dir: bool, current: Option<(u64, Option<SystemTime>)>) -> Self {
        let title = soyuz_core::entry::display_name(&path);
        let mut panel = VersionsPanel::new(path, is_dir, current);
        panel.drag = true;
        VersionsWindow { open: true, title, panel }
    }

    pub fn path(&self) -> &Path {
        &self.panel.model.path
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        let mut cmd = None;
        let keep = tool_window(
            ctx,
            "versions",
            tr!("tools-versions-title"),
            &self.title,
            ph::CLOCK_COUNTER_CLOCKWISE,
            vec2(620.0, 480.0),
            |ui| {
                cmd = self.panel.show(ui);
            },
        );
        if let Some(c) = cmd {
            apply(&mut app.core, c);
        }
        if !keep {
            self.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    // The moved labels, empty texts and compare are tested in `soyuz_appcore::models::versions`.

    #[test]
    fn turkish_texts_format() {
        use crate::i18n::{FluentArgs, t_args_in};
        use soyuz_core::Locale;
        let mut a = FluentArgs::new();
        a.set("count", 3);
        a.set("kind", "folder");
        let body = t_args_in("tools-versions-none-body", &a, Locale::TrTr);
        assert_eq!(body, "Bu birimdeki 3 gölge kopyanın hiçbiri bu klasörün farklı bir sürümünü içermiyor.");
        let mut a = FluentArgs::new();
        a.set("count", 2);
        assert_eq!(t_args_in("tools-sched-every-hours", &a, Locale::TrTr), "2 saatte bir");
        assert_eq!(t_args_in("tools-sched-every-hours", &a, Locale::EnUs), "Every 2 hours");
        let mut a = FluentArgs::new();
        a.set("count", 1234);
        a.set("size", "1,5 KiB");
        assert_eq!(t_args_in("tools-dupes-marked", &a, Locale::TrTr), "1.234 dosya işaretlendi · 1,5 KiB");
    }
}
