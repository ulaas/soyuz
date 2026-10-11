//! Remote locations (Phase T0 model): saved connections and their state, the connect
//! form, host-key and sign-in prompts, and file operations in remote panes (copy / cut /
//! paste across providers, delete, rename, new folder, F5 / F6) submitted as
//! `JobKind::Remote` jobs. Remote files open through a temp-cache download. Each frontend
//! draws the sidebar section and the dialogs (`ui::remote` for the GUI); dialogs are asked
//! for with [`FrontendCmd::Remote`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use soyuz_core::config::{Action, PasteConflict};
use soyuz_core::{ConflictResolution, EntryInfo, Location};
use soyuz_jobs::{JobId, JobKind, RemoteOp};
use soyuz_vfs::registry::HostKeyPrompt;
use soyuz_vfs::{RemoteUri, Scheme};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::ops::fileops::clipboard::ClipMode;
use crate::ops::{NavOpts, navigate};
use crate::services::jobs::{AfterJob, JobMeta, OpKind};
use crate::services::remote::{self as svc, AuthKind, FetchThen, SavedConn};
use crate::state::pane::PaneId;

/// Commands from the sidebar and dialogs (applied after the UI pass).
#[derive(Clone, Debug)]
pub enum RemoteCmd {
    /// Open the connect dialog: blank, for a URI (sign-in retry) or to edit a saved one.
    ConnectDialog {
        uri: Option<String>,
        edit: Option<String>,
        note: Option<String>,
    },
    /// Navigate the focused pane to a server (after the dialog stored the auth).
    Open {
        uri: String,
        new_tab: bool,
    },
    Disconnect(String),
    Remove(String),
    /// Submit a remote job and refresh the panes on those servers when it ends.
    Submit {
        kind: JobKind,
        op: OpKind,
        items: Vec<PathBuf>,
        dest: Option<PathBuf>,
        clear_clipboard: bool,
    },
}

// ---- per-frame pump ----------------------------------------------------------------------

struct PumpState {
    tracked: Vec<(JobId, Vec<RemoteUri>)>,
    was_active: bool,
    last_poll: Instant,
}

fn pump_state() -> &'static Mutex<PumpState> {
    static S: std::sync::OnceLock<Mutex<PumpState>> = std::sync::OnceLock::new();
    S.get_or_init(|| Mutex::new(PumpState { tracked: Vec::new(), was_active: false, last_poll: Instant::now() }))
}

/// Per frame: apply `[remote]`, show host-key / sign-in prompts, refresh remote panes
/// after their jobs (and undo jobs) finish, poll visible remote panes.
pub fn pump(app: &mut SoyuzApp, host: &dyn Host) {
    svc::init(&app.config.remote, host.wake());
    while let Some((prompt, tx)) = svc::take_host_prompt() {
        app.frontend.push(FrontendCmd::Remote(RemotePrompt::HostKey(HostKeyAsk::new(prompt, tx))));
    }
    while let Some((title, body)) = svc::take_notice() {
        app.ui.toasts.push(Toast::warning(title).body(body));
    }
    if let Some(uri) = svc::take_sign_in_request() {
        let note = Some(tr!("remote-sign-in-failed").to_string());
        app.frontend.push(FrontendCmd::Remote(RemotePrompt::Connect(Box::new(ConnectForm::for_uri(&uri, note)))));
    }
    let mut refresh: Vec<RemoteUri> = Vec::new();
    {
        let mut st = pump_state().lock();
        let engine = app.fileops.jobs.engine();
        st.tracked.retain(|(id, uris)| {
            let done = engine.snapshot(*id).is_none_or(|s| s.state.is_terminal());
            if done {
                refresh.extend(uris.iter().cloned());
            }
            !done
        });
        // Any job just finished (e.g. an undo of a remote job): re-list remote panes.
        let active = engine.has_active();
        if st.was_active && !active {
            refresh.extend(remote_panes(app).into_iter().map(|(_, u)| u));
        }
        st.was_active = active;
        if let Some(every) = svc::poll_interval()
            && st.last_poll.elapsed() >= every
        {
            st.last_poll = Instant::now();
            refresh.extend(remote_panes(app).into_iter().map(|(_, u)| u));
        }
        if svc::poll_interval().is_some() || !st.tracked.is_empty() {
            host.redraw_after(std::time::Duration::from_millis(500));
        }
    }
    if !refresh.is_empty() {
        for u in &refresh {
            svc::invalidate_server(u);
        }
        let keys: Vec<String> = refresh.iter().map(RemoteUri::conn_key).collect();
        for (id, u) in remote_panes(app) {
            if keys.contains(&u.conn_key())
                && let Some(p) = app.ws.find_pane_mut(id)
            {
                crate::ops::start_listing(p, &mut app.services, true);
            }
        }
    }
}

/// Visible remote panes of the active tab: `(pane, folder)`.
fn remote_panes(app: &SoyuzApp) -> Vec<(PaneId, RemoteUri)> {
    app.ws
        .tab()
        .panes
        .iter()
        .filter_map(|p| match &p.location {
            Location::Remote(u) => RemoteUri::parse(u).ok().map(|u| (p.id, u)),
            _ => None,
        })
        .collect()
}

pub fn apply(app: &mut SoyuzApp, c: RemoteCmd) {
    match c {
        RemoteCmd::ConnectDialog { uri, edit, note } => {
            let form = match (edit.and_then(|id| svc::connections().into_iter().find(|c| c.id == id)), uri) {
                (Some(conn), _) => ConnectForm::edit(&conn),
                (None, Some(u)) => ConnectForm::for_uri(&u, note),
                (None, None) => ConnectForm::new(),
            };
            app.frontend.push(FrontendCmd::Remote(RemotePrompt::Connect(Box::new(form))));
        }
        RemoteCmd::Open { uri, new_tab } => {
            let loc = Location::Remote(uri);
            if new_tab {
                crate::ops::open_in_new_tab(app, loc, true);
            } else {
                let id = app.ws.pane().id;
                navigate(app, id, loc, NavOpts::user());
            }
        }
        RemoteCmd::Disconnect(uri) => {
            if let Ok(u) = RemoteUri::parse(&uri) {
                svc::disconnect(&u);
                app.ui.toasts.push(Toast::info(tr!("remote-disconnected", server = u.server_label())));
            }
        }
        RemoteCmd::Remove(id) => svc::remove_connection(&id),
        RemoteCmd::Submit { kind, op, items, dest, clear_clipboard } => {
            submit(app, kind, op, items, dest, clear_clipboard);
        }
    }
}

fn submit(
    app: &mut SoyuzApp,
    kind: JobKind,
    op: OpKind,
    items: Vec<PathBuf>,
    dest: Option<PathBuf>,
    clear: bool,
) -> JobId {
    let mut uris: Vec<RemoteUri> = items.iter().chain(dest.iter()).filter_map(|p| RemoteUri::from_path(p)).collect();
    uris.dedup_by_key(|u| u.conn_key());
    let mut spec = app.fileops.jobs.spec(kind);
    if matches!(op, OpKind::Copy | OpKind::Move) {
        spec.options.conflict = match app.config.behaviour.paste_conflict {
            PasteConflict::Ask => None,
            PasteConflict::Overwrite => Some(ConflictResolution::Overwrite),
            PasteConflict::Skip => Some(ConflictResolution::Skip),
            PasteConflict::KeepBoth => Some(ConflictResolution::KeepBoth),
        };
    }
    let mut meta = JobMeta::new(op, &items, dest);
    if clear {
        meta = meta.after(AfterJob::ClearClipboard);
    }
    if matches!(op, OpKind::NewFolder | OpKind::Rename) {
        meta = meta.quiet();
    }
    let id = app.fileops.jobs.submit(spec, meta);
    pump_state().lock().tracked.push((id, uris));
    id
}

// ---- opening entries ----------------------------------------------------------------------

/// Enter / double-click on a remote entry. Returns `true` when handled.
pub fn open_entry(app: &mut SoyuzApp, pane: PaneId, path: &Path, is_dir: bool) -> bool {
    if !svc::is_remote_path(path) {
        return false;
    }
    if is_dir {
        let loc = Location::Remote(path.to_string_lossy().into_owned());
        if app.config.general.open_folders_in == soyuz_core::config::OpenFoldersIn::NewTab {
            crate::ops::open_in_new_tab(app, loc, true);
        } else {
            navigate(app, pane, loc, NavOpts::user());
        }
        return true;
    }
    let entry = app.ws.find_pane(pane).and_then(|p| p.entries.iter().find(|e| e.path == path).cloned());
    let Some(e) = entry else { return true };
    if !svc::is_fetching(&e) && svc::cached_copy(&e).is_none() {
        let mut t = Toast::info(tr!("remote-downloading", name = &e.name)).body(tr!("remote-downloading-body"));
        t.duration = Some(2.5);
        app.ui.toasts.push(t);
    }
    svc::fetch(&e, FetchThen::Launch, app.services.mailbox.postman());
    true
}

/// What the preview pane shows for a remote entry: the cached local copy once it is
/// downloaded (a download starts on demand, up to `[remote] preview_max_mb`).
pub fn preview_target(app: &SoyuzApp, e: &EntryInfo) -> Option<EntryInfo> {
    if e.is_dir_like() || e.size > svc::preview_max_bytes() {
        return None;
    }
    match svc::cached_copy(e) {
        Some(p) => {
            let mut local = e.clone();
            local.path = p;
            Some(local)
        }
        None => {
            svc::fetch(e, FetchThen::Preview, app.services.mailbox.postman());
            None
        }
    }
}

// ---- actions ------------------------------------------------------------------------------

fn focused_remote(app: &SoyuzApp) -> Option<RemoteUri> {
    match &app.ws.pane().location {
        Location::Remote(u) => RemoteUri::parse(u).ok(),
        _ => None,
    }
}

fn other_pane_location(app: &SoyuzApp) -> Option<Location> {
    app.ws.tab().other().map(|p| p.location.clone())
}

fn clipboard_has_remote(app: &SoyuzApp) -> bool {
    app.fileops.clipboard.for_paste().is_some_and(|(_, p)| p.iter().any(|p| svc::is_remote_path(p)))
}

/// Actions this module handles in the current state (routed by `actions::execute`).
pub fn handles(app: &SoyuzApp, a: &Action) -> bool {
    use Action as A;
    if *a == A::ConnectToServer {
        return true;
    }
    let here_remote = focused_remote(app).is_some();
    let other_remote = matches!(other_pane_location(app), Some(Location::Remote(_)));
    match a {
        A::Copy | A::Cut | A::Delete | A::DeletePermanent | A::SecureWipe | A::Rename | A::NewFolder | A::NewFile => {
            here_remote
        }
        A::Duplicate | A::PasteAsLink | A::PasteShortcut | A::OpenTerminalHere | A::OpenInTerminal => here_remote,
        A::RevealInSystem | A::Properties | A::BatchRename | A::SetAttributes | A::TouchTimestamps => here_remote,
        A::Paste | A::PasteAndVerify => here_remote || clipboard_has_remote(app),
        A::CopyToOtherPane | A::MoveToOtherPane => here_remote || other_remote,
        _ => false,
    }
}

pub fn execute(app: &mut SoyuzApp, a: Action, host: &dyn Host) {
    use Action as A;
    let here = focused_remote(app);
    let pane = app.ws.pane();
    let sel: Vec<PathBuf> = pane.selected_entries().iter().map(|e| e.path.clone()).collect();
    let here_path = match &pane.location {
        Location::Remote(u) => Some(PathBuf::from(u)),
        Location::Dir(d) => Some(d.clone()),
        _ => None,
    };
    match a {
        A::ConnectToServer => app.frontend.push(FrontendCmd::Remote(RemotePrompt::Connect(Box::default()))),
        A::Copy | A::Cut => {
            if sel.is_empty() {
                return;
            }
            let mode = if a == A::Cut { ClipMode::Cut } else { ClipMode::Copy };
            host.copy_text(sel.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n"));
            let n = sel.len();
            app.fileops.clipboard.set(mode, sel);
            let title =
                if mode == ClipMode::Cut { tr!("remote-cut", count = n) } else { tr!("remote-copied", count = n) };
            let mut t = Toast::info(title).body(tr!("remote-clipboard-body"));
            t.duration = Some(2.0);
            app.ui.toasts.push(t);
        }
        A::Paste | A::PasteAndVerify => {
            let Some(dest) = here_path else { return };
            let paste = move |app: &mut SoyuzApp, clip: Option<(ClipMode, Vec<PathBuf>)>| {
                let Some((mode, paths)) = clip else {
                    app.ui
                        .toasts
                        .push(Toast::info(tr!("remote-nothing-to-paste")).body(tr!("remote-nothing-to-paste-body")));
                    return;
                };
                let mv = mode == ClipMode::Cut;
                transfer(app, paths, dest, mv, mv);
            };
            // The in-process clipboard first (memory only); the system clipboard may
            // answer a few frames later (Linux: another process).
            match app.fileops.clipboard.for_paste() {
                Some(clip) => paste(app, Some(clip)),
                None => crate::services::os_clipboard::with_files(app, Box::new(paste)),
            }
        }
        A::CopyToOtherPane | A::MoveToOtherPane => {
            let dest = match other_pane_location(app) {
                Some(Location::Remote(u)) => PathBuf::from(u),
                Some(Location::Dir(d)) => d,
                _ => {
                    app.ui.toasts.push(Toast::info(tr!("remote-no-other-folder")));
                    return;
                }
            };
            if !sel.is_empty() {
                transfer(app, sel, dest, a == A::MoveToOtherPane, false);
            }
        }
        A::Delete | A::DeletePermanent | A::SecureWipe => {
            if sel.is_empty() {
                return;
            }
            let name =
                if sel.len() == 1 { soyuz_core::entry::display_name(&sel[0]).to_string() } else { String::new() };
            let server = here.as_ref().map(RemoteUri::server_label).unwrap_or_default();
            let msg = tr!("remote-delete-message", count = sel.len(), name = name, server = server);
            app.frontend.push(FrontendCmd::Remote(RemotePrompt::ConfirmDelete { message: msg, paths: sel }));
        }
        A::Rename => {
            let pane = app.ws.pane();
            let target = pane
                .cursor
                .as_deref()
                .and_then(|c| pane.entry(c))
                .filter(|e| pane.selection.contains(&e.name))
                .or_else(|| pane.selected_entries().first().copied())
                .cloned();
            if let Some(e) = target {
                let prompt = RemotePrompt::Name { target: NameTarget::Rename(e.path.clone()), name: e.name.clone() };
                app.frontend.push(FrontendCmd::Remote(prompt));
            }
        }
        A::NewFolder => {
            if let Some(u) = here {
                let names: Vec<&str> = app.ws.pane().entries.iter().map(|e| e.name.as_str()).collect();
                let name = crate::ops::fileops::names::unique_in(&names, tr!("remote-new-folder-name"), false);
                app.frontend.push(FrontendCmd::Remote(RemotePrompt::Name { target: NameTarget::NewFolder(u), name }));
            }
        }
        A::Properties => {
            let pane = app.ws.pane();
            if let Some(e) = pane.cursor.as_deref().and_then(|c| pane.entry(c)) {
                let size = soyuz_core::format_size(e.size, soyuz_core::SizeStyle::Decimal);
                let when = e
                    .modified
                    .map(|m| chrono::DateTime::<chrono::Local>::from(m).format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| tr!("remote-unknown-time").into());
                let line = if e.is_dir_like() {
                    tr!("remote-props-folder", when = &when)
                } else {
                    tr!("remote-props-file", size = size, when = &when)
                };
                let body = format!("{line}\n{}", e.path.display());
                app.ui.toasts.push(Toast::info(e.name.clone()).body(body));
            }
        }
        other => {
            let what = match other {
                A::NewFile => "new-file",
                A::Duplicate => "duplicate",
                A::PasteAsLink | A::PasteShortcut => "links",
                A::OpenTerminalHere | A::OpenInTerminal => "terminal",
                A::RevealInSystem => "reveal",
                _ => "other",
            };
            app.ui.toasts.push(Toast::info(tr!("remote-unavailable", what = what)));
        }
    }
}

/// Copy / move `paths` into `dest` where at least one side is remote; plain local
/// transfers go to the regular copy engine.
pub fn transfer(app: &mut SoyuzApp, paths: Vec<PathBuf>, dest: PathBuf, mv: bool, clear: bool) -> Option<JobId> {
    if paths.is_empty() {
        return None;
    }
    let any_remote = svc::is_remote_path(&dest) || paths.iter().any(|p| svc::is_remote_path(p));
    if !any_remote {
        let effect =
            if mv { crate::ops::fileops::dnd::DropEffect::Move } else { crate::ops::fileops::dnd::DropEffect::Copy };
        return crate::ops::fileops::transfer(app, paths, dest, effect);
    }
    if mv
        && paths
            .iter()
            .all(|p| RemoteUri::from_path(p).and_then(|u| u.parent()).map(|u| u.to_path_buf()) == Some(dest.clone()))
    {
        return None; // moving into the folder it is already in
    }
    let op = if mv { OpKind::Move } else { OpKind::Copy };
    let kind = JobKind::Remote(RemoteOp::Transfer { sources: paths.clone(), dest_dir: dest.clone(), mv });
    Some(submit(app, kind, op, paths, Some(dest), clear))
}

/// The confirmed "delete on the server" job.
pub fn delete_cmd(paths: Vec<PathBuf>) -> Cmd {
    Cmd::Remote(RemoteCmd::Submit {
        kind: JobKind::Remote(RemoteOp::Delete { paths: paths.clone() }),
        op: OpKind::Delete,
        items: paths,
        dest: None,
        clear_clipboard: false,
    })
}

// ---- prompts ------------------------------------------------------------------------------

/// A dialog the frontend should show.
#[derive(Clone, Debug, PartialEq)]
pub enum RemotePrompt {
    /// "Connect to server…" (blank, for a URI, or editing a saved connection).
    Connect(Box<ConnectForm>),
    /// "Trust this host key?"
    HostKey(HostKeyAsk),
    /// Confirm deleting `paths` on the server; "Delete" runs [`delete_cmd`].
    ConfirmDelete { message: String, paths: Vec<PathBuf> },
    /// New folder / rename on a server, prefilled with `name`.
    Name { target: NameTarget, name: String },
}

/// What a [`RemotePrompt::Name`] names.
#[derive(Clone, Debug, PartialEq)]
pub enum NameTarget {
    /// A new folder in this server folder.
    NewFolder(RemoteUri),
    /// Rename this remote path.
    Rename(PathBuf),
}

/// The job for a typed name: `Err` = not a valid name (stay open), `Ok(None)` = nothing
/// to do (same name), `Ok(Some)` = submit.
pub fn name_submit(target: &NameTarget, name: &str) -> Result<Option<Cmd>, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(tr!("remote-error-name").into());
    }
    match target {
        NameTarget::NewFolder(dir) => {
            let path = dir.join(&name).to_path_buf();
            Ok(Some(Cmd::Remote(RemoteCmd::Submit {
                kind: JobKind::Remote(RemoteOp::Mkdir { path: path.clone() }),
                op: OpKind::NewFolder,
                items: vec![path],
                dest: None,
                clear_clipboard: false,
            })))
        }
        NameTarget::Rename(from) => {
            let Some(u) = RemoteUri::from_path(from) else { return Ok(None) };
            let Some(parent) = u.parent() else { return Ok(None) };
            let to = parent.join(&name).to_path_buf();
            if &to == from {
                return Ok(None);
            }
            Ok(Some(Cmd::Remote(RemoteCmd::Submit {
                kind: JobKind::Remote(RemoteOp::Rename { from: from.clone(), to }),
                op: OpKind::Rename,
                items: vec![from.clone()],
                dest: None,
                clear_clipboard: false,
            })))
        }
    }
}

/// An SSH server presented an unknown or changed key: the worker waits for the answer.
/// Clones share one answer; when every clone is dropped unanswered the worker sees a
/// closed channel ("don't trust").
#[derive(Clone, Debug)]
pub struct HostKeyAsk {
    pub prompt: HostKeyPrompt,
    reply: Arc<Mutex<Option<Sender<bool>>>>,
}

impl HostKeyAsk {
    pub fn new(prompt: HostKeyPrompt, tx: Sender<bool>) -> Self {
        HostKeyAsk { prompt, reply: Arc::new(Mutex::new(Some(tx))) }
    }

    /// Answer once (later answers are ignored).
    pub fn answer(&self, trust: bool) {
        if let Some(tx) = self.reply.lock().take() {
            let _ = tx.send(trust);
        }
    }
}

impl PartialEq for HostKeyAsk {
    fn eq(&self, other: &Self) -> bool {
        self.prompt == other.prompt && Arc::ptr_eq(&self.reply, &other.reply)
    }
}

/// Protocols offered by the connect form. Windows opens SMB shares as `\\server\share`
/// paths through its own redirector, so SMB is offered elsewhere only.
#[cfg(not(windows))]
pub const PROTOCOLS: &[(Scheme, &str)] = &[
    (Scheme::Sftp, "SFTP"),
    (Scheme::Smb, "SMB"),
    (Scheme::WebDav, "WebDAV"),
    (Scheme::Ftp, "FTP"),
    (Scheme::S3, "S3"),
];
#[cfg(windows)]
pub const PROTOCOLS: &[(Scheme, &str)] =
    &[(Scheme::Sftp, "SFTP"), (Scheme::WebDav, "WebDAV"), (Scheme::Ftp, "FTP"), (Scheme::S3, "S3")];

/// The "Connect to server…" form.
#[derive(Clone, PartialEq)]
pub struct ConnectForm {
    /// Editing this saved connection.
    pub edit: Option<SavedConn>,
    pub proto: Scheme,
    /// TLS (WebDAV over HTTPS, explicit FTPS).
    pub tls: bool,
    pub host: String,
    pub port: String,
    pub user: String,
    pub path: String,
    pub auth: AuthKind,
    pub secret: String,
    pub key_file: String,
    pub endpoint: String,
    pub region: String,
    pub name: String,
    pub save: bool,
    pub remember: bool,
    /// Shown above the form (e.g. why sign-in is asked again).
    pub note: Option<String>,
    pub error: Option<String>,
}

/// Never prints the secret.
impl std::fmt::Debug for ConnectForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectForm")
            .field("edit", &self.edit)
            .field("proto", &self.proto)
            .field("host", &self.host)
            .field("user", &self.user)
            .field("path", &self.path)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl ConnectForm {
    pub fn new() -> Self {
        ConnectForm {
            edit: None,
            proto: Scheme::Sftp,
            tls: true,
            host: String::new(),
            port: String::new(),
            user: String::new(),
            path: String::new(),
            auth: AuthKind::Password,
            secret: String::new(),
            key_file: String::new(),
            endpoint: String::new(),
            region: String::new(),
            name: String::new(),
            save: true,
            remember: true,
            note: None,
            error: None,
        }
    }

    fn fill_from(&mut self, u: &RemoteUri) {
        (self.proto, self.tls) = match u.scheme {
            Scheme::WebDav => (Scheme::WebDav, false),
            Scheme::WebDavs => (Scheme::WebDav, true),
            Scheme::Ftp => (Scheme::Ftp, false),
            Scheme::Ftps => (Scheme::Ftp, true),
            other => (other, true),
        };
        self.host = u.host.clone();
        self.port = u.port.map(|p| p.to_string()).unwrap_or_default();
        self.user = u.user.clone().unwrap_or_default();
        self.path = if u.is_root() { String::new() } else { u.path.clone() };
        self.endpoint = u.query_value("endpoint").unwrap_or_default().to_string();
        self.region = u.query_value("region").unwrap_or_default().to_string();
    }

    /// Prefilled for `uri` (and its saved connection, if any).
    pub fn for_uri(uri: &str, note: Option<String>) -> Self {
        let mut d = ConnectForm::new();
        if let Ok(u) = RemoteUri::parse(uri) {
            match svc::saved_for(&u).filter(|c| svc::is_saved(&c.id)) {
                Some(c) => d = ConnectForm::edit(&c),
                None => d.fill_from(&u),
            }
            d.path = if u.is_root() { String::new() } else { u.path.clone() };
        }
        d.note = note;
        d
    }

    pub fn edit(c: &SavedConn) -> Self {
        let mut d = ConnectForm::new();
        if let Some(u) = c.parsed() {
            d.fill_from(&u);
        }
        d.name = c.name.clone();
        d.auth = c.auth;
        d.key_file = c.key_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        d.remember = c.has_secret;
        d.edit = Some(c.clone());
        d
    }

    pub fn scheme(&self) -> Scheme {
        match (self.proto, self.tls) {
            (Scheme::WebDav, true) => Scheme::WebDavs,
            (Scheme::Ftp, true) => Scheme::Ftps,
            (s, _) => s,
        }
    }

    pub fn build(&self) -> Result<RemoteUri, String> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err(
                if self.proto == Scheme::S3 { tr!("remote-error-bucket") } else { tr!("remote-error-server") }.into()
            );
        }
        let port = match self.port.trim() {
            "" => None,
            p => Some(p.parse::<u16>().map_err(|_| tr!("remote-error-port", port = p))?),
        };
        let user = Some(self.user.trim().to_string()).filter(|u| !u.is_empty());
        if self.proto == Scheme::Sftp && user.is_none() {
            return Err(tr!("remote-error-sftp-user").into());
        }
        let mut query = Vec::new();
        if self.proto == Scheme::S3 {
            if !self.endpoint.trim().is_empty() {
                query.push(("endpoint".to_string(), self.endpoint.trim().to_string()));
            }
            if !self.region.trim().is_empty() {
                query.push(("region".to_string(), self.region.trim().to_string()));
            }
        }
        let scheme = self.scheme();
        let port = port.filter(|&p| p != scheme.default_port());
        let u = RemoteUri {
            scheme,
            user,
            host: if scheme == Scheme::S3 { host.to_string() } else { host.to_ascii_lowercase() },
            port,
            path: soyuz_vfs::uri::normalize_path(self.path.trim()),
            query,
        };
        // Round-trip through the parser so the stored form is canonical.
        RemoteUri::parse(&u.display()).map_err(|e| e.to_string())
    }

    pub fn auth_options(&self) -> Vec<(AuthKind, &'static str)> {
        match self.proto {
            Scheme::Sftp => {
                vec![
                    (AuthKind::Password, tr!("remote-auth-password")),
                    (AuthKind::KeyFile, tr!("remote-auth-key-file")),
                    (AuthKind::Agent, tr!("remote-auth-agent")),
                ]
            }
            Scheme::S3 => vec![
                (AuthKind::Password, tr!("remote-auth-access-keys")),
                (AuthKind::Anonymous, tr!("remote-auth-public")),
            ],
            Scheme::Smb => {
                vec![(AuthKind::Password, tr!("remote-auth-password")), (AuthKind::Anonymous, tr!("remote-auth-guest"))]
            }
            _ => vec![
                (AuthKind::Password, tr!("remote-auth-password")),
                (AuthKind::Anonymous, tr!("remote-auth-anonymous")),
            ],
        }
    }
}

impl Default for ConnectForm {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectForm {
    /// Store the sign-in for this session (and the saved connection, off the UI thread);
    /// returns the command that opens the server.
    pub fn connect(&self, uri: RemoteUri) -> Cmd {
        let secret = Some(self.secret.clone()).filter(|s| !s.is_empty());
        let key_file = Some(self.key_file.trim()).filter(|k| !k.is_empty()).map(PathBuf::from);
        let uses_secret = !matches!(self.auth, AuthKind::Anonymous | AuthKind::Agent);
        // This session: always use what was typed (an empty field keeps a stored secret).
        svc::set_session_auth(&uri, self.auth, key_file.clone(), secret.clone().filter(|_| uses_secret));
        if self.save {
            let root = uri.root();
            let name = if self.name.trim().is_empty() { root.server_label() } else { self.name.trim().to_string() };
            let conn = SavedConn {
                id: self.edit.as_ref().map(|c| c.id.clone()).unwrap_or_default(),
                name,
                uri: uri.display(),
                auth: self.auth,
                key_file,
                has_secret: self.edit.as_ref().is_some_and(|c| c.has_secret),
            };
            // `Some("")` deletes a stored secret, `None` keeps it.
            let store = match (uses_secret && self.remember, secret) {
                (true, Some(s)) => Some(s),
                (true, None) => None,
                (false, _) => Some(String::new()),
            };
            // Credential store + small JSON write: off the UI thread.
            let _ = std::thread::Builder::new().name("soyuz-remote-save".into()).spawn(move || {
                if let Some(w) = svc::save_connection(conn, store.as_deref()) {
                    log::warn!("{w}");
                }
            });
        }
        Cmd::Remote(RemoteCmd::Open { uri: uri.display(), new_tab: false })
    }
}
