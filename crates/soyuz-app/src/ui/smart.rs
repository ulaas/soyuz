//! Smart folders (roadmap F5): saved query-language searches shown as live virtual
//! folders in the sidebar. Opening one shows its results as `Location::Search` (sortable
//! and actionable like any folder); while the background index can answer the query the
//! results refresh by themselves as the index changes (`services::index::pump`).
//!
//! Stored in `smart_folders.toml` next to the config (`[[folder]] name / query /
//! include_hidden`), loaded on a worker at startup and saved through the ordered store
//! worker. Created from the current search (search bar), edited / renamed / deleted from
//! the sidebar context menu. Also hosts the index status chip of the status bar.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crossbeam_channel::Receiver;
use egui::{Align, Align2, CornerRadius, FontId, Key, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use serde::{Deserialize, Serialize};
use soyuz_core::Location;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::{self, NavOpts};
use crate::services::index::{self as ix};
use crate::services::search::{self as svc, SearchSpec};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dialogs::conflict::{Btn, button};
use crate::ui::dialogs::{Dialog, DialogCtx};
use crate::ui::toasts::Toast;
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator, spinner};

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

/// Per frame (from `ui::search::pump`): load the store once, drive the index service.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    STATE.with_borrow_mut(|s| {
        if !s.started {
            s.started = true;
            let (tx, rx) = crossbeam_channel::bounded(1);
            s.rx = Some(rx);
            let c = ctx.clone();
            let _ = std::thread::Builder::new().name("soyuz-smart-load".into()).spawn(move || {
                // A broken file is backed up and never overwritten (store_file).
                let store = crate::services::store_file::load(&store_path(), SmartStore::from_toml).value();
                let _ = tx.send(store);
                c.request_repaint();
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
    ix::pump(app, ctx);
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

pub fn apply(app: &mut SoyuzApp, cmd: SmartCmd) {
    match cmd {
        SmartCmd::Open { index, new_tab } => open(app, index, new_tab),
        SmartCmd::Edit(index) => {
            let f = index.and_then(|i| folders().get(i).cloned()).unwrap_or_default();
            app.ui.dialogs.push(SmartDialog::new(index, f));
        }
        SmartCmd::FromSearch => {
            let form = app.ui.search.form.clone();
            let name = match &app.ws.pane().location {
                Location::Search(id) => svc::spec(*id).map(|s| s.title).unwrap_or_else(|| form.title()),
                _ => form.title(),
            };
            let f = SmartFolder { name, query: ix::form_to_ql(&form), include_hidden: form.include_hidden };
            app.ui.dialogs.push(SmartDialog::new(None, f));
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
                app.ui.dialogs.push(ConfirmDelete { index, name: f.name.clone() });
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
            crate::ui::settings::open(app, Some(crate::ui::settings::Section::Preview));
        }
        SmartCmd::RebuildIndex => {
            if ix::rebuild() {
                app.ui.toasts.push(Toast::info(tr!("views-smart-rebuilding")).body(tr!("views-smart-rebuilding-body")));
            }
        }
    }
}

/// Register the search behind smart folder `i` and show it.
fn open(app: &mut SoyuzApp, i: usize, new_tab: bool) {
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
    app.ui.focus = crate::app::Focus::List;
}

/// The smart folder shown at `loc`, if any.
fn shown_name(loc: &Location) -> Option<String> {
    match loc {
        Location::Search(id) => svc::spec(*id).and_then(|s| s.smart),
        _ => None,
    }
}

/// Collapsed-set key of the sidebar section (persisted; the header is translated).
const SECTION: &str = "Smart folders";

/// Sidebar section (drawn when there are smart folders or indexing is on).
pub fn sidebar_section(ui: &mut Ui, collapsed: &mut BTreeSet<String>, current: &Location, cmds: &mut Vec<Cmd>) {
    let list = folders();
    if list.is_empty() && !ix::is_enabled() {
        return;
    }
    let mut open = !collapsed.contains(SECTION);
    collapsible_header(ui, tr!("views-smart-section"), &mut open);
    if open {
        collapsed.remove(SECTION);
    } else {
        collapsed.insert(SECTION.into());
        return;
    }
    let shown = shown_name(current);
    for (i, f) in list.iter().enumerate() {
        let active = shown.as_deref() == Some(f.name.as_str());
        let r = NavRow { icon: ph::FOLDER_SIMPLE_DOTTED, label: &f.name, active, icon_color: None, usage: None }
            .show(ui)
            .on_hover_text(&f.query);
        if r.clicked() {
            cmds.push(Cmd::Smart(SmartCmd::Open { index: i, new_tab: false }));
        }
        if r.middle_clicked() {
            cmds.push(Cmd::Smart(SmartCmd::Open { index: i, new_tab: true }));
        }
        r.context_menu(|ui| {
            ui.set_width(210.0);
            if menu_item(ui, Some(ph::FOLDER_OPEN), tr!("views-smart-open"), None, true).clicked() {
                cmds.push(Cmd::Smart(SmartCmd::Open { index: i, new_tab: false }));
                ui.close();
            }
            if menu_item(ui, Some(ph::PLUS), tr!("views-smart-open-new-tab"), None, true).clicked() {
                cmds.push(Cmd::Smart(SmartCmd::Open { index: i, new_tab: true }));
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("views-smart-edit"), None, true).clicked() {
                cmds.push(Cmd::Smart(SmartCmd::Edit(Some(i))));
                ui.close();
            }
            if menu_item(ui, Some(ph::TRASH), tr!("views-smart-delete"), None, true).clicked() {
                cmds.push(Cmd::Smart(SmartCmd::Delete(i)));
                ui.close();
            }
        });
    }
    let r = NavRow {
        icon: ph::PLUS,
        label: tr!("views-smart-new"),
        active: false,
        icon_color: Some(palette(ui.ctx()).text_tertiary),
        usage: None,
    }
    .show(ui)
    .on_hover_text(tr!("views-smart-new-tip"));
    if r.clicked() {
        cmds.push(Cmd::Smart(SmartCmd::Edit(None)));
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

/// Status bar chip: index activity; click opens the index settings.
pub fn status_chip(ui: &mut Ui, cmds: &mut Vec<Cmd>) {
    use soyuz_index::Phase;
    let Some(st) = ix::status() else { return };
    let p = palette(ui.ctx());
    let busy =
        matches!(st.phase, Phase::Starting | Phase::Crawling | Phase::Verifying | Phase::CatchingUp | Phase::Contents);
    let failed = matches!(st.phase, Phase::Failed(_));
    let label = match &st.phase {
        Phase::Crawling => match st.progress() {
            Some(pr) => tr!("views-index-chip-pct", pct = (pr * 100.0).round() as u64),
            None => tr!("views-index-chip-indexing").to_string(),
        },
        Phase::Verifying | Phase::CatchingUp => tr!("views-index-chip-updating").into(),
        Phase::Contents => tr!("views-index-chip-contents").into(),
        Phase::Starting => tr!("views-index-chip-starting").into(),
        Phase::Idle => tr!("views-index-chip-idle").into(),
        Phase::Failed(_) => tr!("views-index-chip-off").into(),
    };
    let font = FontId::proportional(11.5);
    let tw = ui.painter().layout_no_wrap(label.clone(), font.clone(), p.text_secondary).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2(tw + 26.0, 18.0), Sense::click());
    crate::ui::a11y::button(&resp, &label);
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface2);
    }
    let icon_c = r.left_center() + vec2(10.0, 0.0);
    let color = if failed { p.warning } else { p.text_secondary };
    if busy {
        spinner(ui, icon_c, 4.5, p.accent);
    } else {
        ui.painter().text(icon_c, Align2::CENTER_CENTER, ph::DATABASE, icons::font(12.0), color);
    }
    ui.painter().text(r.left_center() + vec2(20.0, 0.0), Align2::LEFT_CENTER, &label, font, color);
    let mut tip = status_line(&st);
    if !st.ready_roots.is_empty() {
        tip.push_str(&format!(
            "\n{}",
            st.ready_roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join("\n")
        ));
    }
    tip.push('\n');
    tip.push_str(tr!("views-index-chip-tip"));
    let resp = resp.on_hover_text(tip);
    if resp.clicked() {
        cmds.push(Cmd::Smart(SmartCmd::OpenSettings));
    }
    resp.context_menu(|ui| {
        ui.set_width(200.0);
        if menu_item(ui, Some(ph::GEAR_SIX), tr!("views-index-settings"), None, true).clicked() {
            cmds.push(Cmd::Smart(SmartCmd::OpenSettings));
            ui.close();
        }
        if menu_item(ui, Some(ph::ARROWS_CLOCKWISE), tr!("views-index-rebuild"), None, !failed).clicked() {
            cmds.push(Cmd::Smart(SmartCmd::RebuildIndex));
            ui.close();
        }
        if menu_item(ui, Some(ph::FOLDER_SIMPLE_DOTTED), tr!("views-smart-new"), None, true).clicked() {
            cmds.push(Cmd::Smart(SmartCmd::Edit(None)));
            ui.close();
        }
    });
}

// ---- dialogs ------------------------------------------------------------------------

/// Create / edit a smart folder.
pub struct SmartDialog {
    index: Option<usize>,
    folder: SmartFolder,
    focus: bool,
}

impl SmartDialog {
    pub fn new(index: Option<usize>, folder: SmartFolder) -> Self {
        SmartDialog { index, folder, focus: true }
    }
}

/// Query cheat sheet: `(example, what it does)`; the examples are query syntax.
fn help() -> [(&'static str, &'static str); 8] {
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

impl Dialog for SmartDialog {
    fn title(&self) -> &str {
        if self.index.is_some() { tr!("views-smart-dialog-edit") } else { tr!("views-smart-dialog-new") }
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::FOLDER_SIMPLE_DOTTED)
    }

    fn width(&self) -> f32 {
        520.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let label = |ui: &mut Ui, s: &str| {
            ui.label(RichText::new(s).font(FontId::proportional(11.5)).color(p.text_secondary));
        };
        label(ui, tr!("views-smart-name"));
        let n = ui.add(
            TextEdit::singleline(&mut self.folder.name)
                .hint_text(tr!("views-smart-name-hint"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if self.focus {
            n.request_focus();
            self.focus = false;
        }
        ui.add_space(8.0);
        label(ui, tr!("views-smart-query"));
        ui.add(
            TextEdit::singleline(&mut self.folder.query)
                .hint_text("kind:image modified:<30d -screenshot")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        let parsed = soyuz_index::parse(&self.folder.query);
        ui.add_space(4.0);
        match (&parsed, self.folder.query.trim().is_empty()) {
            (_, true) => {}
            (Err(e), false) => {
                ui.label(RichText::new(&e.msg).font(FontId::proportional(11.5)).color(p.danger));
            }
            (Ok(_), false) => {
                let plan = ix::plan_from_query(&self.folder.query, self.folder.include_hidden).ok();
                let live = plan.as_ref().is_some_and(ix::can_serve);
                let text = if live {
                    tr!("views-smart-live")
                } else if ix::is_enabled() {
                    tr!("views-smart-not-covered")
                } else {
                    tr!("views-smart-index-off")
                };
                ui.label(RichText::new(text).font(FontId::proportional(11.5)).color(p.text_tertiary));
            }
        }
        ui.add_space(6.0);
        ui.checkbox(&mut self.folder.include_hidden, tr!("views-smart-include-hidden"));
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(p.bg_surface2)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                for (example, what) in help() {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(vec2(250.0, 16.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_width(250.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(example).font(FontId::monospace(11.0)).color(p.text_primary),
                                )
                                .truncate(),
                            );
                        });
                        ui.label(RichText::new(what).font(FontId::proportional(11.0)).color(p.text_tertiary));
                    });
                }
            });
        let err = match &parsed {
            _ if self.folder.name.trim().is_empty() => Some(tr!("views-smart-need-name").to_string()),
            _ if self.folder.query.trim().is_empty() => Some(tr!("views-smart-type-query").to_string()),
            Err(e) => Some(e.msg.clone()),
            Ok(_) => None,
        };
        let taken = folders()
            .iter()
            .enumerate()
            .any(|(i, f)| Some(i) != self.index && f.name.eq_ignore_ascii_case(self.folder.name.trim()));
        let err = err.or_else(|| taken.then(|| tr!("views-smart-name-taken").to_string()));
        ui.add_space(14.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, tr!("common-save"), Btn::Primary).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
            if let Some(e) = &err
                && !self.folder.name.is_empty()
            {
                ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.text_tertiary));
            }
        });
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            ok = true;
        }
        if ok && err.is_none() {
            ctx.cmds.push(Cmd::Smart(SmartCmd::Save { index: self.index, folder: self.folder.clone() }));
            return false;
        }
        !cancel
    }
}

struct ConfirmDelete {
    index: usize,
    name: String,
}

impl Dialog for ConfirmDelete {
    fn title(&self) -> &str {
        tr!("views-smart-delete-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::TRASH)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(tr!("views-smart-delete-confirm", name = &self.name)).color(p.text_primary));
        ui.label(RichText::new(tr!("views-smart-delete-body")).font(FontId::proportional(11.5)).color(p.text_tertiary));
        ui.add_space(14.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, tr!("common-delete"), Btn::Danger).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
        });
        if ok {
            ctx.cmds.push(Cmd::Smart(SmartCmd::ConfirmDelete(self.index)));
            return false;
        }
        !cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
