//! Search bar (PLAN M6, app side): Ctrl+Shift+F / F3 opens a bar under the toolbar with
//! the query field and option pills (name mode, case, contents, size, date, kind,
//! hidden, .gitignore). Results stream into the focused pane as `Location::Search`
//! (sortable, actionable, Path column), with a status strip showing progress, the result
//! count and a cancel button. Searches can be saved (sidebar "Searches" section, stored
//! in `searches.toml` next to the config; `[[search.saved]]` entries from the config are
//! listed too). Ctrl+F keeps filtering the current folder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Margin, Panel, Popup, Rect, RectAlign, Response, Sense,
    Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use soyuz_core::config::Config;
use soyuz_core::{Location, SearchId};
use soyuz_search::SavedSearch;

use crate::app::{Focus, SoyuzApp};
use crate::cmd::Cmd;
use crate::ops::{self, NavOpts};
use crate::services::search::{self as svc, DateRange, KindFilter, NameMode, SearchForm, SearchSpec};
use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::toasts::Toast;
use crate::ui::widgets::{
    NavRow, collapsible_header, menu_check, menu_item, menu_separator, pill_button, spinner, trunc_galley,
};

const ROW_H: f32 = 44.0;
const STATUS_H: f32 = 28.0;

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

pub struct SearchUi {
    pub open: bool,
    pub form: SearchForm,
    focus: bool,
    /// From `[[search.saved]]`.
    config_saved: Vec<SavedSearch>,
    /// User-saved (searches.toml).
    pub saved: Vec<SavedEntry>,
    saving: Option<String>,
    error: Option<String>,
    path: PathBuf,
}

impl SearchUi {
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
        SearchUi { open: false, form, focus: false, config_saved, saved, saving: None, error: None, path }
    }

    /// Sidebar names: config searches first, then the user's.
    pub fn names(&self) -> Vec<String> {
        self.config_saved.iter().map(|s| s.name.clone()).chain(self.saved.iter().map(|s| s.name.clone())).collect()
    }

    fn persist(&self) -> Result<(), String> {
        let text = toml::to_string_pretty(&SavedFile { saved: self.saved.clone() }).map_err(|e| e.to_string())?;
        crate::services::store_file::save_sync(&self.path, text.as_bytes())
    }
}

fn current_dir(app: &SoyuzApp) -> Option<PathBuf> {
    match &app.ws.pane().location {
        Location::Dir(p) => Some(p.clone()),
        _ => None,
    }
}

/// Ctrl+Shift+F / F3 (and SearchContent): open the bar scoped to the current folder.
pub fn open(app: &mut SoyuzApp, content: bool) {
    let s = &mut app.ui.search;
    s.open = true;
    s.focus = true;
    s.error = None;
    s.form.content = content;
    if let Some(d) = current_dir(app) {
        app.ui.search.form.root = d;
    }
}

fn close(app: &mut SoyuzApp, ctx: &egui::Context) {
    app.ui.search.open = false;
    app.ui.focus = Focus::List;
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
}

/// Build the query from the form and show the results in the focused pane.
pub fn run(app: &mut SoyuzApp) {
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

/// Query-language search (F5): the index answers it when it covers the scope, else the
/// disk walk does (narrowed, every hit checked against the whole query).
fn run_indexed(app: &mut SoyuzApp, form: &SearchForm, content_max: u64) {
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

fn register_and_show(app: &mut SoyuzApp, spec: SearchSpec) {
    let id = svc::register(spec);
    let pane = app.ws.pane().id;
    ops::navigate(app, pane, Location::Search(id), NavOpts::user());
    app.ui.focus = Focus::List;
}

fn run_query(app: &mut SoyuzApp, query: soyuz_search::SearchQuery, title: String) {
    let id = svc::register(SearchSpec::new(query, title));
    let pane = app.ws.pane().id;
    ops::navigate(app, pane, Location::Search(id), NavOpts::user());
    app.ui.focus = Focus::List;
}

/// Title prefix marking a flat-view listing (so the toggle can leave it again).
const FLAT_PREFIX: &str = "All files in ";

/// `ToggleFlatView`: list every file below the current folder in one flat, sortable
/// listing (a recursive soyuz-search walk without a name filter, streamed like search
/// results). Toggling again inside the flat listing returns to the folder.
pub fn toggle_flat_view(app: &mut SoyuzApp) {
    let pane = app.ws.pane();
    if let Location::Search(id) = &pane.location {
        if svc::spec(*id).is_some_and(|s| s.title.starts_with(FLAT_PREFIX)) {
            let id = pane.id;
            ops::history_jump(app, id, true, 1);
        }
        return;
    }
    let Some(root) = current_dir(app) else {
        app.ui.toasts.push(Toast::info("Flat view works in folders"));
        return;
    };
    let query = soyuz_search::SearchQuery {
        roots: vec![root.clone()],
        include_hidden: app.ui.show_hidden,
        respect_gitignore: false,
        kinds: Some(soyuz_core::EntryKindSet::FILE | soyuz_core::EntryKindSet::SYMLINK),
        ..Default::default()
    };
    let title = format!("{FLAT_PREFIX}{}", soyuz_core::entry::display_name(&root));
    run_query(app, query, title);
}

/// Run sidebar entry `i` (config searches first, then user ones).
pub fn run_saved(app: &mut SoyuzApp, i: usize) {
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

pub fn remove_saved(app: &mut SoyuzApp, i: usize) {
    let n_cfg = app.ui.search.config_saved.len();
    if i >= n_cfg && i - n_cfg < app.ui.search.saved.len() {
        let removed = app.ui.search.saved.remove(i - n_cfg);
        if let Err(e) = app.ui.search.persist() {
            app.ui.toasts.push(Toast::error("Couldn't save searches").body(e));
        } else {
            app.ui.toasts.push(Toast::info(format!("Removed saved search “{}”", removed.name)));
        }
    }
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
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    svc::init(ctx, app.config.search.threads);
    crate::ui::smart::pump(app, ctx);
    for n in svc::pump(&mut app.ws) {
        app.ui.toasts.push(Toast::error("Search failed").body(n));
    }
    crate::ui::archive::pump(app, ctx);
    if svc::sizing() {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

/// "Calculate folder sizes": selected folders, or every folder in the pane.
pub fn folder_sizes(app: &mut SoyuzApp) {
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
        app.ui.toasts.push(Toast::info("No folders to measure"));
    }
}

// ---- drawing ------------------------------------------------------------------------

fn pill(ui: &mut Ui, r: Rect, id: Id, icon: Option<&str>, label: &str, active: bool, tip: &str) -> Response {
    let p = palette(ui.ctx());
    let resp = ui.interact(r, id, Sense::click());
    let fill = if active {
        p.accent_subtle
    } else if resp.hovered() {
        p.bg_surface2
    } else {
        p.bg_surface1
    };
    ui.painter().rect_filled(r, CornerRadius::same(13), fill);
    let stroke = if active { p.accent.gamma_multiply(0.55) } else { p.border_subtle };
    ui.painter().rect_stroke(r, CornerRadius::same(13), Stroke::new(1.0, stroke), StrokeKind::Inside);
    let c = if active { p.accent } else { p.text_secondary };
    let mut x = r.left() + 10.0;
    if let Some(g) = icon {
        ui.painter().text(pos2(x + 6.0, r.center().y), Align2::CENTER_CENTER, g, icons::font(13.0), c);
        x += 17.0;
    }
    ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), c);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

fn pill_width(ui: &Ui, icon: bool, label: &str) -> f32 {
    let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), Color32::WHITE).size().x;
    w + 20.0 + if icon { 17.0 } else { 0.0 }
}

/// The status of the search shown in the focused pane, if any.
fn shown_search(app: &SoyuzApp) -> Option<SearchId> {
    match app.ws.pane().location {
        Location::Search(id) => Some(id),
        _ => None,
    }
}

pub fn bar(root: &mut Ui, app: &mut SoyuzApp) {
    let shown = shown_search(app);
    if !app.ui.search.open && shown.is_none() {
        return;
    }
    let ctx = root.ctx().clone();
    let p = palette(&ctx);
    let height = if app.ui.search.open { ROW_H } else { 0.0 } + if shown.is_some() { STATUS_H } else { 0.0 };
    Panel::top("soyuz.search")
        .exact_size(height)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 6, right: 8, top: 0, bottom: 2 }))
        .show(root, |ui| {
            let full = ui.available_rect_before_wrap();
            let mut y = full.top();
            if app.ui.search.open {
                let row = Rect::from_min_size(pos2(full.left(), y), vec2(full.width(), ROW_H));
                controls(ui, row, app);
                y += ROW_H;
            }
            if let Some(id) = shown {
                let row = Rect::from_min_size(pos2(full.left(), y), vec2(full.width(), STATUS_H));
                status(ui, row, app, id);
            }
            ui.allocate_rect(full, Sense::hover());
        });
}

fn controls(ui: &mut Ui, row: Rect, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let card = row.shrink2(vec2(0.0, 4.0));
    ui.painter().rect_filled(card, CornerRadius::same(radius::MODAL), p.bg_surface1);
    ui.painter().rect_stroke(
        card,
        CornerRadius::same(radius::MODAL),
        Stroke::new(1.0, p.border_subtle),
        StrokeKind::Inside,
    );
    let cy = card.center().y;
    let running = shown_search(app).is_some_and(svc::is_running);

    // Right side: close, save, run/stop.
    let mut rx = card.right() - 6.0;
    let close_r = Rect::from_center_size(pos2(rx - 13.0, cy), vec2(24.0, 24.0));
    rx = close_r.left() - 4.0;
    let cresp =
        ui.interact(close_r, Id::new("soyuz.search.close"), Sense::click()).on_hover_text("Close search  (Esc)");
    if cresp.hovered() {
        ui.painter().rect_filled(close_r, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(close_r.center(), Align2::CENTER_CENTER, ph::X, icons::font(13.0), p.text_tertiary);
    if cresp.clicked() {
        close(app, &ctx);
        return;
    }
    let star_r = Rect::from_center_size(pos2(rx - 13.0, cy), vec2(24.0, 24.0));
    rx = star_r.left() - 6.0;
    let sresp = ui.interact(star_r, Id::new("soyuz.search.save"), Sense::click()).on_hover_text("Save this search");
    if sresp.hovered() {
        ui.painter().rect_filled(star_r, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(star_r.center(), Align2::CENTER_CENTER, ph::BOOKMARK_SIMPLE, icons::font(14.0), p.text_secondary);
    if sresp.clicked() {
        app.ui.search.saving = Some(app.ui.search.form.title());
    }
    save_popup(ui, &sresp, app);
    let label = if running { "Stop" } else { "Search" };
    let bw = pill_width(ui, true, label) + 6.0;
    let br = Rect::from_center_size(pos2(rx - bw / 2.0, cy), vec2(bw, 26.0));
    rx = br.left() - 10.0;
    let mut bchild = ui.new_child(UiBuilder::new().max_rect(br));
    let run_clicked =
        pill_button(&mut bchild, Some(if running { ph::STOP } else { ph::MAGNIFYING_GLASS }), label, !running)
            .clicked();

    // Left: scope chip.
    let scope = soyuz_fs::strip_verbatim(&app.ui.search.form.root);
    let everywhere = app.ui.search.form.everywhere;
    let scope_name =
        if scope.as_os_str().is_empty() { "…".to_string() } else { soyuz_core::entry::display_name(&scope) };
    let scope_label = if everywhere { "Everywhere".to_string() } else { format!("in {scope_name}") };
    let sw = pill_width(ui, true, &scope_label).min(180.0);
    let scope_r = Rect::from_min_size(pos2(card.left() + 6.0, cy - 13.0), vec2(sw, 26.0));
    let tip =
        if everywhere { "Search every indexed folder".to_string() } else { format!("Search in {}", scope.display()) };
    let scope_icon = if everywhere { ph::DATABASE } else { ph::FOLDER_SIMPLE };
    let sresp =
        pill(ui, scope_r, Id::new("soyuz.search.scope"), Some(scope_icon), &trunc(&scope_label, 22), everywhere, &tip);
    let here = current_dir(app);
    let index_on = crate::services::index::is_enabled();
    let mut rerun = false;
    Popup::menu(&sresp).align(RectAlign::BOTTOM_START).show(|ui| {
        ui.set_width(250.0);
        let form = &mut app.ui.search.form;
        let label = match &here {
            Some(d) => format!("This folder ({})", soyuz_core::entry::display_name(d)),
            None => "This folder".to_string(),
        };
        if menu_check(ui, !form.everywhere, &label, None).clicked() {
            form.everywhere = false;
            if let Some(d) = &here {
                form.root = d.clone();
            }
            rerun = true;
            ui.close();
        }
        let r = menu_check(ui, form.everywhere, "All indexed folders", None).on_hover_text(if index_on {
            "Instant results from the background index"
        } else {
            "Turn on the background index in Settings → Preview & search for instant results"
        });
        if r.clicked() {
            form.everywhere = true;
            rerun = true;
            ui.close();
        }
    });

    // Pills from the right edge leftwards.
    let f = app.ui.search.form.clone();
    let mut pills: Vec<(&'static str, Option<&'static str>, String, bool, &'static str)> = vec![
        ("gitignore", None, ".gitignore".into(), f.respect_gitignore, "Respect .gitignore files"),
        ("hidden", Some(ph::EYE), "Hidden".into(), f.include_hidden, "Include hidden files"),
        (
            "kind",
            None,
            if f.kind == KindFilter::Any { "Kind".into() } else { f.kind.label().into() },
            f.kind != KindFilter::Any,
            "Filter by kind",
        ),
        (
            "date",
            Some(ph::CALENDAR),
            if f.date == DateRange::Any { "Date".into() } else { f.date.label().into() },
            f.date != DateRange::Any,
            "Modified date",
        ),
        (
            "size",
            Some(ph::RULER),
            size_label(&f),
            !f.min_size.trim().is_empty() || !f.max_size.trim().is_empty(),
            "File size",
        ),
        ("content", Some(ph::FILE_TEXT), "Contents".into(), f.content, "Search inside files"),
        ("case", None, "Aa".into(), f.case_sensitive, "Match case"),
    ];
    if !f.content {
        pills.push(("mode", None, f.mode.label().into(), f.mode != NameMode::Substring, "How the name is matched"));
    } else {
        pills.push(("regex", None, ".*".into(), f.content_regex, "Regular expression"));
    }
    let mut x = rx;
    let min_text = 180.0;
    let left_limit = scope_r.right() + 8.0 + min_text;
    let mut clicked: Vec<(&'static str, Response)> = Vec::new();
    for (key, icon, label, active, tip) in pills {
        let w = pill_width(ui, icon.is_some(), &label);
        if x - w < left_limit {
            continue; // too narrow: drop the least important pills first
        }
        let r = Rect::from_min_size(pos2(x - w, cy - 13.0), vec2(w, 26.0));
        x -= w + 5.0;
        let resp = pill(ui, r, Id::new(("soyuz.search.pill", key)), icon, &label, active, tip);
        clicked.push((key, resp));
    }
    let mut changed = false;
    for (key, resp) in &clicked {
        let form = &mut app.ui.search.form;
        match *key {
            "gitignore" if resp.clicked() => {
                form.respect_gitignore = !form.respect_gitignore;
                changed = true;
            }
            "hidden" if resp.clicked() => {
                form.include_hidden = !form.include_hidden;
                changed = true;
            }
            "content" if resp.clicked() => {
                form.content = !form.content;
                app.ui.search.focus = true;
            }
            "case" if resp.clicked() => {
                form.case_sensitive = !form.case_sensitive;
                changed = true;
            }
            "regex" if resp.clicked() => {
                form.content_regex = !form.content_regex;
                changed = true;
            }
            "mode" => {
                Popup::menu(resp).align(RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_width(200.0);
                    for m in NameMode::ALL {
                        let hint = match m {
                            NameMode::Substring => "Name contains the text",
                            NameMode::Glob => "*.rs, IMG_????.jpg",
                            NameMode::Regex => "^report-\\d+\\.pdf$",
                            NameMode::Fuzzy => "Loose match, ranked by relevance",
                        };
                        if menu_check(ui, form.mode == m, m.label(), None).on_hover_text(hint).clicked() {
                            form.mode = m;
                            changed = true;
                            ui.close();
                        }
                    }
                });
            }
            "kind" => {
                Popup::menu(resp).align(RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_width(180.0);
                    for k in KindFilter::ALL {
                        if menu_check(ui, form.kind == k, k.label(), None).clicked() {
                            form.kind = k;
                            changed = true;
                            ui.close();
                        }
                    }
                });
            }
            "date" => {
                Popup::menu(resp).align(RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_width(190.0);
                    for d in DateRange::ALL {
                        if menu_check(ui, form.date == d, d.label(), None).clicked() {
                            form.date = d;
                            changed = true;
                            ui.close();
                        }
                    }
                });
            }
            "size" => {
                Popup::menu(resp)
                    .align(RectAlign::BOTTOM_START)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_width(230.0);
                        ui.label(egui::RichText::new("FILE SIZE").size(10.5).color(p.text_tertiary));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Min").color(p.text_secondary));
                            ui.add(
                                TextEdit::singleline(&mut form.min_size).hint_text("e.g. 10 MB").desired_width(70.0),
                            );
                            ui.label(egui::RichText::new("Max").color(p.text_secondary));
                            ui.add(TextEdit::singleline(&mut form.max_size).hint_text("any").desired_width(70.0));
                        });
                        ui.add_space(4.0);
                        menu_separator(ui);
                        for (label, min, max) in [
                            ("Tiny (< 16 KB)", "", "16k"),
                            ("Large (> 100 MB)", "100M", ""),
                            ("Huge (> 1 GB)", "1G", ""),
                            ("Any size", "", ""),
                        ] {
                            if menu_item(ui, None, label, None, true).clicked() {
                                form.min_size = min.into();
                                form.max_size = max.into();
                                changed = true;
                                ui.close();
                            }
                        }
                    });
            }
            _ => {}
        }
    }

    // Query field fills the middle.
    let field = Rect::from_min_max(
        pos2(scope_r.right() + 8.0, cy - 13.0),
        pos2(x.max(scope_r.right() + 8.0 + 60.0), cy + 13.0),
    );
    let fid = Id::new("soyuz.search.text");
    let focused = ui.memory(|m| m.has_focus(fid));
    ui.painter().rect_filled(field, CornerRadius::same(radius::ROW), p.bg_surface2);
    if focused {
        ui.painter().rect_stroke(
            field,
            CornerRadius::same(radius::ROW),
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        pos2(field.left() + 14.0, cy),
        Align2::CENTER_CENTER,
        if app.ui.search.form.content { ph::FILE_TEXT } else { ph::MAGNIFYING_GLASS },
        icons::font(13.5),
        if focused { p.accent } else { p.text_tertiary },
    );
    let content = app.ui.search.form.content;
    let glob_w = if content { (field.width() * 0.32).clamp(70.0, 140.0) } else { 0.0 };
    let inner = Rect::from_min_max(pos2(field.left() + 28.0, cy - 10.0), pos2(field.right() - 8.0 - glob_w, cy + 10.0));
    let esc = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let indexed = crate::services::index::uses_query_language(&app.ui.search.form)
        && (app.ui.search.form.everywhere || crate::services::index::covers(&app.ui.search.form.root));
    let hint = if content {
        "Search file contents…"
    } else if indexed {
        "Search the index…  kind:image  size:>5MB  modified:<7d  tag:red  -draft"
    } else {
        "Search names in this folder and below…  (kind:image size:>5MB …)"
    };
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    let resp = child.add(
        TextEdit::singleline(&mut app.ui.search.form.text)
            .id(fid)
            .hint_text(hint)
            .frame(Frame::NONE)
            .font(FontId::proportional(13.0))
            .vertical_align(egui::Align::Center)
            .desired_width(inner.width()),
    );
    if app.ui.search.focus {
        resp.request_focus();
        app.ui.search.focus = false;
    }
    let mut submit = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if content {
        let gr = Rect::from_min_max(pos2(inner.right() + 6.0, cy - 10.0), pos2(field.right() - 6.0, cy + 10.0));
        ui.painter().line_segment(
            [pos2(gr.left() - 3.0, gr.top()), pos2(gr.left() - 3.0, gr.bottom())],
            Stroke::new(1.0, p.border_strong),
        );
        let mut gchild = ui.new_child(UiBuilder::new().max_rect(gr.shrink2(vec2(4.0, 0.0))));
        let g = gchild.add(
            TextEdit::singleline(&mut app.ui.search.form.name_glob)
                .hint_text("in files: *.rs")
                .frame(Frame::NONE)
                .font(FontId::proportional(12.0))
                .vertical_align(egui::Align::Center)
                .desired_width(gr.width() - 8.0),
        );
        submit |= g.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    }
    if let Some(err) = app.ui.search.error.clone() {
        let eg = trunc_galley(ui.painter(), &err, FontId::proportional(11.0), p.danger, inner.width() * 0.6);
        let pos = pos2(inner.right() - eg.size().x, cy - eg.size().y / 2.0);
        if app.ui.search.form.text.is_empty() || !focused {
            ui.painter().galley(pos, eg, p.danger);
        }
    }
    if esc {
        close(app, &ctx);
        return;
    }
    if run_clicked && running {
        if let Some(id) = shown_search(app) {
            svc::cancel(id);
        }
    } else if run_clicked || submit {
        run(app);
    } else if (changed || rerun) && shown_search(app).is_some() && !app.ui.search.form.is_empty() {
        // Option changes re-run the visible search.
        run(app);
    }
}

fn size_label(f: &SearchForm) -> String {
    match (f.min_size.trim(), f.max_size.trim()) {
        ("", "") => "Size".into(),
        (a, "") => format!("> {a}"),
        ("", b) => format!("< {b}"),
        (a, b) => format!("{a} – {b}"),
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n { s.chars().take(n - 1).collect::<String>() + "…" } else { s.to_string() }
}

fn save_popup(ui: &mut Ui, anchor: &Response, app: &mut SoyuzApp) {
    if app.ui.search.saving.is_none() {
        return;
    }
    let p = palette(ui.ctx());
    let id = Id::new("soyuz.search.save.popup");
    Popup::open_id(ui.ctx(), id);
    let mut done: Option<bool> = None;
    Popup::new(id, ui.ctx().clone(), anchor.rect, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .open_memory(None)
        .align(RectAlign::BOTTOM_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(260.0);
            ui.label(egui::RichText::new("SAVE SEARCH").size(10.5).color(p.text_tertiary));
            ui.add_space(4.0);
            if let Some(name) = app.ui.search.saving.as_mut() {
                let r = ui.add(TextEdit::singleline(name).desired_width(f32::INFINITY).margin(Margin::symmetric(8, 5)));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    done = Some(true);
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    done = Some(false);
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if pill_button(ui, None, "Save", true).clicked() {
                    done = Some(true);
                }
                if pill_button(ui, None, "Cancel", false).clicked() {
                    done = Some(false);
                }
            });
        });
    if !Popup::is_id_open(ui.ctx(), id) && done.is_none() {
        done = Some(false);
    }
    if let Some(save) = done {
        let name = app.ui.search.saving.take().unwrap_or_default();
        Popup::close_id(ui.ctx(), id);
        if save && !name.trim().is_empty() {
            let form = app.ui.search.form.clone();
            let s = &mut app.ui.search;
            s.saved.retain(|e| e.name != name.trim());
            s.saved.push(SavedEntry { name: name.trim().to_string(), form });
            match s.persist() {
                Ok(()) => app.ui.toasts.push(Toast::success(format!("Saved “{}”", name.trim()))),
                Err(e) => app.ui.toasts.push(Toast::error("Couldn't save the search").body(e)),
            };
        }
    }
}

fn status(ui: &mut Ui, row: Rect, app: &mut SoyuzApp, id: SearchId) {
    let p = palette(ui.ctx());
    let Some(prog) = svc::progress(id) else { return };
    let title = svc::spec(id).map(|s| s.title).unwrap_or_default();
    let cy = row.center().y;
    let mut x = row.left() + 14.0;
    if prog.done {
        let (g, c) = if prog.cancelled { (ph::STOP_CIRCLE, p.warning) } else { (ph::CHECK_CIRCLE, p.success) };
        ui.painter().text(pos2(x, cy), Align2::CENTER_CENTER, g, icons::font_fill(14.0), c);
    } else {
        spinner(ui, pos2(x, cy), 6.0, p.accent);
    }
    x += 14.0;
    let count = crate::ui::preview::render::group_digits(prog.hits);
    let head = format!("{title} — {count} result{}", if prog.hits == 1 { "" } else { "s" });
    let hg =
        trunc_galley(ui.painter(), &head, crate::ui::views::flow::semibold(12.0), p.text_primary, row.width() * 0.45);
    let hw = hg.size().x;
    ui.painter().galley(pos2(x, cy - hg.size().y / 2.0), hg, p.text_primary);
    x += hw + 12.0;
    let secs = prog.elapsed.as_secs_f32();
    let mut detail = if prog.indexed {
        format!("from the index in {} ms", prog.elapsed.as_millis().max(1))
    } else {
        format!("scanned {} in {:.1} s", crate::ui::preview::render::group_digits(prog.scanned), secs)
    };
    if prog.truncated && !prog.cancelled {
        detail.push_str(" · result limit reached");
    }
    if prog.cancelled {
        detail.push_str(" · cancelled");
    }
    if prog.errors > 0 {
        detail.push_str(&format!(" · {} unreadable", prog.errors));
    }
    if !prog.done && !prog.current.as_os_str().is_empty() {
        detail.push_str(&format!(" · {}", soyuz_fs::strip_verbatim(&prog.current).display()));
    }
    // Right side controls.
    let mut rx = row.right() - 8.0;
    if !prog.done {
        let w = pill_width(ui, true, "Cancel");
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(ui, r, Id::new("soyuz.search.cancel"), Some(ph::X), "Cancel", false, "Stop searching").clicked() {
            svc::cancel(id);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    let fuzzy = svc::spec(id).and_then(|s| s.query.name).is_some_and(|n| n.kind == soyuz_core::PatternKind::Fuzzy);
    if fuzzy {
        let w = pill_width(ui, true, "Relevance");
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.rank"),
            Some(ph::SPARKLE),
            "Relevance",
            prog.ranked,
            "Order by match quality",
        )
        .clicked()
        {
            svc::set_ranked(id, !prog.ranked);
        }
    }
    let spec = svc::spec(id);
    let smart = spec.as_ref().and_then(|s| s.smart.clone());
    if let Some(name) = &smart {
        let w = pill_width(ui, true, "Edit");
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.smart.edit"),
            Some(ph::PENCIL_SIMPLE),
            "Edit",
            false,
            "Edit this smart folder",
        )
        .clicked()
            && let Some(i) = crate::ui::smart::folders().iter().position(|f| &f.name == name)
        {
            app.cmds.push(Cmd::Smart(crate::ui::smart::SmartCmd::Edit(Some(i))));
        }
    } else if spec.as_ref().is_some_and(|s| s.plan.is_some()) {
        let w = pill_width(ui, true, "Smart folder");
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.smart.new"),
            Some(ph::FOLDER_SIMPLE_DOTTED),
            "Smart folder",
            false,
            "Save as a smart folder in the sidebar (stays up to date)",
        )
        .clicked()
        {
            app.cmds.push(Cmd::Smart(crate::ui::smart::SmartCmd::FromSearch));
        }
    }
    if !app.ui.search.open && smart.is_none() {
        let w = pill_width(ui, true, "Refine");
        let r = Rect::from_min_size(pos2(rx - w, cy - 11.0), vec2(w, 22.0));
        rx = r.left() - 6.0;
        if pill(
            ui,
            r,
            Id::new("soyuz.search.refine"),
            Some(ph::SLIDERS_HORIZONTAL),
            "Refine",
            false,
            "Edit this search",
        )
        .clicked()
        {
            app.ui.search.open = true;
            app.ui.search.focus = true;
        }
    }
    let dg = trunc_galley(ui.painter(), &detail, FontId::proportional(11.5), p.text_tertiary, (rx - x - 8.0).max(0.0));
    ui.painter().galley(pos2(x, cy - dg.size().y / 2.0), dg, p.text_tertiary);
}

/// Sidebar section with saved searches (called from the sidebar).
pub fn sidebar_section(
    ui: &mut Ui,
    names: &[String],
    collapsed: &mut BTreeSet<String>,
    cmds: &mut Vec<Cmd>,
    n_config: usize,
) {
    if names.is_empty() {
        return;
    }
    let mut open = !collapsed.contains("Searches");
    collapsible_header(ui, "Searches", &mut open);
    if open {
        collapsed.remove("Searches");
    } else {
        collapsed.insert("Searches".into());
        return;
    }
    for (i, name) in names.iter().enumerate() {
        let r =
            NavRow { icon: ph::MAGNIFYING_GLASS, label: name, active: false, icon_color: None, usage: None }.show(ui);
        if r.clicked() {
            cmds.push(Cmd::RunSavedSearch(i));
        }
        r.context_menu(|ui| {
            ui.set_width(200.0);
            if menu_item(ui, Some(ph::MAGNIFYING_GLASS), "Run search", None, true).clicked() {
                cmds.push(Cmd::RunSavedSearch(i));
                ui.close();
            }
            if menu_item(ui, Some(ph::X), "Remove", None, i >= n_config)
                .on_hover_text(if i < n_config { "Defined in config.toml" } else { "" })
                .clicked()
            {
                cmds.push(Cmd::RemoveSavedSearch(i));
                ui.close();
            }
        });
    }
}

impl SearchUi {
    pub fn config_count(&self) -> usize {
        self.config_saved.len()
    }
}
