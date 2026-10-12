//! Integrated terminal sessions (frontend-agnostic): the session list per owner (shared
//! or per tab), which session is active, keyboard ownership, following the pane's folder
//! (`send_cd`, refused while the shell is busy) and the reverse sync from the shell's
//! reported cwd, OSC 52 clipboard requests and terminal links. `ui::panels::terminal`
//! draws the session strip and the terminal view.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_core::config::{KeyChord, KeyName, Mods, Osc52Policy};
use soyuz_term::{CursorStyle, TermEvent, TermSettings, Terminal};

use crate::cmd::Cmd;
use crate::core::{AppCore, Focus};
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;

/// How long a refused `cd` keeps being retried.
const CD_RETRY: Duration = Duration::from_secs(3);

pub struct Session {
    pub id: u64,
    pub term: Option<Terminal>,
    pub error: Option<String>,
    /// Title reported by the shell (often the cwd with shell integration).
    pub title: String,
    pub custom: Option<String>,
    /// Owning tab in per-tab mode.
    pub tab: Option<u64>,
    /// Directory we last told the shell to enter (or it reported).
    pub last_dir: Option<PathBuf>,
    pub pending_cd: Option<(PathBuf, Instant)>,
    pub busy_since: Option<Instant>,
}

impl Session {
    pub fn label(&self) -> String {
        if let Some(c) = &self.custom {
            return c.clone();
        }
        title_label(&self.title).unwrap_or_else(|| tr!("panels-terminal-default-name", number = self.id.to_string()))
    }

    pub fn is_alive(&self) -> bool {
        self.term.as_ref().is_some_and(Terminal::is_alive)
    }

    /// The shell refused a `cd` in the last three seconds.
    pub fn is_busy(&self) -> bool {
        self.busy_since.is_some_and(|t| t.elapsed() < Duration::from_secs(3))
    }
}

/// Short label from a shell title: the last path segment when the title is a path.
pub fn title_label(title: &str) -> Option<String> {
    let t = title.trim();
    if t.is_empty() {
        return None;
    }
    let looks_path = t.contains('\\') || t.starts_with('/') || t.starts_with('~') || t.get(1..3) == Some(":\\");
    if looks_path {
        let trimmed = t.trim_end_matches(['\\', '/']);
        let last = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
        return Some(if last.is_empty() || last.ends_with(':') {
            trimmed.to_string() + "\\"
        } else {
            last.to_string()
        });
    }
    // "Administrator: Windows PowerShell" → keep, but trim long titles.
    Some(if t.chars().count() > 28 { t.chars().take(27).collect::<String>() + "…" } else { t.to_string() })
}

#[derive(Default)]
pub struct TerminalPanel {
    pub sessions: Vec<Session>,
    /// Active session id per owner (tab id, or 0 when shared).
    active: Vec<(u64, u64)>,
    /// The terminal owns keyboard input.
    pub focused: bool,
    /// Session being renamed and the edited text.
    pub renaming: Option<(u64, String)>,
    /// Last focused-pane folder seen (to detect navigation).
    last_pane_dir: Option<(u64, PathBuf)>,
    next_id: u64,
    /// Ask the host to close the bottom panel (the last session went away).
    request_close: bool,
    /// OSC 52 clipboard write waiting for an answer (`terminal.osc52 = "ask"`); a newer
    /// request replaces an unanswered one.
    pub clip_request: Option<String>,
    /// "Always for this session" was picked for OSC 52 writes.
    pub clip_allow_session: bool,
}

impl TerminalPanel {
    pub fn owner(per_tab: bool, tab: u64) -> Option<u64> {
        per_tab.then_some(tab)
    }

    pub fn visible_ids(&self, owner: Option<u64>) -> Vec<u64> {
        self.sessions.iter().filter(|s| s.tab == owner).map(|s| s.id).collect()
    }

    pub fn active_id(&self, owner: Option<u64>) -> Option<u64> {
        let key = owner.unwrap_or(0);
        let ids = self.visible_ids(owner);
        self.active
            .iter()
            .find(|(o, _)| *o == key)
            .map(|(_, id)| *id)
            .filter(|id| ids.contains(id))
            .or(ids.last().copied())
    }

    pub fn set_active(&mut self, owner: Option<u64>, id: u64) {
        let key = owner.unwrap_or(0);
        self.active.retain(|(o, _)| *o != key);
        self.active.push((key, id));
    }

    pub fn session(&self, id: u64) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn session_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// A tab was closed: drop (and thereby kill) the shells it owned in per-tab mode.
    /// Shared sessions (`tab == None`) are untouched.
    pub fn drop_tab(&mut self, tab: u64) {
        self.sessions.retain(|s| s.tab != Some(tab));
        self.active.retain(|(o, _)| *o != tab);
        if self.last_pane_dir.as_ref().is_some_and(|(t, _)| *t == tab) {
            self.last_pane_dir = None;
        }
    }
}

/// The session owner of the focused tab (`None` = shared).
pub fn current_owner(app: &AppCore) -> Option<u64> {
    TerminalPanel::owner(app.config.terminal.per_tab, app.ws.tab().id.0)
}

fn settings_for(app: &AppCore, cwd: &Path) -> TermSettings {
    let c = &app.config.terminal;
    TermSettings {
        shell: (!c.shell.trim().is_empty()).then(|| c.shell.clone()),
        args: c.args.clone(),
        cwd: soyuz_fs::strip_verbatim(cwd),
        env: vec![("TERM_PROGRAM".into(), "soyuz".into())],
        scrollback: c.scrollback.max(100),
        cursor: CursorStyle::Block,
        shell_integration: c.shell_integration,
    }
}

/// The focused pane's real folder (terminals start / follow there).
pub fn pane_dir(app: &AppCore) -> Option<PathBuf> {
    match &app.ws.pane().location {
        Location::Dir(p) => Some(p.clone()),
        Location::Archive { archive, .. } => archive.parent().map(Path::to_path_buf),
        _ => None,
    }
}

/// Start a shell in `cwd` and make it the active session; returns its id.
pub fn spawn_session(app: &mut AppCore, host: &dyn Host, cwd: PathBuf) -> u64 {
    let owner = current_owner(app);
    let t = &mut app.ui.terminal;
    t.next_id += 1;
    // Widget ids must be unique per live terminal: mix in a process-wide counter.
    static UNIQUE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let unique = UNIQUE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let id = t.next_id;
    let settings = settings_for(app, &cwd);
    let (term, error) = match Terminal::spawn(settings, host.wake(), 0x5052_0000 + unique) {
        Ok(term) => (Some(term), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let t = &mut app.ui.terminal;
    t.sessions.push(Session {
        id,
        term,
        error,
        title: String::new(),
        custom: None,
        tab: owner,
        last_dir: Some(cwd),
        pending_cd: None,
        busy_since: None,
    });
    t.set_active(owner, id);
    id
}

/// A new session in the focused pane's folder (the "+" button), focused.
pub fn new_session(app: &mut AppCore, host: &dyn Host) {
    let cwd = pane_dir(app).unwrap_or_else(soyuz_fs::home);
    spawn_session(app, host, cwd);
    app.ui.terminal.focused = true;
}

/// Ctrl+`: open (spawning lazily) and focus; focus when open but unfocused; else close.
pub fn toggle(app: &mut AppCore, host: &dyn Host) {
    if app.ui.bottom.terminal_shown() && app.ui.terminal.focused {
        app.ui.bottom.open = false;
        unfocus(app);
        return;
    }
    focus(app, host);
}

/// Focus the terminal (FocusTerminal action), opening it when needed.
pub fn focus(app: &mut AppCore, host: &dyn Host) {
    app.ui.bottom.show_terminal();
    ensure_session(app, host);
    app.ui.terminal.focused = true;
}

/// Give the keyboard back to the file list.
pub fn unfocus(app: &mut AppCore) {
    app.ui.terminal.focused = false;
    app.ui.focus = Focus::List;
    app.frontend.push(FrontendCmd::ReleaseFocus);
}

/// At least one session for the focused tab.
pub fn ensure_session(app: &mut AppCore, host: &dyn Host) {
    if app.ui.terminal.visible_ids(current_owner(app)).is_empty() {
        let cwd = pane_dir(app).unwrap_or_else(soyuz_fs::home);
        spawn_session(app, host, cwd);
    }
}

/// "Open terminal here": a new integrated session in `dir`.
pub fn open_at(app: &mut AppCore, host: &dyn Host, dir: PathBuf) {
    app.ui.bottom.show_terminal();
    spawn_session(app, host, dir);
    app.ui.terminal.focused = true;
}

/// Run a shell command line in `dir` with its output in the terminal panel: typed into
/// the active session when it is idle (after a `cd`), otherwise into a new session.
pub fn run_command(app: &mut AppCore, host: &dyn Host, dir: PathBuf, line: &str) {
    // Typed text goes through the shell's line editor: a newline/escape in a (crafted)
    // file name would run extra commands no matter how it is quoted.
    if soyuz_term::has_control_chars(line) || soyuz_term::has_control_chars(&dir.to_string_lossy()) {
        app.ui.toasts.push(
            Toast::error(tr!("panels-terminal-command-not-run")).body(tr!("panels-terminal-command-not-run-body")),
        );
        return;
    }
    app.ui.bottom.show_terminal();
    let owner = current_owner(app);
    let reuse = app.ui.terminal.active_id(owner).filter(|id| {
        app.ui.terminal.sessions.iter().any(|s| {
            s.id == *id && s.is_alive() && s.term.as_ref().is_some_and(|t| !t.mode_alt_screen() && t.send_cd(&dir))
        })
    });
    let id = match reuse {
        Some(id) => id,
        None => spawn_session(app, host, dir.clone()),
    };
    if let Some(s) = app.ui.terminal.session_mut(id) {
        s.last_dir = Some(dir);
        if let Some(t) = &s.term {
            t.write(format!("{line}\r").as_bytes());
        }
    }
    app.ui.terminal.focused = true;
}

/// Close (and kill) session `id`; the panel closes with the last session of the tab.
pub fn close_session(app: &mut AppCore, id: u64) {
    let t = &mut app.ui.terminal;
    if let Some(i) = t.sessions.iter().position(|s| s.id == id) {
        let s = t.sessions.remove(i);
        drop(s); // kills the shell
    }
    if app.ui.terminal.visible_ids(current_owner(app)).is_empty() {
        app.ui.terminal.request_close = true;
        unfocus(app);
    }
}

/// Make `id` the active session of the focused tab and focus it.
pub fn activate(app: &mut AppCore, id: u64) {
    let owner = current_owner(app);
    app.ui.terminal.set_active(owner, id);
    app.ui.terminal.focused = true;
}

/// Finish renaming session `id` (an empty name restores the shell's title).
pub fn rename(app: &mut AppCore, id: u64, text: &str) {
    let t = text.trim().to_string();
    if let Some(s) = app.ui.terminal.session_mut(id) {
        s.custom = (!t.is_empty()).then_some(t);
    }
    app.ui.terminal.renaming = None;
}

/// "Open in external terminal" for the active session's folder.
pub fn open_external(app: &mut AppCore, active: u64) {
    let dir = app
        .ui
        .terminal
        .session(active)
        .and_then(|s| s.last_dir.clone())
        .or_else(|| pane_dir(app))
        .unwrap_or_else(soyuz_fs::home);
    app.cmds.push(Cmd::TerminalAt(dir));
}

/// Apply panel requests made while drawing (the host lends the panel state out).
pub fn apply_requests(app: &mut AppCore) {
    if std::mem::take(&mut app.ui.terminal.request_close)
        && app.ui.bottom.tab == crate::models::panels::BottomTab::Terminal
    {
        app.ui.bottom.open = false;
    }
}

/// Chords the app may take while the terminal is focused.
pub fn terminal_passthrough(chord: KeyChord) -> bool {
    let ctrl = chord.mods.contains(Mods::CTRL);
    let shift = chord.mods.contains(Mods::SHIFT);
    let alt = chord.mods.contains(Mods::ALT);
    (ctrl && matches!(chord.key, KeyName::Backtick | KeyName::Tab | KeyName::PageUp | KeyName::PageDown))
        || (ctrl && shift && !matches!(chord.key, KeyName::C | KeyName::V))
        || (alt && !ctrl && matches!(chord.key, KeyName::P))
}

/// Per frame (even while hidden): drain shell events, follow pane navigation, apply
/// reverse sync.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    if app.ui.terminal.sessions.is_empty() {
        return;
    }
    let mut exited = Vec::new();
    let mut navigate: Option<PathBuf> = None;
    let follow_shell = app.config.terminal.follow_shell;
    let mut clip_writes = Vec::new();
    for s in &mut app.ui.terminal.sessions {
        let Some(term) = &s.term else { continue };
        for ev in term.events().try_iter() {
            match ev {
                TermEvent::Title(t) => s.title = t,
                TermEvent::Exited(_) => exited.push(s.id),
                TermEvent::CwdChanged(dir) => {
                    let changed = s.last_dir.as_deref() != Some(dir.as_path());
                    s.last_dir = Some(dir.clone());
                    if follow_shell && changed {
                        navigate = Some(dir);
                    }
                }
                // Untrusted (any program's output can ask): see `terminal.osc52`.
                TermEvent::Clipboard(text) => clip_writes.push(text),
                TermEvent::Bell => {}
            }
        }
    }
    for text in clip_writes {
        match osc52_decision(app.config.terminal.osc52, app.ui.terminal.clip_allow_session) {
            ClipDecision::Copy => host.copy_text(text),
            ClipDecision::Drop => {}
            ClipDecision::Ask => {
                app.ui.terminal.clip_request = Some(text);
                host.redraw();
            }
        }
    }
    for id in exited {
        close_session(app, id);
    }
    apply_requests(app);
    let per_tab = app.config.terminal.per_tab;
    let tab = app.ws.tab().id.0;
    let owner = TerminalPanel::owner(per_tab, tab);
    let active = app.ui.terminal.active_id(owner);
    // Reverse sync: shell cd → pane (only from the active session).
    if let (Some(dir), Some(_)) = (navigate, active)
        && let Location::Dir(cur) = &app.ws.pane().location
        && !same_dir(cur, &dir)
    {
        // The existence check runs on the lister thread; it navigates when `dir` is a folder.
        let pane = app.ws.pane().id;
        app.ui.terminal.last_pane_dir = Some((tab, dir.clone()));
        app.services.lister.probe(pane, &dir);
    }
    // Follow navigation: pane → shell.
    if app.config.terminal.follow_navigation
        && let Some(dir) = pane_dir(app)
    {
        let prev = app.ui.terminal.last_pane_dir.clone();
        let moved = prev.as_ref().is_none_or(|(_, d)| !same_dir(d, &dir));
        let tab_switch = prev.as_ref().is_some_and(|(t, _)| *t != tab);
        if moved {
            app.ui.terminal.last_pane_dir = Some((tab, dir.clone()));
            let follow = !tab_switch || app.config.terminal.cd_on_tab_switch || per_tab;
            if follow
                && let Some(id) = active
                && let Some(s) = app.ui.terminal.session_mut(id)
                && !s.last_dir.as_deref().is_some_and(|d| same_dir(d, &dir))
                // `send_cd` refuses these anyway; don't flag the shell busy.
                && !soyuz_term::has_control_chars(&dir.to_string_lossy())
            {
                s.pending_cd = Some((dir, Instant::now()));
            }
        }
    }
    for s in &mut app.ui.terminal.sessions {
        let Some((dir, since)) = s.pending_cd.clone() else { continue };
        let Some(term) = &s.term else { continue };
        if term.send_cd(&soyuz_fs::strip_verbatim(&dir)) {
            s.last_dir = Some(dir);
            s.pending_cd = None;
            s.busy_since = None;
        } else if since.elapsed() > CD_RETRY {
            s.pending_cd = None;
            s.busy_since = Some(Instant::now());
        } else {
            host.redraw_after(Duration::from_millis(150));
        }
    }
}

/// What to do with an OSC 52 clipboard write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipDecision {
    Copy,
    Drop,
    Ask,
}

pub fn osc52_decision(policy: Osc52Policy, allowed_this_session: bool) -> ClipDecision {
    match policy {
        Osc52Policy::Allow => ClipDecision::Copy,
        Osc52Policy::Deny => ClipDecision::Drop,
        Osc52Policy::Ask if allowed_this_session => ClipDecision::Copy,
        Osc52Policy::Ask => ClipDecision::Ask,
    }
}

/// The user answered the pending OSC 52 request: copy (`always` = for the rest of the
/// session) or drop it.
pub fn answer_clip(app: &mut AppCore, host: &dyn Host, copy: bool, always: bool) {
    let Some(text) = app.ui.terminal.clip_request.take() else { return };
    if copy {
        host.copy_text(text);
        app.ui.terminal.clip_allow_session |= always;
    }
}

/// Characters of a clipboard request shown in the prompt.
const CLIP_PREVIEW_CHARS: usize = 80;

/// One-line preview of an OSC 52 payload: the first 80 characters, line breaks and tabs
/// shown as `⏎` / `⇥`, other hidden controls as `⟨U+XXXX⟩` tokens, `…` when cut.
pub fn clipboard_preview(text: &str) -> String {
    let head: String = text.chars().take(CLIP_PREVIEW_CHARS).collect();
    let cut = text.chars().nth(CLIP_PREVIEW_CHARS).is_some();
    let head = head.replace("\r\n", "\n").replace(['\n', '\r'], "\u{23CE}").replace('\t', "\u{21E5}");
    let mut out = soyuz_core::naming::reveal_controls(&head).into_owned();
    if cut {
        out.push('\u{2026}');
    }
    out
}

pub fn same_dir(a: &Path, b: &Path) -> bool {
    let (a, b) = (soyuz_fs::strip_verbatim(a), soyuz_fs::strip_verbatim(b));
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    if cfg!(windows) { norm(&a).eq_ignore_ascii_case(&norm(&b)) } else { norm(&a) == norm(&b) }
}

/// A web link (`http(s)://`, `mailto:`): the frontend opens it in the browser.
pub fn is_web_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

/// Ctrl+click on a path link: folders open in the pane, files with the default app.
/// Relative paths resolve against the session's working directory (not the app's), and
/// executables are revealed rather than run: terminal output is untrusted text.
pub fn open_path_link(app: &mut AppCore, url: &str, base: Option<&Path>) {
    let Some(path) = resolve_link(url, base) else {
        app.ui.toasts.push(Toast::info(tr!("panels-terminal-link-not-found")).body(url.to_string()));
        return;
    };
    // Classify the target on a worker thread (no disk access on the UI thread).
    let postman = app.services.mailbox.postman();
    let url = url.to_string();
    let spawned = std::thread::Builder::new().name("soyuz-term-link".into()).spawn(move || {
        use crate::services::mailbox::UiMsg;
        if path.is_dir() {
            postman.send(UiMsg::OpenDir(path));
        } else if path.exists() {
            if looks_executable(&path) {
                postman.send(UiMsg::Notice(
                    Toast::info(tr!("panels-terminal-exe-revealed")).body(tr!("panels-terminal-exe-revealed-body")),
                ));
                if let Err(e) = soyuz_fs::launch::reveal_in_system(&path) {
                    postman.send(UiMsg::Notice(Toast::error(tr!("panels-terminal-reveal-failed")).body(e.to_string())));
                }
            } else {
                postman.send(UiMsg::Launch(path));
            }
        } else {
            postman.send(UiMsg::Notice(Toast::info(tr!("panels-terminal-link-not-found")).body(url)));
        }
    });
    if let Err(e) = spawned {
        app.ui.toasts.push(Toast::error(tr!("panels-terminal-link-failed")).body(e.to_string()));
    }
}

/// The filesystem path a terminal link points at. `None` for a relative link when the
/// session's directory is unknown (never resolve against the app's own cwd).
pub fn resolve_link(url: &str, base: Option<&Path>) -> Option<PathBuf> {
    let raw = url.strip_prefix("file://").unwrap_or(url);
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        return Some(p);
    }
    // `C:foo` / `oo` style partial paths are neither absolute nor safely relative.
    if p.has_root() || matches!(p.components().next(), Some(std::path::Component::Prefix(_))) {
        return None;
    }
    base.filter(|b| b.is_absolute()).map(|b| b.join(p))
}

/// Files that would *run* something when opened with the default handler.
pub fn looks_executable(path: &Path) -> bool {
    const EXTS: &[&str] = &[
        "exe",
        "com",
        "bat",
        "cmd",
        "ps1",
        "psm1",
        "msi",
        "msp",
        "scr",
        "pif",
        "cpl",
        "vbs",
        "vbe",
        "js",
        "jse",
        "wsf",
        "wsh",
        "hta",
        "lnk",
        "url",
        "jar",
        "reg",
        "inf",
        "appref-ms",
        "application",
        "msc",
        "sh",
        "bash",
        "zsh",
        "fish",
        "command",
        "desktop",
        "appimage",
        "run",
        "py",
        "pyw",
        "pl",
        "rb",
        "app",
        "pkg",
        "deb",
        "rpm",
    ];
    let ext_hit =
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0) {
            return true;
        }
    }
    ext_hit
}

/// Whether a modifier-less key event targets the terminal (used to keep type-ahead off).
pub fn owns_keyboard(app: &AppCore) -> bool {
    app.ui.terminal.focused && app.ui.bottom.terminal_shown()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_policy_decisions() {
        assert_eq!(osc52_decision(Osc52Policy::Ask, false), ClipDecision::Ask);
        assert_eq!(osc52_decision(Osc52Policy::Ask, true), ClipDecision::Copy);
        assert_eq!(osc52_decision(Osc52Policy::Deny, true), ClipDecision::Drop);
        assert_eq!(osc52_decision(Osc52Policy::Deny, false), ClipDecision::Drop);
        assert_eq!(osc52_decision(Osc52Policy::Allow, false), ClipDecision::Copy);
        assert_eq!(soyuz_core::config::Config::default().terminal.osc52, Osc52Policy::Ask, "asks by default");
    }

    #[test]
    fn clipboard_preview_is_one_short_line() {
        assert_eq!(clipboard_preview("ls -la"), "ls -la");
        assert_eq!(clipboard_preview("a\r\nb\tc"), "a\u{23CE}b\u{21E5}c");
        assert_eq!(clipboard_preview("x\u{1b}[31m"), "x\u{27E8}U+001B\u{27E9}[31m");
        let p = clipboard_preview(&"y".repeat(200));
        assert_eq!(p.chars().count(), 81);
        assert!(p.ends_with('\u{2026}'));
        assert_eq!(clipboard_preview(&"z".repeat(80)), "z".repeat(80));
    }

    #[test]
    fn title_labels() {
        assert_eq!(title_label(r"C:\Users\me\src").as_deref(), Some("src"));
        assert_eq!(title_label("/home/me/").as_deref(), Some("me"));
        assert_eq!(title_label(r"C:\").as_deref(), Some(r"C:\"));
        assert_eq!(title_label("vim").as_deref(), Some("vim"));
        assert_eq!(title_label("  "), None);
        assert!(title_label(&"x".repeat(60)).unwrap().ends_with('…'));
    }

    #[test]
    fn passthrough_keeps_shell_keys() {
        let c = |m: Mods, k: KeyName| terminal_passthrough(KeyChord::new(m, k));
        assert!(c(Mods::CTRL, KeyName::Backtick));
        assert!(c(Mods::CTRL, KeyName::Tab));
        assert!(c(Mods::CTRL | Mods::SHIFT, KeyName::P));
        assert!(c(Mods::ALT, KeyName::P));
        assert!(!c(Mods::CTRL, KeyName::R), "Ctrl+R is reverse search in the shell");
        assert!(!c(Mods::CTRL, KeyName::C));
        assert!(!c(Mods::CTRL | Mods::SHIFT, KeyName::C), "copy stays with the terminal");
        assert!(!c(Mods::empty(), KeyName::F5));
        assert!(!c(Mods::empty(), KeyName::Escape));
    }

    #[test]
    fn dirs_compare_loosely() {
        assert!(same_dir(Path::new("C:/a/b/"), Path::new("C:/a/b")));
        #[cfg(windows)]
        assert!(same_dir(Path::new(r"C:\A\b"), Path::new(r"c:\a\B")));
        assert!(!same_dir(Path::new("/a/b"), Path::new("/a/c")));
    }

    #[test]
    fn links_resolve_against_session_cwd() {
        let base = std::env::temp_dir();
        assert_eq!(resolve_link("src/main.rs", Some(&base)), Some(base.join("src/main.rs")));
        assert_eq!(resolve_link("src/main.rs", None), None, "never the app's cwd");
        assert_eq!(resolve_link("rel", Some(Path::new("relative/base"))), None);
        let abs = base.join("x.txt");
        assert_eq!(resolve_link(&abs.to_string_lossy(), None), Some(abs.clone()));
        #[cfg(windows)]
        assert_eq!(resolve_link("C:evil.exe", Some(&base)), None);
        assert!(looks_executable(Path::new("setup.EXE")));
        assert!(looks_executable(Path::new("x.ps1")));
        assert!(looks_executable(Path::new("run.sh")));
        assert!(!looks_executable(Path::new("notes.txt")));
        assert!(is_web_link("HTTPS://x") && is_web_link("mailto:a@b") && !is_web_link("C:\\x"));
    }
}
