//! Integrated terminal (PLAN M8, app side): lives in the bottom panel's Terminal tab,
//! spawned lazily on first open (Ctrl+`). Several sessions per panel with mini-tabs
//! (new / close / rename, title from the shell); sessions are shared by all tabs or kept
//! per tab (`[terminal] per_tab`). The active session follows the focused pane's folder
//! through `send_cd` (which refuses while the shell is busy), and can optionally drive
//! the pane from the shell's reported cwd (`follow_shell`). The color theme derives from
//! the app palette.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Rect, Sense, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2};
use soyuz_core::Location;
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Keymap, Mods};
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
        title_label(&self.title).unwrap_or_else(|| format!("Terminal {}", self.id))
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
        request_cjk_font();
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
/// reverse sync and load the CJK fallback font once it is ready.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    if let Some(bytes) = take_cjk_font() {
        ctx.add_font(egui::epaint::text::FontInsert::new(
            "soyuz-cjk-fallback",
            egui::FontData::from_owned(bytes.to_vec()),
            vec![
                egui::epaint::text::InsertFontFamily {
                    family: egui::FontFamily::Monospace,
                    priority: egui::epaint::text::FontPriority::Lowest,
                },
                egui::epaint::text::InsertFontFamily {
                    family: egui::FontFamily::Proportional,
                    priority: egui::epaint::text::FontPriority::Lowest,
                },
            ],
        ));
    }
    if app.ui.terminal.sessions.is_empty() {
        return;
    }
    let mut exited = Vec::new();
    let mut navigate: Option<PathBuf> = None;
    let follow_shell = app.config.terminal.follow_shell;
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
                TermEvent::Clipboard(text) => ctx.copy_text(text),
                TermEvent::Bell => {}
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
        && dir.is_dir()
    {
        let pane = app.ws.pane().id;
        app.ui.terminal.last_pane_dir = Some((tab, dir.clone()));
        app.cmds.push(Cmd::Navigate { pane, location: Location::Dir(dir), select: None });
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
        resp.context_menu(|ui| {
            ui.set_width(200.0);
            if menu_item(ui, Some(ph::PENCIL_SIMPLE), "Rename", None, true).clicked() {
                rename = true;
                ui.close();
            }
            if menu_item(ui, Some(ph::PLUS), "New terminal", None, true).clicked() {
                new_session = true;
                ui.close();
            }
            if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), "Open external terminal here", None, true).clicked() {
                external = true;
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::X), "Close", None, true).clicked() {
                to_close = Some(*id);
                ui.close();
            }
        });
        if rename {
            app.ui.terminal.renaming = Some((*id, label));
        }
    }
    let plus = Rect::from_min_size(pos2(x + 2.0, strip.top() + 4.0), vec2(22.0, 22.0));
    let presp = ui.interact(plus, Id::new("soyuz.term.new"), Sense::click()).on_hover_text("New terminal");
    if presp.hovered() {
        ui.painter().rect_filled(plus, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(plus.center(), Align2::CENTER_CENTER, ph::PLUS, icons::font(13.0), p.text_secondary);
    new_session |= presp.clicked();
    // Right side: busy hint + external terminal button.
    let ext = Rect::from_min_size(pos2(strip.right() - 26.0, strip.top() + 4.0), vec2(22.0, 22.0));
    let eresp =
        ui.interact(ext, Id::new("soyuz.term.external"), Sense::click()).on_hover_text("Open external terminal here");
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
            "Terminal busy — didn't change folder",
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
    let mut esc = false;
    let mut wants = false;
    if let Some(s) = app.ui.terminal.sessions.iter().find(|s| s.id == active) {
        match &s.term {
            Some(term) => {
                let mut child = ui.new_child(UiBuilder::new().max_rect(inner).id_salt(("soyuz.term", s.id)));
                child.set_clip_rect(inner);
                let r = TerminalView::new(term)
                    .focused(focused)
                    .theme(&theme)
                    .font(FontId::monospace(font_size))
                    .show(&mut child);
                wants = r.wants_focus;
                link = r.link_clicked;
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
                    format!("Couldn't start the shell: {}", s.error.as_deref().unwrap_or("unknown error")),
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
        open_link(app, &ctx, &url);
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
fn open_link(app: &mut SoyuzApp, ctx: &egui::Context, url: &str) {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        ctx.open_url(egui::OpenUrl::new_tab(url));
        return;
    }
    let path = PathBuf::from(url.strip_prefix("file://").unwrap_or(url));
    if path.is_dir() {
        let pane = app.ws.pane().id;
        app.cmds.push(Cmd::Navigate { pane, location: Location::Dir(path), select: None });
    } else if path.exists() {
        crate::ops::launch(app, &path);
    } else {
        app.ui.toasts.push(Toast::info("Link target not found").body(url.to_string()));
    }
}

// ---- CJK / emoji fallback font -------------------------------------------------------

fn cjk_slot() -> &'static Mutex<Option<Arc<[u8]>>> {
    static S: OnceLock<Mutex<Option<Arc<[u8]>>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Candidate system fonts with CJK coverage.
fn cjk_candidates() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\YuGothR.ttc",
            r"C:\Windows\Fonts\malgun.ttf",
            r"C:\Windows\Fonts\simsun.ttc",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &["/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ]
    }
}

/// Load a CJK-capable system font in the background (optional nicety).
fn request_cjk_font() {
    let _ = std::thread::Builder::new().name("soyuz-cjk-font".into()).spawn(|| {
        for c in cjk_candidates() {
            if let Ok(bytes) = std::fs::read(c) {
                if let Ok(mut s) = cjk_slot().lock() {
                    *s = Some(bytes.into());
                }
                return;
            }
        }
    });
}

fn take_cjk_font() -> Option<Arc<[u8]>> {
    cjk_slot().lock().ok()?.take()
}

/// Context-menu rows: integrated + external terminal at `dir`.
pub fn menu_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, dir: &Path) {
    if menu_item(ui, Some(ph::TERMINAL_WINDOW), "Open terminal here", Some("Ctrl+`"), true).clicked() {
        cmds.push(Cmd::OpenTerminalPanel(dir.to_path_buf()));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), "Open external terminal here", None, true).clicked() {
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
}
