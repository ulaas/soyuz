//! "Edit in Soyuz": built-in text, hex and image editors in a dockable
//! panel with one tab per document.
//!
//! * Open with `Action::OpenInEditor` (F4 in the file list, "Edit in Soyuz" in the
//!   context menu, the palette). Text files open in the text editor, images in the image
//!   editor, everything else (and text over `[editor] max_text_mib`) in the hex editor.
//! * All file I/O runs on worker threads ([`io`]); saving optionally puts the previous
//!   version in the trash first (`[editor] backup_to_trash`) and then writes atomically.
//! * Open files are watched: a clean document reloads by itself, a modified one shows a
//!   prompt. Closing a modified document (or the window) asks first.
//! * The open paths are part of the session.
//!
//! This module is the toolkit-free model (documents, I/O, commands, key routing);
//! `ui::editors` draws the panel and the views and feeds them input.

pub mod commands;
pub mod hex;
pub mod image;
pub mod io;
pub mod text;

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Mods};

use io::{Answer, LoadResult, Msg, Stamp, Want};

use crate::core::{AppCore, Focus};
use crate::host::Host;
use crate::keys::input::{InputEvent, InputFrame};
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::services::mailbox::Wake;
use crate::services::preview_gfx::{Uploader, no_upload};

/// Most files one "Edit in Soyuz" opens at once.
const MAX_OPEN_AT_ONCE: usize = 20;

/// Modifier state of a key press as the editors read it (`command` is Ctrl, or Cmd on
/// macOS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyMods {
    pub shift: bool,
    pub command: bool,
    pub alt: bool,
}

/// One input event for a focused editor view (the frontend converts its own events).
#[derive(Clone, Debug, PartialEq)]
pub enum EditInput {
    /// Typed text.
    Text(String),
    /// Text committed by an input method.
    ImeCommit(String),
    Copy,
    Cut,
    Paste(String),
    /// A key press (repeats included); `None` for keys without a neutral name.
    Key(Option<KeyName>, KeyMods),
}

pub enum DocKind {
    Loading,
    Failed(String),
    Text(Box<text::TextDoc>),
    Hex(Box<hex::HexDoc>),
    Image(Box<image::ImageDoc>),
}

/// The file changed on disk while the document had unsaved changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum External {
    Changed,
    Deleted,
}

pub struct Doc {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub kind: DocKind,
    pub want: Want,
    stamp: Option<Stamp>,
    /// A save is running.
    pub saving: bool,
    save_token: Option<(u64, text::encoding::Eol)>,
    pub external: Option<External>,
    close_after_save: bool,
}

impl Doc {
    pub fn is_dirty(&self) -> bool {
        match &self.kind {
            DocKind::Text(t) => t.is_dirty(),
            DocKind::Hex(h) => h.is_dirty(),
            DocKind::Image(i) => i.is_dirty(),
            _ => false,
        }
    }

    /// Ask the frontend to focus the document's view.
    pub fn request_focus(&mut self) {
        match &mut self.kind {
            DocKind::Text(t) => t.focus_request = true,
            DocKind::Hex(h) => h.focus_request = true,
            DocKind::Image(i) => i.focus_request = true,
            _ => {}
        }
    }
}

pub struct Editors {
    pub docs: Vec<Doc>,
    pub active: usize,
    /// The panel is shown.
    pub open: bool,
    /// The editor fills the area of the file panes.
    pub maximized: bool,
    next_id: u64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    watcher: Option<io::DocWatcher>,
    /// Makes the image editor's textures (the frontend's uploader).
    upload: Uploader,
    /// "Save all and quit" is in progress.
    quit_after_save: bool,
    /// The user chose to quit without saving.
    quit_allowed: bool,
    prompting_quit: bool,
    /// A command only the frontend can carry out (`EditorSaveAs`, `EditorResizeImage`:
    /// the terminal UI's bars), taken by the frontend ([`commands`]).
    pub request: Option<Action>,
}

impl Default for Editors {
    fn default() -> Self {
        Editors::new(no_upload())
    }
}

impl Editors {
    /// `upload` makes the frontend's textures for the image editor.
    pub fn new(upload: Uploader) -> Self {
        let (tx, rx) = unbounded();
        Editors {
            docs: Vec::new(),
            active: 0,
            open: false,
            maximized: false,
            next_id: 1,
            tx,
            rx,
            watcher: None,
            upload,
            quit_after_save: false,
            quit_allowed: false,
            prompting_quit: false,
            request: None,
        }
    }

    pub fn active_doc(&self) -> Option<&Doc> {
        self.docs.get(self.active)
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut Doc> {
        self.docs.get_mut(self.active)
    }

    pub fn dirty_count(&self) -> usize {
        self.docs.iter().filter(|d| d.is_dirty()).count()
    }

    fn watcher(&mut self, wake: Wake) -> &io::DocWatcher {
        let tx = self.tx.clone();
        self.watcher.get_or_insert_with(|| io::DocWatcher::start(tx, wake))
    }
}

// ---- actions ------------------------------------------------------------------------

pub fn handles(a: &Action) -> bool {
    matches!(a, Action::OpenInEditor | Action::ToggleEditorPanel | Action::FocusEditor)
}

pub fn execute(app: &mut AppCore, a: Action) {
    match a {
        Action::OpenInEditor => {
            let files: Vec<PathBuf> = app
                .ws
                .pane()
                .selected_entries()
                .iter()
                .filter(|e| !e.is_dir_like())
                .map(|e| e.path.clone())
                .take(MAX_OPEN_AT_ONCE)
                .collect();
            if files.is_empty() {
                app.ui.toasts.push(Toast::info(tr!("editors-select-file")));
                return;
            }
            for f in files {
                open_path(app, &f, Want::Auto);
            }
        }
        Action::ToggleEditorPanel => {
            let e = &mut app.editors;
            e.open = !e.open;
            if e.open {
                if let Some(d) = e.active_doc_mut() {
                    d.request_focus();
                }
            } else {
                e.maximized = false;
                app.ui.focus = Focus::List;
            }
        }
        Action::FocusEditor => commands::focus(app),
        _ => {}
    }
}

/// Open (or switch to) `path` in the editor panel.
pub fn open_path(app: &mut AppCore, path: &Path, want: Want) {
    let wake = app.wake.clone();
    let e = &mut app.editors;
    e.open = true;
    if let Some(i) = e.docs.iter().position(|d| d.path == path) {
        e.active = i;
        e.docs[i].request_focus();
        return;
    }
    let id = e.next_id;
    e.next_id += 1;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    e.docs.push(Doc {
        id,
        path: path.to_path_buf(),
        name,
        kind: DocKind::Loading,
        want,
        stamp: None,
        saving: false,
        save_token: None,
        external: None,
        close_after_save: false,
    });
    e.active = e.docs.len() - 1;
    e.watcher(wake).add(path);
    start_load(app, id, false);
}

fn limits(app: &AppCore) -> io::Limits {
    io::Limits {
        max_text_bytes: app.config.editor.max_text_mib.max(1) * 1024 * 1024,
        max_image_mp: app.config.editor.max_image_mp.max(1),
    }
}

fn start_load(app: &mut AppCore, doc: u64, reload: bool) {
    let limits = limits(app);
    let wake = app.wake.clone();
    let tx = app.editors.tx.clone();
    if let Some(d) = app.editors.docs.iter().find(|d| d.id == doc) {
        io::spawn_load(doc, d.path.clone(), d.want, limits, reload, tx, wake);
    }
}

/// Reopen a document as text or hex (discarding unsaved changes is asked by the UI).
pub fn reopen_as(app: &mut AppCore, idx: usize, want: Want) {
    let Some(d) = app.editors.docs.get_mut(idx) else { return };
    d.want = want;
    d.kind = DocKind::Loading;
    d.external = None;
    let id = d.id;
    start_load(app, id, false);
}

/// Save document `idx` (no-op while a save is running or nothing changed).
pub fn save(app: &mut AppCore, idx: usize) {
    let backup = app.config.editor.backup_to_trash;
    let quality = app.config.editor.jpeg_quality;
    let wake = app.wake.clone();
    let tx = app.editors.tx.clone();
    let Some(d) = app.editors.docs.get_mut(idx) else { return };
    if d.saving || !d.is_dirty() {
        return;
    }
    let job = match &mut d.kind {
        DocKind::Text(t) => {
            if t.read_only {
                return;
            }
            let (bytes, token) = t.save_snapshot();
            d.save_token = Some(token);
            io::SaveJob::Bytes(bytes)
        }
        DocKind::Hex(h) => io::SaveJob::Patches(h.patches.map().clone()),
        DocKind::Image(im) => {
            let mode = im.save_mode();
            if mode == image::ops::SaveMode::Unsupported {
                app.ui.toasts.push(Toast::warning(tr!("editors-image-format-readonly")));
                return;
            }
            io::SaveJob::Image { full: im.full.clone(), ops: im.ops.clone(), mode, quality }
        }
        _ => return,
    };
    d.saving = true;
    io::spawn_save(d.id, d.path.clone(), job, backup, tx, wake);
}

/// Close document `idx`, asking first when it has unsaved changes.
pub fn close(app: &mut AppCore, idx: usize, force: bool) {
    let Some(d) = app.editors.docs.get(idx) else { return };
    if d.is_dirty() && !force {
        let prompt = UnsavedPrompt::close(&d.name, d.id, app.editors.tx.clone());
        app.frontend.push(FrontendCmd::EditorUnsaved(prompt));
        return;
    }
    let d = app.editors.docs.remove(idx);
    if let Some(w) = &app.editors.watcher {
        w.remove(&d.path);
    }
    let e = &mut app.editors;
    if e.active >= e.docs.len() {
        e.active = e.docs.len().saturating_sub(1);
    } else if idx < e.active {
        e.active -= 1;
    }
    if e.docs.is_empty() {
        e.open = false;
        e.maximized = false;
        app.ui.focus = Focus::List;
    } else if let Some(d) = e.active_doc_mut() {
        d.request_focus();
    }
}

// ---- per-frame ----------------------------------------------------------------------

/// Apply worker results. Called every frame before drawing.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    while let Ok(msg) = app.editors.rx.try_recv() {
        match msg {
            Msg::Loaded { doc, reload, stamp, result } => on_loaded(app, doc, reload, stamp, result),
            Msg::Saved { doc, result, backup } => on_saved(app, doc, result, backup),
            Msg::Changed { path, stamp } => on_changed(app, &path, stamp),
            Msg::Close { doc, answer } => {
                let Some(idx) = app.editors.docs.iter().position(|d| d.id == doc) else { continue };
                match answer {
                    Answer::Save => {
                        app.editors.docs[idx].close_after_save = true;
                        save(app, idx);
                    }
                    Answer::Discard => close(app, idx, true),
                    Answer::Cancel => app.editors.docs[idx].request_focus(),
                }
            }
            Msg::Quit(answer) => {
                app.editors.prompting_quit = false;
                match answer {
                    Answer::Save => {
                        app.editors.quit_after_save = true;
                        for i in 0..app.editors.docs.len() {
                            save(app, i);
                        }
                    }
                    Answer::Discard => {
                        app.editors.quit_allowed = true;
                        host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Close);
                    }
                    Answer::Cancel => {}
                }
            }
        }
    }
    if app.editors.quit_after_save && !app.editors.docs.iter().any(|d| d.saving) {
        app.editors.quit_after_save = false;
        if app.editors.dirty_count() == 0 {
            app.editors.quit_allowed = true;
            host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Close);
        }
    }
}

fn on_loaded(app: &mut AppCore, doc: u64, reload: bool, stamp: Option<Stamp>, r: LoadResult) {
    let wake = app.wake.clone();
    let upload = app.editors.upload.clone();
    let Some(d) = app.editors.docs.iter_mut().find(|d| d.id == doc) else { return };
    d.stamp = stamp;
    d.external = None;
    match (r, &mut d.kind) {
        (LoadResult::Text(fresh), DocKind::Text(t)) if reload => t.reload_from(*fresh),
        (LoadResult::Hex { len, .. }, DocKind::Hex(h)) if reload => h.reload(len),
        (LoadResult::Image(l), DocKind::Image(i)) if reload => i.reload(l),
        (LoadResult::Text(fresh), _) => d.kind = DocKind::Text(fresh),
        (LoadResult::Hex { len, too_large }, _) => {
            let mut h = hex::HexDoc::new(d.path.clone(), len, wake);
            h.read_only = too_large;
            h.too_large_for_text = too_large;
            d.kind = DocKind::Hex(Box::new(h));
        }
        (LoadResult::Image(l), _) => {
            d.kind = DocKind::Image(Box::new(image::ImageDoc::new(d.path.clone(), l, wake, upload)))
        }
        (LoadResult::Error(e), _) => {
            if reload {
                app.ui.toasts.push(Toast::error(tr!("editors-reload-failed", name = &d.name)).body(e));
            } else {
                d.kind = DocKind::Failed(e);
            }
        }
    }
}

fn on_saved(app: &mut AppCore, doc: u64, result: Result<Option<Stamp>, String>, backup: Option<String>) {
    let Some(idx) = app.editors.docs.iter().position(|d| d.id == doc) else { return };
    let d = &mut app.editors.docs[idx];
    d.saving = false;
    match result {
        Ok(stamp) => {
            d.stamp = stamp;
            d.external = None;
            let token = d.save_token.take();
            let mut reload = false;
            match &mut d.kind {
                DocKind::Text(t) => {
                    if let Some(tok) = token {
                        t.saved(tok);
                    }
                }
                DocKind::Hex(h) => {
                    h.patches.clear();
                    h.reload(stamp.map_or(h.len, |s| s.0));
                }
                DocKind::Image(_) => reload = true,
                _ => {}
            }
            let name = d.name.clone();
            let close_now = d.close_after_save;
            if reload {
                let id = d.id;
                start_load(app, id, true);
            }
            let mut t = Toast::success(tr!("editors-saved", name = &name));
            if let Some(note) = backup {
                t = Toast::warning(tr!("editors-saved", name = &name))
                    .body(tr!("editors-saved-no-backup", note = note));
            }
            t.duration = Some(2.0);
            app.ui.toasts.push_replacing(t);
            if close_now {
                close(app, idx, true);
            }
        }
        Err(e) => {
            d.close_after_save = false;
            d.save_token = None;
            let name = d.name.clone();
            app.editors.quit_after_save = false;
            app.ui.toasts.push(Toast::error(tr!("editors-save-failed", name = name)).body(e));
        }
    }
}

fn on_changed(app: &mut AppCore, path: &Path, stamp: Option<Stamp>) {
    let Some(d) = app.editors.docs.iter_mut().find(|d| d.path == path) else { return };
    if d.saving || d.stamp == stamp || matches!(d.kind, DocKind::Loading) {
        return;
    }
    match stamp {
        None => d.external = Some(External::Deleted),
        Some(_) if d.is_dirty() => d.external = Some(External::Changed),
        Some(_) => {
            let id = d.id;
            let name = d.name.clone();
            start_load(app, id, true);
            let mut t = Toast::info(tr!("editors-reloaded", name = name)).body(tr!("editors-reloaded-body"));
            t.duration = Some(2.5);
            app.ui.toasts.push_replacing(t);
        }
    }
}

/// Reload document `idx` from disk (drops unsaved changes).
pub fn reload(app: &mut AppCore, idx: usize) {
    if let Some(d) = app.editors.docs.get_mut(idx) {
        d.external = None;
        let id = d.id;
        start_load(app, id, true);
    }
}

// ---- view ----------------------------------------------------------------------------

/// Whether the editor replaces the file panes this frame.
pub fn maximized(app: &AppCore) -> bool {
    app.editors.open && app.editors.maximized
}

// ---- keyboard -----------------------------------------------------------------------

/// Chords the editor keeps for itself while it has focus (everything else reaches the
/// app's global bindings).
pub fn editor_reserved(c: KeyChord) -> bool {
    let ctrl = c.mods.intersects(Mods::CTRL | Mods::CMD);
    let alt = c.mods.contains(Mods::ALT);
    if !ctrl && !alt {
        // Typing, navigation and editing keys; plain F-keys go to the app.
        return !matches!(
            c.key,
            KeyName::F1
                | KeyName::F2
                | KeyName::F4
                | KeyName::F5
                | KeyName::F6
                | KeyName::F7
                | KeyName::F8
                | KeyName::F9
                | KeyName::F10
                | KeyName::F11
                | KeyName::F12
        );
    }
    if ctrl && !alt {
        return matches!(
            c.key,
            KeyName::A
                | KeyName::C
                | KeyName::X
                | KeyName::V
                | KeyName::Z
                | KeyName::Y
                | KeyName::D
                | KeyName::Home
                | KeyName::End
                | KeyName::ArrowLeft
                | KeyName::ArrowRight
                | KeyName::ArrowUp
                | KeyName::ArrowDown
                | KeyName::Backspace
                | KeyName::Delete
                | KeyName::Enter
                | KeyName::Insert
        );
    }
    false
}

/// A key the editor takes for itself.
enum EdKey {
    /// A command bound in `KeyContext::Editor` ([`commands::binding`]).
    Command(Action),
    Escape,
}

/// What the frontend does after [`handle_keys`].
#[derive(Debug, Default)]
pub struct EditorKeys {
    /// Esc had nothing to close: drop keyboard focus (back to the file list).
    pub release_focus: bool,
    /// Global bindings to run (after the consumed events are removed).
    pub actions: Vec<Action>,
}

/// Keyboard dispatch while the editor has focus: the editor's commands first (bound in
/// `KeyContext::Editor`, [`commands`]; Esc closes a bar or leaves the editor), then global
/// app bindings that the editor does not reserve. Marks the chords it takes in `frame`.
/// Editor commands that change the config come back in [`EditorKeys::actions`].
pub fn handle_keys(app: &mut AppCore, frame: &mut InputFrame) -> EditorKeys {
    let mut ed: Vec<EdKey> = Vec::new();
    let mut out = EditorKeys::default();
    let km = &app.keymap;
    for i in 0..frame.events.len() {
        if frame.consumed[i] {
            continue;
        }
        let InputEvent::Chord(chord) = &frame.events[i] else { continue };
        let chord = *chord;
        let k = match commands::binding(km, chord) {
            Some(a) => Some(EdKey::Command(a)),
            None if chord.key == KeyName::Escape && chord.mods.is_empty() => Some(EdKey::Escape),
            None => None,
        };
        if let Some(k) = k {
            ed.push(k);
            frame.consumed[i] = true;
            continue;
        }
        if editor_reserved(chord) {
            continue;
        }
        if let Some(a) = km.lookup(KeyContext::Global, chord) {
            out.actions.push(a.clone());
            frame.consumed[i] = true;
        }
    }
    let idx = app.editors.active;
    for k in ed {
        match k {
            EdKey::Command(a) if commands::needs_host(&a) => out.actions.push(a),
            EdKey::Command(a) => commands::run(app, &a),
            EdKey::Escape => {
                let handled = match app.editors.docs.get_mut(idx).map(|d| &mut d.kind) {
                    Some(DocKind::Text(t)) => t.close_bars(),
                    Some(DocKind::Hex(h)) => h.close_bars(),
                    Some(DocKind::Image(i)) => i.close_bars(),
                    _ => false,
                };
                if !handled {
                    // Esc with nothing to close: back to the file list.
                    app.ui.focus = Focus::List;
                    out.release_focus = true;
                }
            }
        }
    }
    out
}

// ---- window close / session -------------------------------------------------------

/// Window close with unsaved documents: cancel it and ask. Returns `true` when the
/// close request was taken over.
pub fn intercept_close(app: &mut AppCore, host: &dyn Host, close_requested: bool) -> bool {
    if !close_requested || app.editors.quit_allowed {
        return false;
    }
    let dirty = app.editors.dirty_count();
    if dirty == 0 {
        return false;
    }
    host.window(crate::host::WindowId::Current, crate::host::WindowCmd::CancelClose);
    if !app.editors.prompting_quit {
        app.editors.prompting_quit = true;
        host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Minimized(false));
        host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Focus);
        let names: Vec<String> = app.editors.docs.iter().filter(|d| d.is_dirty()).map(|d| d.name.clone()).collect();
        app.frontend.push(FrontendCmd::EditorUnsaved(UnsavedPrompt::quit(&names, app.editors.tx.clone())));
    }
    true
}

/// Paths of the open documents (session).
pub fn session_paths(app: &AppCore) -> Vec<PathBuf> {
    app.editors.docs.iter().map(|d| d.path.clone()).collect()
}

/// Reopen the documents of the last session (missing files are skipped).
pub fn restore(app: &mut AppCore, paths: &[PathBuf]) {
    for p in paths.iter().filter(|p| p.is_file()).take(MAX_OPEN_AT_ONCE) {
        open_path(app, p, Want::Auto);
    }
    if !app.editors.docs.is_empty() {
        app.editors.active = 0;
    }
}

// ---- "save changes?" ---------------------------------------------------------------

/// "Save changes?" for one document or for the whole window: the frontend shows it
/// ([`FrontendCmd::EditorUnsaved`]) and calls [`UnsavedPrompt::answer`] once; a prompt
/// dismissed without an answer must answer [`Answer::Cancel`].
#[derive(Clone, Debug)]
pub struct UnsavedPrompt {
    pub title: String,
    pub body: String,
    /// The document to close (`None`: quitting).
    pub target: Option<u64>,
    tx: Sender<Msg>,
}

impl UnsavedPrompt {
    pub fn close(name: &str, doc: u64, tx: Sender<Msg>) -> Self {
        UnsavedPrompt {
            title: tr!("editors-unsaved-title", name = name),
            body: tr!("editors-unsaved-body").into(),
            target: Some(doc),
            tx,
        }
    }

    pub fn quit(names: &[String], tx: Sender<Msg>) -> Self {
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
        UnsavedPrompt { title, body, target: None, tx }
    }

    /// Deliver the answer (applied by the next [`pump`]).
    pub fn answer(&self, a: Answer) {
        let _ = self.tx.send(match self.target {
            Some(doc) => Msg::Close { doc, answer: a },
            None => Msg::Quit(a),
        });
    }
}

// ---- SOYUZ_SCRIPT hooks -------------------------------------------------------------

/// `edit:<path>` opens a file; `editor:<command>` drives the active document:
/// `save`, `close`, `type=<text>`, `goto=<line>`, `find=<text>`, `hex=<offset>=<byte>`,
/// `image=<rr|rl|fh|fv>`, `maximize`, `dock=<right|bottom>`.
pub fn devscript(app: &mut AppCore, host: &dyn Host, verb: &str, arg: &str) {
    let arg = arg.trim();
    if verb == "edit" {
        open_path(app, Path::new(arg), Want::Auto);
        return;
    }
    let (cmd, val) = arg.split_once('=').unwrap_or((arg, ""));
    let idx = app.editors.active;
    match cmd {
        "save" => save(app, idx),
        "close" => close(app, idx, false),
        "maximize" => app.editors.maximized = !app.editors.maximized,
        "dock" => {
            let dock = if val == "bottom" {
                soyuz_core::config::EditorDock::Bottom
            } else {
                soyuz_core::config::EditorDock::Right
            };
            crate::models::settings::update_config(app, host, |c| c.editor.dock = dock);
        }
        _ => {
            let Some(d) = app.editors.docs.get_mut(idx) else { return };
            match (&mut d.kind, cmd) {
                (DocKind::Text(t), "type") => t.devscript_type(&val.replace("\\n", "\n")),
                (DocKind::Text(t), "goto") => t.goto_line(val.parse().unwrap_or(1), 1),
                (DocKind::Text(t), "find") => t.devscript_find(val),
                (DocKind::Hex(h), "hex") => {
                    if let Some((off, b)) = val.split_once('=')
                        && let (Some(off), Ok(b)) = (hex::model::parse_offset(off, 0), u8::from_str_radix(b, 16))
                    {
                        h.devscript_set(off, b);
                    }
                }
                (DocKind::Image(i), "image") => {
                    let op = match val {
                        "rl" => image::ops::ImgOp::RotateCcw,
                        "fh" => image::ops::ImgOp::FlipH,
                        "fv" => image::ops::ImgOp::FlipV,
                        _ => image::ops::ImgOp::RotateCw,
                    };
                    i.push(op);
                }
                _ => log::warn!("SOYUZ_SCRIPT editor:{arg}: not applicable"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_chords() {
        let c = |m: Mods, k: KeyName| editor_reserved(KeyChord::new(m, k));
        assert!(c(Mods::empty(), KeyName::A), "typing");
        assert!(c(Mods::SHIFT, KeyName::ArrowLeft));
        assert!(c(Mods::CTRL, KeyName::Z), "undo stays in the editor");
        assert!(c(Mods::CTRL, KeyName::V));
        assert!(c(Mods::CTRL | Mods::SHIFT, KeyName::Z), "redo");
        assert!(!c(Mods::CTRL | Mods::SHIFT, KeyName::P), "the palette still opens");
        assert!(!c(Mods::CTRL, KeyName::Backtick), "terminal toggle");
        assert!(!c(Mods::CTRL, KeyName::Tab));
        assert!(!c(Mods::empty(), KeyName::F9));
        assert!(!c(Mods::ALT, KeyName::ArrowLeft));
    }
}
