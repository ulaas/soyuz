//! Integrated terminal (PLAN M8, app side): lives in the bottom panel's Terminal tab,
//! spawned lazily on first open (Ctrl+`). Several sessions per panel with mini-tabs
//! (new / close / rename, title from the shell); sessions are shared by all tabs or kept
//! per tab (`[terminal] per_tab`). The active session follows the focused pane's folder
//! through `send_cd` (which refuses while the shell is busy), and can optionally drive
//! the pane from the shell's reported cwd (`follow_shell`). The color theme derives from
//! the app palette.

use std::path::{Path, PathBuf};

use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Rect, Sense, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2};
use soyuz_core::Location;
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Keymap, Mods, Osc52Policy};
use soyuz_term::{CursorStyle, TermEvent, TermSettings, TermTheme, Terminal, TerminalView};

use crate::app::{Focus, SoyuzApp};
use crate::cmd::Cmd;
use crate::theme::icons::{self, ph};
use crate::theme::{Palette, palette};
use crate::ui::panels::BottomTab;
use crate::ui::toasts::Toast;
use crate::ui::widgets::{menu_item, menu_separator};

const STRIP_H: f32 = 30.0;
/// How long a refused `cd` keeps being retried.
const CD_RETRY: Duration = Duration::from_secs(3);

pub struct Session {
    pub id: u64,
    term: Option<Terminal>,
    error: Option<String>,
    /// Title reported by the shell (often the cwd with shell integration).
    title: String,
    custom: Option<String>,
    /// Owning tab in per-tab mode.
    tab: Option<u64>,
    /// Directory we last told the shell to enter (or it reported).
    last_dir: Option<PathBuf>,
    pending_cd: Option<(PathBuf, Instant)>,
    busy_since: Option<Instant>,
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
    renaming: Option<(u64, String)>,
    theme: Option<(bool, String, TermTheme)>,
    /// Last focused-pane folder seen (to detect navigation).
    last_pane_dir: Option<(u64, PathBuf)>,
    next_id: u64,
    rect: Option<Rect>,
    cjk_requested: bool,
    /// Ask the host to close the bottom panel (the last session went away).
    request_close: bool,
    /// OSC 52 clipboard write waiting for an answer (`terminal.osc52 = "ask"`); a newer
    /// request replaces an unanswered one.
    clip_request: Option<String>,
    /// "Always for this session" was picked for OSC 52 writes.
    clip_allow_session: bool,
}

impl TerminalPanel {
    fn owner(per_tab: bool, tab: u64) -> Option<u64> {
        per_tab.then_some(tab)
    }

    fn visible_ids(&self, owner: Option<u64>) -> Vec<u64> {
        self.sessions.iter().filter(|s| s.tab == owner).map(|s| s.id).collect()
    }

    fn active_id(&self, owner: Option<u64>) -> Option<u64> {
        let key = owner.unwrap_or(0);
        let ids = self.visible_ids(owner);
        self.active
            .iter()
            .find(|(o, _)| *o == key)
            .map(|(_, id)| *id)
            .filter(|id| ids.contains(id))
            .or(ids.last().copied())
    }

    fn set_active(&mut self, owner: Option<u64>, id: u64) {
        let key = owner.unwrap_or(0);
        self.active.retain(|(o, _)| *o != key);
        self.active.push((key, id));
    }

    fn session_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// A tab was closed: drop (and thereby kill) the shells it owned in per-tab mode
    /// (REVIEW-2 #9). Shared sessions (`tab == None`) are untouched.
    pub fn drop_tab(&mut self, tab: u64) {
        self.sessions.retain(|s| s.tab != Some(tab));
        self.active.retain(|(o, _)| *o != tab);
        if self.last_pane_dir.as_ref().is_some_and(|(t, _)| *t == tab) {
            self.last_pane_dir = None;
        }
    }
}

fn settings_for(app: &SoyuzApp, cwd: &Path) -> TermSettings {
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
fn pane_dir(app: &SoyuzApp) -> Option<PathBuf> {
    match &app.ws.pane().location {
        Location::Dir(p) => Some(p.clone()),
        Location::Archive { archive, .. } => archive.parent().map(Path::to_path_buf),
        _ => None,
    }
}

fn spawn_session(app: &mut SoyuzApp, ctx: &egui::Context, cwd: PathBuf) -> u64 {
    let per_tab = app.config.terminal.per_tab;
    let tab = app.ws.tab().id.0;
    let owner = TerminalPanel::owner(per_tab, tab);
    let t = &mut app.ui.terminal;
    t.next_id += 1;
    // Widget ids must be unique per live terminal: mix in a process-wide counter.
    static UNIQUE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let unique = UNIQUE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let id = t.next_id;
    let settings = settings_for(app, &cwd);
    let (term, error) = match Terminal::spawn(settings, ctx.clone(), 0x5052_0000 + unique) {
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
    if !t.cjk_requested {
        t.cjk_requested = true;
        // CJK / Hangul fallbacks join every font family (incl. the terminal's).
        use crate::theme::font_fallback::{Script, request};
        request(&[Script::Cjk, Script::Hangul]);
    }
    id
}

/// Ctrl+`: open (spawning lazily) and focus; focus when open but unfocused; else close.
pub fn toggle(app: &mut SoyuzApp, ctx: &egui::Context) {
    let showing = app.ui.bottom.open && app.ui.bottom.tab == BottomTab::Terminal;
    if showing && app.ui.terminal.focused {
        app.ui.bottom.open = false;
        unfocus(app, ctx);
        return;
    }
    app.ui.bottom.open = true;
    app.ui.bottom.tab = BottomTab::Terminal;
    ensure_session(app, ctx);
    app.ui.terminal.focused = true;
}

/// Focus the terminal (FocusTerminal action), opening it when needed.
pub fn focus(app: &mut SoyuzApp, ctx: &egui::Context) {
    app.ui.bottom.open = true;
    app.ui.bottom.tab = BottomTab::Terminal;
    ensure_session(app, ctx);
    app.ui.terminal.focused = true;
}

fn unfocus(app: &mut SoyuzApp, ctx: &egui::Context) {
    app.ui.terminal.focused = false;
    app.ui.focus = Focus::List;
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
}

fn ensure_session(app: &mut SoyuzApp, ctx: &egui::Context) {
    let owner = TerminalPanel::owner(app.config.terminal.per_tab, app.ws.tab().id.0);
    if app.ui.terminal.visible_ids(owner).is_empty() {
        let cwd = pane_dir(app).unwrap_or_else(soyuz_fs::home);
        spawn_session(app, ctx, cwd);
    }
}

/// "Open terminal here": a new integrated session in `dir`.
pub fn open_at(app: &mut SoyuzApp, ctx: &egui::Context, dir: PathBuf) {
    app.ui.bottom.open = true;
    app.ui.bottom.tab = BottomTab::Terminal;
    spawn_session(app, ctx, dir);
    app.ui.terminal.focused = true;
}

/// Run a shell command line in `dir` with its output in the terminal panel: typed into
/// the active session when it is idle (after a `cd`), otherwise into a new session.
pub fn run_command(app: &mut SoyuzApp, ctx: &egui::Context, dir: PathBuf, line: &str) {
    // Typed text goes through the shell's line editor: a newline/escape in a (crafted)
    // file name would run extra commands no matter how it is quoted (REVIEW-2 #3).
    if soyuz_term::has_control_chars(line) || soyuz_term::has_control_chars(&dir.to_string_lossy()) {
        app.ui.toasts.push(
            Toast::error(tr!("panels-terminal-command-not-run")).body(tr!("panels-terminal-command-not-run-body")),
        );
        return;
    }
    app.ui.bottom.open = true;
    app.ui.bottom.tab = BottomTab::Terminal;
    let owner = TerminalPanel::owner(app.config.terminal.per_tab, app.ws.tab().id.0);
    let reuse = app.ui.terminal.active_id(owner).filter(|id| {
        app.ui.terminal.sessions.iter().any(|s| {
            s.id == *id && s.is_alive() && s.term.as_ref().is_some_and(|t| !t.mode_alt_screen() && t.send_cd(&dir))
        })
    });
    let id = match reuse {
        Some(id) => id,
        None => spawn_session(app, ctx, dir.clone()),
    };
    if let Some(s) = app.ui.terminal.session_mut(id) {
        s.last_dir = Some(dir);
        if let Some(t) = &s.term {
            t.write(format!("{line}\r").as_bytes());
        }
    }
    app.ui.terminal.focused = true;
}

fn close_session(app: &mut SoyuzApp, ctx: &egui::Context, id: u64) {
    let t = &mut app.ui.terminal;
    if let Some(i) = t.sessions.iter().position(|s| s.id == id) {
        let s = t.sessions.remove(i);
        drop(s); // kills the shell
    }
    let owner = TerminalPanel::owner(app.config.terminal.per_tab, app.ws.tab().id.0);
    if app.ui.terminal.visible_ids(owner).is_empty() {
        app.ui.terminal.request_close = true;
        unfocus(app, ctx);
    }
}

/// Apply panel requests made while drawing (the host lends the panel state out).
pub fn apply_requests(app: &mut SoyuzApp) {
    if std::mem::take(&mut app.ui.terminal.request_close) && app.ui.bottom.tab == BottomTab::Terminal {
        app.ui.bottom.open = false;
    }
}

/// Key handling while the terminal owns the keyboard: only app-level chords that a
/// shell never needs reach the keymap (Ctrl+`, Ctrl+Tab, Ctrl+Shift+…, Alt+P, F-keys
/// with Ctrl). Returns the actions to run.
pub fn dispatch_keys(ctx: &egui::Context, keymap: &Keymap, cmd_is_ctrl: bool) -> Vec<Action> {
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        i.events.retain(|ev| {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { return true };
            let Some(name) = crate::keys::key_name(*key) else { return true };
            let chord = KeyChord::new(crate::keys::mods(*modifiers), name).normalized(cmd_is_ctrl);
            if !terminal_passthrough(chord) {
                return true;
            }
            match keymap.lookup(KeyContext::Terminal, chord) {
                Some(a) => {
                    out.push(a.clone());
                    false
                }
                None => true,
            }
        });
    });
    out
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

/// Theme from the palette (`[terminal] theme = auto`), or the built-in dark / light.
pub fn theme_for(p: &Palette, setting: &str) -> TermTheme {
    match setting.trim().to_ascii_lowercase().as_str() {
        "dark" => return TermTheme::dark(),
        "light" => return TermTheme::light(),
        _ => {}
    }
    let mut t = if p.dark { TermTheme::dark() } else { TermTheme::light() };
    t.fg = p.text_primary;
    t.bg = if p.dark { p.bg_sunken } else { p.bg_surface1 };
    t.cursor = p.accent;
    let [r, g, b, _] = p.accent.to_array();
    t.selection = Color32::from_rgba_unmultiplied(r, g, b, if p.dark { 77 } else { 60 });
    t.ansi[1] = p.danger;
    t.ansi[2] = p.success;
    t.ansi[3] = p.warning;
    t.ansi[4] = p.accent;
    t.ansi[5] = p.glyph_video;
    t.ansi[6] = p.info;
    t.ansi[12] = p.accent_hover;
    t
}

/// Per frame (even while hidden): drain shell events, follow pane navigation, apply
/// reverse sync.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
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
            ClipDecision::Copy => ctx.copy_text(text),
            ClipDecision::Drop => {}
            ClipDecision::Ask => {
                app.ui.terminal.clip_request = Some(text);
                ctx.request_repaint();
            }
        }
    }
    for id in exited {
        close_session(app, ctx, id);
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
                // `send_cd` refuses these anyway (REVIEW-2 #3); don't flag the shell busy.
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
            ctx.request_repaint_after(Duration::from_millis(150));
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

/// Non-blocking prompt for a pending OSC 52 write (drawn over everything, bottom
/// centre, every frame while a request waits): Allow once / Always for this session /
/// Deny.
pub fn clipboard_prompt(ctx: &egui::Context, app: &mut SoyuzApp) {
    use crate::ui::dialogs::conflict::{Btn, button};
    let Some(text) = app.ui.terminal.clip_request.clone() else { return };
    let p = palette(ctx);
    let screen = ctx.content_rect();
    let bottom = if app.ui.status_visible { 36.0 } else { 12.0 };
    let width = 460.0_f32.min(screen.width() - 32.0);
    let mut answer: Option<ClipDecision> = None;
    let mut always = false;
    egui::Area::new(Id::new("soyuz-osc52-prompt"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -bottom))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.bg_surface2)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(14))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha(60),
                })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::CLIPBOARD_TEXT, 16.0).color(p.warning));
                        ui.label(
                            egui::RichText::new(tr!("panels-terminal-clip-title"))
                                .font(FontId::proportional(13.0))
                                .color(p.text_primary),
                        );
                    });
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(p.bg_sunken)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let preview = clipboard_preview(&text);
                            let shown = if preview.is_empty() {
                                tr!("panels-terminal-clip-empty").to_string()
                            } else {
                                preview
                            };
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(shown).font(FontId::monospace(11.5)).color(p.text_secondary),
                                )
                                .truncate(),
                            );
                        });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(tr!("panels-terminal-clip-warning"))
                            .font(FontId::proportional(11.0))
                            .color(p.text_tertiary),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if button(ui, tr!("panels-terminal-clip-deny"), Btn::Ghost).clicked() {
                            answer = Some(ClipDecision::Drop);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if button(ui, tr!("panels-terminal-clip-allow-once"), Btn::Primary).clicked() {
                                answer = Some(ClipDecision::Copy);
                            }
                            if button(ui, tr!("panels-terminal-clip-always"), Btn::Secondary)
                                .on_hover_text(tr!("panels-terminal-clip-always-tip"))
                                .clicked()
                            {
                                answer = Some(ClipDecision::Copy);
                                always = true;
                            }
                        });
                    });
                });
        });
    match answer {
        Some(ClipDecision::Copy) => {
            ctx.copy_text(text);
            app.ui.terminal.clip_request = None;
            app.ui.terminal.clip_allow_session |= always;
        }
        Some(_) => app.ui.terminal.clip_request = None,
        None => {}
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let (a, b) = (soyuz_fs::strip_verbatim(a), soyuz_fs::strip_verbatim(b));
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    if cfg!(windows) { norm(&a).eq_ignore_ascii_case(&norm(&b)) } else { norm(&a) == norm(&b) }
}

/// Body of the Terminal tab (inside the bottom panel card).
pub fn body(ui: &mut Ui, rect: Rect, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    ensure_session(app, &ctx);
    let per_tab = app.config.terminal.per_tab;
    let owner = TerminalPanel::owner(per_tab, app.ws.tab().id.0);
    let ids = app.ui.terminal.visible_ids(owner);
    let Some(active) = app.ui.terminal.active_id(owner) else { return };

    // ---- session strip ----
    let strip = Rect::from_min_size(rect.min + vec2(8.0, 0.0), vec2(rect.width() - 16.0, STRIP_H));
    let mut x = strip.left();
    let mut to_close = None;
    let mut to_activate = None;
    let mut new_session = false;
    let mut external = false;
    for id in &ids {
        let (label, alive, busy) = {
            let s = app.ui.terminal.sessions.iter().find(|s| s.id == *id).expect("visible id");
            (s.label(), s.is_alive(), s.busy_since.is_some_and(|t| t.elapsed() < Duration::from_secs(3)))
        };
        let is_active = *id == active;
        let renaming = app.ui.terminal.renaming.as_ref().filter(|(rid, _)| rid == id).map(|(_, t)| t.clone());
        let font = FontId::proportional(12.0);
        let text_w = ui.painter().layout_no_wrap(label.clone(), font.clone(), p.text_primary).size().x.min(180.0);
        let w = if renaming.is_some() { 160.0 } else { text_w + 52.0 };
        let r = Rect::from_min_size(pos2(x, strip.top() + 3.0), vec2(w, STRIP_H - 6.0));
        x += w + 4.0;
        let resp = ui.interact(r, Id::new(("soyuz.term.tab", *id)), Sense::click());
        let hovered = resp.hovered();
        let fill = if is_active {
            p.bg_surface2
        } else if hovered {
            p.bg_surface2.gamma_multiply(0.55)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, CornerRadius::same(6), fill);
        if is_active {
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(r.left() + 8.0, r.bottom() - 2.0), pos2(r.right() - 8.0, r.bottom())),
                CornerRadius::same(1),
                p.accent,
            );
        }
        let dot = if !alive {
            p.danger
        } else if busy {
            p.warning
        } else {
            p.success
        };
        ui.painter().circle_filled(pos2(r.left() + 12.0, r.center().y), 3.0, dot);
        if let Some(mut text) = renaming {
            let er = Rect::from_min_max(pos2(r.left() + 20.0, r.top() + 2.0), pos2(r.right() - 6.0, r.bottom() - 2.0));
            let mut child = ui.new_child(UiBuilder::new().max_rect(er));
            let edit = child.add(
                TextEdit::singleline(&mut text).font(font.clone()).desired_width(er.width()).frame(egui::Frame::NONE),
            );
            edit.request_focus();
            let done = edit.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter));
            if done {
                let t = text.trim().to_string();
                if let Some(s) = app.ui.terminal.session_mut(*id) {
                    s.custom = (!t.is_empty()).then_some(t);
                }
                app.ui.terminal.renaming = None;
            } else {
                app.ui.terminal.renaming = Some((*id, text));
            }
            continue;
        }
        let c = if is_active { p.text_primary } else { p.text_secondary };
        let galley = crate::ui::widgets::trunc_galley(ui.painter(), &label, font, c, 180.0);
        ui.painter().galley(pos2(r.left() + 22.0, r.center().y - galley.size().y / 2.0), galley, c);
        let close_r = Rect::from_center_size(pos2(r.right() - 13.0, r.center().y), vec2(16.0, 16.0));
        let close = ui.interact(close_r, Id::new(("soyuz.term.close", *id)), Sense::click());
        if hovered || close.hovered() || is_active {
            if close.hovered() {
                ui.painter().rect_filled(close_r, CornerRadius::same(4), p.bg_surface3);
            }
            ui.painter().text(close_r.center(), Align2::CENTER_CENTER, ph::X, icons::font(11.0), p.text_tertiary);
        }
        if close.clicked() || resp.middle_clicked() {
            to_close = Some(*id);
        } else if resp.double_clicked() {
            app.ui.terminal.renaming = Some((*id, label.clone()));
        } else if resp.clicked() {
            to_activate = Some(*id);
        }
        let mut rename = false;
        crate::ui::native::popup::context_menu(&resp, |ui| {
            ui.set_min_width(200.0);
            if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("common-rename"), None, true).clicked() {
                rename = true;
                ui.close();
            }
            if menu_item(ui, Some(ph::PLUS), tr!("panels-terminal-new"), None, true).clicked() {
                new_session = true;
                ui.close();
            }
            if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("panels-terminal-open-external"), None, true).clicked() {
                external = true;
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::X), tr!("common-close"), None, true).clicked() {
                to_close = Some(*id);
                ui.close();
            }
        });
        if rename {
            app.ui.terminal.renaming = Some((*id, label));
        }
    }
    let plus = Rect::from_min_size(pos2(x + 2.0, strip.top() + 4.0), vec2(22.0, 22.0));
    let presp = ui.interact(plus, Id::new("soyuz.term.new"), Sense::click()).on_hover_text(tr!("panels-terminal-new"));
    if presp.hovered() {
        ui.painter().rect_filled(plus, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(plus.center(), Align2::CENTER_CENTER, ph::PLUS, icons::font(13.0), p.text_secondary);
    new_session |= presp.clicked();
    // Right side: busy hint + external terminal button.
    let ext = Rect::from_min_size(pos2(strip.right() - 26.0, strip.top() + 4.0), vec2(22.0, 22.0));
    let eresp = ui
        .interact(ext, Id::new("soyuz.term.external"), Sense::click())
        .on_hover_text(tr!("panels-terminal-open-external"));
    if eresp.hovered() {
        ui.painter().rect_filled(ext, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(ext.center(), Align2::CENTER_CENTER, ph::ARROW_SQUARE_OUT, icons::font(13.0), p.text_secondary);
    external |= eresp.clicked();
    if let Some(s) = app.ui.terminal.sessions.iter().find(|s| s.id == active)
        && s.busy_since.is_some_and(|t| t.elapsed() < Duration::from_secs(3))
    {
        ui.painter().text(
            pos2(ext.left() - 10.0, strip.center().y),
            Align2::RIGHT_CENTER,
            tr!("panels-terminal-busy"),
            FontId::proportional(11.0),
            p.warning,
        );
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    // ---- terminal ----
    let term_rect = Rect::from_min_max(pos2(rect.left() + 8.0, strip.bottom() + 2.0), rect.max - vec2(8.0, 6.0));
    let theme_key = (p.dark, app.config.terminal.theme.clone());
    let needs_theme = app.ui.terminal.theme.as_ref().is_none_or(|(d, s, _)| (*d, s.clone()) != theme_key);
    if needs_theme {
        let th = theme_for(&p, &app.config.terminal.theme);
        app.ui.terminal.theme = Some((theme_key.0, theme_key.1, th));
    }
    let theme = app.ui.terminal.theme.as_ref().map(|(_, _, t)| t.clone()).unwrap_or_default();
    ui.painter().rect_filled(term_rect, CornerRadius::same(6), theme.bg);
    let inner = term_rect.shrink2(vec2(8.0, 6.0));
    app.ui.terminal.rect = Some(term_rect);
    // Clicking anywhere else gives the keyboard back to the file view.
    let (pressed, ptr) = ui.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos()));
    if pressed && app.ui.terminal.focused && ptr.is_some_and(|pt| !term_rect.contains(pt) && !strip.contains(pt)) {
        app.ui.terminal.focused = false;
    }
    let focused = app.ui.terminal.focused;
    let font_size = app.config.terminal.font_size.clamp(7.0, 32.0);
    let mut link = None;
    let mut link_base: Option<PathBuf> = None;
    let mut esc = false;
    let mut wants = false;
    if let Some(s) = app.ui.terminal.sessions.iter().find(|s| s.id == active) {
        match &s.term {
            Some(term) => {
                if crate::ui::a11y::active(ui.ctx()) {
                    // Screen readers: a terminal node named after the session, its visible
                    // text as the value. Added before the view so it never covers it.
                    let node = ui.interact(inner, Id::new(("soyuz.term.a11y", s.id)), Sense::hover());
                    let name = tr!("panels-terminal-a11y", name = s.label());
                    let text = term.screen_text();
                    crate::ui::a11y::with(ui.ctx(), node.id, |n| {
                        n.set_role(egui::accesskit::Role::Terminal);
                        n.set_label(name);
                        n.set_value(text.trim_end());
                    });
                }
                let mut child = ui.new_child(UiBuilder::new().max_rect(inner).id_salt(("soyuz.term", s.id)));
                child.set_clip_rect(inner);
                let r = TerminalView::new(term)
                    .focused(focused)
                    .theme(&theme)
                    .font(crate::theme::fonts::terminal(ui.ctx(), font_size))
                    .show(&mut child);
                wants = r.wants_focus;
                link = r.link_clicked;
                if link.is_some() {
                    link_base = term.cwd().or_else(|| s.last_dir.clone());
                }
                esc = r.escape_on_empty_line;
            }
            None => {
                ui.painter().text(
                    inner.center() - vec2(0.0, 10.0),
                    Align2::CENTER_CENTER,
                    ph::WARNING_CIRCLE,
                    icons::font_fill(26.0),
                    p.danger,
                );
                ui.painter().text(
                    inner.center() + vec2(0.0, 18.0),
                    Align2::CENTER_CENTER,
                    tr!(
                        "panels-terminal-shell-failed",
                        error = s.error.clone().unwrap_or_else(|| tr!("panels-terminal-unknown-error").to_string())
                    ),
                    FontId::proportional(12.0),
                    p.text_secondary,
                );
            }
        }
    }
    if focused {
        ui.painter().rect_stroke(
            term_rect,
            CornerRadius::same(6),
            Stroke::new(1.0, p.accent.gamma_multiply(0.45)),
            egui::StrokeKind::Inside,
        );
    }
    if wants {
        app.ui.terminal.focused = true;
    }
    if esc && app.config.terminal.escape_returns_focus {
        unfocus(app, &ctx);
    }
    if let Some(url) = link {
        open_link(app, &ctx, &url, link_base.as_deref());
    }

    if let Some(id) = to_activate {
        app.ui.terminal.set_active(owner, id);
        app.ui.terminal.focused = true;
    }
    if let Some(id) = to_close {
        close_session(app, &ctx, id);
    }
    if new_session {
        let cwd = pane_dir(app).unwrap_or_else(soyuz_fs::home);
        spawn_session(app, &ctx, cwd);
        app.ui.terminal.focused = true;
    }
    if external {
        let dir = app
            .ui
            .terminal
            .sessions
            .iter()
            .find(|s| s.id == active)
            .and_then(|s| s.last_dir.clone())
            .or_else(|| pane_dir(app))
            .unwrap_or_else(soyuz_fs::home);
        app.cmds.push(Cmd::TerminalAt(dir));
    }
}

/// Ctrl+click on a link: URLs go to the browser, paths open in the pane / default app.
/// Relative paths resolve against the session's working directory (not the app's), and
/// executables are revealed rather than run: terminal output is untrusted text
/// (REVIEW-2 #24).
fn open_link(app: &mut SoyuzApp, ctx: &egui::Context, url: &str, base: Option<&Path>) {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        ctx.open_url(egui::OpenUrl::new_tab(url));
        return;
    }
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
fn resolve_link(url: &str, base: Option<&Path>) -> Option<PathBuf> {
    let raw = url.strip_prefix("file://").unwrap_or(url);
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        return Some(p);
    }
    // `C:foo` / `oo` style partial paths are neither absolute nor safely relative.
    if p.has_root() || matches!(p.components().next(), Some(std::path::Component::Prefix(_))) {
        return None;
    }
    base.filter(|b| b.is_absolute()).map(|b| b.join(p))
}

/// Files that would *run* something when opened with the default handler.
fn looks_executable(path: &Path) -> bool {
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

/// Context-menu rows: integrated + external terminal at `dir`.
pub fn menu_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, dir: &Path) {
    if menu_item(ui, Some(ph::TERMINAL_WINDOW), tr!("panels-terminal-open-here"), Some("Ctrl+`"), true).clicked() {
        cmds.push(Cmd::OpenTerminalPanel(dir.to_path_buf()));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("panels-terminal-open-external"), None, true).clicked() {
        cmds.push(Cmd::TerminalAt(dir.to_path_buf()));
        ui.close();
    }
}

/// Whether a modifier-less key event targets the terminal (used to keep type-ahead off).
pub fn owns_keyboard(app: &SoyuzApp) -> bool {
    app.ui.terminal.focused && app.ui.bottom.open && app.ui.bottom.tab == BottomTab::Terminal
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
    fn palette_theme() {
        let t = theme_for(&crate::theme::ORBIT_DARK, "auto");
        assert_eq!(t.bg, crate::theme::ORBIT_DARK.bg_sunken);
        assert_eq!(t.cursor, crate::theme::ORBIT_DARK.accent);
        let l = theme_for(&crate::theme::ORBIT_LIGHT, "auto");
        assert_eq!(l.fg, crate::theme::ORBIT_LIGHT.text_primary);
        assert_eq!(theme_for(&crate::theme::ORBIT_LIGHT, "dark"), TermTheme::dark());
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
    }
}
