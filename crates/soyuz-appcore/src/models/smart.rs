//! Smart folders (Phase T0 model): saved query-language searches shown as live virtual
//! folders. Opening one shows its results as `Location::Search` (sortable and actionable
//! like any folder); while the background index can answer the query the results refresh
//! by themselves as the index changes (`services::index::pump`).
//!
//! Stored in `smart_folders.toml` next to the config (`[[folder]] name / query /
//! include_hidden`), loaded on a worker at startup and saved through the ordered store
//! worker. Each frontend draws the sidebar section, the editor and the index status
//! (`ui::smart` for the GUI); dialogs are asked for with [`FrontendCmd::Smart`].

use std::cell::RefCell;
use std::path::PathBuf;

use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use soyuz_core::Location;

use crate::core::AppCore;
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::ops::{self, NavOpts};
use crate::services::index::{self as ix};
use crate::services::search::{self as svc, SearchSpec};

/// One smart folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SmartFolder {
    pub name: String,
    /// Query-language text (`kind:image modified:<7d in:"~/Pictures"`).
    pub query: String,
    pub include_hidden: bool,
}

/// The persisted list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartStore {
    #[serde(default, rename = "folder")]
    pub folders: Vec<SmartFolder>,
}

impl SmartStore {
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Validate and store `f` at `index` (or append). Names are unique (case-insensitive).
    pub fn upsert(&mut self, index: Option<usize>, mut f: SmartFolder) -> Result<usize, String> {
        f.name = f.name.trim().to_string();
        f.query = f.query.trim().to_string();
        if f.name.is_empty() {
            return Err(tr!("views-smart-need-name").into());
        }
        if f.query.is_empty() {
            return Err(tr!("views-smart-query-empty").into());
        }
        soyuz_index::parse(&f.query).map_err(|e| e.msg)?;
        if self.folders.iter().enumerate().any(|(i, o)| Some(i) != index && o.name.eq_ignore_ascii_case(&f.name)) {
            return Err(tr!("views-smart-name-exists", name = &f.name));
        }
        match index {
            Some(i) if i < self.folders.len() => {
                self.folders[i] = f;
                Ok(i)
            }
            _ => {
                self.folders.push(f);
                Ok(self.folders.len() - 1)
            }
        }
    }
}

#[derive(Default)]
struct State {
    store: SmartStore,
    loaded: bool,
    started: bool,
    rx: Option<Receiver<SmartStore>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn store_path() -> PathBuf {
    soyuz_core::config::config_dir().join("smart_folders.toml")
}

/// Per frame (from `models::search::pump`): load the store once, drive the index service.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    STATE.with_borrow_mut(|s| {
        if !s.started {
            s.started = true;
            let (tx, rx) = crossbeam_channel::bounded(1);
            s.rx = Some(rx);
            let wake = host.wake();
            let _ = std::thread::Builder::new().name("soyuz-smart-load".into()).spawn(move || {
                // A broken file is backed up and never overwritten (store_file).
                let store = crate::services::store_file::load(&store_path(), SmartStore::from_toml).value();
                let _ = tx.send(store);
                wake();
            });
        }
        if let Some(store) = s.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            // Keep folders created before the file finished loading.
            let early = std::mem::take(&mut s.store.folders);
            s.store = store;
            for f in early {
                let _ = s.store.upsert(None, f);
            }
            s.loaded = true;
            s.rx = None;
        }
    });
    ix::pump(app, host);
}

fn save(s: &State) {
    crate::services::store_file::save_async(store_path(), s.store.to_toml().into_bytes());
}

/// The smart folders, in sidebar order.
pub fn folders() -> Vec<SmartFolder> {
    STATE.with_borrow(|s| s.store.folders.clone())
}

/// Commands from the sidebar, dialogs, search bar and status bar.
#[derive(Clone, Debug)]
pub enum SmartCmd {
    Open {
        index: usize,
        new_tab: bool,
    },
    /// Open the editor (`None` = new).
    Edit(Option<usize>),
    /// Editor result.
    Save {
        index: Option<usize>,
        folder: SmartFolder,
    },
    /// Ask before deleting.
    Delete(usize),
    ConfirmDelete(usize),
    /// New smart folder from the search shown in the focused pane.
    FromSearch,
    /// Settings → Preview & search (index status).
    OpenSettings,
    RebuildIndex,
}

/// A dialog the frontend should show.
#[derive(Clone, Debug, PartialEq)]
pub enum SmartPrompt {
    /// Create (`index: None`) or edit a smart folder; the result comes back as
    /// [`SmartCmd::Save`].
    Edit { index: Option<usize>, folder: SmartFolder },
    /// Ask before deleting; "Delete" sends [`SmartCmd::ConfirmDelete`].
    ConfirmDelete { index: usize, name: String },
}

/// Whether another smart folder than `index` is called `name` (case-insensitive).
pub fn name_taken(index: Option<usize>, name: &str) -> bool {
    folders().iter().enumerate().any(|(i, f)| Some(i) != index && f.name.eq_ignore_ascii_case(name.trim()))
}

pub fn apply(app: &mut AppCore, cmd: SmartCmd) {
    match cmd {
        SmartCmd::Open { index, new_tab } => open(app, index, new_tab),
        SmartCmd::Edit(index) => {
            let f = index.and_then(|i| folders().get(i).cloned()).unwrap_or_default();
            app.frontend.push(FrontendCmd::Smart(SmartPrompt::Edit { index, folder: f }));
        }
        SmartCmd::FromSearch => {
            let form = app.ui.search.form.clone();
            let name = match &app.ws.pane().location {
                Location::Search(id) => svc::spec(*id).map(|s| s.title).unwrap_or_else(|| form.title()),
                _ => form.title(),
            };
            let f = SmartFolder { name, query: ix::form_to_ql(&form), include_hidden: form.include_hidden };
            app.frontend.push(FrontendCmd::Smart(SmartPrompt::Edit { index: None, folder: f }));
        }
        SmartCmd::Save { index, folder } => {
            let res = STATE.with_borrow_mut(|s| {
                let r = s.store.upsert(index, folder.clone());
                if r.is_ok() {
                    save(s);
                }
                r
            });
            match res {
                Ok(i) => {
                    app.ui.toasts.push(Toast::success(tr!("views-smart-saved", name = folder.name.trim())));
                    open(app, i, false);
                }
                Err(e) => {
                    app.ui.toasts.push(Toast::error(tr!("views-smart-save-failed")).body(e));
                }
            }
        }
        SmartCmd::Delete(index) => {
            if let Some(f) = folders().get(index) {
                app.frontend.push(FrontendCmd::Smart(SmartPrompt::ConfirmDelete { index, name: f.name.clone() }));
            }
        }
        SmartCmd::ConfirmDelete(index) => {
            let removed = STATE.with_borrow_mut(|s| {
                (index < s.store.folders.len()).then(|| {
                    let f = s.store.folders.remove(index);
                    save(s);
                    f
                })
            });
            if let Some(f) = removed {
                app.ui.toasts.push(
                    Toast::info(tr!("views-smart-deleted", name = &f.name)).body(tr!("views-smart-deleted-body")),
                );
            }
        }
        SmartCmd::OpenSettings => {
            app.frontend.push(FrontendCmd::OpenSettings(Some(crate::models::settings::Section::Preview)));
        }
        SmartCmd::RebuildIndex => {
            if ix::rebuild() {
                app.ui.toasts.push(Toast::info(tr!("views-smart-rebuilding")).body(tr!("views-smart-rebuilding-body")));
            }
        }
    }
}

/// Hide index actions that can't do anything right now: Rebuild while indexing is off,
/// Save search when the focused pane shows no search.
pub fn palette_shows(app: &AppCore, a: &soyuz_core::config::Action) -> bool {
    use soyuz_core::config::Action as A;
    match a {
        A::RebuildIndex => ix::is_enabled(),
        A::SaveSearchAsSmartFolder => matches!(app.ws.pane().location, Location::Search(_)),
        _ => true,
    }
}

/// Register the search behind smart folder `i` and show it.
fn open(app: &mut AppCore, i: usize, new_tab: bool) {
    let Some(f) = folders().get(i).cloned() else { return };
    let plan = match ix::plan_from_query(&f.query, f.include_hidden) {
        Ok(p) => p,
        Err(e) => {
            app.ui.toasts.push(Toast::error(tr!("views-smart-error-title", name = &f.name)).body(e));
            return;
        }
    };
    let content_max = app.config.search.content_max_mib.max(1) * 1024 * 1024;
    let query = match ix::live_query(&plan, app.config.search.respect_gitignore, content_max) {
        Ok(q) => q,
        Err(_) if ix::can_serve(&plan) => soyuz_search::SearchQuery::default(),
        Err(e) => {
            app.ui.toasts.push(Toast::error(tr!("views-smart-error-title", name = &f.name)).body(e));
            return;
        }
    };
    let spec = SearchSpec { query, title: f.name.clone(), plan: Some(plan), smart: Some(f.name) };
    // The search bar would show an unrelated query; the status strip has "Edit".
    app.ui.search.open = false;
    let loc = Location::Search(svc::register(spec));
    if new_tab {
        ops::open_in_new_tab(app, loc, true);
    } else {
        let pane = app.ws.pane().id;
        ops::navigate(app, pane, loc, NavOpts::user());
    }
    app.ui.focus = crate::core::Focus::List;
}

/// The smart folder shown at `loc`, if any.
pub fn shown_name(loc: &Location) -> Option<String> {
    match loc {
        Location::Search(id) => svc::spec(*id).and_then(|s| s.smart),
        _ => None,
    }
}

/// Short status text for the index (status bar chip and settings).
pub fn status_line(st: &soyuz_index::IndexStatus) -> String {
    use soyuz_index::Phase;
    match &st.phase {
        Phase::Starting => tr!("views-index-starting").into(),
        Phase::Crawling => match st.progress() {
            Some(p) => tr!("views-index-crawling-pct", pct = (p * 100.0).round() as u64, count = st.scanned),
            None => tr!("views-index-crawling", count = st.scanned),
        },
        Phase::Verifying => tr!("views-index-verifying", count = st.scanned),
        Phase::CatchingUp => tr!("views-index-catching-up").into(),
        Phase::Contents => tr!("views-index-contents", count = st.content_files),
        Phase::Idle => tr!("views-index-idle", files = st.files, folders = st.folders),
        Phase::Failed(e) => tr!("views-index-failed", error = e.to_string()),
    }
}

/// The background index stopped with an error (no rebuild then).
pub fn index_failed() -> bool {
    ix::status().is_some_and(|st| matches!(st.phase, soyuz_index::Phase::Failed(_)))
}

/// Query cheat sheet for the smart-folder editor: `(example, what it does)`; the
/// examples are query syntax (shown untranslated, monospace).
pub fn query_help() -> [(&'static str, &'static str); 8] {
    [
        ("report \"exact phrase\"", tr!("views-smart-help-words")),
        ("*.pdf  ext:pdf,docx", tr!("views-smart-help-pattern")),
        ("kind:image  kind:folder", tr!("views-smart-help-kind")),
        ("size:>5MB  size:1k..2M", tr!("views-smart-help-size")),
        ("modified:<7d  created:2026-01", tr!("views-smart-help-age")),
        ("tag:red  path:docs  in:\"~/Projects\"", tr!("views-smart-help-tag")),
        ("content:todo", tr!("views-smart-help-content")),
        ("-draft  a OR b  (a b)", tr!("views-smart-help-logic")),
    ]
}

/// How a valid, non-empty `query` will be answered: live by the index, not covered by
/// the indexed roots, or a plain search because the index is off. `None` for an empty or
/// unparsable query (the editor shows [`validate`]'s error instead).
pub fn coverage_text(query: &str, include_hidden: bool) -> Option<&'static str> {
    if query.trim().is_empty() || soyuz_index::parse(query).is_err() {
        return None;
    }
    let live = ix::plan_from_query(query, include_hidden).ok().as_ref().is_some_and(ix::can_serve);
    Some(if live {
        tr!("views-smart-live")
    } else if ix::is_enabled() {
        tr!("views-smart-not-covered")
    } else {
        tr!("views-smart-index-off")
    })
}

/// Why folder `f` (the one at `editing`, or a new one) cannot be saved: no name, no
/// query, a query that does not parse, or a name another smart folder has. `None`: OK.
pub fn validate(f: &SmartFolder, editing: Option<usize>) -> Option<String> {
    if f.name.trim().is_empty() {
        return Some(tr!("views-smart-need-name").to_string());
    }
    if f.query.trim().is_empty() {
        return Some(tr!("views-smart-type-query").to_string());
    }
    if let Err(e) = soyuz_index::parse(&f.query) {
        return Some(e.msg);
    }
    name_taken(editing, &f.name).then(|| tr!("views-smart-name-taken").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_and_help() {
        assert_eq!(validate(&f("", "a"), None).as_deref(), Some(tr!("views-smart-need-name")));
        assert_eq!(validate(&f("x", " "), None).as_deref(), Some(tr!("views-smart-type-query")));
        assert!(validate(&f("x", "(a b"), None).is_some());
        assert!(coverage_text("", false).is_none());
        assert!(coverage_text("kind:image", false).is_some());
        assert!(query_help().iter().all(|(e, w)| !e.is_empty() && !w.is_empty()));
    }

    fn f(name: &str, query: &str) -> SmartFolder {
        SmartFolder { name: name.into(), query: query.into(), include_hidden: false }
    }

    #[test]
    fn store_round_trip_and_validation() {
        let mut s = SmartStore::default();
        assert_eq!(s.upsert(None, f(" Recent photos ", "kind:image modified:<7d")).unwrap(), 0);
        assert_eq!(s.upsert(None, f("Big", "size:>1GB")).unwrap(), 1);
        assert_eq!(s.folders[0].name, "Recent photos", "trimmed");
        assert!(s.upsert(None, f("recent PHOTOS", "x")).unwrap_err().contains("already"));
        assert!(s.upsert(None, f("", "x")).is_err());
        assert!(s.upsert(None, f("Bad", "size:>lots")).is_err());
        assert!(s.upsert(None, f("Empty", "  ")).is_err());
        // Rename in place keeps the position; the same name is fine for the same entry.
        assert_eq!(s.upsert(Some(1), f("Huge files", "size:>1GB")).unwrap(), 1);
        assert_eq!(s.upsert(Some(1), f("huge FILES", "size:>2GB")).unwrap(), 1);

        let text = s.to_toml();
        assert!(text.contains("[[folder]]"), "{text}");
        let back = SmartStore::from_toml(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(SmartStore::from_toml("").unwrap(), SmartStore::default());
        assert!(SmartStore::from_toml("folder = 3").is_err());
        let partial = SmartStore::from_toml("[[folder]]\nname = \"x\"").unwrap();
        assert_eq!(partial.folders[0].query, "");
    }

    #[test]
    fn persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smart_folders.toml");
        let mut s = SmartStore::default();
        s.upsert(None, f("Docs", "kind:document in:\"C:\\Users\\me\\Documents\"")).unwrap();
        crate::services::store_file::save_sync(&path, s.to_toml().as_bytes()).unwrap();
        let loaded = crate::services::store_file::load(&path, SmartStore::from_toml).value();
        assert_eq!(loaded, s);
        let plan = ix::plan_from_query(&loaded.folders[0].query, false).unwrap();
        assert_eq!(plan.scope, vec![PathBuf::from("C:\\Users\\me\\Documents")]);
    }
}
