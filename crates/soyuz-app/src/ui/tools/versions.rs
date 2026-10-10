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

use crossbeam_channel::{Receiver, bounded};
use egui::{Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use soyuz_core::config::DragDefault;
use soyuz_core::{DateFormat, SizeStyle, format_size, format_time};
use soyuz_fs::snapshots::{PathVersion, VersionOptions, VersionsReport};
use soyuz_jobs::JobKind;

use super::{button, tool_window};
use crate::app::SoyuzApp;
use crate::services::jobs::{JobMeta, OpKind};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::toasts::Toast;
use crate::ui::widgets::trunc_galley;

/// Settings the panel needs (refreshed every frame from the config by [`begin_frame`], so
/// panels inside dialogs that have no app access still follow the config).
#[derive(Clone, Debug)]
struct Cfg {
    enabled: bool,
    probe_max: u32,
    date: DateFormat,
    size: SizeStyle,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { enabled: true, probe_max: 64, date: DateFormat::default(), size: SizeStyle::Binary }
    }
}

thread_local! {
    static CFG: RefCell<Cfg> = RefCell::new(Cfg::default());
}

pub fn begin_frame(app: &SoyuzApp) {
    CFG.with_borrow_mut(|c| {
        c.enabled = app.config.sync.previous_versions;
        c.probe_max = app.config.sync.shadow_probe_max;
        c.date = app.config.general.date_format.clone();
        c.size = app.config.general.size_style;
    });
}

/// Previous versions are on in the config and this platform has a snapshot provider.
pub fn enabled() -> bool {
    cfg!(windows) && CFG.with_borrow(|c| c.enabled)
}

fn cfg() -> Cfg {
    CFG.with_borrow(|c| c.clone())
}

/// What the user asked for (applied with app access by `tools::apply`).
#[derive(Clone, Debug)]
pub enum VersionCmd {
    /// Copy the version back over the original (conflict prompt, undoable).
    Restore { version: PathBuf, original: PathBuf },
    /// Copy the version into a chosen folder.
    CopyTo { version: PathBuf, start: PathBuf },
    /// Open: a folder is browsed in the pane, a file opens from a read-only temp copy.
    Open { version: PathBuf, is_dir: bool, label: String },
    /// Start an elevated Soyuz (shadow copies need administrator rights).
    Elevate,
}

/// Side-by-side facts of the current file and one version (computed on a worker).
#[derive(Clone, Debug, Default)]
struct Comparison {
    same_content: Option<bool>,
    note: String,
}

pub struct VersionsPanel {
    path: PathBuf,
    is_dir: bool,
    rx: Option<Receiver<Result<VersionsReport, String>>>,
    report: Option<Result<VersionsReport, String>>,
    selected: Option<usize>,
    compare: Option<(usize, Option<Receiver<Comparison>>, Option<Comparison>)>,
    current: Option<(u64, Option<SystemTime>)>,
    /// Rows can be dragged out (copy by default: snapshots are read-only). Only in the
    /// tool window, which shares the main window; the Properties tab is its own OS window.
    drag: bool,
}

impl VersionsPanel {
    pub fn new(path: PathBuf, is_dir: bool, current: Option<(u64, Option<SystemTime>)>) -> Self {
        let mut p =
            VersionsPanel { path, is_dir, rx: None, report: None, selected: None, compare: None, current, drag: false };
        p.refresh();
        p
    }

    fn refresh(&mut self) {
        let c = cfg();
        if !c.enabled {
            self.report = Some(Err(tr!("tools-versions-off").into()));
            return;
        }
        let (tx, rx) = bounded(1);
        let path = self.path.clone();
        let wake = super::wake();
        let opts = VersionOptions { probe_max: c.probe_max };
        let spawned = std::thread::Builder::new().name("soyuz-versions".into()).spawn(move || {
            let r = soyuz_fs::snapshots::previous_versions(&path, &opts).map_err(|e| e.to_string());
            let _ = tx.send(r);
            wake();
        });
        match spawned {
            Ok(_) => {
                self.rx = Some(rx);
                self.report = None;
                self.selected = None;
                self.compare = None;
            }
            Err(e) => self.report = Some(Err(e.to_string())),
        }
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.rx
            && let Ok(r) = rx.try_recv()
        {
            self.report = Some(r);
            self.rx = None;
        }
        if let Some((_, rx @ Some(_), slot)) = &mut self.compare
            && let Some(Ok(c)) = rx.as_ref().map(|r| r.try_recv())
        {
            *slot = Some(c);
            *rx = None;
        }
    }

    /// Draw the panel; returns a command to apply.
    pub fn show(&mut self, ui: &mut Ui) -> Option<VersionCmd> {
        self.poll();
        let p = palette(ui.ctx());
        let c = cfg();
        let mut out = None;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr!("tools-versions-intro")).font(FontId::proportional(11.5)).color(p.text_tertiary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::ui::widgets::icon_button(
                    ui,
                    ph::ARROW_CLOCKWISE,
                    tr!("tools-versions-look-again"),
                    self.rx.is_none(),
                )
                .clicked()
                {
                    self.refresh();
                }
            });
        });
        ui.add_space(4.0);
        let report = match &self.report {
            None => {
                super::progress_bar(ui, ui.available_width(), None);
                ui.label(
                    RichText::new(tr!("tools-versions-looking"))
                        .font(FontId::proportional(12.0))
                        .color(p.text_secondary),
                );
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
            let (title, body) = empty_text(&report, self.is_dir);
            note(ui, if report.denied > 0 { ph::SHIELD_WARNING } else { ph::CLOCK_COUNTER_CLOCKWISE }, &title, &body);
            if report.denied > 0
                && button(ui, Some(ph::SHIELD_CHECK), tr!("tools-versions-elevate"), 0, true)
                    .on_hover_text(tr!("tools-versions-elevate-tip"))
                    .clicked()
            {
                out = Some(VersionCmd::Elevate);
            }
            return out;
        }
        if report.denied > 0 {
            ui.label(
                RichText::new(tr!("tools-versions-denied-more", count = report.denied))
                    .font(FontId::proportional(11.5))
                    .color(p.warning),
            );
        }
        // List.
        let row_h = 40.0;
        let list_h = (ui.available_height() - 120.0).clamp(80.0, 260.0);
        let mut clicked = None;
        ScrollArea::vertical()
            .id_salt(("versions", self.path.clone()))
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
                    let sel = self.selected == Some(i);
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
            self.selected = Some(i);
        }
        ui.add_space(6.0);
        let sel = self.selected.and_then(|i| report.versions.get(i).map(|v| (i, v.clone())));
        ui.horizontal(|ui| {
            let on = sel.is_some();
            if button(ui, Some(ph::ARROW_SQUARE_OUT), tr!("tools-open"), 0, on).clicked()
                && let Some((_, v)) = &sel
            {
                out = Some(VersionCmd::Open {
                    version: v.path.clone(),
                    is_dir: v.is_dir,
                    label: snapshot_label(v, &c.date),
                });
            }
            if !self.is_dir
                && button(ui, Some(ph::GIT_DIFF), tr!("tools-compare-start"), 0, on).clicked()
                && let Some((i, v)) = &sel
            {
                self.start_compare(*i, v);
            }
            if button(ui, Some(ph::COPY), tr!("tools-versions-copy-to"), 0, on).clicked()
                && let Some((_, v)) = &sel
            {
                let start = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
                out = Some(VersionCmd::CopyTo { version: v.path.clone(), start });
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("tools-versions-restore"), 1, on)
                    .on_hover_text(tr!("tools-versions-restore-tip"))
                    .clicked()
                    && let Some((_, v)) = &sel
                {
                    out = Some(VersionCmd::Restore { version: v.path.clone(), original: self.path.clone() });
                }
            });
        });
        // Side-by-side comparison.
        if let Some((i, _, res)) = &self.compare
            && let Some(v) = report.versions.get(*i)
        {
            ui.add_space(8.0);
            self.compare_view(ui, v, res.as_ref(), &c);
        }
        out
    }

    fn start_compare(&mut self, i: usize, v: &PathVersion) {
        let (tx, rx) = bounded(1);
        let (cur, old) = (self.path.clone(), v.path.clone());
        let wake = super::wake();
        let _ = std::thread::Builder::new().name("soyuz-version-cmp".into()).spawn(move || {
            let _ = tx.send(compare_files(&cur, &old));
            wake();
        });
        self.compare = Some((i, Some(rx), None));
    }

    fn compare_view(&self, ui: &mut Ui, v: &PathVersion, res: Option<&Comparison>, c: &Cfg) {
        let p = palette(ui.ctx());
        let w = (ui.available_width() - 10.0) / 2.0;
        let cur = self.current;
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
        let (txt, col) = match res {
            None => (tr!("tools-versions-comparing").to_string(), p.text_tertiary),
            Some(Comparison { same_content: Some(true), .. }) => (tr!("tools-versions-same").into(), p.success),
            Some(Comparison { same_content: Some(false), note }) => {
                (tr!("tools-versions-differ", note = note), p.warning)
            }
            Some(Comparison { note, .. }) => (note.clone(), p.text_tertiary),
        };
        ui.label(RichText::new(txt).font(FontId::proportional(11.5)).color(col));
    }
}

/// Title + body for an empty result, explaining why.
pub fn empty_text(r: &VersionsReport, is_dir: bool) -> (String, String) {
    let kind = if is_dir { "folder" } else { "file" };
    if r.denied > 0 {
        (
            tr!("tools-versions-admin-needed").into(),
            tr!("tools-versions-admin-needed-body", count = r.denied, kind = kind),
        )
    } else if r.snapshots_seen > 0 {
        (tr!("tools-versions-none").into(), tr!("tools-versions-none-body", count = r.snapshots_seen, kind = kind))
    } else {
        (tr!("tools-versions-no-copies").into(), tr!("tools-versions-no-copies-body").into())
    }
}

/// "Shadow copy · 8 Oct 2026 14:00" (or the snapshot number when its time is unknown).
pub fn snapshot_label(v: &PathVersion, date: &DateFormat) -> String {
    match v.snapshot.created {
        Some(t) => tr!("tools-versions-snapshot-time", provider = provider_name(v), date = format_time(t, date)),
        None => tr!("tools-versions-snapshot-seq", provider = provider_name(v), seq = v.snapshot.seq.to_string()),
    }
}

/// The snapshot provider's display name (soyuz-fs names it in English).
fn provider_name(v: &PathVersion) -> &'static str {
    match v.snapshot.provider {
        "Shadow copy" => tr!("tools-versions-shadow-copy"),
        other => other,
    }
}

fn version_detail(v: &PathVersion, date: &DateFormat, size: SizeStyle) -> String {
    let sz = format_size(v.size, size);
    match (v.modified.map(|t| format_time(t, date)), v.is_dir) {
        (Some(modified), true) => tr!("tools-versions-detail-folder", modified = modified),
        (Some(modified), false) => tr!("tools-versions-detail-file", size = sz, modified = modified),
        (None, true) => tr!("tools-compare-meta-folder-only").to_string(),
        (None, false) => sz,
    }
}

fn compare_files(cur: &Path, old: &Path) -> Comparison {
    const MAX: u64 = 512 * 1024 * 1024;
    let (Ok(a), Ok(b)) = (std::fs::metadata(cur), std::fs::metadata(old)) else {
        return Comparison { same_content: None, note: tr!("tools-versions-unreadable").into() };
    };
    if a.len() != b.len() {
        let d = a.len() as i64 - b.len() as i64;
        let note = tr!(
            "tools-versions-size-diff",
            size = format_size(d.unsigned_abs(), SizeStyle::Binary),
            dir = if d > 0 { "larger" } else { "smaller" }
        );
        return Comparison { same_content: Some(false), note };
    }
    if a.len() > MAX {
        return Comparison { same_content: None, note: tr!("tools-versions-too-large").into() };
    }
    let ctl = soyuz_core::OpControl::new();
    let h = |p: &Path| soyuz_fs::hash_file(p, soyuz_core::HashAlgo::Blake3, &ctl, &mut ()).ok();
    match (h(cur), h(old)) {
        (Some(x), Some(y)) => Comparison {
            same_content: Some(x == y),
            note: if x == y { String::new() } else { tr!("tools-versions-different-bytes").into() },
        },
        _ => Comparison { same_content: None, note: tr!("tools-versions-unreadable").into() },
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
        &self.panel.path
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
            apply(app, c);
        }
        if !keep {
            self.open = false;
        }
    }
}

/// Apply a [`VersionCmd`] (UI thread; disk work goes to jobs / workers).
pub fn apply(app: &mut SoyuzApp, cmd: VersionCmd) {
    match cmd {
        VersionCmd::Restore { version, original } => {
            let Some(dest) = original.parent().map(Path::to_path_buf) else { return };
            let name = soyuz_core::entry::display_name(&original);
            let kind =
                JobKind::Copy { sources: vec![version.clone()], dest_dir: dest.clone(), rename_to: Some(name.clone()) };
            let mut spec = app.fileops.jobs.spec(kind);
            // Ask before replacing the current file (the prompt offers keep both / replace).
            spec.options.conflict = None;
            spec.options.label = Some(tr!("tools-versions-job-label", name = &name));
            let meta = JobMeta::new(OpKind::Restore, std::slice::from_ref(&version), Some(dest));
            app.fileops.jobs.submit(spec, meta);
        }
        VersionCmd::CopyTo { version, start } => {
            app.ui.dialogs.push(crate::ui::dialogs::new_item::TransferDialog::new(vec![version], start, false));
        }
        VersionCmd::Elevate => {
            let postman = app.services.mailbox.postman();
            let _ = std::thread::Builder::new().name("soyuz-elevate".into()).spawn(move || {
                let res = std::env::current_exe()
                    .map_err(|e| e.to_string())
                    .and_then(|exe| soyuz_fs::launch::run_elevated(&exe, &[]).map_err(|e| e.to_string()));
                if let Err(e) = res {
                    postman.send(crate::services::mailbox::UiMsg::Notice(
                        Toast::error(tr!("tools-versions-elevate-failed")).body(e),
                    ));
                }
            });
        }
        VersionCmd::Open { version, is_dir: true, .. } => {
            let id = app.ws.pane().id;
            crate::ops::navigate(app, id, soyuz_core::Location::Dir(version), crate::ops::NavOpts::user());
        }
        VersionCmd::Open { version, is_dir: false, label } => {
            let postman = app.services.mailbox.postman();
            let spawned = std::thread::Builder::new().name("soyuz-version-open".into()).spawn(move || {
                if let Err(e) = open_temp_copy(&version, &label) {
                    postman.send(crate::services::mailbox::UiMsg::Notice(
                        Toast::error(tr!("tools-versions-open-failed")).body(e),
                    ));
                }
            });
            if let Err(e) = spawned {
                app.ui.toasts.push(Toast::error(tr!("tools-versions-open-failed")).body(e.to_string()));
            }
        }
    }
}

/// Copy a snapshot file to a read-only temp file and open it with its default app
/// (apps often can't open `\\?\GLOBALROOT` paths themselves). Worker thread.
fn open_temp_copy(version: &Path, label: &str) -> Result<(), String> {
    let stamp: String = label.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
    let dir = std::env::temp_dir().join("soyuz-versions").join(if stamp.is_empty() { "v".into() } else { stamp });
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = version.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "version".into());
    let dst = dir.join(name);
    if dst.exists() {
        let mut perm = std::fs::metadata(&dst).map_err(|e| e.to_string())?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = std::fs::set_permissions(&dst, perm);
    }
    std::fs::copy(version, &dst).map_err(|e| e.to_string())?;
    if let Ok(m) = std::fs::metadata(&dst) {
        let mut perm = m.permissions();
        perm.set_readonly(true);
        let _ = std::fs::set_permissions(&dst, perm);
    }
    soyuz_fs::launch::open_default(&dst).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_fs::snapshots::{Snapshot, shadow_root};
    use std::time::Duration;

    #[test]
    fn empty_results_explain_why() {
        let none = VersionsReport::default();
        assert_eq!(empty_text(&none, false).0, "No shadow copies on this volume");
        let denied = VersionsReport { denied: 3, ..VersionsReport::default() };
        let (t, b) = empty_text(&denied, false);
        assert_eq!(t, "Administrator rights needed");
        assert!(b.contains("3 shadow copies") && b.contains("administrator"), "{b}");
        let seen = VersionsReport { snapshots_seen: 1, ..VersionsReport::default() };
        assert!(
            empty_text(&seen, true).1.contains("1 shadow copy of this volume holds a different version of this folder")
        );
    }

    #[test]
    fn labels_use_time_or_number() {
        let snap = Snapshot {
            provider: "Shadow copy",
            id: "HarddiskVolumeShadowCopy7".into(),
            root: shadow_root("HarddiskVolumeShadowCopy7"),
            volume: PathBuf::from(r"C:\"),
            created: None,
            seq: 7,
        };
        let v = PathVersion { snapshot: snap.clone(), path: "x".into(), size: 2048, modified: None, is_dir: false };
        assert_eq!(snapshot_label(&v, &DateFormat::default()), "Shadow copy #7");
        let v2 = PathVersion {
            snapshot: Snapshot { created: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)), ..snap },
            ..v
        };
        assert!(snapshot_label(&v2, &DateFormat::default()).starts_with("Shadow copy  ·  "));
        assert!(
            version_detail(&v2, &DateFormat::default(), SizeStyle::Binary).starts_with("2.0 KiB")
                || version_detail(&v2, &DateFormat::default(), SizeStyle::Binary).contains("KiB")
        );
    }

    #[test]
    fn compare_files_reports_same_and_different() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (d.path().join("a"), d.path().join("b"), d.path().join("c"));
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"hello").unwrap();
        std::fs::write(&c, b"hellO").unwrap();
        assert_eq!(compare_files(&a, &b).same_content, Some(true));
        assert_eq!(compare_files(&a, &c).same_content, Some(false));
        std::fs::write(&c, b"hello world").unwrap();
        let r = compare_files(&c, &a);
        assert_eq!(r.same_content, Some(false));
        assert!(r.note.contains("larger"), "{}", r.note);
    }

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
