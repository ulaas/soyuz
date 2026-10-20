//! Archives (Phase T0 model): Enter on a supported archive browses it as
//! `Location::Archive`; entries inside open (folders navigate, files are extracted to a
//! temp folder and launched); extract here / to "name/" / to… / selected entries and
//! "Compress…" submit soyuz-jobs Extract / Compress jobs via `Cmd::SubmitJob`; editing a
//! browsed zip (delete / rename / add) runs an EditArchive job. Each frontend draws the
//! dialogs (`ui::archive` for the GUI); they are asked for with [`FrontendCmd::Archive`].

use std::path::{Path, PathBuf};

use soyuz_archive::ArchiveFormat;
use soyuz_core::{Location, split_vpath};
use soyuz_jobs::{JobKind, JobOptions, JobSpec};

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::ops::{self, NavOpts};
use crate::services::archive as svc;
use crate::state::pane::PaneId;

/// Enter / double-click handling for archives. Returns `true` when handled.
pub fn open_entry(app: &mut AppCore, pane: PaneId, path: &Path, is_dir: bool) -> bool {
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

pub fn extract_spec(archive: &Path, entries: Option<Vec<usize>>, dest: PathBuf, strip: usize) -> JobSpec {
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
fn selected_archives(app: &AppCore) -> Vec<PathBuf> {
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
fn browsed(app: &AppCore) -> Option<(PathBuf, String)> {
    match &app.ws.pane().location {
        Location::Archive { archive, inner } => Some((archive.clone(), inner.clone())),
        _ => None,
    }
}

pub fn extract_here(app: &mut AppCore) {
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

pub fn extract_to_folder(app: &mut AppCore) {
    let archives = match browsed(app) {
        Some((a, _)) => vec![a],
        None => selected_archives(app),
    };
    for a in archives {
        let parent = a.parent().map(Path::to_path_buf).unwrap_or_default();
        app.cmds.push(Cmd::ExtractNewFolder { archive: a, parent, selection: None });
    }
}

/// "Extract to…" dialog (in an archive: the selected entries, else the selected archives).
pub fn extract_to_dialog(app: &mut AppCore) {
    let other = other_pane_dir(app);
    if let Some((archive, _)) = browsed(app) {
        let dest = other.or_else(|| archive.parent().map(Path::to_path_buf)).unwrap_or_default();
        let selection = selected_inner(app);
        app.frontend.push(FrontendCmd::Archive(ArchivePrompt::Extract(ExtractForm::new(
            vec![archive],
            selection,
            dest,
        ))));
        return;
    }
    let archives = selected_archives(app);
    if archives.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("views-archive-select-to-extract")));
        return;
    }
    let dest = other.or_else(|| archives[0].parent().map(Path::to_path_buf)).unwrap_or_default();
    app.frontend.push(FrontendCmd::Archive(ArchivePrompt::Extract(ExtractForm::new(archives, None, dest))));
}

fn other_pane_dir(app: &AppCore) -> Option<PathBuf> {
    let tab = app.ws.tab();
    if !tab.dual {
        return None;
    }
    tab.panes.get(1 - tab.focused.min(1)).and_then(|p| p.location.as_fs_path().map(Path::to_path_buf))
}

/// Selected entries of the browsed archive as `(inner paths, strip components)`.
fn selected_inner(app: &AppCore) -> Option<(Vec<(String, bool)>, usize)> {
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

fn extract_selected_to(app: &mut AppCore, dest: PathBuf) {
    let Some((archive, _)) = browsed(app) else { return };
    match selected_inner(app) {
        Some((items, strip)) => submit_entries(app, archive, items, strip, dest),
        None => app.cmds.push(Cmd::SubmitJob(Box::new(extract_spec(&archive, None, dest, 0)))),
    }
}

/// Resolve inner paths (folders expand to their contents) to entry indices on a worker,
/// then submit the job.
fn submit_entries(app: &mut AppCore, archive: PathBuf, items: Vec<(String, bool)>, strip: usize, dest: PathBuf) {
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
pub fn create_dialog(app: &mut AppCore) {
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
    app.frontend.push(FrontendCmd::Archive(ArchivePrompt::Create(CreateForm::new(sources, dir.clone(), base))));
}

/// Check an archive's integrity on a worker; reports with a toast.
pub fn test_archive(app: &mut AppCore) {
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
pub fn pump(app: &mut AppCore) {
    svc::remember_wake(&app.wake);
    pump_edits(app);
    if let Some((pane, archive, wrong)) = svc::take_password_request() {
        app.frontend.push(FrontendCmd::Archive(ArchivePrompt::Password { pane, archive, wrong }));
    }
    for spec in svc::take_pending_jobs() {
        submit_job(app, spec);
    }
}

/// Submit an extract / compress job to the app-wide jobs service (activity panel,
/// prompts and completion toasts come from there).
pub fn submit_job(app: &mut AppCore, spec: soyuz_jobs::JobSpec) -> soyuz_jobs::JobId {
    use crate::services::jobs::{JobMeta, OpKind};
    let roots = spec.kind.touched_roots();
    // Media tools (Phase V) carry their own card facts (tool, output).
    let meta =
        crate::models::media_tools::job_meta(&spec.kind).unwrap_or_else(|| JobMeta::new(OpKind::Other, &roots, None));
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

/// `Cmd::ExtractEntries` handler.
pub fn extract_entries(app: &mut AppCore, archive: PathBuf, items: Vec<(String, bool)>, strip: usize, dest: PathBuf) {
    submit_entries(app, archive, items, strip, dest);
}

/// `Cmd::ExtractNewFolder` handler: the free folder name is found on a worker (it can
/// take many checks on a slow drive), then the extract is submitted.
pub fn extract_new_folder(
    app: &mut AppCore,
    archive: PathBuf,
    parent: PathBuf,
    selection: Option<(Vec<(String, bool)>, usize)>,
) {
    let postman = app.services.mailbox.postman();
    let spawned = std::thread::Builder::new().name("soyuz-extract-dest".into()).spawn(move || {
        let dest = unique_dir(&parent, &archive_stem(&soyuz_core::entry::display_name(&archive)));
        let cmd = match selection {
            Some((items, strip)) => Cmd::ExtractEntries { archive, items, strip, dest },
            None => Cmd::SubmitJob(Box::new(extract_spec(&archive, None, dest, 0))),
        };
        postman.send(crate::services::mailbox::UiMsg::Cmds(vec![cmd]));
    });
    if let Err(e) = spawned {
        app.ui.toasts.push(Toast::error(tr!("views-archive-extract-failed")).body(e.to_string()));
    }
}

// ---- prompts --------------------------------------------------------------------------

/// A dialog the frontend should show.
#[derive(Clone, Debug, PartialEq)]
pub enum ArchivePrompt {
    /// A listing needs a password (`wrong`: the last one did not work); see [`unlock`].
    Password { pane: PaneId, archive: PathBuf, wrong: bool },
    /// "Extract to…".
    Extract(ExtractForm),
    /// "Compress…".
    Create(CreateForm),
    /// Rename an entry of a browsed zip; see [`rename_cmd`].
    Rename { archive: PathBuf, from: String, is_dir: bool },
}

/// Remember the password for `archive` and re-list the pane that asked.
pub fn unlock(pane: PaneId, archive: &Path, password: String) -> Cmd {
    svc::set_password(archive, password);
    Cmd::PaneAction(pane, soyuz_core::config::Action::Refresh)
}

/// The "Extract to…" form: the archives (or the selected entries of one) and where to.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractForm {
    pub archives: Vec<PathBuf>,
    /// Selected inner paths `(path, is_dir)` and the components to strip.
    pub selection: Option<(Vec<(String, bool)>, usize)>,
    pub dest: String,
    /// Each archive into its own new folder.
    pub subfolder: bool,
}

impl ExtractForm {
    pub fn new(archives: Vec<PathBuf>, selection: Option<(Vec<(String, bool)>, usize)>, dest: PathBuf) -> Self {
        let dest = soyuz_fs::strip_verbatim(&dest).display().to_string();
        ExtractForm { archives, selection, dest, subfolder: false }
    }

    /// One line describing what is extracted.
    pub fn summary(&self) -> String {
        match (&self.selection, self.archives.len()) {
            (Some((items, _)), _) => tr!(
                "views-archive-extract-what",
                count = items.len(),
                name = soyuz_core::entry::display_name(&self.archives[0])
            ),
            (None, 1) => soyuz_core::entry::display_name(&self.archives[0]),
            (None, n) => tr!("views-archive-n-archives", count = n),
        }
    }

    /// The jobs to run, or `None` while no destination is typed.
    pub fn submit(&self) -> Option<Vec<Cmd>> {
        let base = PathBuf::from(self.dest.trim());
        if base.as_os_str().is_empty() {
            return None;
        }
        let mut cmds = Vec::new();
        for a in &self.archives {
            if self.subfolder {
                let (archive, parent, selection) = (a.clone(), base.clone(), self.selection.clone());
                cmds.push(Cmd::ExtractNewFolder { archive, parent, selection });
                continue;
            }
            let dest = base.clone();
            match &self.selection {
                Some((items, strip)) => {
                    cmds.push(Cmd::ExtractEntries { archive: a.clone(), items: items.clone(), strip: *strip, dest })
                }
                None => cmds.push(Cmd::SubmitJob(Box::new(extract_spec(a, None, dest, 0)))),
            }
        }
        Some(cmds)
    }
}

/// Formats offered by the create form (writable ones only).
pub fn writable_formats() -> Vec<ArchiveFormat> {
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

pub fn format_label(f: ArchiveFormat) -> &'static str {
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

/// The compression level's name.
pub fn level_label(level: u8) -> &'static str {
    match level {
        0 => tr!("views-archive-level-store"),
        1..=3 => tr!("views-archive-level-fast"),
        4..=6 => tr!("views-archive-level-normal"),
        _ => tr!("views-archive-level-max"),
    }
}

/// The "Compress…" form.
#[derive(Clone, PartialEq)]
pub struct CreateForm {
    pub sources: Vec<PathBuf>,
    pub dir: PathBuf,
    pub name: String,
    pub format: ArchiveFormat,
    pub level: u8,
    pub password: String,
}

/// Never prints the password.
impl std::fmt::Debug for CreateForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateForm")
            .field("sources", &self.sources)
            .field("dir", &self.dir)
            .field("name", &self.name)
            .field("format", &self.format)
            .field("level", &self.level)
            .finish_non_exhaustive()
    }
}

impl CreateForm {
    pub fn new(sources: Vec<PathBuf>, dir: PathBuf, base: String) -> Self {
        CreateForm {
            sources,
            dir,
            name: format!("{base}.zip"),
            format: ArchiveFormat::Zip,
            level: 6,
            password: String::new(),
        }
    }

    /// Switch the format, keeping the name's stem.
    pub fn set_format(&mut self, f: ArchiveFormat) {
        let stem = archive_stem(&self.name);
        self.format = f;
        self.name = format!("{stem}.{}", f.extensions()[0]);
        if !f.supports_password() {
            self.password.clear();
        }
    }

    /// The compress job; `Err` keeps the form open (with a toast to show, if any).
    pub fn submit(&self) -> Result<Cmd, Option<Toast>> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(None);
        }
        let dest = self.dir.join(name);
        if dest.exists() {
            return Err(Some(
                Toast::warning(tr!("views-archive-exists", name = name)).body(tr!("views-archive-exists-body")),
            ));
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
        Ok(Cmd::SubmitJob(Box::new(spec)))
    }
}

/// The rename job for entry `from` of `archive` (`None` when the name is unchanged).
/// `name` must pass [`valid_entry_name`].
pub fn rename_cmd(archive: &Path, from: &str, name: &str) -> Option<Cmd> {
    let to = match from.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{name}"),
        None => name.to_string(),
    };
    if to == from {
        return None;
    }
    let label = tr!("views-archive-job-rename", name = from, archive = soyuz_core::entry::display_name(archive));
    let edits = vec![soyuz_archive::ZipEdit::Rename { from: from.to_string(), to }];
    Some(Cmd::SubmitJob(Box::new(edit_spec(archive, edits, label, 0))))
}

// ---- editing inside a zip --------------------------------------------------------

/// Whether `archive` (by name) is a zip that can be edited (not a split `.001` set).
pub fn is_editable_zip_name(archive: &Path) -> bool {
    archive.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
        svc::format_by_name(name) == Some(ArchiveFormat::Zip) && !name.to_ascii_lowercase().ends_with(".001")
    })
}

/// The zip browsed in the focused pane, when it can be edited: `(archive, inner dir)`.
fn editable_zip(app: &AppCore) -> Option<(PathBuf, String)> {
    browsed(app).filter(|(archive, _)| is_editable_zip_name(archive))
}

/// Whether `a` edits the browsed zip (Delete / Rename / Paste inside a zip).
pub fn handles_edit(app: &AppCore, a: &soyuz_core::config::Action) -> bool {
    use soyuz_core::config::Action as A;
    matches!(a, A::Delete | A::DeletePermanent | A::Rename | A::Paste) && editable_zip(app).is_some()
}

type EditJobs = std::sync::Mutex<Vec<(soyuz_jobs::JobId, PathBuf)>>;

/// Edit jobs whose completion refreshes the panes browsing their archive.
fn edit_jobs() -> &'static EditJobs {
    static J: std::sync::OnceLock<EditJobs> = std::sync::OnceLock::new();
    J.get_or_init(Default::default)
}

pub fn edit_spec(archive: &Path, edits: Vec<soyuz_archive::ZipEdit>, label: String, level: u8) -> JobSpec {
    JobSpec {
        kind: JobKind::EditArchive { archive: archive.to_path_buf(), edits, level: Some(level.min(9)) },
        options: JobOptions { label: Some(label), ..Default::default() },
    }
}

/// Delete / Rename / Paste inside the browsed zip (rewrite job; the original goes to the
/// trash, so it can be undone).
pub fn execute_edit(app: &mut AppCore, a: soyuz_core::config::Action) {
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
            [(path, is_dir)] => app.frontend.push(FrontendCmd::Archive(ArchivePrompt::Rename {
                archive,
                from: path.clone(),
                is_dir: *is_dir,
            })),
            _ => {
                app.ui.toasts.push(Toast::info(tr!("views-archive-select-one-to-rename")));
            }
        },
        A::Paste => {
            // The system clipboard may answer a few frames later (Linux: another process).
            let then = move |app: &mut crate::core::AppCore, os: crate::services::os_clipboard::Files| {
                let clip = os.or_else(|| app.fileops.clipboard.for_paste());
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
            };
            crate::services::os_clipboard::with_files(app, Box::new(then));
        }
        _ => {}
    }
}

/// Track an edit job so the panes browsing its archive refresh when it ends.
fn track_edit_job(id: soyuz_jobs::JobId, archive: PathBuf) {
    if let Ok(mut j) = edit_jobs().lock() {
        j.push((id, archive));
    }
}

/// Per frame: refresh archive panes after their edit job finished.
fn pump_edits(app: &mut AppCore) {
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

/// A valid single entry name (no separators, not `.` / `..`).
pub fn valid_entry_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".."
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
