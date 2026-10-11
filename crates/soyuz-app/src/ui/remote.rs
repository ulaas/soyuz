//! Remote locations in the UI: the "Connect to server…" dialog, the
//! sidebar's Network section (saved connections with their connection state), the
//! "trust this host key?" prompt, sign-in retry, and file operations in remote panes
//! (copy / cut / paste across providers, delete, rename, new folder, F5 / F6) submitted
//! as `JobKind::Remote` jobs. Remote files open through a temp-cache download.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crossbeam_channel::Sender;
use egui::{Key, Margin, RichText, TextEdit, Ui};
use parking_lot::Mutex;
use soyuz_core::config::{Action, PasteConflict};
use soyuz_core::{ConflictResolution, EntryInfo, Location};
use soyuz_jobs::{JobId, JobKind, RemoteOp};
use soyuz_vfs::known_hosts::HostKeyStatus;
use soyuz_vfs::registry::{ConnState, HostKeyPrompt};
use soyuz_vfs::{RemoteUri, Scheme};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::fileops::clipboard::{ClipMode, Clipboard};
use crate::ops::{NavOpts, navigate};
use crate::services::jobs::{AfterJob, JobMeta, OpKind};
use crate::services::remote::{self as svc, AuthKind, FetchThen, SavedConn};
use crate::state::pane::PaneId;
use crate::theme::{icons::ph, palette};
use crate::ui::dialogs::{ConfirmDialog, Dialog, DialogCtx, footer};
use crate::ui::toasts::Toast;
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator};

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
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    let c = ctx.clone();
    svc::init(&app.config.remote, std::sync::Arc::new(move || c.request_repaint()));
    while let Some((prompt, tx)) = svc::take_host_prompt() {
        app.ui.dialogs.push(HostKeyDialog { prompt, tx: Some(tx) });
    }
    if let Some(uri) = svc::take_sign_in_request() {
        let note = Some(tr!("remote-sign-in-failed").to_string());
        app.ui.dialogs.push(ConnectDialog::for_uri(&uri, note));
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
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
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
            let d = match (edit.and_then(|id| svc::connections().into_iter().find(|c| c.id == id)), uri) {
                (Some(conn), _) => ConnectDialog::edit(&conn),
                (None, Some(u)) => ConnectDialog::for_uri(&u, note),
                (None, None) => ConnectDialog::new(),
            };
            app.ui.dialogs.push(d);
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

fn clipboard_paths(app: &SoyuzApp) -> Option<(ClipMode, Vec<PathBuf>)> {
    app.fileops.clipboard.for_paste().or_else(Clipboard::os_paths_hook)
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

pub fn execute(app: &mut SoyuzApp, a: Action, ctx: &egui::Context) {
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
        A::ConnectToServer => app.ui.dialogs.push(ConnectDialog::new()),
        A::Copy | A::Cut => {
            if sel.is_empty() {
                return;
            }
            let mode = if a == A::Cut { ClipMode::Cut } else { ClipMode::Copy };
            ctx.copy_text(sel.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n"));
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
            let Some((mode, paths)) = clipboard_paths(app) else {
                app.ui
                    .toasts
                    .push(Toast::info(tr!("remote-nothing-to-paste")).body(tr!("remote-nothing-to-paste-body")));
                return;
            };
            let mv = mode == ClipMode::Cut;
            transfer(app, paths, dest, mv, mv);
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
            let cmd = Cmd::Remote(RemoteCmd::Submit {
                kind: JobKind::Remote(RemoteOp::Delete { paths: sel.clone() }),
                op: OpKind::Delete,
                items: sel,
                dest: None,
                clear_clipboard: false,
            });
            app.ui.dialogs.push(ConfirmDialog::new(tr!("remote-delete-title"), &msg, tr!("common-delete"), cmd));
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
                app.ui.dialogs.push(NameDialog::rename(e.path.clone(), e.name.clone()));
            }
        }
        A::NewFolder => {
            if let Some(u) = here {
                let names: Vec<&str> = app.ws.pane().entries.iter().map(|e| e.name.as_str()).collect();
                let name = crate::ops::fileops::names::unique_in(&names, tr!("remote-new-folder-name"), false);
                app.ui.dialogs.push(NameDialog::new_folder(u, name));
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

// ---- sidebar ------------------------------------------------------------------------------

/// The sidebar's "Network" section: saved connections (dot = connection state) and
/// "Connect to server…".
pub fn sidebar_section(
    ui: &mut Ui,
    collapsed: &mut BTreeSet<String>,
    current: &Location,
    pane: PaneId,
    cmds: &mut Vec<Cmd>,
) {
    // `name` keys the collapsed state (settings); the header shows the translation.
    let name = "Network";
    let mut open = !collapsed.contains(name);
    collapsible_header(ui, tr!("remote-network"), &mut open);
    if open {
        collapsed.remove(name);
    } else {
        collapsed.insert(name.to_string());
        return;
    }
    let p = palette(ui.ctx());
    let here = match current {
        Location::Remote(u) => RemoteUri::parse(u).ok().map(|u| u.conn_key()),
        _ => None,
    };
    for c in svc::connections() {
        let Some(uri) = c.parsed() else { continue };
        let state = svc::conn_state(&uri);
        let (color, state_text) = match &state {
            ConnState::Connected => (Some(p.success), tr!("remote-state-connected").to_string()),
            ConnState::Connecting => (Some(p.warning), tr!("remote-state-connecting").to_string()),
            ConnState::Failed(m) => (Some(p.danger), tr!("remote-state-failed", error = m)),
            ConnState::Idle => (None, tr!("remote-state-idle").to_string()),
        };
        let icon = match uri.scheme {
            Scheme::S3 => ph::CLOUD,
            Scheme::WebDav | Scheme::WebDavs => ph::GLOBE,
            Scheme::Fake => ph::FLASK,
            _ => ph::HARD_DRIVES,
        };
        let active = here.as_deref() == Some(uri.conn_key().as_str());
        let r = NavRow { icon, label: &c.name, active, icon_color: color, usage: None }.show(ui);
        let r = r.on_hover_text(format!("{}\n{} · {}", uri.display(), uri.scheme.label(), state_text));
        if r.clicked() {
            cmds.push(Cmd::FocusPane(pane));
            cmds.push(Cmd::Navigate { pane, location: Location::Remote(uri.display()), select: None });
        }
        if r.middle_clicked() {
            cmds.push(Cmd::OpenInNewTab { location: Location::Remote(uri.display()), activate: false });
        }
        let saved = svc::is_saved(&c.id);
        crate::ui::native::popup::context_menu(&r, |ui| {
            ui.set_width(220.0);
            if menu_item(ui, Some(ph::BROWSERS), tr!("remote-open-new-tab"), None, true).clicked() {
                cmds.push(Cmd::Remote(RemoteCmd::Open { uri: uri.display(), new_tab: true }));
                ui.close();
            }
            if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("remote-edit-connection"), None, saved).clicked() {
                cmds.push(Cmd::Remote(RemoteCmd::ConnectDialog { uri: None, edit: Some(c.id.clone()), note: None }));
                ui.close();
            }
            let connected = matches!(state, ConnState::Connected | ConnState::Failed(_));
            if menu_item(ui, Some(ph::PLUGS), tr!("remote-disconnect"), None, connected).clicked() {
                cmds.push(Cmd::Remote(RemoteCmd::Disconnect(uri.display())));
                ui.close();
            }
            if menu_item(ui, Some(ph::CLIPBOARD_TEXT), tr!("remote-copy-address"), None, true).clicked() {
                cmds.push(Cmd::CopyText(uri.display()));
                ui.close();
            }
            if saved {
                menu_separator(ui);
                if menu_item(ui, Some(ph::TRASH), tr!("remote-remove"), None, true).clicked() {
                    cmds.push(Cmd::Remote(RemoteCmd::Remove(c.id.clone())));
                    ui.close();
                }
            }
        });
    }
    let r =
        NavRow { icon: ph::PLUS, label: tr!("remote-connect"), active: false, icon_color: Some(p.accent), usage: None }
            .show(ui)
            .on_hover_text(tr!("remote-connect-tip"));
    if r.clicked() {
        cmds.push(Cmd::Remote(RemoteCmd::ConnectDialog { uri: None, edit: None, note: None }));
    }
}

// ---- dialogs ------------------------------------------------------------------------------

fn field_label(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(11.5).color(p.text_tertiary));
    ui.add_space(2.0);
}

fn text_field(ui: &mut Ui, value: &mut String, hint: &str, password: bool) -> egui::Response {
    ui.add(
        TextEdit::singleline(value)
            .password(password)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(10, 7)),
    )
}

/// Protocols offered by the connect dialog.
const PROTOCOLS: [(Scheme, &str); 4] =
    [(Scheme::Sftp, "SFTP"), (Scheme::WebDav, "WebDAV"), (Scheme::Ftp, "FTP"), (Scheme::S3, "S3")];

pub struct ConnectDialog {
    /// Editing this saved connection (id).
    edit: Option<SavedConn>,
    proto: Scheme,
    /// TLS (WebDAV over HTTPS, explicit FTPS).
    tls: bool,
    host: String,
    port: String,
    user: String,
    path: String,
    auth: AuthKind,
    secret: String,
    key_file: String,
    endpoint: String,
    region: String,
    name: String,
    save: bool,
    remember: bool,
    note: Option<String>,
    error: Option<String>,
    focus: bool,
}

impl ConnectDialog {
    pub fn new() -> Self {
        ConnectDialog {
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
            focus: true,
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
        let mut d = ConnectDialog::new();
        if let Ok(u) = RemoteUri::parse(uri) {
            match svc::saved_for(&u).filter(|c| svc::is_saved(&c.id)) {
                Some(c) => d = ConnectDialog::edit(&c),
                None => d.fill_from(&u),
            }
            d.path = if u.is_root() { String::new() } else { u.path.clone() };
        }
        d.note = note;
        d.focus = true;
        d
    }

    pub fn edit(c: &SavedConn) -> Self {
        let mut d = ConnectDialog::new();
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

    fn scheme(&self) -> Scheme {
        match (self.proto, self.tls) {
            (Scheme::WebDav, true) => Scheme::WebDavs,
            (Scheme::Ftp, true) => Scheme::Ftps,
            (s, _) => s,
        }
    }

    fn build(&self) -> Result<RemoteUri, String> {
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

    fn auth_options(&self) -> Vec<(AuthKind, &'static str)> {
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
            _ => vec![
                (AuthKind::Password, tr!("remote-auth-password")),
                (AuthKind::Anonymous, tr!("remote-auth-anonymous")),
            ],
        }
    }
}

impl Default for ConnectDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl Dialog for ConnectDialog {
    fn title(&self) -> &str {
        if self.edit.is_some() { tr!("remote-dialog-edit-title") } else { tr!("remote-dialog-connect-title") }
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::GLOBE)
    }
    fn width(&self) -> f32 {
        480.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        if let Some(n) = &self.note {
            ui.label(RichText::new(n).color(p.warning));
            ui.add_space(4.0);
        }
        // Protocol.
        field_label(ui, tr!("remote-protocol"));
        ui.horizontal(|ui| {
            let mut proto = self.proto;
            crate::ui::settings::widgets::segmented(ui, &mut proto, &PROTOCOLS);
            if proto != self.proto {
                self.proto = proto;
                if !self.auth_options().iter().any(|(a, _)| *a == self.auth) {
                    self.auth = AuthKind::Password;
                }
            }
            if matches!(self.proto, Scheme::WebDav | Scheme::Ftp) {
                ui.add_space(10.0);
                ui.checkbox(&mut self.tls, if self.proto == Scheme::WebDav { "HTTPS" } else { "TLS (FTPS)" });
            }
        });
        let s3 = self.proto == Scheme::S3;
        // Server / bucket and port.
        ui.columns(2, |cols| {
            let (a, b) = cols.split_at_mut(1);
            let ui = &mut a[0];
            field_label(ui, if s3 { tr!("remote-bucket") } else { tr!("remote-server") });
            let r = text_field(ui, &mut self.host, if s3 { "my-bucket" } else { "example.com" }, false);
            if self.focus {
                r.request_focus();
                self.focus = false;
            }
            let ui = &mut b[0];
            if s3 {
                field_label(ui, tr!("remote-region"));
                text_field(ui, &mut self.region, "us-east-1", false);
            } else {
                field_label(ui, tr!("remote-port"));
                let def = self.scheme().default_port().to_string();
                text_field(ui, &mut self.port, &def, false);
            }
        });
        if s3 {
            field_label(ui, tr!("remote-endpoint"));
            text_field(ui, &mut self.endpoint, "https://s3.amazonaws.com, http://127.0.0.1:9000 …", false);
        }
        // Sign-in.
        field_label(ui, tr!("remote-sign-in-with"));
        let mut auth = self.auth;
        crate::ui::settings::widgets::segmented(ui, &mut auth, &self.auth_options());
        self.auth = auth;
        if self.auth != AuthKind::Anonymous {
            ui.columns(2, |cols| {
                let (a, b) = cols.split_at_mut(1);
                let ui = &mut a[0];
                field_label(ui, if s3 { tr!("remote-access-key") } else { tr!("remote-user") });
                text_field(ui, &mut self.user, if s3 { "AKIA…" } else { tr!("remote-user-hint") }, false);
                let ui = &mut b[0];
                let (label, hint) = match (self.auth, s3) {
                    (_, true) => (tr!("remote-secret-key"), ""),
                    (AuthKind::KeyFile, _) => (tr!("remote-key-passphrase"), tr!("remote-key-passphrase-hint")),
                    (AuthKind::Agent, _) => ("", ""),
                    _ => (tr!("remote-password"), ""),
                };
                if !label.is_empty() {
                    field_label(ui, label);
                    let keep = self.edit.as_ref().is_some_and(|c| c.has_secret);
                    let hint = if keep && self.secret.is_empty() { tr!("remote-secret-unchanged") } else { hint };
                    text_field(ui, &mut self.secret, hint, true);
                }
            });
            if self.auth == AuthKind::KeyFile {
                field_label(ui, tr!("remote-key-file-label"));
                text_field(ui, &mut self.key_file, r"C:\Users\me\.ssh\id_ed25519", false);
            }
            if self.auth == AuthKind::Agent {
                ui.add_space(4.0);
                let note = if cfg!(windows) { tr!("remote-agent-note") } else { tr!("remote-agent-note-unix") };
                ui.label(RichText::new(note).size(11.5).color(p.text_tertiary));
            }
        }
        field_label(ui, if s3 { tr!("remote-prefix") } else { tr!("remote-folder") });
        text_field(ui, &mut self.path, "/", false);
        // Save.
        ui.add_space(10.0);
        ui.checkbox(&mut self.save, tr!("remote-save"));
        if self.save {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.add(
                    TextEdit::singleline(&mut self.name)
                        .hint_text(tr!("remote-name-hint"))
                        .desired_width(220.0)
                        .margin(Margin::symmetric(8, 5)),
                );
            });
            if self.auth != AuthKind::Anonymous && self.auth != AuthKind::Agent {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.checkbox(
                        &mut self.remember,
                        if cfg!(target_os = "linux") { tr!("remote-remember-linux") } else { tr!("remote-remember") },
                    );
                });
            }
        }
        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(p.danger));
        }
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("remote-connect-button"), tr!("common-cancel"));
        if ok || enter {
            match self.build() {
                Ok(uri) => {
                    self.connect(uri, ctx);
                    return false;
                }
                Err(e) => self.error = Some(e),
            }
        }
        !cancel
    }
}

impl ConnectDialog {
    fn connect(&mut self, uri: RemoteUri, ctx: &mut DialogCtx<'_>) {
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
        ctx.cmds.push(Cmd::Remote(RemoteCmd::Open { uri: uri.display(), new_tab: false }));
    }
}

/// "Trust this host key?" — shown when an SSH server presents an unknown or changed key.
pub struct HostKeyDialog {
    prompt: HostKeyPrompt,
    tx: Option<Sender<bool>>,
}

impl HostKeyDialog {
    fn answer(&mut self, trust: bool) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(trust);
        }
    }
}

impl Drop for HostKeyDialog {
    fn drop(&mut self) {
        self.answer(false); // closed with Esc / click outside: do not trust
    }
}

impl Dialog for HostKeyDialog {
    fn title(&self) -> &str {
        match self.prompt.status {
            HostKeyStatus::Changed { .. } => tr!("remote-hostkey-changed-title"),
            _ => tr!("remote-hostkey-trust-title"),
        }
    }
    fn icon(&self) -> Option<&'static str> {
        Some(match self.prompt.status {
            HostKeyStatus::Changed { .. } => ph::SHIELD_WARNING,
            _ => ph::FINGERPRINT,
        })
    }
    fn width(&self) -> f32 {
        500.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let host = if self.prompt.port == 22 {
            self.prompt.host.clone()
        } else {
            format!("{}:{}", self.prompt.host, self.prompt.port)
        };
        let changed = matches!(self.prompt.status, HostKeyStatus::Changed { .. });
        if changed {
            ui.label(RichText::new(tr!("remote-hostkey-changed-body", host = &host)).color(p.danger));
        } else {
            ui.label(RichText::new(tr!("remote-hostkey-first-body", host = &host)).color(p.text_secondary));
        }
        ui.add_space(10.0);
        let card = egui::Frame::new()
            .fill(p.bg_surface2)
            .corner_radius(egui::CornerRadius::same(crate::theme::tokens::radius::CARD))
            .inner_margin(Margin::symmetric(12, 10));
        card.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&self.prompt.algorithm).size(11.5).color(p.text_tertiary));
            ui.add_space(2.0);
            ui.label(RichText::new(&self.prompt.fingerprint).font(egui::FontId::monospace(12.5)).color(p.text_primary));
            if let HostKeyStatus::Changed { previous } = &self.prompt.status {
                ui.add_space(8.0);
                ui.label(RichText::new(tr!("remote-hostkey-previous")).size(11.5).color(p.text_tertiary));
                ui.label(RichText::new(previous).font(egui::FontId::monospace(12.0)).color(p.text_secondary));
            }
        });
        let (ok, cancel) = footer(
            ui,
            if changed { tr!("remote-hostkey-trust-new") } else { tr!("remote-hostkey-trust") },
            tr!("common-cancel"),
        );
        if ok {
            self.answer(true);
            return false;
        }
        if cancel {
            self.answer(false);
            return false;
        }
        true
    }
}

/// New folder / rename on a server.
pub struct NameDialog {
    /// `Some(dir)` = new folder in `dir`; `None` = rename `from`.
    dir: Option<RemoteUri>,
    from: Option<PathBuf>,
    name: String,
    error: Option<String>,
    focus: bool,
}

impl NameDialog {
    fn new_folder(dir: RemoteUri, name: String) -> Self {
        NameDialog { dir: Some(dir), from: None, name, error: None, focus: true }
    }

    fn rename(from: PathBuf, name: String) -> Self {
        NameDialog { dir: None, from: Some(from), name, error: None, focus: true }
    }
}

impl Dialog for NameDialog {
    fn title(&self) -> &str {
        if self.dir.is_some() { tr!("remote-new-folder-title") } else { tr!("common-rename") }
    }
    fn icon(&self) -> Option<&'static str> {
        Some(if self.dir.is_some() { ph::FOLDER_PLUS } else { ph::PENCIL_SIMPLE })
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let r = text_field(ui, &mut self.name, tr!("remote-name"), false);
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        if let Some(e) = &self.error {
            ui.add_space(4.0);
            ui.label(RichText::new(e).color(p.danger));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(
            ui,
            if self.dir.is_some() { tr!("remote-create") } else { tr!("common-rename") },
            tr!("common-cancel"),
        );
        if ok || enter {
            let name = self.name.trim().to_string();
            if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
                self.error = Some(tr!("remote-error-name").into());
                return true;
            }
            match (&self.dir, &self.from) {
                (Some(dir), _) => {
                    let path = dir.join(&name).to_path_buf();
                    ctx.cmds.push(Cmd::Remote(RemoteCmd::Submit {
                        kind: JobKind::Remote(RemoteOp::Mkdir { path: path.clone() }),
                        op: OpKind::NewFolder,
                        items: vec![path],
                        dest: None,
                        clear_clipboard: false,
                    }));
                }
                (None, Some(from)) => {
                    let Some(u) = RemoteUri::from_path(from) else { return false };
                    let Some(parent) = u.parent() else { return false };
                    let to = parent.join(&name).to_path_buf();
                    if &to != from {
                        ctx.cmds.push(Cmd::Remote(RemoteCmd::Submit {
                            kind: JobKind::Remote(RemoteOp::Rename { from: from.clone(), to: to.clone() }),
                            op: OpKind::Rename,
                            items: vec![from.clone()],
                            dest: None,
                            clear_clipboard: false,
                        }));
                    }
                }
                _ => {}
            }
            return false;
        }
        !cancel
    }
}
