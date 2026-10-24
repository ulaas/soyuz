//! Search (Phase T0 model): the search form and its options, saved searches
//! (`searches.toml` next to the config plus `[[search.saved]]` from the config), running a
//! search into the focused pane as `Location::Search`, the flat view and folder sizes.
//! Each frontend draws the bar (`ui::search` for the GUI).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::config::Config;
use soyuz_core::{Location, SearchId};
use soyuz_search::SavedSearch;

use crate::core::{AppCore, Focus};
use crate::host::Host;
use crate::models::toasts::Toast;
use crate::ops::{self, NavOpts};
use crate::services::search::{self as svc, DateRange, KindFilter, NameMode, SearchForm, SearchSpec};

/// A user-saved search (the form, so relative dates stay relative).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedEntry {
    pub name: String,
    pub form: SearchForm,
}

#[derive(Default, Serialize, Deserialize)]
struct SavedFile {
    #[serde(default)]
    saved: Vec<SavedEntry>,
}

/// The search bar's state.
pub struct SearchState {
    /// The bar is shown.
    pub open: bool,
    pub form: SearchForm,
    /// The frontend should focus the query field (consumed by the frontend).
    pub focus: bool,
    /// From `[[search.saved]]`.
    config_saved: Vec<SavedSearch>,
    /// User-saved (searches.toml).
    pub saved: Vec<SavedEntry>,
    /// The "save this search" prompt is open with this name.
    pub saving: Option<String>,
    /// Why the form could not be run.
    pub error: Option<String>,
    path: PathBuf,
    /// Searches that are flat views (so the toggle can leave them again).
    flat: BTreeSet<SearchId>,
}

/// The per-window part of the search bar. Every in-process window keeps its own, and
/// `models::windows` swaps it into [`SearchState`] while the window is handled. Saved
/// searches and flat views stay shared, so saving in one window never overwrites what
/// another window saved.
pub struct SearchUi {
    pub open: bool,
    pub form: SearchForm,
    pub focus: bool,
    pub saving: Option<String>,
    pub error: Option<String>,
}

impl SearchUi {
    /// A closed bar with the form's defaults from the config.
    pub fn new(config: &Config) -> Self {
        SearchUi {
            open: false,
            form: SearchForm { respect_gitignore: config.search.respect_gitignore, ..Default::default() },
            focus: false,
            saving: None,
            error: None,
        }
    }
}

impl SearchState {
    /// Exchange the per-window fields with `w` (its own inverse).
    pub fn swap_window(&mut self, w: &mut SearchUi) {
        std::mem::swap(&mut self.open, &mut w.open);
        std::mem::swap(&mut self.form, &mut w.form);
        std::mem::swap(&mut self.focus, &mut w.focus);
        std::mem::swap(&mut self.saving, &mut w.saving);
        std::mem::swap(&mut self.error, &mut w.error);
    }

    pub fn new(config: &Config) -> Self {
        let path = soyuz_core::config::config_dir().join("searches.toml");
        // A broken file is backed up and protected from being overwritten (store_file).
        let saved = crate::services::store_file::load_toml::<SavedFile>(&path).value().saved;
        let config_saved = config
            .search
            .saved
            .iter()
            .filter_map(|e| {
                SavedSearch::from_config(e).map_err(|err| log::warn!("[[search.saved]] {}: {err}", e.name)).ok()
            })
            .collect();
        let form = SearchForm { respect_gitignore: config.search.respect_gitignore, ..Default::default() };
        SearchState {
            open: false,
            form,
            focus: false,
            config_saved,
            saved,
            saving: None,
            error: None,
            path,
            flat: BTreeSet::new(),
        }
    }

    /// Sidebar names: config searches first, then the user's.
    pub fn names(&self) -> Vec<String> {
        self.config_saved.iter().map(|s| s.name.clone()).chain(self.saved.iter().map(|s| s.name.clone())).collect()
    }

    /// How many of [`SearchState::names`] come from the config (those can't be removed).
    pub fn config_count(&self) -> usize {
        self.config_saved.len()
    }

    fn persist(&self) -> Result<(), String> {
        let text = toml::to_string_pretty(&SavedFile { saved: self.saved.clone() }).map_err(|e| e.to_string())?;
        crate::services::store_file::save_sync(&self.path, text.as_bytes())
    }
}

/// The focused pane's folder, if it shows one.
pub fn current_dir(app: &AppCore) -> Option<PathBuf> {
    match &app.ws.pane().location {
        Location::Dir(p) => Some(p.clone()),
        _ => None,
    }
}

/// The search shown in the focused pane, if any.
pub fn shown_search(app: &AppCore) -> Option<SearchId> {
    match app.ws.pane().location {
        Location::Search(id) => Some(id),
        _ => None,
    }
}

/// Ctrl+Shift+F / F3 (and SearchContent): open the bar scoped to the current folder.
pub fn open(app: &mut AppCore, content: bool) {
    let s = &mut app.ui.search;
    s.open = true;
    s.focus = true;
    s.error = None;
    s.form.content = content;
    if let Some(d) = current_dir(app) {
        app.ui.search.form.root = d;
    }
}

/// Close the bar and give the file list the focus back.
pub fn close(app: &mut AppCore) {
    app.ui.search.open = false;
    app.ui.focus = Focus::List;
}

/// "Refine": open the bar for the search on screen.
pub fn refine(app: &mut AppCore) {
    app.ui.search.open = true;
    app.ui.search.focus = true;
}

/// Build the query from the form and show the results in the focused pane.
pub fn run(app: &mut AppCore) {
    if app.ui.search.form.root.as_os_str().is_empty()
        && let Some(d) = current_dir(app)
    {
        app.ui.search.form.root = d;
    }
    let form = app.ui.search.form.clone();
    let max = app.config.search.content_max_mib.max(1) * 1024 * 1024;
    if crate::services::index::uses_query_language(&form) || form.everywhere {
        run_indexed(app, &form, max);
        return;
    }
    if form.content
        && let Ok(plan) = crate::services::index::plan_from_form(&form)
        && crate::services::index::can_serve(&plan)
    {
        // Content search answered by the content index (FTS5).
        match form.build(SystemTime::now(), max) {
            Ok(query) => {
                app.ui.search.error = None;
                register_and_show(app, SearchSpec { query, title: form.title(), plan: Some(plan), smart: None });
            }
            Err(e) => app.ui.search.error = Some(e),
        }
        return;
    }
    match form.build(SystemTime::now(), max) {
        Ok(query) => {
            app.ui.search.error = None;
            run_query(app, query, form.title());
        }
        Err(e) => app.ui.search.error = Some(e),
    }
}

/// Query-language search: the index answers it when it covers the scope, else the
/// disk walk does (narrowed, every hit checked against the whole query).
fn run_indexed(app: &mut AppCore, form: &SearchForm, content_max: u64) {
    let ix = crate::services::index::plan_from_form(form).and_then(|plan| {
        let live = crate::services::index::live_query(&plan, form.respect_gitignore, content_max);
        match live {
            Ok(q) => Ok((plan, q)),
            Err(_) if crate::services::index::can_serve(&plan) => Ok((plan, soyuz_search::SearchQuery::default())),
            Err(e) => Err(e),
        }
    });
    match ix {
        Ok((plan, query)) => {
            app.ui.search.error = None;
            register_and_show(app, SearchSpec { query, title: form.title(), plan: Some(plan), smart: None });
        }
        Err(e) => app.ui.search.error = Some(e),
    }
}

fn register_and_show(app: &mut AppCore, spec: SearchSpec) {
    let id = svc::register(spec);
    let pane = app.ws.pane().id;
    ops::navigate(app, pane, Location::Search(id), NavOpts::user());
    app.ui.focus = Focus::List;
}

/// Show the results of `query` in the focused pane (also used by smart search).
pub fn run_query(app: &mut AppCore, query: soyuz_search::SearchQuery, title: String) -> SearchId {
    let id = svc::register(SearchSpec::new(query, title));
    let pane = app.ws.pane().id;
    ops::navigate(app, pane, Location::Search(id), NavOpts::user());
    app.ui.focus = Focus::List;
    id
}

/// What the bar asks for after a frame: `stop` (Stop pressed while the search on screen
/// runs), `run_now` (Run pressed or Enter), `changed` (an option changed: re-runs the
/// visible search).
pub fn submit(app: &mut AppCore, stop: bool, run_now: bool, changed: bool) {
    let shown = shown_search(app);
    if stop {
        if let Some(id) = shown {
            svc::cancel(id);
        }
    } else if run_now {
        run(app);
    } else if changed && shown.is_some() && !app.ui.search.form.is_empty() {
        // Option changes re-run the visible search.
        run(app);
    }
}

/// `ToggleFlatView`: list every file below the current folder in one flat, sortable
/// listing (a recursive soyuz-search walk without a name filter, streamed like search
/// results). Toggling again inside the flat listing returns to the folder.
pub fn toggle_flat_view(app: &mut AppCore) {
    let pane = app.ws.pane();
    if let Location::Search(id) = &pane.location {
        if app.ui.search.flat.contains(id) {
            let id = pane.id;
            ops::history_jump(app, id, true, 1);
        }
        return;
    }
    let Some(root) = current_dir(app) else {
        app.ui.toasts.push(Toast::info(tr!("views-search-flat-needs-folder")));
        return;
    };
    let query = soyuz_search::SearchQuery {
        roots: vec![root.clone()],
        include_hidden: app.ui.show_hidden,
        respect_gitignore: false,
        kinds: Some(soyuz_core::EntryKindSet::FILE | soyuz_core::EntryKindSet::SYMLINK),
        ..Default::default()
    };
    let title = tr!("views-search-flat-title", folder = soyuz_core::entry::display_name(&root));
    let id = run_query(app, query, title);
    app.ui.search.flat.insert(id);
}

/// Run sidebar entry `i` (config searches first, then user ones).
pub fn run_saved(app: &mut AppCore, i: usize) {
    let n_cfg = app.ui.search.config_saved.len();
    if let Some(s) = app.ui.search.config_saved.get(i).cloned() {
        let mut q = s.query;
        q.roots = q.roots.iter().map(|r| expand_home(r)).collect();
        if q.roots.is_empty() {
            q.roots.push(soyuz_fs::home());
        }
        run_query(app, q, s.name);
    } else if let Some(s) = app.ui.search.saved.get(i - n_cfg).cloned() {
        app.ui.search.form = s.form;
        app.ui.search.open = true;
        run(app);
    }
}

pub fn remove_saved(app: &mut AppCore, i: usize) {
    let n_cfg = app.ui.search.config_saved.len();
    if i >= n_cfg && i - n_cfg < app.ui.search.saved.len() {
        let removed = app.ui.search.saved.remove(i - n_cfg);
        if let Err(e) = app.ui.search.persist() {
            app.ui.toasts.push(Toast::error(tr!("views-search-save-failed")).body(e));
        } else {
            app.ui.toasts.push(Toast::info(tr!("views-search-removed", name = &removed.name)));
        }
    }
}

/// Start the "save this search" prompt, named after the form.
pub fn begin_save(app: &mut AppCore) {
    app.ui.search.saving = Some(app.ui.search.form.title());
}

/// Close the "save this search" prompt; `save` stores the form under the typed name
/// (replacing a search of the same name).
pub fn finish_save(app: &mut AppCore, save: bool) {
    let typed = app.ui.search.saving.take().unwrap_or_default();
    let name = typed.trim();
    if !save || name.is_empty() {
        return;
    }
    let form = app.ui.search.form.clone();
    let s = &mut app.ui.search;
    s.saved.retain(|e| e.name != name);
    s.saved.push(SavedEntry { name: name.to_string(), form });
    match s.persist() {
        Ok(()) => app.ui.toasts.push(Toast::success(tr!("views-search-saved", name = name))),
        Err(e) => app.ui.toasts.push(Toast::error(tr!("views-search-save-one-failed")).body(e)),
    };
}

fn expand_home(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if s == "~" {
        soyuz_fs::home()
    } else if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        soyuz_fs::home().join(rest)
    } else {
        p.to_path_buf()
    }
}

/// Per frame: stream search hits / folder sizes into panes, surface errors.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    svc::init(&app.wake, app.config.search.threads);
    svc::sync_document_text(&app.config);
    crate::models::smart::pump(app, host);
    // Searches and folder sizes stream into the panes of every window.
    let notices = svc::pump(&mut crate::models::windows::all_workspaces_mut(app));
    for n in notices {
        app.ui.toasts.push(Toast::error(tr!("views-search-failed")).body(n));
    }
    crate::models::archive::pump(app);
    if svc::sizing() {
        host.redraw_after(std::time::Duration::from_millis(200));
    }
}

/// "Calculate folder sizes": selected folders, or every folder in the pane.
pub fn folder_sizes(app: &mut AppCore) {
    let pane = app.ws.pane();
    if !matches!(pane.location, Location::Dir(_) | Location::Search(_)) {
        return;
    }
    let sel: Vec<PathBuf> =
        pane.selected_entries().iter().filter(|e| e.is_dir_like()).map(|e| e.path.clone()).collect();
    let dirs = if sel.is_empty() {
        pane.entries.iter().filter(|e| e.is_dir_like()).map(|e| e.path.clone()).collect()
    } else {
        sel
    };
    let id = pane.id;
    // Make sure the Size column is visible so the values can be seen arriving.
    let p = app.ws.pane_mut();
    if !p.view.columns.iter().any(|c| c.column == soyuz_core::Column::Size && c.visible) {
        p.view.columns.push(soyuz_core::ColumnSpec::new(soyuz_core::Column::Size));
    }
    let n = svc::start_folder_sizes(id, dirs);
    if n == 0 {
        app.ui.toasts.push(Toast::info(tr!("views-search-no-folders-to-measure")));
    }
}

// ---- status row -------------------------------------------------------------------------

/// What the status row under a search shows (the GUI's strip and the TUI's row): the
/// head (`"x" · 12 hits`), the detail (scanned / from the index, limit reached,
/// cancelled, unreadable, the folder being read) and which buttons exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchStatus {
    pub done: bool,
    pub cancelled: bool,
    pub head: String,
    /// The detail parts joined with ` · `.
    pub detail: String,
    /// The search is still running: Cancel is offered.
    pub can_cancel: bool,
    /// A fuzzy name search: Relevance is offered.
    pub fuzzy: bool,
    /// Results are ordered by relevance.
    pub ranked: bool,
    /// The smart folder this search shows (Edit is offered).
    pub smart: Option<String>,
    /// An index plan backs it: "Smart folder" (make one) is offered.
    pub can_make_smart: bool,
    /// The bar is closed and it is not a smart folder: Refine is offered.
    pub can_refine: bool,
}

/// A button of the status row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusAct {
    Cancel,
    /// Toggle the relevance order.
    Relevance,
    /// Edit the smart folder shown.
    EditSmart,
    /// Make a smart folder from the search.
    MakeSmart,
    Refine,
}

/// The status row of search `id` (`None`: it is no longer known).
pub fn status_line(app: &AppCore, id: SearchId) -> Option<SearchStatus> {
    let prog = svc::progress(id)?;
    let spec = svc::spec(id);
    let title = spec.as_ref().map(|s| s.title.clone()).unwrap_or_default();
    let head = tr!("views-search-head", title = title, count = prog.hits);
    let secs = prog.elapsed.as_secs_f32();
    let mut parts = vec![if prog.indexed {
        tr!("views-search-from-index", ms = prog.elapsed.as_millis().max(1) as u64)
    } else {
        tr!("views-search-scanned", count = prog.scanned, secs = soyuz_core::locale::format_decimal(f64::from(secs), 1))
    }];
    if prog.truncated && !prog.cancelled {
        parts.push(tr!("views-search-limit-reached").to_string());
    }
    if prog.cancelled {
        parts.push(tr!("views-search-cancelled").to_string());
    }
    if prog.errors > 0 {
        parts.push(tr!("views-search-unreadable", count = prog.errors));
    }
    if !prog.done && !prog.current.as_os_str().is_empty() {
        parts.push(soyuz_fs::strip_verbatim(&prog.current).display().to_string());
    }
    let fuzzy =
        spec.as_ref().and_then(|s| s.query.name.as_ref()).is_some_and(|n| n.kind == soyuz_core::PatternKind::Fuzzy);
    let smart = spec.as_ref().and_then(|s| s.smart.clone());
    let can_make_smart = smart.is_none() && spec.as_ref().is_some_and(|s| s.plan.is_some());
    let can_refine = !app.ui.search.open && smart.is_none();
    Some(SearchStatus {
        done: prog.done,
        cancelled: prog.cancelled,
        head,
        detail: parts.join(" · "),
        can_cancel: !prog.done,
        fuzzy,
        ranked: prog.ranked,
        smart,
        can_make_smart,
        can_refine,
    })
}

/// Press status-row button `act` for search `id`.
pub fn status_act(app: &mut AppCore, id: SearchId, act: StatusAct) {
    match act {
        StatusAct::Cancel => svc::cancel(id),
        StatusAct::Relevance => {
            if let Some(p) = svc::progress(id) {
                svc::set_ranked(id, !p.ranked);
            }
        }
        StatusAct::EditSmart => {
            let name = svc::spec(id).and_then(|s| s.smart);
            if let Some(name) = name
                && let Some(i) = crate::models::smart::folders().iter().position(|f| f.name == name)
            {
                app.cmds.push(crate::cmd::Cmd::Smart(crate::models::smart::SmartCmd::Edit(Some(i))));
            }
        }
        StatusAct::MakeSmart => {
            app.cmds.push(crate::cmd::Cmd::Smart(crate::models::smart::SmartCmd::FromSearch));
        }
        StatusAct::Refine => refine(app),
    }
}

// ---- option cycling (keyboard frontends) -------------------------------------------------

fn cycle<T: Copy + PartialEq>(all: &[T], now: T, forward: bool) -> T {
    let n = all.len();
    let i = all.iter().position(|x| *x == now).unwrap_or(0);
    all[if forward { (i + 1) % n } else { (i + n - 1) % n }]
}

/// The next (or previous) name mode.
pub fn cycle_mode(form: &mut SearchForm, forward: bool) {
    form.mode = cycle(&NameMode::ALL, form.mode, forward);
}

/// The next (or previous) date range.
pub fn cycle_date(form: &mut SearchForm, forward: bool) {
    form.date = cycle(&DateRange::ALL, form.date, forward);
}

/// The next (or previous) kind filter.
pub fn cycle_kind(form: &mut SearchForm, forward: bool) {
    form.kind = cycle(&KindFilter::ALL, form.kind, forward);
}

// ---- labels ---------------------------------------------------------------------------

/// The size option's label (`> 100M`, `< 16k`, `1k – 2M`).
pub fn size_label(f: &SearchForm) -> String {
    match (f.min_size.trim(), f.max_size.trim()) {
        ("", "") => tr!("views-search-size").into(),
        (a, "") => format!("> {a}"),
        ("", b) => format!("< {b}"),
        (a, b) => format!("{a} – {b}"),
    }
}

/// Translated name-mode label (`NameMode::label` stays English).
pub fn mode_label(m: NameMode) -> &'static str {
    match m {
        NameMode::Substring => tr!("views-search-mode-contains"),
        NameMode::Glob => tr!("views-search-mode-glob"),
        NameMode::Regex => tr!("views-search-mode-regex"),
        NameMode::Fuzzy => tr!("views-search-mode-fuzzy"),
    }
}

/// Translated kind-filter label.
pub fn kind_label(k: KindFilter) -> &'static str {
    match k {
        KindFilter::Any => tr!("views-search-kind-any"),
        KindFilter::Files => tr!("views-search-kind-files"),
        KindFilter::Folders => tr!("views-search-kind-folders"),
        KindFilter::Images => tr!("views-search-kind-images"),
        KindFilter::Videos => tr!("views-search-kind-videos"),
        KindFilter::Audio => tr!("views-search-kind-audio"),
        KindFilter::Documents => tr!("views-search-kind-documents"),
        KindFilter::Archives => tr!("views-search-kind-archives"),
        KindFilter::Code => tr!("views-search-kind-code"),
    }
}

/// Translated date-range label.
pub fn date_label(d: DateRange) -> &'static str {
    match d {
        DateRange::Any => tr!("views-search-date-any"),
        DateRange::Today => tr!("views-search-date-today"),
        DateRange::Week => tr!("views-search-date-week"),
        DateRange::Month => tr!("views-search-date-month"),
        DateRange::Year => tr!("views-search-date-year"),
        DateRange::OlderThanYear => tr!("views-search-date-older"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_wraps_both_ways() {
        let mut f = SearchForm::default();
        assert_eq!(f.mode, NameMode::Substring);
        cycle_mode(&mut f, true);
        assert_eq!(f.mode, NameMode::Glob);
        cycle_mode(&mut f, false);
        cycle_mode(&mut f, false);
        assert_eq!(f.mode, NameMode::Fuzzy);
        for _ in 0..DateRange::ALL.len() {
            cycle_date(&mut f, true);
        }
        assert_eq!(f.date, DateRange::Any);
        cycle_kind(&mut f, false);
        assert_eq!(f.kind, *KindFilter::ALL.last().unwrap());
        cycle_kind(&mut f, true);
        assert_eq!(f.kind, KindFilter::Any);
    }
}
