//! Archive integration in the UI (PLAN M7): Enter on a supported archive browses it as
//! `Location::Archive`; entries inside open (folders navigate, files are extracted to a
//! temp folder and launched); extract here / to "name/" / to… / selected entries and
//! "Compress…" submit soyuz-jobs Extract / Compress jobs via `Cmd::SubmitJob`; a
//! password prompt appears when a listing needs one.

use std::path::{Path, PathBuf};

use egui::{Align2, CornerRadius, FontId, Key, Margin, RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2};
use soyuz_archive::ArchiveFormat;
use soyuz_core::{Location, split_vpath};
use soyuz_jobs::{JobKind, JobOptions, JobSpec};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::{self, NavOpts};
use crate::services::archive as svc;
use crate::state::pane::PaneId;
use crate::theme::icons::ph;
use crate::theme::palette;
use crate::ui::dialogs::{Dialog, DialogCtx, footer};
use crate::ui::toasts::Toast;
use crate::ui::widgets::{menu_item, menu_separator, submenu};

/// Enter / double-click handling for archives. Returns `true` when handled.
pub fn open_entry(app: &mut SoyuzApp, pane: PaneId, path: &Path, is_dir: bool) -> bool {
    let in_archive = app.ws.find_pane(pane).is_some_and(|p| matches!(p.location, Location::Archive { .. }));
    if in_archive && let Some((archive, inner)) = split_vpath(path) {
        if is_dir {
            ops::navigate(app, pane, Location::Archive { archive, inner }, NavOpts::user());
        } else {
            svc::open_in_app(
                &archive,
                &inner,
                crate::services::jobs::extract_limits(&app.config),
                app.services.mailbox.postman(),
            );
            app.ui.toasts.push(Toast::info(tr!("views-archive-opening", name = soyuz_core::entry::display_name(path))));
        }
        return true;
    }
    if !is_dir && svc::is_browsable(path) {
        ops::navigate(
            app,
            pane,
            Location::Archive { archive: path.to_path_buf(), inner: String::new() },
            NavOpts::user(),
        );
        return true;
    }
    false
}

/// Breadcrumb-style mapping: the location an archive child path opens (`None` for
/// paths outside an archive).
pub fn child_location(path: &Path, is_dir: bool) -> Option<Location> {
    let (archive, inner) = split_vpath(path)?;
    if is_dir {
        Some(Location::Archive { archive, inner })
    } else {
        let parent = inner.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        Some(Location::Archive { archive, inner: parent })
    }
}

/// Archive name without its (possibly double) extension: `a.tar.gz` → `a`.
pub fn archive_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if let Some(f) = svc::format_by_name(name) {
        for ext in f.extensions() {
            if lower.ends_with(&format!(".{ext}")) {
                return name[..name.len() - ext.len() - 1].to_string();
            }
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// A destination that doesn't exist yet: `base`, `base (2)`, `base (3)`, …
pub fn unique_dir(parent: &Path, base: &str) -> PathBuf {
    let first = parent.join(base);
    if !first.exists() {
        return first;
    }
    (2..1000).map(|i| parent.join(format!("{base} ({i})"))).find(|p| !p.exists()).unwrap_or(first)
}

fn extract_spec(archive: &Path, entries: Option<Vec<usize>>, dest: PathBuf, strip: usize) -> JobSpec {
    let label = tr!("views-archive-job-extract", name = soyuz_core::entry::display_name(archive));
    JobSpec {
        kind: JobKind::Extract {
            archive: archive.to_path_buf(),
            entries,
            dest_dir: dest,
            password: svc::password_for(archive),
            strip_components: strip,
        },
        options: JobOptions { label: Some(label), ..Default::default() },
    }
}

/// Archive files among the focused pane's selection (plain folders only).
fn selected_archives(app: &SoyuzApp) -> Vec<PathBuf> {
    let pane = app.ws.pane();
    if !matches!(pane.location, Location::Dir(_) | Location::Search(_)) {
        return Vec::new();
    }
    pane.selected_entries()
        .iter()
        .filter(|e| !e.is_dir_like() && svc::is_browsable(&e.path))
        .map(|e| e.path.clone())
        .collect()
}

/// The archive being browsed in the focused pane: `(archive, inner)`.
fn browsed(app: &SoyuzApp) -> Option<(PathBuf, String)> {
    match &app.ws.pane().location {
        Location::Archive { archive, inner } => Some((archive.clone(), inner.clone())),
        _ => None,
    }
}

pub fn extract_here(app: &mut SoyuzApp) {
    if let Some((archive, _)) = browsed(app) {
        let dest = archive.parent().map(Path::to_path_buf).unwrap_or_default();
        extract_selected_to(app, dest);
        return;
    }
    for a in selected_archives(app) {
        let dest = a.parent().map(Path::to_path_buf).unwrap_or_default();
        app.cmds.push(Cmd::SubmitJob(Box::new(extract_spec(&a, None, dest, 0))));
    }
}

pub fn extract_to_folder(app: &mut SoyuzApp) {
    let archives = match browsed(app) {
        Some((a, _)) => vec![a],
        None => selected_archives(app),
    };
    for a in archives {
        let parent = a.parent().map(Path::to_path_buf).unwrap_or_default();
        let dest = unique_dir(&parent, &archive_stem(&soyuz_core::entry::display_name(&a)));
        app.cmds.push(Cmd::SubmitJob(Box::new(extract_spec(&a, None, dest, 0))));
    }
}

/// "Extract to…" dialog (in an archive: the selected entries, else the selected archives).
pub fn extract_to_dialog(app: &mut SoyuzApp) {
    let other = other_pane_dir(app);
    if let Some((archive, _)) = browsed(app) {
        let dest = other.or_else(|| archive.parent().map(Path::to_path_buf)).unwrap_or_default();
        let selection = selected_inner(app);
        app.ui.dialogs.push(ExtractDialog::new(vec![archive], selection, dest));
        return;
    }
    let archives = selected_archives(app);
    if archives.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("views-archive-select-to-extract")));
        return;
    }
    let dest = other.or_else(|| archives[0].parent().map(Path::to_path_buf)).unwrap_or_default();
    app.ui.dialogs.push(ExtractDialog::new(archives, None, dest));
}

fn other_pane_dir(app: &SoyuzApp) -> Option<PathBuf> {
    let tab = app.ws.tab();
    if !tab.dual {
        return None;
    }
    tab.panes.get(1 - tab.focused.min(1)).and_then(|p| p.location.as_fs_path().map(Path::to_path_buf))
}

/// Selected entries of the browsed archive as `(inner paths, strip components)`.
fn selected_inner(app: &SoyuzApp) -> Option<(Vec<(String, bool)>, usize)> {
    let (_, inner) = browsed(app)?;
    let pane = app.ws.pane();
    let sel: Vec<(String, bool)> = pane
        .selected_entries()
        .iter()
        .filter_map(|e| split_vpath(&e.path).map(|(_, i)| (i, e.is_dir_like())))
        .collect();
    let strip = if inner.is_empty() { 0 } else { inner.split('/').count() };
    (!sel.is_empty() && !pane.selection.is_empty()).then_some((sel, strip))
}

fn extract_selected_to(app: &mut SoyuzApp, dest: PathBuf) {
    let Some((archive, _)) = browsed(app) else { return };
    match selected_inner(app) {
        Some((items, strip)) => submit_entries(app, archive, items, strip, dest),
        None => app.cmds.push(Cmd::SubmitJob(Box::new(extract_spec(&archive, None, dest, 0)))),
    }
}

/// Resolve inner paths (folders expand to their contents) to entry indices on a worker,
/// then submit the job.
fn submit_entries(app: &mut SoyuzApp, archive: PathBuf, items: Vec<(String, bool)>, strip: usize, dest: PathBuf) {
    let postman = app.services.mailbox.postman();
    let _ = std::thread::Builder::new().name("soyuz-archive-select".into()).spawn(move || {
        let pw = svc::password_for(&archive);
        let result = soyuz_archive::open(&archive, pw.as_deref()).and_then(|mut r| {
            let entries = r.entries()?;
            Ok(indices_for(entries.iter().map(|e| e.path.as_str()), &items))
        });
        match result {
            Ok(idx) if !idx.is_empty() => {
                svc::submit_job_detached(extract_spec(&archive, Some(idx), dest, strip));
            }
            Ok(_) => {
                postman.send(crate::services::mailbox::UiMsg::Notice(Toast::info(tr!(
                    "views-archive-nothing-to-extract"
                ))));
            }
            Err(e) => {
                postman.send(crate::services::mailbox::UiMsg::Notice(
                    Toast::error(tr!("views-archive-read-failed")).body(e.to_string()),
                ));
            }
        }
    });
}

/// Entry indices matching the selected inner paths (folders include everything below).
pub fn indices_for<'a>(paths: impl Iterator<Item = &'a str>, items: &[(String, bool)]) -> Vec<usize> {
    let norm: Vec<(String, bool)> = items.iter().map(|(p, d)| (soyuz_core::location::normalize_inner(p), *d)).collect();
    paths
        .enumerate()
        .filter(|(_, p)| {
            let p = soyuz_core::location::normalize_inner(p);
            norm.iter().any(|(sel, dir)| p == *sel || (*dir && p.starts_with(&format!("{sel}/"))))
        })
        .map(|(i, _)| i)
        .collect()
}

/// "Compress…" dialog for the selection (or the current folder's items).
pub fn create_dialog(app: &mut SoyuzApp) {
    let pane = app.ws.pane();
    let Location::Dir(dir) = &pane.location else {
        app.ui.toasts.push(Toast::info(tr!("views-archive-compress-needs-folder")));
        return;
    };
    let selected = pane.selected_entries();
    let sources: Vec<PathBuf> = selected.iter().map(|e| e.path.clone()).collect();
    if sources.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("views-archive-select-to-compress")));
        return;
    }
    let base = if sources.len() == 1 {
        let n = soyuz_core::entry::display_name(&sources[0]);
        // The listing knows the kind; no stat on the UI thread.
        if selected[0].is_dir_like() { n } else { archive_stem(&n) }
    } else {
        soyuz_core::entry::display_name(dir)
    };
    app.ui.dialogs.push(CreateDialog::new(sources, dir.clone(), base));
}

/// Check an archive's integrity on a worker; reports with a toast.
pub fn test_archive(app: &mut SoyuzApp) {
    let targets = match browsed(app) {
        Some((a, _)) => vec![a],
        None => selected_archives(app),
    };
    for a in targets {
        let postman = app.services.mailbox.postman();
        let _ = std::thread::Builder::new().name("soyuz-archive-test".into()).spawn(move || {
            let name = soyuz_core::entry::display_name(&a);
            let pw = svc::password_for(&a);
            let ctl = soyuz_core::OpControl::new();
            let t = soyuz_archive::open(&a, pw.as_deref()).and_then(|mut r| r.test(&ctl, &mut ()));
            let toast = match t {
                Ok(rep) if rep.bad.is_empty() => Toast::success(tr!("views-archive-test-ok", name = &name))
                    .body(tr!("views-archive-test-ok-body", count = rep.entries)),
                Ok(rep) => Toast::error(tr!("views-archive-test-damaged", name = &name, count = rep.bad.len()))
                    .body(rep.bad.iter().take(5).map(|(p, e)| format!("{p}: {e}")).collect::<Vec<_>>().join("\n")),
                Err(e) => Toast::error(tr!("views-archive-test-failed", name = &name)).body(e.to_string()),
            };
            postman.send(crate::services::mailbox::UiMsg::Notice(toast));
        });
    }
}

/// Per frame: password prompts and job completion toasts.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    svc::remember_ctx(ctx);
    pump_edits(app);
    if let Some((pane, archive, wrong)) = svc::take_password_request() {
        app.ui.dialogs.push(PasswordDialog::new(pane, archive, wrong));
    }
    for spec in svc::take_pending_jobs() {
        submit_job(app, spec);
    }
}

/// Submit an extract / compress job to the app-wide jobs service (activity panel,
/// prompts and completion toasts come from there).
pub fn submit_job(app: &mut SoyuzApp, spec: soyuz_jobs::JobSpec) -> soyuz_jobs::JobId {
    use crate::services::jobs::{JobMeta, OpKind};
    let roots = spec.kind.touched_roots();
    let meta = JobMeta::new(OpKind::Other, &roots, None);
    let edited = match &spec.kind {
        JobKind::EditArchive { archive, .. } => Some(archive.clone()),
        _ => None,
    };
    let id = app.fileops.jobs.submit(spec, meta);
    if let Some(a) = edited {
        track_edit_job(id, a);
    }
    id
}

/// Items-menu rows for archives (extract / compress / browse).
pub fn context_items(
    ui: &mut Ui,
    cmds: &mut Vec<Cmd>,
    pane: PaneId,
    location: &Location,
    any_archive: bool,
    has_selection: bool,
) {
    let act = |ui: &mut Ui, cmds: &mut Vec<Cmd>, icon: &str, label: &str, a: soyuz_core::config::Action| {
        if menu_item(ui, Some(icon), label, None, true).clicked() {
            cmds.push(Cmd::PaneAction(pane, a));
            ui.close();
        }
    };
    use soyuz_core::config::Action as A;
    match location {
        Location::Archive { .. } => {
            menu_separator(ui);
            act(
                ui,
                cmds,
                ph::TRAY_ARROW_DOWN,
                if has_selection { tr!("views-archive-extract-selected") } else { tr!("views-archive-extract-all") },
                A::ExtractTo,
            );
            act(ui, cmds, ph::ARROW_LINE_DOWN, tr!("views-archive-extract-here"), A::ExtractHere);
            edit_context_items(ui, cmds, pane, location, has_selection);
        }
        _ => {
            if any_archive {
                menu_separator(ui);
                submenu(ui, Some(ph::FILE_ZIP), tr!("views-archive-extract"), |ui| {
                    ui.set_width(220.0);
                    act(ui, cmds, ph::ARROW_LINE_DOWN, tr!("views-archive-extract-here"), A::ExtractHere);
                    act(ui, cmds, ph::FOLDER_SIMPLE, tr!("views-archive-extract-to-folder"), A::ExtractToFolder);
                    act(ui, cmds, ph::TRAY_ARROW_DOWN, tr!("views-archive-extract-to"), A::ExtractTo);
                    menu_separator(ui);
                    act(ui, cmds, ph::CHECK_CIRCLE, tr!("views-archive-test"), A::TestArchive);
                });
            }
            if has_selection && matches!(location, Location::Dir(_)) {
                act(ui, cmds, ph::PACKAGE, tr!("views-archive-compress-menu"), A::CreateArchive);
            }
        }
    }
}

// ---- dialogs ------------------------------------------------------------------------

pub struct PasswordDialog {
    pane: PaneId,
    archive: PathBuf,
    wrong: bool,
    text: String,
    focus: bool,
}

impl PasswordDialog {
    pub fn new(pane: PaneId, archive: PathBuf, wrong: bool) -> Self {
        PasswordDialog { pane, archive, wrong, text: String::new(), focus: true }
    }
}

impl Dialog for PasswordDialog {
    fn title(&self) -> &str {
        tr!("views-archive-password-title")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::LOCK_SIMPLE)
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let name = soyuz_core::entry::display_name(&self.archive);
        ui.label(RichText::new(tr!("views-archive-password-body", name = &name)).color(p.text_secondary));
        if self.wrong {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("views-archive-password-wrong")).color(p.danger));
        }
        ui.add_space(10.0);
        let r = ui.add(
            TextEdit::singleline(&mut self.text)
                .password(true)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("views-archive-unlock"), tr!("common-cancel"));
        if (ok || enter) && !self.text.is_empty() {
            svc::set_password(&self.archive, std::mem::take(&mut self.text));
            ctx.cmds.push(Cmd::PaneAction(self.pane, soyuz_core::config::Action::Refresh));
            return false;
        }
        !cancel
    }
}

pub struct ExtractDialog {
    archives: Vec<PathBuf>,
    selection: Option<(Vec<(String, bool)>, usize)>,
    dest: String,
    subfolder: bool,
}

impl ExtractDialog {
    pub fn new(archives: Vec<PathBuf>, selection: Option<(Vec<(String, bool)>, usize)>, dest: PathBuf) -> Self {
        let dest = soyuz_fs::strip_verbatim(&dest).display().to_string();
        ExtractDialog { archives, selection, dest, subfolder: false }
    }
}

impl Dialog for ExtractDialog {
    fn title(&self) -> &str {
        tr!("views-archive-extract")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::TRAY_ARROW_DOWN)
    }
    fn width(&self) -> f32 {
        460.0
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let what = match (&self.selection, self.archives.len()) {
            (Some((items, _)), _) => tr!(
                "views-archive-extract-what",
                count = items.len(),
                name = soyuz_core::entry::display_name(&self.archives[0])
            ),
            (None, 1) => soyuz_core::entry::display_name(&self.archives[0]),
            (None, n) => tr!("views-archive-n-archives", count = n),
        };
        ui.label(RichText::new(what).color(p.text_secondary));
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("views-archive-destination")).size(11.5).color(p.text_tertiary));
        ui.add_space(2.0);
        let r =
            ui.add(TextEdit::singleline(&mut self.dest).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
        ui.add_space(6.0);
        ui.checkbox(&mut self.subfolder, tr!("views-archive-subfolder"));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("views-archive-extract"), tr!("common-cancel"));
        if ok || enter {
            let base = PathBuf::from(self.dest.trim());
            if base.as_os_str().is_empty() {
                return true;
            }
            for a in &self.archives {
                let dest = if self.subfolder {
                    unique_dir(&base, &archive_stem(&soyuz_core::entry::display_name(a)))
                } else {
                    base.clone()
                };
                match &self.selection {
                    Some((items, strip)) => ctx.cmds.push(Cmd::ExtractEntries {
                        archive: a.clone(),
                        items: items.clone(),
                        strip: *strip,
                        dest,
                    }),
                    None => ctx.cmds.push(Cmd::SubmitJob(Box::new(extract_spec(a, None, dest, 0)))),
                }
            }
            return false;
        }
        !cancel
    }
}

/// Formats offered by the create dialog (writable ones only).
fn writable_formats() -> Vec<ArchiveFormat> {
    [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarXz,
        ArchiveFormat::TarZst,
        ArchiveFormat::Tar,
    ]
    .into_iter()
    .filter(ArchiveFormat::can_write)
    .collect()
}

fn format_label(f: ArchiveFormat) -> &'static str {
    match f {
        ArchiveFormat::Zip => "ZIP",
        ArchiveFormat::SevenZ => "7z",
        ArchiveFormat::TarGz => "tar.gz",
        ArchiveFormat::TarXz => "tar.xz",
        ArchiveFormat::TarZst => "tar.zst",
        ArchiveFormat::TarBz2 => "tar.bz2",
        _ => "tar",
    }
}

pub struct CreateDialog {
    sources: Vec<PathBuf>,
    dir: PathBuf,
    name: String,
    format: ArchiveFormat,
    level: u8,
    password: String,
    focus: bool,
}

impl CreateDialog {
    pub fn new(sources: Vec<PathBuf>, dir: PathBuf, base: String) -> Self {
        CreateDialog {
            sources,
            dir,
            name: format!("{base}.zip"),
            format: ArchiveFormat::Zip,
            level: 6,
            password: String::new(),
            focus: true,
        }
    }

    fn set_format(&mut self, f: ArchiveFormat) {
        let stem = archive_stem(&self.name);
        self.format = f;
        self.name = format!("{stem}.{}", f.extensions()[0]);
        if !f.supports_password() {
            self.password.clear();
        }
    }
}

impl Dialog for CreateDialog {
    fn title(&self) -> &str {
        tr!("views-archive-compress")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::PACKAGE)
    }
    fn width(&self) -> f32 {
        480.0
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let n = self.sources.len();
        ui.label(
            RichText::new(if n == 1 {
                soyuz_core::entry::display_name(&self.sources[0])
            } else {
                tr!("common-items", count = n)
            })
            .color(p.text_secondary),
        );
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("views-archive-name")).size(11.5).color(p.text_tertiary));
        ui.add_space(2.0);
        let r =
            ui.add(TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
        if self.focus {
            r.request_focus();
            let stem_len = archive_stem(&self.name).chars().count();
            if let Some(mut state) = TextEdit::load_state(ui.ctx(), r.id) {
                let range =
                    egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(stem_len));
                state.cursor.set_char_range(Some(range));
                state.store(ui.ctx(), r.id);
            }
            self.focus = false;
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("views-archive-format")).size(11.5).color(p.text_tertiary));
        ui.add_space(2.0);
        // Text segmented control.
        let formats = writable_formats();
        let cell = vec2(68.0, 26.0);
        let (rect, _) = ui.allocate_exact_size(vec2(cell.x * formats.len() as f32 + 4.0, cell.y + 4.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.bg_surface2);
        for (i, f) in formats.iter().enumerate() {
            let r = egui::Rect::from_min_size(pos2(rect.left() + 2.0 + i as f32 * cell.x, rect.top() + 2.0), cell);
            let resp = ui.interact(r, ui.id().with(("fmt", i)), Sense::click());
            let sel = *f == self.format;
            if sel {
                ui.painter().rect_filled(r, CornerRadius::same(5), p.bg_surface1);
                ui.painter().rect_stroke(
                    r,
                    CornerRadius::same(5),
                    Stroke::new(1.0, p.border_subtle),
                    StrokeKind::Inside,
                );
            }
            let c = if sel {
                p.text_primary
            } else if resp.hovered() {
                p.text_secondary
            } else {
                p.text_tertiary
            };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, format_label(*f), FontId::proportional(12.0), c);
            if resp.clicked() {
                self.set_format(*f);
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("views-archive-compression")).size(11.5).color(p.text_tertiary));
            let mut lvl = self.level as f32;
            ui.add(egui::Slider::new(&mut lvl, 0.0..=9.0).step_by(1.0).show_value(false));
            self.level = lvl.round() as u8;
            let label = match self.level {
                0 => tr!("views-archive-level-store"),
                1..=3 => tr!("views-archive-level-fast"),
                4..=6 => tr!("views-archive-level-normal"),
                _ => tr!("views-archive-level-max"),
            };
            ui.label(RichText::new(format!("{label} ({})", self.level)).size(11.5).color(p.text_secondary));
        });
        if self.format.supports_password() {
            ui.add_space(8.0);
            ui.label(RichText::new(tr!("views-archive-password-optional")).size(11.5).color(p.text_tertiary));
            ui.add_space(2.0);
            ui.add(
                TextEdit::singleline(&mut self.password)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(10, 7)),
            );
            if self.format == ArchiveFormat::SevenZ && !self.password.is_empty() {
                ui.label(RichText::new(tr!("views-archive-names-encrypted")).size(11.0).color(p.text_tertiary));
            }
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("views-archive-compress"), tr!("common-cancel"));
        if ok || enter {
            let name = self.name.trim();
            if name.is_empty() {
                return true;
            }
            let dest = self.dir.join(name);
            if dest.exists() {
                ctx.cmds.push(Cmd::Toast(
                    Toast::warning(tr!("views-archive-exists", name = name)).body(tr!("views-archive-exists-body")),
                ));
                return true;
            }
            let label = tr!("views-archive-job-compress", name = name);
            let spec = JobSpec {
                kind: JobKind::Compress {
                    sources: self.sources.clone(),
                    dest,
                    format: self.format,
                    level: self.level,
                    password: (!self.password.is_empty()).then(|| self.password.clone()),
                },
                options: JobOptions { label: Some(label), ..Default::default() },
            };
            ctx.cmds.push(Cmd::SubmitJob(Box::new(spec)));
            return false;
        }
        !cancel
    }
}

/// `Cmd::ExtractEntries` handler.
pub fn extract_entries(app: &mut SoyuzApp, archive: PathBuf, items: Vec<(String, bool)>, strip: usize, dest: PathBuf) {
    submit_entries(app, archive, items, strip, dest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        assert_eq!(archive_stem("photos.zip"), "photos");
        assert_eq!(archive_stem("src.tar.gz"), "src");
        assert_eq!(archive_stem("x.tgz"), "x");
        assert_eq!(archive_stem("notes.txt"), "notes");
        assert_eq!(archive_stem("README"), "README");
    }

    #[test]
    fn archive_breadcrumb_mapping() {
        let dir = child_location(Path::new("C:/d/a.zip!/x/y"), true).unwrap();
        assert_eq!(dir, Location::Archive { archive: PathBuf::from("C:/d/a.zip"), inner: "x/y".into() });
        let file = child_location(Path::new("C:/d/a.zip!/x/f.txt"), false).unwrap();
        assert_eq!(file, Location::Archive { archive: PathBuf::from("C:/d/a.zip"), inner: "x".into() });
        assert_eq!(child_location(Path::new("C:/d/plain"), true), None);
        // The breadcrumb for an archive location: folders up to the archive, the archive
        // itself, then the inner segments.
        let loc = Location::Archive { archive: PathBuf::from("/data/a.zip"), inner: "x/y".into() };
        let crumbs = crate::state::crumbs::crumbs(&loc, None);
        let labels: Vec<&str> = crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(&labels[labels.len() - 3..], ["a.zip", "x", "y"]);
        assert_eq!(crumbs[crumbs.len() - 3].kind, crate::state::crumbs::CrumbKind::Archive);
        assert_eq!(crumbs.last().unwrap().location, loc);
        assert_eq!(loc.parent(), Some(Location::Archive { archive: PathBuf::from("/data/a.zip"), inner: "x".into() }));
    }

    #[test]
    fn selected_indices_expand_folders() {
        let paths = ["a.txt", "dir/", "dir/b.txt", "dir/sub/c.txt", "dirx/d.txt", "e.txt"];
        let idx = indices_for(paths.iter().copied(), &[("dir".into(), true), ("e.txt".into(), false)]);
        assert_eq!(idx, vec![1, 2, 3, 5]);
    }

    #[test]
    fn unique_destinations() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(unique_dir(d.path(), "x"), d.path().join("x"));
        std::fs::create_dir(d.path().join("x")).unwrap();
        assert_eq!(unique_dir(d.path(), "x"), d.path().join("x (2)"));
    }
}

// ---- editing inside a zip (F9) --------------------------------------------------------

/// Whether `archive` (by name) is a zip that can be edited (not a split `.001` set).
fn is_editable_zip_name(archive: &Path) -> bool {
    archive.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
        svc::format_by_name(name) == Some(ArchiveFormat::Zip) && !name.to_ascii_lowercase().ends_with(".001")
    })
}

/// The zip browsed in the focused pane, when it can be edited: `(archive, inner dir)`.
fn editable_zip(app: &SoyuzApp) -> Option<(PathBuf, String)> {
    browsed(app).filter(|(archive, _)| is_editable_zip_name(archive))
}

/// Whether `a` edits the browsed zip (Delete / Rename / Paste inside a zip).
pub fn handles_edit(app: &SoyuzApp, a: &soyuz_core::config::Action) -> bool {
    use soyuz_core::config::Action as A;
    matches!(a, A::Delete | A::DeletePermanent | A::Rename | A::Paste) && editable_zip(app).is_some()
}

type EditJobs = std::sync::Mutex<Vec<(soyuz_jobs::JobId, PathBuf)>>;

/// Edit jobs whose completion refreshes the panes browsing their archive.
fn edit_jobs() -> &'static EditJobs {
    static J: std::sync::OnceLock<EditJobs> = std::sync::OnceLock::new();
    J.get_or_init(Default::default)
}

fn edit_spec(archive: &Path, edits: Vec<soyuz_archive::ZipEdit>, label: String, level: u8) -> JobSpec {
    JobSpec {
        kind: JobKind::EditArchive { archive: archive.to_path_buf(), edits, level: Some(level.min(9)) },
        options: JobOptions { label: Some(label), ..Default::default() },
    }
}

/// Delete / Rename / Paste inside the browsed zip (rewrite job; the original goes to the
/// trash, so it can be undone).
pub fn execute_edit(app: &mut SoyuzApp, a: soyuz_core::config::Action) {
    use soyuz_core::config::Action as A;
    let Some((archive, inner)) = editable_zip(app) else { return };
    let zip_name = soyuz_core::entry::display_name(&archive);
    let level = app.config.archive.edit_level;
    let selected: Vec<(String, bool)> = app
        .ws
        .pane()
        .selected_entries()
        .iter()
        .filter_map(|e| split_vpath(&e.path).map(|(_, i)| (i, e.is_dir_like())))
        .collect();
    match a {
        A::Delete | A::DeletePermanent => {
            if selected.is_empty() {
                app.ui.toasts.push(Toast::info(tr!("views-archive-select-to-delete")));
                return;
            }
            let n = selected.len();
            let edits = selected.into_iter().map(|(path, _)| soyuz_archive::ZipEdit::Delete { path }).collect();
            let label = tr!("views-archive-job-delete", count = n, archive = &zip_name);
            app.cmds.push(Cmd::SubmitJob(Box::new(edit_spec(&archive, edits, label, level))));
        }
        A::Rename => match selected.as_slice() {
            [(path, is_dir)] => app.ui.dialogs.push(RenameEntryDialog::new(archive, path.clone(), *is_dir)),
            _ => {
                app.ui.toasts.push(Toast::info(tr!("views-archive-select-one-to-rename")));
            }
        },
        A::Paste => {
            let clip = crate::ops::fileops::clipboard::Clipboard::os_paths_hook()
                .or_else(|| app.fileops.clipboard.for_paste());
            let Some((_, sources)) = clip.filter(|(_, p)| !p.is_empty()) else {
                app.ui.toasts.push(
                    Toast::info(tr!("views-archive-nothing-to-add")).body(tr!("views-archive-nothing-to-add-body")),
                );
                return;
            };
            let n = sources.len();
            let edits = vec![soyuz_archive::ZipEdit::Add { sources, dest_dir: inner }];
            let label = tr!("views-archive-job-add", count = n, archive = &zip_name);
            app.cmds.push(Cmd::SubmitJob(Box::new(edit_spec(&archive, edits, label, level))));
        }
        _ => {}
    }
}

/// Context-menu rows for a browsed zip.
fn edit_context_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, pane: PaneId, location: &Location, has_selection: bool) {
    let Location::Archive { archive, .. } = location else { return };
    if !is_editable_zip_name(archive) {
        return;
    }
    use soyuz_core::config::Action as A;
    let mut act = |ui: &mut Ui, icon: &str, label: &str, a: A| {
        if menu_item(ui, Some(icon), label, None, true).clicked() {
            cmds.push(Cmd::PaneAction(pane, a));
            ui.close();
        }
    };
    menu_separator(ui);
    act(ui, ph::FILE_PLUS, tr!("views-archive-add-here"), A::Paste);
    if has_selection {
        act(ui, ph::PENCIL_SIMPLE, tr!("views-archive-rename-menu"), A::Rename);
        act(ui, ph::TRASH, tr!("views-archive-delete-menu"), A::DeletePermanent);
    }
}

/// Track an edit job so the panes browsing its archive refresh when it ends.
fn track_edit_job(id: soyuz_jobs::JobId, archive: PathBuf) {
    if let Ok(mut j) = edit_jobs().lock() {
        j.push((id, archive));
    }
}

/// Per frame: refresh archive panes after their edit job finished.
fn pump_edits(app: &mut SoyuzApp) {
    let done: Vec<PathBuf> = {
        let Ok(mut jobs) = edit_jobs().lock() else { return };
        if jobs.is_empty() {
            return;
        }
        let mut done = Vec::new();
        jobs.retain(|(id, archive)| {
            let finished = app.fileops.jobs.snapshot(*id).is_none_or(|s| s.state.is_terminal());
            if finished {
                done.push(archive.clone());
            }
            !finished
        });
        done
    };
    for archive in done {
        let panes: Vec<PaneId> = app
            .ws
            .tabs
            .iter()
            .flat_map(|t| t.panes.iter())
            .filter(|p| matches!(&p.location, Location::Archive { archive: a, .. } if *a == archive))
            .map(|p| p.id)
            .collect();
        for p in panes {
            app.cmds.push(Cmd::PaneAction(p, soyuz_core::config::Action::Refresh));
        }
    }
}

pub struct RenameEntryDialog {
    archive: PathBuf,
    from: String,
    is_dir: bool,
    text: String,
    focus: bool,
}

impl RenameEntryDialog {
    pub fn new(archive: PathBuf, from: String, is_dir: bool) -> Self {
        let text = from.rsplit('/').next().unwrap_or_default().to_string();
        RenameEntryDialog { archive, from, is_dir, text, focus: true }
    }
}

/// A valid single entry name (no separators, not `.` / `..`).
fn valid_entry_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".."
}

impl Dialog for RenameEntryDialog {
    fn title(&self) -> &str {
        tr!("views-archive-rename-title")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::PENCIL_SIMPLE)
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let kind = if self.is_dir { "folder" } else { "file" };
        ui.label(
            RichText::new(tr!(
                "views-archive-rename-body",
                kind = kind,
                name = &self.from,
                archive = soyuz_core::entry::display_name(&self.archive)
            ))
            .color(p.text_secondary),
        );
        ui.add_space(10.0);
        let r =
            ui.add(TextEdit::singleline(&mut self.text).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        let name = self.text.trim().to_string();
        let valid = valid_entry_name(&name);
        if !valid && !name.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("views-archive-no-slashes")).color(p.danger));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("common-rename"), tr!("common-cancel"));
        if (ok || enter) && valid {
            let to = match self.from.rsplit_once('/') {
                Some((parent, _)) => format!("{parent}/{name}"),
                None => name,
            };
            if to != self.from {
                let label = tr!(
                    "views-archive-job-rename",
                    name = &self.from,
                    archive = soyuz_core::entry::display_name(&self.archive)
                );
                let edits = vec![soyuz_archive::ZipEdit::Rename { from: self.from.clone(), to }];
                ctx.cmds.push(Cmd::SubmitJob(Box::new(edit_spec(&self.archive, edits, label, 0))));
            }
            return false;
        }
        !cancel
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn editable_names() {
        assert!(is_editable_zip_name(Path::new("C:/x/a.zip")));
        assert!(!is_editable_zip_name(Path::new("C:/x/a.zip.001")));
        assert!(!is_editable_zip_name(Path::new("C:/x/a.7z")));
        assert!(valid_entry_name("new name.txt"));
        assert!(!valid_entry_name("a/b") && !valid_entry_name("..") && !valid_entry_name(""));
    }
}
