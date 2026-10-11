//! Command palette (Ctrl+Shift+P / Ctrl+K) and quick "Go to folder" (Ctrl+P).
//!
//! One centered, animated overlay with a fuzzy-ranked list. Prefixes switch modes (see
//! [`query`]): `:` path with folder completion, `@` bookmarks / recent folders, `/`
//! recursive search, `>` shell command in the terminal panel, `#` switch tab. Recently
//! used commands float to the top; folder visits are tracked with frecency and
//! persisted in `palette.toml` together with recent shell commands and marks.
//!
//! This module is the frontend-free half: state, result building, key handling and
//! execution. The GUI draws it in `crate::ui::palette`.

pub mod catalog;
pub mod chips;
pub mod frecency;
pub mod fuzzy;
pub mod query;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_core::config::Action;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::geom::Point;
use crate::host::Host;
use crate::icon::IconId;
use crate::ops::{self, NavOpts};
use crate::state::crumbs::{location_title, split_for_completion};

use frecency::{Memory, now_secs};
use fuzzy::Fuzzy;
use query::Mode;

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
    pub icon: IconId,
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
    fn new(exec: Exec, title: impl Into<String>, icon: IconId, section: &'static str) -> Self {
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
    pub base: Mode,
    pub input: String,
    pub selected: usize,
    /// Frontend time the palette opened at (`NAN` until its first drawn frame).
    pub opened_at: f64,
    /// Focus the input on the next frame.
    pub focus_input: bool,
    /// Move the input cursor to the end on the next frame.
    pub cursor_to_end: bool,
    pub scroll_to_selected: bool,
    pub results: Vec<Item>,
    /// Input, base mode, source version and `i18n::generation()` of the cached results.
    results_key: Option<(String, Mode, u64, u64)>,
    /// Bumped whenever sources change (forces a rebuild).
    version: u64,
    pub memory: Memory,
    pub dirty: bool,
    memory_path: PathBuf,
    last_save: Instant,
    last_dir: Option<PathBuf>,
    /// Pointer position when the palette opened (hover selects only after it moved).
    pub pointer_at_open: Option<Point>,
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
pub fn begin_frame(app: &mut SoyuzApp, host: &dyn Host) {
    crate::services::config_watch::pump(app, host);
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
    v.extend(crate::automation::menu::palette_actions());
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
                if c.terminal {
                    tr!("actions-palette-terminal-detail", command = line.trim())
                } else {
                    line.trim().to_string()
                }
            })
            .unwrap_or(e.description),
        Action::RunScript(id) => crate::automation::menu::palette_detail(id).unwrap_or(e.description),
        _ => e.description,
    };
    it.chord = chord_for(app, a);
    it.tag = Some(crate::i18n::labels::action_category(e.category).to_string());
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
            let mut it = action_item(app, a, tr!("actions-section-recent"));
            it.tag = Some(tr!("actions-tag-recent").into());
            out.push(it);
        }
        let mut rest: Vec<Item> = actions
            .iter()
            .filter(|a| !recent.iter().take(6).any(|r| r == *a))
            .map(|a| action_item(app, a, tr!("actions-section-all-commands")))
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
            ca.cmp(&cb).then_with(|| soyuz_core::natural_cmp(&a.title, &b.title, true))
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
        let mut it = action_item(app, a, tr!("actions-section-commands"));
        it.score = m.score;
        it.hl = m.hl;
        if m.recent {
            it.tag = Some(tr!("actions-tag-recent").into());
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

/// Pure command ranking: title matches count double (the translated or the English
/// title, so English muscle memory keeps working), keywords / category / config name
/// match too; recently used commands get a boost that fades with their position.
pub fn score_actions(entries: &[(Action, catalog::Entry)], recent: &[Action], q: &str) -> Vec<Ranked> {
    let mut f = Fuzzy::new(q);
    let mut out = Vec::new();
    for (index, (a, e)) in entries.iter().enumerate() {
        let title = f.indices(&e.title);
        let title_en = if e.title_en == e.title { None } else { f.score(&e.title_en) };
        let extra = f.score(&format!(
            "{} {} {} {} {}",
            e.title,
            e.keywords,
            crate::i18n::labels::action_category(e.category),
            e.category.label(),
            a.name().replace('_', " ")
        ));
        let (score, hl) = match (title, title_en, extra) {
            (Some((s, hl)), _, _) => (s as i64 * 2, hl),
            (None, Some(s), _) => (s as i64 * 2, Vec::new()),
            (None, None, Some(s)) => (s as i64, Vec::new()),
            (None, None, None) => continue,
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

/// Bookmarks, favorites and frecency folders, deduplicated. The kind (`"Bookmark"`,
/// `"Favorite"`, `"Recent"`, `"Here"`) is an identifier, not shown as is.
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
            (false, _) => tr!("actions-section-folders"),
            (true, "Recent") => tr!("actions-section-recent-frequent"),
            (true, _) => tr!("actions-section-bookmarks-favorites"),
        };
        let glyph = match kind {
            "Bookmark" => IconId::BookmarkSimple,
            "Favorite" => IconId::Star,
            _ => crate::models::sidebar::favorite_icon(&path),
        };
        let mut it = Item::new(Exec::Go(Location::Dir(path.clone())), label, glyph, section);
        it.detail = path_text;
        it.tint = if matches!(kind, "Recent" | "Here") { Tint::Folder } else { Tint::Warning };
        it.tag = Some(
            if current.as_deref().is_some_and(|c| same_path(c, &shown)) {
                tr!("actions-tag-current")
            } else {
                match kind {
                    "Bookmark" => tr!("actions-tag-bookmark"),
                    "Favorite" => tr!("actions-tag-favorite"),
                    "Recent" => tr!("actions-tag-recent"),
                    _ => tr!("actions-tag-here"),
                }
            }
            .into(),
        );
        it.hl = hl;
        let kind_rank = if bookmarks_first && kind != "Recent" { 10_000 } else { 0 };
        it.score = score + (fr.max(0.0).ln_1p() * 40.0) as i64 + kind_rank;
        out.push(it);
    }
    if q.trim().is_empty() {
        // Sections: recent first (Ctrl+P), bookmarks first (`@`).
        let recent = tr!("actions-section-recent-frequent");
        out.sort_by(|a, b| {
            let ra = (a.section == recent) != bookmarks_first;
            let rb = (b.section == recent) != bookmarks_first;
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
        let mut it = Item::new(
            Exec::Go(loc.clone()),
            tr!("actions-palette-go-to", path = loc.display()),
            IconId::ArrowSquareRight,
            tr!("actions-section-path"),
        );
        it.detail = tr!("actions-palette-go-hint").into();
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
                crate::models::sidebar::favorite_icon(&target),
                tr!("actions-section-folders"),
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
                    g.section = tr!("actions-section-path");
                    g
                }));
            }
        }
    } else if loading {
        let mut it =
            Item::new(Exec::Nothing, tr!("actions-palette-listing"), IconId::Hourglass, tr!("actions-section-folders"));
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
        .unwrap_or_else(|| tr!("actions-palette-this-folder").into());
    let section = tr!("actions-section-search");
    if q.trim().is_empty() {
        let mut it = Item::new(Exec::Nothing, tr!("actions-palette-search-empty"), IconId::MagnifyingGlass, section);
        it.detail = tr!("actions-palette-search-empty-detail", dir = &dir);
        it.tint = Tint::Neutral;
        return vec![it];
    }
    let mut a = Item::new(
        Exec::Search { text: q.to_string(), content: false },
        tr!("actions-palette-search-names", query = q),
        IconId::MagnifyingGlass,
        section,
    );
    a.detail = tr!("actions-palette-in-subfolders", dir = &dir);
    let mut b = Item::new(
        Exec::Search { text: q.to_string(), content: true },
        tr!("actions-palette-search-contents", query = q),
        IconId::FileSearch,
        section,
    );
    b.detail = tr!("actions-palette-in-subfolders", dir = &dir);
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
        let mut it = Item::new(
            Exec::Shell(q.to_string()),
            tr!("actions-palette-run", command = q),
            IconId::Terminal,
            tr!("actions-section-shell"),
        );
        it.detail = tr!("actions-palette-run-detail", dir = &dir);
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
            IconId::ClockCounterClockwise,
            tr!("actions-section-recent-commands"),
        );
        it.detail = tr!("actions-palette-shell-recent-hint").into();
        it.tint = Tint::Neutral;
        it.hl = hl;
        it.score = s as i64;
        it.exec = Exec::Shell(line.clone());
        it.tag = Some(tr!("actions-tag-recent").into());
        out.push(it);
    }
    if out.is_empty() {
        let mut it = Item::new(
            Exec::Nothing,
            tr!("actions-palette-shell-empty"),
            IconId::Terminal,
            tr!("actions-section-shell"),
        );
        it.detail = tr!("actions-palette-in-dir", dir = &dir);
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
        let mut it = Item::new(
            Exec::Tab(i),
            label,
            if t.pinned { IconId::PushPin } else { IconId::Browsers },
            tr!("actions-section-tabs"),
        );
        it.detail = detail;
        it.hl = hl;
        it.score = score;
        it.tint = Tint::Folder;
        if i == app.ws.active {
            it.tag = Some(tr!("actions-tag-current").into());
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

/// Modifiers held when an item runs: `command` opens folders in a new tab, `shift` turns
/// a folder completion into "complete only" (no navigation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunMods {
    pub command: bool,
    pub shift: bool,
}

impl RunMods {
    pub const NONE: RunMods = RunMods { command: false, shift: false };
}

/// Run `exec` (Enter, a click, Tab completion, a prefix chip).
pub fn run(app: &mut SoyuzApp, host: &dyn Host, exec: Exec, mods: RunMods) {
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
            crate::ui::panels::terminal::run_command(app, crate::host::egui_ctx(host), dir, &line);
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
// Per frame
// ---------------------------------------------------------------------------

/// (Re)build the results when the query or the sources changed; `:` paths rebuild every
/// frame while their folder is listed. Keeps the selection in range.
pub fn refresh_results(app: &mut SoyuzApp) {
    let key = (app.palette.input.clone(), app.palette.base, app.palette.version, crate::i18n::generation());
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
}

/// The palette keys of one frame, read by the frontend before the input field sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyInput {
    /// A global palette chord (command palette / jump to folder): toggles or switches mode.
    pub toggle: Option<Mode>,
    /// Esc: clears the input, or closes the palette when the input is empty (or only a
    /// mode prefix). The frontend reads no other key in that frame.
    pub escape: bool,
    /// Down / Ctrl+J / Ctrl+N.
    pub down: bool,
    /// Up / Ctrl+K / Ctrl+P.
    pub up: bool,
    pub page_down: bool,
    pub page_up: bool,
    /// Enter, with its modifiers.
    pub enter: Option<RunMods>,
    /// Tab: complete the selected item.
    pub tab: bool,
}

/// Apply one frame's palette keys (selection, Esc, Enter, Tab, mode toggles).
pub fn apply_keys(app: &mut SoyuzApp, host: &dyn Host, k: KeyInput) {
    let n = app.palette.results.len();
    let mut exec: Option<(Exec, RunMods)> = None;
    let pal = &mut app.palette;
    if k.escape {
        if pal.input.is_empty() || pal.input.chars().count() == 1 && pal.mode().prefix().is_some() {
            pal.open = false;
        } else {
            pal.input.clear();
            pal.cursor_to_end = true;
        }
    } else {
        if n > 0 {
            let cur = pal.selected.min(n - 1);
            let next = if k.down {
                (cur + 1) % n
            } else if k.up {
                (cur + n - 1) % n
            } else if k.page_down {
                (cur + 8).min(n - 1)
            } else if k.page_up {
                cur.saturating_sub(8)
            } else {
                cur
            };
            if next != cur || k.down || k.up {
                pal.selected = next;
                pal.scroll_to_selected = true;
            }
        }
        if let Some(m) = k.enter
            && let Some(item) = pal.results.get(pal.selected)
        {
            exec = Some((item.exec.clone(), m));
        }
        if k.tab
            && let Some(item) = pal.results.get(pal.selected)
            && let Some(text) = completion_of(item)
        {
            exec = Some((Exec::Complete { text, go: None }, RunMods::NONE));
        }
    }
    if let Some(mode) = k.toggle {
        if app.palette.base == mode && !app.palette.input.is_empty() || app.palette.base != mode {
            open(app, mode, "");
        } else {
            close(app);
        }
        return;
    }
    if let Some((e, m)) = exec {
        run(app, host, e, m);
    }
}

/// Uppercase section header text; Turkish maps i → İ and ı → I.
pub fn upper(s: &str) -> String {
    if crate::i18n::current() == soyuz_core::Locale::TrTr {
        s.chars()
            .flat_map(|c| match c {
                'i' => vec!['İ'],
                'ı' => vec!['I'],
                c => c.to_uppercase().collect(),
            })
            .collect()
    } else {
        s.to_uppercase()
    }
}

#[cfg(test)]
mod tests;
