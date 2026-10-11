//! App side of the background index.
//!
//! * [`pump`] (every frame) starts / restarts / stops the `soyuz-index` worker when
//!   `[search.index]` changes (debounced, so typing a root doesn't restart it per key),
//!   feeds it the color tags, caches its status for the UI and refreshes open smart
//!   folders when the index changes.
//! * [`plan_from_form`] turns the search bar (query-language text + option pills) into an
//!   [`IndexPlan`]; `services::search::attach` runs it on the index when [`can_serve`]
//!   says the index covers the scope, else walks the disk with [`live_query`] and checks
//!   each hit with a `soyuz_index::Matcher`.
//!
//! The database lives in `config::local_data_dir()/index/` (machine-local, not roaming;
//! `portable-data/index/` in portable mode). All state is UI-thread-local; the worker and
//! queries run on their own threads.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use soyuz_core::Location;
use soyuz_core::config::SearchIndex;
use soyuz_index::query::{Range, TimeSpec};
use soyuz_index::{Expr, Index, IndexConfig, IndexStatus, KindClass, Phase, SearchRequest, Term};
use soyuz_search::{SearchHandle, SearchQuery};

use crate::app::SoyuzApp;
use crate::services::search::{DateRange, KindFilter, NameMode, SearchForm};
use crate::state::pane::{ListingStatus, PaneId};

/// Config edits settle for this long before the worker restarts.
const RESTART_DEBOUNCE: Duration = Duration::from_millis(1200);

#[derive(Default)]
struct Reg {
    index: Option<Index>,
    applied: Option<IndexConfig>,
    pending: Option<(Option<IndexConfig>, Instant)>,
    started: bool,
    use_for_search: bool,
    tags_version: Option<u64>,
    status: Option<IndexStatus>,
    /// `rescan_interval_secs` last handed to the running worker.
    rescan_secs: Option<u64>,
    /// Smart folder panes: generation they last showed, and when.
    refreshed: HashMap<PaneId, (u64, Instant)>,
}

thread_local! {
    static REG: RefCell<Reg> = RefCell::new(Reg::default());
}

/// Folder holding the index database.
pub fn index_dir() -> PathBuf {
    soyuz_core::config::local_data_dir().join("index")
}

/// A configured root (`~`, `%VAR%` / `$VAR` expanded).
pub fn expand_root(s: &str) -> PathBuf {
    PathBuf::from(soyuz_fs::expand_env(s.trim()))
}

/// `IndexConfig` for `[search.index]` (no I/O).
pub fn index_config(c: &SearchIndex, debounce: Duration) -> IndexConfig {
    IndexConfig {
        db_path: index_dir().join("soyuz-index.db"),
        roots: c.roots.iter().filter(|r| !r.trim().is_empty()).map(|r| expand_root(r)).collect(),
        exclude: c.effective_exclude(),
        content: c.content,
        content_max_bytes: c.content_max_kib.max(1).saturating_mul(1024),
        content_extensions: c
            .content_extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty())
            .collect(),
        usn: c.usn_journal,
        throttle_ms: c.throttle_ms,
        watch_debounce: debounce,
    }
}

/// Per frame: reconcile the worker with the config, push tags, refresh smart folders.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    let c = &app.config.search.index;
    let debounce = Duration::from_millis(app.config.behaviour.watch_debounce_ms.clamp(100, 5000));
    let desired = c.enabled.then(|| index_config(c, debounce));
    let refresh_every = Duration::from_secs(c.smart_refresh_secs.max(1));
    let use_for_search = c.use_for_search;
    let rescan_secs = c.rescan_interval_secs.max(1);
    let changed = REG.with_borrow_mut(|r| {
        r.use_for_search = use_for_search;
        if desired == r.applied {
            r.pending = None;
            return false;
        }
        // Apply the startup config right away; later edits after they settle.
        let now = Instant::now();
        let ready = match &r.pending {
            _ if !r.started => true,
            Some((p, since)) if *p == desired => since.elapsed() >= RESTART_DEBOUNCE,
            _ => {
                r.pending = Some((desired.clone(), now));
                false
            }
        };
        if !ready {
            ctx.request_repaint_after(RESTART_DEBOUNCE);
            return false;
        }
        r.started = true;
        r.pending = None;
        if let Some(old) = r.index.take() {
            // Stopping waits for the worker (bounded); never on the UI thread.
            let _ = std::thread::Builder::new().name("soyuz-index-stop".into()).spawn(move || drop(old));
        }
        let wake_ctx = ctx.clone();
        r.index = desired.clone().map(|cfg| Index::start(cfg, Arc::new(move || wake_ctx.request_repaint())));
        r.applied = desired.clone();
        r.tags_version = None;
        r.status = None;
        r.rescan_secs = None;
        true
    });
    if changed {
        log::info!("index: {}", if app.config.search.index.enabled { "started" } else { "off" });
    }
    let index_gen = REG.with_borrow_mut(|r| {
        let ix = r.index.as_ref()?;
        // Polled roots (watch limit reached): applied live, no restart needed.
        if r.rescan_secs != Some(rescan_secs) {
            r.rescan_secs = Some(rescan_secs);
            ix.set_rescan_interval(Duration::from_secs(rescan_secs));
        }
        let v = crate::services::tags::version();
        if r.tags_version != Some(v) {
            r.tags_version = Some(v);
            ix.set_tags(crate::services::tags::snapshot());
        }
        let st = ix.status();
        if !matches!(st.phase, Phase::Idle | Phase::Failed(_)) {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
        if r.status.as_ref().map(|o| &o.phase) != Some(&st.phase) {
            log::debug!("index phase: {:?}", st.phase);
        }
        r.status = Some(st);
        Some(ix.generation())
    });
    let Some(index_gen) = index_gen else { return };
    // Live smart folders: re-run on index changes (throttled), keeping the selection.
    let mut live = Vec::new();
    for tab in &mut app.ws.tabs {
        for pane in &mut tab.panes {
            let Location::Search(id) = pane.location else { continue };
            let Some(spec) = crate::services::search::spec(id) else { continue };
            let Some(plan) = spec.plan.as_ref().filter(|_| spec.smart.is_some()) else { continue };
            live.push(pane.id);
            if !matches!(pane.status, ListingStatus::Ready)
                || pane.refreshing
                || crate::services::search::is_running(id)
            {
                continue;
            }
            let due = REG.with_borrow_mut(|r| {
                let servable = can_serve_in(r, plan);
                let e = r.refreshed.entry(pane.id).or_insert((index_gen, Instant::now()));
                if e.0 == index_gen {
                    return false;
                }
                if e.1.elapsed() >= refresh_every && servable {
                    *e = (index_gen, Instant::now());
                    return true;
                }
                ctx.request_repaint_after(refresh_every);
                false
            });
            if due {
                crate::services::search::refresh_soft(pane, id);
            }
        }
    }
    REG.with_borrow_mut(|r| r.refreshed.retain(|p, _| live.contains(p)));
}

/// Cached status (`None` when indexing is off).
pub fn status() -> Option<IndexStatus> {
    REG.with_borrow(|r| r.status.clone())
}

/// Index roots without a change watcher (Linux: the inotify watch limit is used up);
/// they are re-checked every `[search.index] rescan_interval_secs`, so changes show up
/// late. Empty when indexing is off or every root is watched.
pub fn polled_roots() -> Vec<PathBuf> {
    REG.with_borrow(|r| r.status.as_ref().map(|s| s.polled_roots.clone()).unwrap_or_default())
}

pub fn is_enabled() -> bool {
    REG.with_borrow(|r| r.index.is_some())
}

pub fn generation() -> u64 {
    REG.with_borrow(|r| r.index.as_ref().map(Index::generation).unwrap_or(0))
}

/// Drop the index and crawl again.
pub fn rebuild() -> bool {
    REG.with_borrow(|r| {
        r.index.as_ref().map(|i| {
            i.rebuild();
            true
        })
    })
    .unwrap_or(false)
}

/// The indexed roots (canonical once the worker has started).
pub fn roots() -> Vec<PathBuf> {
    REG.with_borrow(|r| {
        r.status
            .as_ref()
            .map(|s| s.roots.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| r.applied.as_ref().map(|c| c.roots.clone()))
            .unwrap_or_default()
    })
}

/// What a search asks of the index.
#[derive(Clone, Debug)]
pub struct IndexPlan {
    pub expr: Expr,
    /// Restrict to these folders; empty = every indexed root.
    pub scope: Vec<PathBuf>,
    pub include_hidden: bool,
    /// 0 = unlimited.
    pub limit: usize,
    /// A smart folder: uses the index even when the search bar is told not to.
    pub smart: bool,
}

impl IndexPlan {
    /// Folder that hit names are shown relative to (empty = full paths).
    pub fn name_root(&self) -> PathBuf {
        match self.scope.as_slice() {
            [one] => one.clone(),
            [] => common_ancestor(&roots()),
            many => common_ancestor(many),
        }
    }
}

fn common_ancestor(paths: &[PathBuf]) -> PathBuf {
    let Some(first) = paths.first() else { return PathBuf::new() };
    let mut acc = first.clone();
    for p in &paths[1..] {
        while !p.starts_with(&acc) {
            if !acc.pop() {
                return PathBuf::new();
            }
        }
    }
    acc
}

fn can_serve_in(r: &Reg, plan: &IndexPlan) -> bool {
    let (Some(ix), Some(st)) = (r.index.as_ref(), r.status.as_ref()) else { return false };
    if !r.use_for_search && !plan.smart {
        return false;
    }
    if plan.expr.uses_content() && !(st.content_enabled && matches!(st.phase, Phase::Idle | Phase::Verifying)) {
        return false;
    }
    if plan.scope.is_empty() {
        !st.roots.is_empty() && st.roots.iter().all(|root| st.ready_roots.contains(root))
    } else {
        plan.scope.iter().all(|d| ix.covers(d))
    }
}

/// Whether the index can answer `plan` right now (enabled, scope fully indexed, and
/// contents indexed when the query needs them).
pub fn can_serve(plan: &IndexPlan) -> bool {
    REG.with_borrow(|r| can_serve_in(r, plan))
}

/// Whether `dir` is fully indexed (search bar hint).
pub fn covers(dir: &Path) -> bool {
    REG.with_borrow(|r| r.use_for_search && r.index.as_ref().is_some_and(|i| i.covers(dir)))
}

/// Run `plan` on the index (`None` when it can't serve it).
pub fn search(plan: &IndexPlan, wake: Arc<dyn Fn() + Send + Sync>) -> Option<SearchHandle> {
    REG.with_borrow(|r| {
        if !can_serve_in(r, plan) {
            return None;
        }
        let req = SearchRequest {
            expr: plan.expr.clone(),
            include_hidden: plan.include_hidden,
            scope: plan.scope.clone(),
            limit: plan.limit,
            now: SystemTime::now(),
        };
        r.index.as_ref().map(|i| i.search_async(req, wake))
    })
}

/// Whether the search bar treats the text as query language (`Contains` / `Glob` name
/// modes; `Regex` / `Fuzzy` and content searches keep their own matching).
pub fn uses_query_language(form: &SearchForm) -> bool {
    !form.content && !form.case_sensitive && matches!(form.mode, NameMode::Substring | NameMode::Glob)
}

fn age(days: i64) -> i64 {
    days * 86_400
}

/// The option pills as query terms.
fn pill_terms(form: &SearchForm) -> Result<Vec<Expr>, String> {
    let mut out = Vec::new();
    let size = |s: &str| -> Result<Option<u64>, String> {
        match s.trim() {
            "" => Ok(None),
            t => soyuz_index::query::parse_size(t).map(Some).ok_or_else(|| tr!("services-index-bad-size", size = t)),
        }
    };
    let (min, max) = (size(&form.min_size)?, size(&form.max_size)?);
    if let (Some(a), Some(b)) = (min, max)
        && a > b
    {
        return Err(tr!("services-index-size-range").into());
    }
    if min.is_some() || max.is_some() {
        out.push(Expr::Term(Term::Size(Range { lo: min, hi: max.map(|m| m.saturating_add(1)) })));
    }
    let time = match form.date {
        DateRange::Any => None,
        DateRange::Today => Some(TimeSpec::Age { min: None, max: Some(age(1)) }),
        DateRange::Week => Some(TimeSpec::Age { min: None, max: Some(age(7)) }),
        DateRange::Month => Some(TimeSpec::Age { min: None, max: Some(age(30)) }),
        DateRange::Year => Some(TimeSpec::Age { min: None, max: Some(age(365)) }),
        DateRange::OlderThanYear => Some(TimeSpec::Age { min: Some(age(365)), max: None }),
    };
    if let Some(t) = time {
        out.push(Expr::Term(Term::Modified(t)));
    }
    let kind = match form.kind {
        KindFilter::Any => None,
        KindFilter::Files => Some(KindClass::File),
        KindFilter::Folders => Some(KindClass::Folder),
        KindFilter::Images => Some(KindClass::Image),
        KindFilter::Videos => Some(KindClass::Video),
        KindFilter::Audio => Some(KindClass::Audio),
        KindFilter::Documents => Some(KindClass::Document),
        KindFilter::Archives => Some(KindClass::Archive),
        KindFilter::Code => Some(KindClass::Code),
    };
    if let Some(k) = kind {
        out.push(Expr::Term(Term::Kind(k)));
    }
    Ok(out)
}

/// Search bar → plan. `Err` carries a message for the bar (parse errors, bad sizes).
pub fn plan_from_form(form: &SearchForm) -> Result<IndexPlan, String> {
    let text = form.text.trim();
    let mut expr = if form.content {
        let mut parts = Vec::new();
        if !text.is_empty() {
            parts.push(Expr::Term(Term::Content(text.to_string())));
        }
        let g = form.name_glob.trim();
        if !g.is_empty() {
            parts.push(Expr::Term(if g.contains(['*', '?', '[']) {
                Term::NameGlob(g.to_string())
            } else {
                Term::Name(g.to_string())
            }));
        }
        Expr::And(parts)
    } else {
        soyuz_index::parse(text).map_err(|e| e.msg)?
    };
    for t in pill_terms(form)? {
        expr = expr.and(t);
    }
    if expr.is_empty() {
        return Err(tr!("services-index-empty-query").into());
    }
    let scope = if form.everywhere { Vec::new() } else { vec![soyuz_fs::strip_verbatim(&form.root)] };
    if scope.iter().any(|s| s.as_os_str().is_empty()) {
        return Err(tr!("services-index-no-folder").into());
    }
    Ok(IndexPlan {
        expr,
        scope,
        include_hidden: form.include_hidden,
        limit: SearchQuery::default().max_results,
        smart: false,
    })
}

/// The disk-walk fallback for `plan` (narrowed with what the walker can filter itself;
/// hits are re-checked against the whole query).
pub fn live_query(plan: &IndexPlan, respect_gitignore: bool, content_max: u64) -> Result<SearchQuery, String> {
    let mut roots = plan.scope.clone();
    if roots.is_empty() {
        roots = self::roots();
    }
    if roots.is_empty() {
        return Err(tr!("services-index-no-roots").into());
    }
    let base = SearchQuery {
        roots,
        include_hidden: plan.include_hidden,
        respect_gitignore,
        content_max_size: content_max,
        max_results: plan.limit,
        ..Default::default()
    };
    soyuz_index::to_search_query(&plan.expr, base, SystemTime::now())
}

/// The search bar as query-language text (smart folders created from a search).
pub fn form_to_ql(form: &SearchForm) -> String {
    let mut parts: Vec<String> = Vec::new();
    let quote = |s: &str| if s.contains(char::is_whitespace) { format!("\"{s}\"") } else { s.to_string() };
    let text = form.text.trim();
    if form.content {
        if !text.is_empty() {
            parts.push(format!("content:{}", quote(text)));
        }
        let g = form.name_glob.trim();
        if !g.is_empty() {
            parts.push(format!("name:{}", quote(g)));
        }
    } else if !text.is_empty() {
        parts.push(text.to_string());
    }
    let (min, max) = (form.min_size.trim().replace(' ', ""), form.max_size.trim().replace(' ', ""));
    match (min.is_empty(), max.is_empty()) {
        (false, false) => parts.push(format!("size:{min}..{max}")),
        (false, true) => parts.push(format!("size:>={min}")),
        (true, false) => parts.push(format!("size:<={max}")),
        (true, true) => {}
    }
    let date = match form.date {
        DateRange::Any => None,
        DateRange::Today => Some("modified:<1d"),
        DateRange::Week => Some("modified:<7d"),
        DateRange::Month => Some("modified:<30d"),
        DateRange::Year => Some("modified:<1y"),
        DateRange::OlderThanYear => Some("modified:>1y"),
    };
    parts.extend(date.map(str::to_string));
    let kind = match form.kind {
        KindFilter::Any => None,
        KindFilter::Files => Some("file"),
        KindFilter::Folders => Some("folder"),
        KindFilter::Images => Some("image"),
        KindFilter::Videos => Some("video"),
        KindFilter::Audio => Some("audio"),
        KindFilter::Documents => Some("document"),
        KindFilter::Archives => Some("archive"),
        KindFilter::Code => Some("code"),
    };
    parts.extend(kind.map(|k| format!("kind:{k}")));
    if !form.everywhere && !form.root.as_os_str().is_empty() {
        parts.push(format!("in:\"{}\"", soyuz_fs::strip_verbatim(&form.root).display()));
    }
    parts.join(" ")
}

/// A smart folder's query → plan (its `in:` terms become the scope so the index range
/// scan and the "is it indexed" check both see them).
pub fn plan_from_query(query: &str, include_hidden: bool) -> Result<IndexPlan, String> {
    let expr = soyuz_index::parse(query).map_err(|e| e.msg)?;
    let mut scope = Vec::new();
    let rest = match expr {
        Expr::Term(Term::In(p)) => {
            scope.push(p);
            Expr::And(Vec::new())
        }
        Expr::And(items) => {
            let mut keep = Vec::new();
            for i in items {
                match i {
                    Expr::Term(Term::In(p)) if scope.is_empty() => scope.push(p),
                    other => keep.push(other),
                }
            }
            Expr::And(keep)
        }
        other => other,
    };
    Ok(IndexPlan { expr: rest, scope, include_hidden, limit: SearchQuery::default().max_results, smart: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(text: &str) -> SearchForm {
        SearchForm { text: text.into(), root: PathBuf::from("/data"), ..Default::default() }
    }

    #[test]
    fn form_plans() {
        let p = plan_from_form(&form("report kind:document")).unwrap();
        assert_eq!(p.scope, vec![PathBuf::from("/data")]);
        assert_eq!(p.expr.terms().len(), 2);
        let mut f = form("x");
        f.min_size = "1k".into();
        f.date = DateRange::Week;
        f.kind = KindFilter::Images;
        f.everywhere = true;
        let p = plan_from_form(&f).unwrap();
        assert!(p.scope.is_empty());
        assert_eq!(p.expr.terms().len(), 4);
        assert!(plan_from_form(&form("")).is_err());
        assert!(plan_from_form(&form("size:>huge")).unwrap_err().contains("size"));
        let mut f = form("x");
        f.min_size = "2M".into();
        f.max_size = "1M".into();
        assert!(plan_from_form(&f).is_err());
        let mut f = form("TODO");
        f.content = true;
        f.name_glob = "*.rs".into();
        assert!(plan_from_form(&f).unwrap().expr.uses_content());
        assert!(!uses_query_language(&f));
        assert!(uses_query_language(&form("x")));
    }

    #[test]
    fn form_round_trips_through_query_text() {
        let mut f = form("annual report");
        f.min_size = "1 MB".into();
        f.date = DateRange::Month;
        f.kind = KindFilter::Documents;
        let q = form_to_ql(&f);
        assert!(q.starts_with("annual report size:>=1MB modified:<30d kind:document in:"), "{q}");
        let a = plan_from_form(&f).unwrap();
        let b = plan_from_query(&q, false).unwrap();
        assert_eq!(b.scope, vec![PathBuf::from("/data")]);
        // Same terms (the `in:` became the scope).
        assert_eq!(a.expr.terms().len(), b.expr.terms().len());
        let mut c = form("TODO");
        c.content = true;
        c.name_glob = "*.rs".into();
        c.everywhere = true;
        assert_eq!(form_to_ql(&c), "content:TODO name:*.rs");
    }

    #[test]
    fn live_fallback_narrows_the_walk() {
        let p = plan_from_form(&form("report ext:pdf")).unwrap();
        let q = live_query(&p, true, 1 << 20).unwrap();
        assert_eq!(q.roots, vec![PathBuf::from("/data")]);
        assert_eq!(q.extensions, ["pdf"]);
        assert!(q.respect_gitignore);
        assert_eq!(q.name.unwrap().text, "report");
    }

    #[test]
    fn common_ancestors() {
        assert_eq!(common_ancestor(&[PathBuf::from("/a/b/c"), PathBuf::from("/a/b/d")]), PathBuf::from("/a/b"));
        assert_eq!(common_ancestor(&[PathBuf::from("/a")]), PathBuf::from("/a"));
        assert_eq!(common_ancestor(&[]), PathBuf::new());
        let plan = IndexPlan {
            expr: Expr::And(vec![]),
            scope: vec!["/x".into()],
            include_hidden: false,
            limit: 0,
            smart: false,
        };
        assert_eq!(plan.name_root(), PathBuf::from("/x"));
    }

    #[test]
    fn config_mapping() {
        let c = SearchIndex {
            roots: vec!["~".into(), " ".into()],
            content_extensions: vec![".MD".into(), "".into()],
            ..Default::default()
        };
        let ic = index_config(&c, Duration::from_millis(300));
        assert_eq!(ic.roots, vec![soyuz_fs::home()]);
        assert_eq!(ic.content_extensions, ["md"]);
        assert!(ic.db_path.ends_with("soyuz-index.db"));
        assert!(!can_serve(&IndexPlan {
            expr: Expr::And(vec![]),
            scope: vec![],
            include_hidden: false,
            limit: 0,
            smart: true
        }));
    }
}
