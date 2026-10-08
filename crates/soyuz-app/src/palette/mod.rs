//! Command palette (Ctrl+Shift+P / Ctrl+K) and quick "Go to folder" (Ctrl+P).
//!
//! One centered, animated overlay with a fuzzy-ranked list. Prefixes switch modes (see
//! [`query`]): `:` path with folder completion, `@` bookmarks / recent folders, `/`
//! recursive search, `>` shell command in the terminal panel, `#` switch tab. Recently
//! used commands float to the top; folder visits are tracked with frecency and
//! persisted in `palette.toml` together with recent shell commands and marks.

pub mod catalog;
pub mod frecency;
pub mod fuzzy;
pub mod query;
pub mod widgets;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::emath::TSTransform;
use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Key, LayerId, Margin, Modal, Modifiers, Order, Rect, RichText,
    Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2,
};
use soyuz_core::Location;
use soyuz_core::config::{Action, KeyContext};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::{self, NavOpts};
use crate::state::crumbs::{location_title, split_for_completion};
use crate::theme::icons::ph;
use crate::theme::{Palette, icons, palette};
use crate::ui::widgets::trunc_galley;

use frecency::{Memory, now_secs};
use fuzzy::Fuzzy;
use query::Mode;
use widgets::{highlighted, icon_tile, paint_chips};

const ROW_H: f32 = 46.0;
const HEADER_H: f32 = 26.0;
const INPUT_H: f32 = 54.0;
const FOOTER_H: f32 = 36.0;
const MAX_LIST_H: f32 = 430.0;
const MAX_RESULTS: usize = 200;

/// What running an item does.
#[derive(Clone, Debug, PartialEq)]
pub enum Exec {
    Action(Action),
    Go(Location),
    /// Replace the input (Tab completion; Enter also navigates for paths).
    Complete {
        text: String,
        go: Option<Location>,
    },
    Search {
        text: String,
        content: bool,
    },
    Shell(String),
    Tab(usize),
    /// Insert a mode prefix.
    Prefix(char),
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Folder,
    Neutral,
    Success,
    Warning,
    Info,
}

/// One palette row.
#[derive(Clone, Debug)]
pub struct Item {
    pub exec: Exec,
    pub title: String,
    pub detail: String,
    pub icon: &'static str,
    pub tint: Tint,
    /// Shortcut shown as chips.
    pub chord: Option<String>,
    /// Small right-aligned label (category / "recent").
    pub tag: Option<String>,
    /// Matched char indices in `title`.
    pub hl: Vec<u32>,
    pub section: &'static str,
    pub score: i64,
}

impl Item {
    fn new(exec: Exec, title: impl Into<String>, icon: &'static str, section: &'static str) -> Self {
        Item {
            exec,
            title: title.into(),
            detail: String::new(),
            icon,
            tint: Tint::Accent,
            chord: None,
            tag: None,
            hl: Vec::new(),
            section,
            score: 0,
        }
    }
}

pub struct PaletteState {
    pub open: bool,
    base: Mode,
    pub input: String,
    selected: usize,
    opened_at: f64,
    focus_input: bool,
    cursor_to_end: bool,
    scroll_to_selected: bool,
    results: Vec<Item>,
    results_key: Option<(String, Mode, u64)>,
    /// Bumped whenever sources change (forces a rebuild).
    version: u64,
    pub memory: Memory,
    pub dirty: bool,
    memory_path: PathBuf,
    last_save: Instant,
    last_dir: Option<PathBuf>,
    pointer_at_open: Option<egui::Pos2>,
}

impl PaletteState {
    pub fn new(memory_path: PathBuf) -> Self {
        let memory = Memory::load(&memory_path);
        PaletteState {
            open: false,
            base: Mode::Commands,
            input: String::new(),
            selected: 0,
            opened_at: 0.0,
            focus_input: false,
            cursor_to_end: false,
            scroll_to_selected: false,
            results: Vec::new(),
            results_key: None,
            version: 0,
            memory,
            dirty: false,
            memory_path,
            last_save: Instant::now(),
            last_dir: None,
            pointer_at_open: None,
        }
    }

    pub fn mode(&self) -> Mode {
        query::parse(&self.input, self.base).0
    }

    pub fn save_now(&mut self) {
        if self.dirty {
            match self.memory.save_async(&self.memory_path) {
                Ok(()) => self.dirty = false,
                Err(e) => log::warn!("cannot save palette memory: {e}"),
            }
            self.last_save = Instant::now();
        }
    }
}

/// Open the palette in `base` mode with optional initial text (e.g. `":"`).
pub fn open(app: &mut SoyuzApp, base: Mode, text: &str) {
    let p = &mut app.palette;
    if p.open && p.base == base && text.is_empty() {
        close(app);
        return;
    }
    p.open = true;
    p.base = base;
    p.input = text.to_string();
    p.selected = 0;
    p.opened_at = f64::NAN;
    p.focus_input = true;
    p.cursor_to_end = true;
    p.results_key = None;
    p.scroll_to_selected = true;
    p.pointer_at_open = None;
}

pub fn close(app: &mut SoyuzApp) {
    app.palette.open = false;
    app.palette.input.clear();
    app.palette.save_now();
}

/// The palette or the settings window owns the keyboard.
pub fn owns_keyboard(app: &SoyuzApp) -> bool {
    app.palette.open || app.settings.is_open()
}

/// Per frame, before input: folder frecency, config hot reload, throttled saves.
pub fn begin_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    crate::services::config_watch::pump(app, ctx);
    if let Some(dir) = app.ws.pane().location.as_fs_path().map(soyuz_fs::strip_verbatim)
        && app.palette.last_dir.as_deref() != Some(dir.as_path())
    {
        app.palette.memory.visit(&dir, now_secs());
        app.palette.last_dir = Some(dir);
        app.palette.dirty = true;
        app.palette.version += 1;
    }
    if app.palette.dirty && app.palette.last_save.elapsed() > Duration::from_secs(5) {
        app.palette.save_now();
    }
}

// ---------------------------------------------------------------------------
// Result building
// ---------------------------------------------------------------------------

fn chord_for(app: &SoyuzApp, a: &Action) -> Option<String> {
    app.keymap
        .chords_for(a)
        .first()
        .map(|c| c.to_string())
        .or_else(|| app.keymap.sequences_for(a).first().map(|s| soyuz_core::config::format_sequence(s)))
}

/// Actions offered by the palette: `Action::all()` minus bookmark slots without a
/// bookmark, plus the custom commands.
fn palette_actions(app: &SoyuzApp) -> Vec<Action> {
    let n_bm = app.config.bookmarks.len();
    let mut v: Vec<Action> = Action::all()
        .into_iter()
        .filter(|a| {
            !matches!(a, Action::OpenBookmark(n) if *n as usize > n_bm)
                && !crate::actions::unavailable(a)
                && crate::ui::workspaces::palette_shows(app, a)
        })
        .collect();
    v.extend(
        app.config
            .custom_commands
            .iter()
            .filter(|c| !c.name.trim().is_empty())
            .map(|c| Action::CustomCommand(c.name.clone())),
    );
    v
}

fn action_item(app: &SoyuzApp, a: &Action, section: &'static str) -> Item {
    let e = catalog::entry(a);
    let title = crate::ui::workspaces::palette_title(app, a).unwrap_or(e.title);
    let mut it = Item::new(Exec::Action(a.clone()), title, e.icon, section);
    it.detail = match a {
        Action::OpenBookmark(n) => app
            .config
            .bookmarks
            .get(*n as usize - 1)
            .map(|b| format!("{} — {}", b.name, b.path.display()))
            .unwrap_or(e.description),
        Action::CustomCommand(name) => app
            .config
            .custom_commands
            .iter()
            .find(|c| &c.name == name)
            .map(|c| {
                let line = format!("{} {}", c.command, crate::commands::join_args(&c.args));
                if c.terminal { format!("Terminal · {}", line.trim()) } else { line.trim().to_string() }
            })
            .unwrap_or(e.description),
        _ => e.description,
    };
    it.chord = chord_for(app, a);
    it.tag = Some(e.category.label().to_string());
    it.tint = match e.category {
        soyuz_core::config::ActionCategory::FileOps => Tint::Info,
        soyuz_core::config::ActionCategory::View => Tint::Success,
        soyuz_core::config::ActionCategory::Jobs => Tint::Warning,
        soyuz_core::config::ActionCategory::Navigation => Tint::Folder,
        _ => Tint::Accent,
    };
    it
}

/// Rank actions for `q` (empty = recent first, then by category).
pub fn rank_commands(app: &SoyuzApp, q: &str) -> Vec<Item> {
    let actions = palette_actions(app);
    let recent: Vec<Action> = app.palette.memory.commands.iter().filter_map(|s| s.parse::<Action>().ok()).collect();
    let mut out = Vec::new();
    if q.trim().is_empty() {
        for a in recent.iter().filter(|a| actions.contains(a)).take(6) {
            let mut it = action_item(app, a, "Recent");
            it.tag = Some("recent".into());
            out.push(it);
        }
        let mut rest: Vec<Item> = actions
            .iter()
            .filter(|a| !recent.iter().take(6).any(|r| r == *a))
            .map(|a| action_item(app, a, "All commands"))
            .collect();
        rest.sort_by(|a, b| {
            let ca = match &a.exec {
                Exec::Action(x) => x.category(),
                _ => soyuz_core::config::ActionCategory::Tools,
            };
            let cb = match &b.exec {
                Exec::Action(x) => x.category(),
                _ => soyuz_core::config::ActionCategory::Tools,
            };
            ca.cmp(&cb).then_with(|| a.title.cmp(&b.title))
        });
        out.extend(rest);
        return out;
    }
    let entries: Vec<(Action, catalog::Entry)> = actions
        .iter()
        .map(|a| {
            let mut e = catalog::entry(a);
            if let Some(t) = crate::ui::workspaces::palette_title(app, a) {
                e.title = t;
            }
            (a.clone(), e)
        })
        .collect();
    for m in score_actions(&entries, &recent, q) {
        let a = &entries[m.index].0;
        let mut it = action_item(app, a, "Commands");
        it.score = m.score;
        it.hl = m.hl;
        if m.recent {
            it.tag = Some("recent".into());
        }
        out.push(it);
    }
    out
}

/// One ranked action.
#[derive(Clone, Debug, PartialEq)]
pub struct Ranked {
    pub index: usize,
    pub score: i64,
    pub hl: Vec<u32>,
    pub recent: bool,
}

/// Pure command ranking: title matches count double, keywords / description / category
/// match too; recently used commands get a boost that fades with their position.
pub fn score_actions(entries: &[(Action, catalog::Entry)], recent: &[Action], q: &str) -> Vec<Ranked> {
    let mut f = Fuzzy::new(q);
    let mut out = Vec::new();
    for (index, (a, e)) in entries.iter().enumerate() {
        let title = f.indices(&e.title);
        let extra =
            f.score(&format!("{} {} {} {}", e.title, e.keywords, e.category.label(), a.name().replace('_', " ")));
        let (score, hl) = match (title, extra) {
            (Some((s, hl)), _) => (s as i64 * 2, hl),
            (None, Some(s)) => (s as i64, Vec::new()),
            (None, None) => continue,
        };
        let boost = recent.iter().position(|r| r == a).map_or(0, |i| (12 - i.min(11) as i64) * 6);
        out.push(Ranked { index, score: score + boost, hl, recent: boost > 0 });
    }
    out.sort_by(|a, b| {
        b.score.cmp(&a.score).then_with(|| entries[a.index].1.title.len().cmp(&entries[b.index].1.title.len()))
    });
    // Drop weak scattered matches once there are strong ones.
    if q.trim().chars().count() >= 3
        && let Some(best) = out.first().map(|r| r.score)
    {
        out.retain(|r| r.score * 3 >= best);
    }
    out
}

/// Bookmarks, favorites and frecency folders, deduplicated.
fn folder_sources(app: &SoyuzApp) -> Vec<(PathBuf, String, &'static str, f64)> {
    let now = now_secs();
    let mut v: Vec<(PathBuf, String, &'static str, f64)> = Vec::new();
    let mut push = |path: PathBuf, label: String, kind: &'static str, fr: f64| {
        if let Some(existing) = v.iter_mut().find(|x| same_path(&x.0, &path)) {
            existing.3 = existing.3.max(fr);
            return;
        }
        v.push((path, label, kind, fr));
    };
    for b in &app.config.bookmarks {
        push(b.path.clone(), b.name.clone(), "Bookmark", 0.0);
    }
    for f in &app.ui.sidebar.favorites {
        push(f.path.clone(), f.name.clone(), "Favorite", 0.0);
    }
    for f in app.palette.memory.top_folders(now).into_iter().take(300) {
        push(f.path.clone(), folder_name(&f.path), "Recent", f.score(now));
    }
    // Subfolders of the current folder (already listed; no I/O).
    let show_hidden = app.ui.show_hidden;
    for e in app.ws.pane().entries.iter().filter(|e| e.is_dir_like() && (show_hidden || !e.is_hidden())).take(500) {
        push(e.path.clone(), e.name.clone(), "Here", -1.0);
    }
    v
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { a == b }
}

fn folder_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// Folder jump (Ctrl+P) and `@`: fuzzy over names and paths, boosted by frecency.
pub fn rank_folders(app: &SoyuzApp, q: &str, bookmarks_first: bool) -> Vec<Item> {
    let sources = folder_sources(app);
    let current = app.ws.pane().location.as_fs_path().map(soyuz_fs::strip_verbatim);
    let mut f = Fuzzy::new(q);
    let mut out = Vec::new();
    for (path, label, kind, fr) in sources {
        if kind == "Here" && q.trim().is_empty() {
            continue;
        }
        let shown = soyuz_fs::strip_verbatim(&path);
        let path_text = shown.to_string_lossy().into_owned();
        let title_m = f.indices(&label);
        let path_m = f.score(&path_text);
        let (score, hl) = match (title_m, path_m) {
            (Some((s, hl)), p) => ((s as i64) * 2 + p.unwrap_or(0) as i64 / 2, hl),
            (None, Some(s)) => (s as i64, Vec::new()),
            (None, None) => continue,
        };
        let section = match (q.trim().is_empty(), kind) {
            (false, _) => "Folders",
            (true, "Recent") => "Recent & frequent",
            (true, _) => "Bookmarks & favorites",
        };
        let glyph = match kind {
            "Bookmark" => ph::BOOKMARK_SIMPLE,
            "Favorite" => ph::STAR,
            _ => crate::ui::sidebar::favorite_glyph(&path),
        };
        let mut it = Item::new(Exec::Go(Location::Dir(path.clone())), label, glyph, section);
        it.detail = path_text;
        it.tint = if matches!(kind, "Recent" | "Here") { Tint::Folder } else { Tint::Warning };
        it.tag = Some(if current.as_deref().is_some_and(|c| same_path(c, &shown)) {
            "current".into()
        } else {
            kind.to_lowercase()
        });
        it.hl = hl;
        let kind_rank = if bookmarks_first && kind != "Recent" { 10_000 } else { 0 };
        it.score = score + (fr.max(0.0).ln_1p() * 40.0) as i64 + kind_rank;
        out.push(it);
    }
    if q.trim().is_empty() {
        // Sections: recent first (Ctrl+P), bookmarks first (`@`).
        out.sort_by(|a, b| {
            let ra = (a.section == "Recent & frequent") != bookmarks_first;
            let rb = (b.section == "Recent & frequent") != bookmarks_first;
            rb.cmp(&ra).then_with(|| b.score.cmp(&a.score))
        });
    } else {
        out.sort_by_key(|a| std::cmp::Reverse(a.score));
    }
    out
}

fn expand_tilde(s: &str) -> String {
    if s == "~" {
        return soyuz_fs::home().to_string_lossy().into_owned();
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        return soyuz_fs::home().join(rest).to_string_lossy().into_owned();
    }
    s.to_string()
}

fn sep() -> char {
    std::path::MAIN_SEPARATOR
}

/// `:` mode: "go to" plus completions of the typed parent folder (aux listing, async).
fn rank_paths(app: &mut SoyuzApp, q: &str, prefix: Option<char>) -> Vec<Item> {
    let current = app.ws.pane().location.as_fs_path().map(soyuz_fs::strip_verbatim);
    let typed = expand_tilde(q.trim());
    let absolute = query::looks_like_path(&typed) || Path::new(&typed).is_absolute();
    // Resolve relative input against the current folder.
    let full = if absolute || typed.is_empty() {
        typed.clone()
    } else {
        match &current {
            Some(c) => c.join(&typed).to_string_lossy().into_owned(),
            None => typed.clone(),
        }
    };
    let (dir, partial) = if full.is_empty() {
        match &current {
            Some(c) => (c.clone(), String::new()),
            None => return Vec::new(),
        }
    } else {
        match split_for_completion(&full) {
            Some((d, p)) => (d, p),
            None => (PathBuf::from(format!("{full}{}", sep())), String::new()),
        }
    };
    let mut out = Vec::new();
    if !q.trim().is_empty()
        && let Some(loc) = Location::parse(&full)
    {
        let mut it =
            Item::new(Exec::Go(loc.clone()), format!("Go to {}", loc.display()), ph::ARROW_SQUARE_RIGHT, "Path");
        it.detail = "Enter to open · Ctrl+Enter opens in a new tab".into();
        it.score = i64::MAX;
        out.push(it);
    }
    let entries = app.services.aux_entries(&dir);
    let loading =
        entries.is_none() && matches!(app.services.aux_state(&dir), Some(crate::services::AuxState::Loading(_)));
    if let Some(entries) = entries {
        let mut f = Fuzzy::new(&partial);
        let mut comps: Vec<Item> = Vec::new();
        for e in entries.iter().filter(|e| e.is_dir_like()) {
            if !app.ui.show_hidden && e.is_hidden() {
                continue;
            }
            let Some((score, hl)) = f.indices(&e.name) else { continue };
            let target = dir.join(&e.name);
            let shown = soyuz_fs::strip_verbatim(&target).to_string_lossy().into_owned();
            let text = format!("{}{shown}{}", prefix.map(String::from).unwrap_or_default(), sep());
            let mut it = Item::new(
                Exec::Complete { text, go: Some(Location::Dir(target.clone())) },
                e.name.clone(),
                crate::ui::sidebar::favorite_glyph(&target),
                "Folders",
            );
            it.detail = shown;
            it.tint = Tint::Folder;
            it.hl = hl;
            it.tag = Some("Tab".into());
            it.score = score as i64 - e.name.len() as i64;
            comps.push(it);
        }
        if partial.is_empty() {
            comps.sort_by(|a, b| soyuz_core::natural_cmp(&a.title, &b.title, true));
            out.extend(comps);
        } else {
            // A partial name: matching folders first, "go to" the literal text last.
            comps.sort_by_key(|a| std::cmp::Reverse(a.score));
            if !comps.is_empty() {
                let go = std::mem::take(&mut out);
                out = comps;
                out.extend(go.into_iter().map(|mut g| {
                    g.section = "Path";
                    g
                }));
            }
        }
    } else if loading {
        let mut it = Item::new(Exec::Nothing, "Listing folder…", ph::HOURGLASS, "Folders");
        it.detail = soyuz_fs::strip_verbatim(&dir).to_string_lossy().into_owned();
        it.tint = Tint::Neutral;
        out.push(it);
    }
    out
}

fn rank_search(app: &SoyuzApp, q: &str) -> Vec<Item> {
    let dir = app
        .ws
        .pane()
        .location
        .as_fs_path()
        .map(|p| soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned())
        .unwrap_or_else(|| "this folder".into());
    if q.trim().is_empty() {
        let mut it = Item::new(Exec::Nothing, "Type what to search for", ph::MAGNIFYING_GLASS, "Search");
        it.detail = format!("Names in {dir} and below · globs like *.rs work");
        it.tint = Tint::Neutral;
        return vec![it];
    }
    let mut a = Item::new(
        Exec::Search { text: q.to_string(), content: false },
        format!("Search names for “{q}”"),
        ph::MAGNIFYING_GLASS,
        "Search",
    );
    a.detail = format!("in {dir} and subfolders");
    let mut b = Item::new(
        Exec::Search { text: q.to_string(), content: true },
        format!("Search file contents for “{q}”"),
        ph::FILE_SEARCH,
        "Search",
    );
    b.detail = format!("in {dir} and subfolders");
    b.tint = Tint::Info;
    vec![a, b]
}

fn rank_shell(app: &SoyuzApp, q: &str) -> Vec<Item> {
    let dir = app
        .ws
        .pane()
        .location
        .as_fs_path()
        .map(|p| soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned())
        .unwrap_or_else(|| soyuz_fs::home().to_string_lossy().into_owned());
    let mut out = Vec::new();
    if !q.trim().is_empty() {
        let mut it = Item::new(Exec::Shell(q.to_string()), format!("Run  {q}"), ph::TERMINAL, "Shell");
        it.detail = format!("in {dir} · output in the terminal panel");
        it.tint = Tint::Success;
        out.push(it);
    }
    let mut f = Fuzzy::new(q);
    for line in &app.palette.memory.shell {
        if line == q.trim() {
            continue;
        }
        let Some((s, hl)) = f.indices(line) else { continue };
        let mut it = Item::new(
            Exec::Complete { text: format!(">{line}"), go: None },
            line.clone(),
            ph::CLOCK_COUNTER_CLOCKWISE,
            "Recent commands",
        );
        it.detail = "Tab to edit · Enter to run".into();
        it.tint = Tint::Neutral;
        it.hl = hl;
        it.score = s as i64;
        it.exec = Exec::Shell(line.clone());
        it.tag = Some("recent".into());
        out.push(it);
    }
    if out.is_empty() {
        let mut it = Item::new(Exec::Nothing, "Type a command to run", ph::TERMINAL, "Shell");
        it.detail = format!("in {dir}");
        it.tint = Tint::Neutral;
        out.push(it);
    }
    out
}

fn rank_tabs(app: &SoyuzApp, q: &str) -> Vec<Item> {
    let mut f = Fuzzy::new(q);
    let mut out = Vec::new();
    for (i, t) in app.ws.tabs.iter().enumerate() {
        let loc = &t.active().location;
        let title = location_title(loc);
        let label = format!("{}  {title}", i + 1);
        let detail = loc.display();
        let (score, hl) = match (f.indices(&label), f.score(&detail)) {
            (Some((s, hl)), _) => (s as i64 * 2, hl),
            (None, Some(s)) => (s as i64, Vec::new()),
            _ => continue,
        };
        let mut it = Item::new(Exec::Tab(i), label, if t.pinned { ph::PUSH_PIN } else { ph::BROWSERS }, "Tabs");
        it.detail = detail;
        it.hl = hl;
        it.score = score;
        it.tint = Tint::Folder;
        if i == app.ws.active {
            it.tag = Some("current".into());
        }
        if i < 8 {
            it.chord = chord_for(app, &Action::SelectTab(i as u8 + 1));
        }
        out.push(it);
    }
    if !q.trim().is_empty() {
        out.sort_by_key(|a| std::cmp::Reverse(a.score));
    }
    out
}

fn build(app: &mut SoyuzApp) -> Vec<Item> {
    let input = app.palette.input.clone();
    let (mode, q) = query::parse(&input, app.palette.base);
    let mut items = match mode {
        Mode::Commands => {
            let mut v = Vec::new();
            if query::looks_like_path(q.trim()) {
                v = rank_paths(app, q, Some(':')).into_iter().take(1).collect();
            }
            v.extend(rank_commands(app, q));
            v
        }
        Mode::Folders => rank_folders(app, q, false),
        Mode::Bookmarks => rank_folders(app, q, true),
        Mode::Path => {
            let prefix = if app.palette.base == Mode::Folders && !input.trim_start().starts_with(':') {
                None
            } else {
                Some(':')
            };
            rank_paths(app, q, prefix)
        }
        Mode::Search => rank_search(app, q),
        Mode::Shell => rank_shell(app, q),
        Mode::Tabs => rank_tabs(app, q),
    };
    items.truncate(MAX_RESULTS);
    items
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

fn run(app: &mut SoyuzApp, ctx: &egui::Context, exec: Exec, mods: Modifiers) {
    match exec {
        Exec::Action(a) => {
            app.palette.memory.used_command(&a.to_string());
            app.palette.dirty = true;
            close(app);
            app.cmds.push(Cmd::Action(a));
        }
        Exec::Go(loc) => {
            close(app);
            go(app, loc, mods.command);
        }
        Exec::Complete { text, go: target } => {
            if let Some(loc) = target.filter(|_| !mods.shift) {
                close(app);
                go(app, loc, mods.command);
            } else {
                app.palette.input = text;
                app.palette.cursor_to_end = true;
                app.palette.selected = 0;
            }
        }
        Exec::Search { text, content } => {
            close(app);
            crate::ui::search::open(app, content);
            app.ui.search.form.text = text;
            crate::ui::search::run(app);
        }
        Exec::Shell(line) => {
            app.palette.memory.used_shell(&line);
            app.palette.dirty = true;
            close(app);
            let dir = app.ws.pane().location.as_fs_path().map(soyuz_fs::strip_verbatim).unwrap_or_else(soyuz_fs::home);
            crate::ui::panels::terminal::run_command(app, ctx, dir, &line);
        }
        Exec::Tab(i) => {
            close(app);
            app.cmds.push(Cmd::SelectTab(i));
        }
        Exec::Prefix(c) => {
            app.palette.input = c.to_string();
            app.palette.cursor_to_end = true;
            app.palette.selected = 0;
        }
        Exec::Nothing => {}
    }
}

fn go(app: &mut SoyuzApp, loc: Location, new_tab: bool) {
    if let Location::Dir(p) = &loc
        && p.as_os_str().is_empty()
    {
        return;
    }
    if new_tab {
        ops::open_in_new_tab(app, loc, true);
    } else {
        let id = app.ws.pane().id;
        ops::navigate(app, id, loc, NavOpts::user());
    }
    app.ui.focus = crate::app::Focus::List;
}

/// Completion target of the selected item (Tab).
fn completion_of(item: &Item) -> Option<String> {
    match &item.exec {
        Exec::Complete { text, .. } => Some(text.clone()),
        Exec::Shell(line) => Some(format!(">{line}")),
        Exec::Go(Location::Dir(p)) => Some(format!(":{}{}", soyuz_fs::strip_verbatim(p).display(), sep())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn tint_color(p: &Palette, t: Tint) -> Color32 {
    match t {
        Tint::Accent => p.accent,
        Tint::Folder => p.glyph_folder,
        Tint::Neutral => p.text_tertiary,
        Tint::Success => p.success,
        Tint::Warning => p.warning,
        Tint::Info => p.info,
    }
}

/// Draw the palette (when open) and the pending key-sequence hint.
pub fn show(ctx: &egui::Context, app: &mut SoyuzApp) {
    sequence_hint(ctx, app);
    if !app.palette.open {
        return;
    }
    let p = palette(ctx);
    let now = ctx.input(|i| i.time);
    if app.palette.opened_at.is_nan() {
        app.palette.opened_at = now;
    }
    let t = if app.ui.reduced_motion { 1.0 } else { ((now - app.palette.opened_at) / 0.16).clamp(0.0, 1.0) as f32 };
    let ease = 1.0 - (1.0 - t).powi(3);
    if t < 1.0 {
        ctx.request_repaint();
    }

    // Keyboard first (before the text field sees the keys).
    handle_keys(ctx, app);
    if !app.palette.open {
        return;
    }

    // (Re)build results when the query or sources changed; paths rebuild while listing.
    let key = (app.palette.input.clone(), app.palette.base, app.palette.version);
    let mode = app.palette.mode();
    if app.palette.results_key.as_ref() != Some(&key) || mode == Mode::Path {
        let changed = app.palette.results_key.as_ref().map(|k| &k.0) != Some(&key.0);
        app.palette.results = build(app);
        app.palette.results_key = Some(key);
        if changed {
            app.palette.selected = 0;
            app.palette.scroll_to_selected = true;
        }
    }
    let n = app.palette.results.len();
    if n > 0 && app.palette.selected >= n {
        app.palette.selected = n - 1;
    }

    let screen = ctx.content_rect();
    let width = (screen.width() - 32.0).clamp(320.0, 660.0);
    let top = (screen.height() * 0.13).clamp(24.0, 120.0);
    let id = Id::new("soyuz.palette");
    let area = Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, top));
    let frame = Frame::new()
        .fill(p.bg_surface3)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::ZERO)
        .shadow(egui::epaint::Shadow {
            offset: [0, 18],
            blur: 56,
            spread: 0,
            color: Color32::from_black_alpha((p.shadow_alpha * 255.0 * 1.2).min(255.0) as u8),
        });
    let mut clicked: Option<(Exec, Modifiers)> = None;
    let resp = Modal::new(id)
        .area(area)
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(((if p.dark { 140.0 } else { 80.0 }) * ease) as u8))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_opacity(ease);
            crate::ui::a11y::container(ui.ctx(), ui.unique_id(), egui::accesskit::Role::Dialog, "Command palette");
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            input_row(ui, app, &p, width);
            divider(ui, &p);
            clicked = list(ui, app, &p, width);
            footer(ui, app, &p, width, &mut clicked);
        });
    // Scale-in from 97 % around the top center, sliding down 8 px.
    let s = 0.97 + 0.03 * ease;
    let pivot = pos2(screen.center().x, top);
    let tr = TSTransform::new(pivot.to_vec2() * (1.0 - s) + vec2(0.0, -8.0 * (1.0 - ease)), s);
    ctx.set_transform_layer(LayerId::new(Order::Foreground, id), tr);
    if let Some((exec, mods)) = clicked {
        run(app, ctx, exec, mods);
    } else if resp.backdrop_response.clicked() {
        close(app);
    }
}

fn handle_keys(ctx: &egui::Context, app: &mut SoyuzApp) {
    let n = app.palette.results.len();
    let mut exec: Option<(Exec, Modifiers)> = None;
    let mut toggle: Option<Mode> = None;
    let cmd_is_ctrl = app.config.keys.cmd_is_ctrl;
    let km = &app.keymap;
    let pal = &mut app.palette;
    ctx.input_mut(|i| {
        let mods = i.modifiers;
        // Global palette chords toggle / switch modes.
        i.events.retain(|ev| {
            let Some(chord) = crate::keys::event_chord(ev, mods, cmd_is_ctrl) else { return true };
            if chord.mods.is_empty() {
                return true;
            }
            match km.lookup(KeyContext::Global, chord) {
                Some(Action::CommandPalette) => {
                    toggle = Some(Mode::Commands);
                    false
                }
                Some(Action::JumpToFolderDialog) => {
                    toggle = Some(Mode::Folders);
                    false
                }
                _ => true,
            }
        });
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            if pal.input.is_empty() || pal.input.chars().count() == 1 && pal.mode().prefix().is_some() {
                pal.open = false;
            } else {
                pal.input.clear();
                pal.cursor_to_end = true;
            }
            return;
        }
        let down = i.consume_key(Modifiers::NONE, Key::ArrowDown)
            || i.consume_key(Modifiers::CTRL, Key::J)
            || i.consume_key(Modifiers::CTRL, Key::N);
        let up = i.consume_key(Modifiers::NONE, Key::ArrowUp)
            || i.consume_key(Modifiers::CTRL, Key::K)
            || i.consume_key(Modifiers::CTRL, Key::P);
        let pgdn = i.consume_key(Modifiers::NONE, Key::PageDown);
        let pgup = i.consume_key(Modifiers::NONE, Key::PageUp);
        if n > 0 {
            let cur = pal.selected.min(n - 1);
            let next = if down {
                (cur + 1) % n
            } else if up {
                (cur + n - 1) % n
            } else if pgdn {
                (cur + 8).min(n - 1)
            } else if pgup {
                cur.saturating_sub(8)
            } else {
                cur
            };
            if next != cur || down || up {
                pal.selected = next;
                pal.scroll_to_selected = true;
            }
        }
        let enter_mods = [Modifiers::NONE, Modifiers::CTRL, Modifiers::SHIFT, Modifiers::COMMAND];
        for m in enter_mods {
            if i.consume_key(m, Key::Enter) {
                if let Some(item) = pal.results.get(pal.selected) {
                    exec = Some((item.exec.clone(), m));
                }
                break;
            }
        }
        if i.consume_key(Modifiers::NONE, Key::Tab)
            && let Some(item) = pal.results.get(pal.selected)
            && let Some(text) = completion_of(item)
        {
            exec = Some((Exec::Complete { text, go: None }, Modifiers::NONE));
        }
        i.consume_key(Modifiers::SHIFT, Key::Tab);
    });
    if let Some(mode) = toggle {
        if app.palette.base == mode && !app.palette.input.is_empty() || app.palette.base != mode {
            open(app, mode, "");
        } else {
            close(app);
        }
        return;
    }
    if let Some((e, m)) = exec {
        run(app, ctx, e, m);
    }
}

fn divider(ui: &mut Ui, p: &Palette) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, p.border_subtle);
}

fn input_row(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, INPUT_H), Sense::hover());
    let mode = app.palette.mode();
    let painter = ui.painter();
    let mut x = rect.left() + 18.0;
    let glyph = match mode {
        Mode::Commands => ph::COMMAND,
        Mode::Folders | Mode::Path => ph::FOLDER_OPEN,
        Mode::Bookmarks => ph::BOOKMARK_SIMPLE,
        Mode::Search => ph::MAGNIFYING_GLASS,
        Mode::Shell => ph::TERMINAL,
        Mode::Tabs => ph::BROWSERS,
    };
    painter.text(pos2(x + 9.0, rect.center().y), Align2::CENTER_CENTER, glyph, icons::font(19.0), p.accent);
    x += 30.0;
    // Mode chip for prefixed / folder modes.
    if mode != Mode::Commands {
        let label = mode.label();
        let g = painter.layout_no_wrap(label.to_string(), FontId::proportional(11.5), p.accent);
        let chip = Rect::from_min_size(pos2(x, rect.center().y - 11.0), vec2(g.size().x + 16.0, 22.0));
        painter.rect_filled(chip, CornerRadius::same(11), p.accent_subtle);
        painter.galley(pos2(chip.left() + 8.0, chip.center().y - g.size().y / 2.0), g, p.accent);
        x = chip.right() + 8.0;
    }
    // "esc" hint on the right.
    let esc_left = paint_chips(ui, p, "Esc", rect.right() - 16.0, rect.center().y, false);
    let edit_rect = Rect::from_min_max(pos2(x, rect.top() + 8.0), pos2(esc_left - 10.0, rect.bottom() - 8.0));
    let hint = mode.placeholder();
    let mut child = ui
        .new_child(egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let te = TextEdit::singleline(&mut app.palette.input)
        .id(Id::new("soyuz.palette.input"))
        .frame(Frame::NONE)
        .font(FontId::proportional(16.5))
        .text_color(p.text_primary)
        .hint_text(RichText::new(hint).color(p.text_tertiary).size(15.0))
        .desired_width(edit_rect.width())
        .margin(Margin::symmetric(0, 4));
    let out = te.show(&mut child);
    if app.palette.focus_input {
        out.response.request_focus();
        app.palette.focus_input = false;
    }
    if !out.response.has_focus() {
        // Keep typing in the palette even after a click elsewhere in it.
        out.response.request_focus();
    }
    if app.palette.cursor_to_end {
        app.palette.cursor_to_end = false;
        let mut state = out.state.clone();
        let end = egui::text::CCursor::new(app.palette.input.chars().count());
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        state.store(ui.ctx(), out.response.id);
    }
}

fn list(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, width: f32) -> Option<(Exec, Modifiers)> {
    let items = &app.palette.results;
    let mut clicked = None;
    if items.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 120.0), Sense::hover());
        let painter = ui.painter();
        painter.text(
            rect.center() - vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            ph::MAGNIFYING_GLASS,
            icons::font(26.0),
            p.text_tertiary,
        );
        painter.text(
            rect.center() + vec2(0.0, 16.0),
            Align2::CENTER_CENTER,
            "No matches",
            FontId::proportional(13.0),
            p.text_secondary,
        );
        return None;
    }
    // Heights with section headers.
    let mut total = 8.0;
    let mut last: Option<&str> = None;
    for it in items {
        if last != Some(it.section) {
            total += HEADER_H;
            last = Some(it.section);
        }
        total += ROW_H;
    }
    let height = total.min(MAX_LIST_H);
    let selected = app.palette.selected;
    let scroll_to = std::mem::take(&mut app.palette.scroll_to_selected);
    // Mouse hover selects only after the pointer moved (not when the palette opens under it).
    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
    let moved = match (app.palette.pointer_at_open, pointer) {
        (None, Some(pp)) => {
            app.palette.pointer_at_open = Some(pp);
            false
        }
        (Some(a), Some(b)) => a.distance(b) > 3.0,
        _ => false,
    };
    let mut hover_sel = None;
    egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, true]).show(
        ui,
        |ui| {
            ui.set_width(width);
            ui.add_space(4.0);
            let mut last: Option<&str> = None;
            for (i, it) in items.iter().enumerate() {
                if last != Some(it.section) {
                    let (r, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
                    ui.painter().text(
                        pos2(r.left() + 20.0, r.bottom() - 7.0),
                        Align2::LEFT_BOTTOM,
                        it.section.to_uppercase(),
                        FontId::proportional(10.5),
                        p.text_tertiary,
                    );
                    last = Some(it.section);
                }
                let (outer, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
                let r = outer.shrink2(vec2(8.0, 2.0));
                let is_sel = i == selected;
                if scroll_to && is_sel {
                    ui.scroll_to_rect(outer.expand2(vec2(0.0, 6.0)), None);
                }
                if moved && resp.hovered() && !is_sel {
                    hover_sel = Some(i);
                }
                if resp.clicked() {
                    clicked = Some((it.exec.clone(), ui.input(|i| i.modifiers)));
                }
                if !ui.is_rect_visible(outer) {
                    continue;
                }
                let sel_t = ui.ctx().animate_bool_with_time(
                    Id::new(("soyuz.palette.row", i)),
                    is_sel,
                    crate::theme::anim(ui.ctx(), 0.08),
                );
                if sel_t > 0.0 {
                    ui.painter().rect_filled(r, CornerRadius::same(9), p.accent_subtle.gamma_multiply(sel_t));
                    let bar = Rect::from_min_size(pos2(r.left(), r.center().y - 9.0), vec2(3.0, 18.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(2), p.accent.gamma_multiply(sel_t));
                }
                row(ui, p, it, r, is_sel);
            }
            ui.add_space(4.0);
        },
    );
    if let Some(h) = hover_sel {
        app.palette.selected = h;
    }
    clicked
}

fn row(ui: &Ui, p: &Palette, it: &Item, r: Rect, selected: bool) {
    let tile = Rect::from_center_size(pos2(r.left() + 24.0, r.center().y), vec2(30.0, 30.0));
    let tint = tint_color(p, it.tint);
    icon_tile(ui, tile, it.icon, tint, 16.0);
    // Right side: chips + tag.
    let mut right = r.right() - 12.0;
    if let Some(c) = &it.chord {
        right = paint_chips(ui, p, c, right, r.center().y, selected) - 6.0;
    }
    if let Some(tag) = &it.tag {
        let g = ui.painter().layout_no_wrap(tag.clone(), FontId::proportional(10.5), p.text_tertiary);
        let pos = pos2(right - g.size().x, r.center().y - g.size().y / 2.0);
        right = pos.x - 10.0;
        ui.painter().galley(pos, g, p.text_tertiary);
    }
    let text_left = tile.right() + 12.0;
    let max_w = (right - text_left).max(40.0);
    let title_color = if selected { p.text_primary } else { p.text_primary.gamma_multiply(0.92) };
    let mut job = highlighted(&it.title, &it.hl, 13.5, title_color, p.accent);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let title = ui.painter().layout_job(job);
    let has_detail = !it.detail.is_empty();
    let ty = if has_detail { r.center().y - 9.0 } else { r.center().y };
    ui.painter().galley(pos2(text_left, ty - title.size().y / 2.0), title, title_color);
    if has_detail {
        let g = trunc_galley(ui.painter(), &it.detail, FontId::proportional(11.5), p.text_tertiary, max_w);
        ui.painter().galley(pos2(text_left, r.center().y + 9.0 - g.size().y / 2.0), g, p.text_tertiary);
    }
}

fn footer(ui: &mut Ui, app: &SoyuzApp, p: &Palette, width: f32, clicked: &mut Option<(Exec, Modifiers)>) {
    divider(ui, p);
    let (rect, _) = ui.allocate_exact_size(vec2(width, FOOTER_H), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius { nw: 0, ne: 0, sw: 14, se: 14 },
        if p.dark { p.bg_surface2.gamma_multiply(0.6) } else { p.bg_surface2 },
    );
    let cy = rect.center().y;
    let mut x = rect.left() + 16.0;
    let hint = |x: &mut f32, keys: &str, label: &str| {
        let w = widgets::chips_width(ui, keys);
        paint_chips(ui, p, keys, *x + w, cy, false);
        *x += w + 4.0;
        let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.0), p.text_tertiary);
        ui.painter().galley(pos2(*x, cy - g.size().y / 2.0), g.clone(), p.text_tertiary);
        *x += g.size().x + 12.0;
    };
    hint(&mut x, "↑↓", "move");
    hint(&mut x, "Enter", "run");
    if matches!(app.palette.mode(), Mode::Path | Mode::Shell | Mode::Folders | Mode::Bookmarks) {
        hint(&mut x, "Tab", "complete");
    }
    // Prefix legend (clickable), right-aligned.
    let mut rx = rect.right() - 12.0;
    for m in Mode::PREFIXED.iter().rev() {
        let c = m.prefix().unwrap_or(' ');
        let label = format!("{c} {}", m.label().to_lowercase());
        let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.0), p.text_secondary);
        let r = Rect::from_min_size(pos2(rx - g.size().x - 12.0, cy - 10.0), vec2(g.size().x + 12.0, 20.0));
        if r.left() < x {
            break;
        }
        let resp = ui.interact(r, Id::new(("soyuz.palette.prefix", c)), Sense::click());
        let active = app.palette.mode() == *m;
        let fill = if active {
            p.accent_subtle
        } else if resp.hovered() {
            p.bg_surface3
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, CornerRadius::same(10), fill);
        if !active {
            ui.painter().rect_stroke(r, CornerRadius::same(10), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
        }
        let color = if active { p.accent } else { p.text_secondary };
        ui.painter().galley(pos2(r.left() + 6.0, cy - g.size().y / 2.0), g, color);
        if resp.clicked() {
            *clicked = Some((Exec::Prefix(c), Modifiers::NONE));
        }
        rx = r.left() - 5.0;
    }
}

/// Bottom-center chip while a key sequence or mark is pending ("G …").
fn sequence_hint(ctx: &egui::Context, app: &SoyuzApp) {
    let Some(label) = app.keyseq.pending_label() else { return };
    let p = palette(ctx);
    let screen = ctx.content_rect();
    let font = FontId::proportional(12.5);
    let g = ctx.fonts_mut(|f| f.layout_no_wrap(label, font, p.text_primary));
    let size = vec2(g.size().x + 46.0, 30.0);
    let rect = Rect::from_center_size(pos2(screen.center().x, screen.bottom() - 60.0), size);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("soyuz.keyseq")));
    painter.add(
        egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(80) }
            .as_shape(rect, CornerRadius::same(15)),
    );
    painter.rect_filled(rect, CornerRadius::same(15), p.bg_surface3);
    painter.rect_stroke(
        rect,
        CornerRadius::same(15),
        Stroke::new(1.0, p.accent.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.left() + 21.0, rect.center().y),
        Align2::CENTER_CENTER,
        ph::KEYBOARD,
        icons::font(14.0),
        p.accent,
    );
    painter.galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, p.text_primary);
}

#[cfg(test)]
mod tests;
