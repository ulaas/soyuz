//! Context menus (items / background / column header) and the reusable View, Sort and
//! Group-by menu sections shared with the toolbar. Every entry dispatches through
//! `Cmd::PaneAction`, so menus, keys and toolbar behave identically.

use egui::Ui;
use soyuz_core::config::{Action, Keymap};
use soyuz_core::{Column, Location, SortDir, ViewMode};

use crate::actions::GROUPS;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::state::pane::{ContextTarget, Pane, PaneId};
use crate::theme::icons::ph;
use crate::ui::views::ViewCtx;
use crate::ui::widgets::{menu_caption, menu_check, menu_item, menu_separator, submenu};

/// `[context_menu] hide`: action names left out of the context menus (set from the
/// config at startup and on every config change).
static HIDDEN: parking_lot::RwLock<Vec<Action>> = parking_lot::RwLock::new(Vec::new());

/// Parse `[context_menu] hide`; returns warnings for unknown names.
pub fn set_hidden(names: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut hidden = Vec::new();
    for n in names {
        match n.trim().parse::<Action>() {
            Ok(a) => hidden.push(a),
            Err(_) => warnings.push(format!("[context_menu] hide: unknown action '{n}'")),
        }
    }
    *HIDDEN.write() = hidden;
    warnings
}

fn is_hidden(a: &Action) -> bool {
    HIDDEN.read().contains(a)
}

/// A menu row bound to an action (shortcut hint from the keymap).
fn action_item(ui: &mut Ui, km: &Keymap, cmds: &mut Vec<Cmd>, pane: PaneId, icon: &str, label: &str, action: Action) {
    if is_hidden(&action) {
        return;
    }
    let sc = chord_label(km, &action);
    if menu_item(ui, Some(icon), label, sc.as_deref(), true).clicked() {
        cmds.push(Cmd::PaneAction(pane, action));
        ui.close();
    }
}

pub fn pane_menu(ui: &mut Ui, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    ui.set_width(260.0);
    match pane.context {
        ContextTarget::Items if !pane.selection.is_empty() => items_menu(ui, pane, v),
        _ => background_menu(ui, pane, v),
    }
}

fn items_menu(ui: &mut Ui, pane: &Pane, v: &mut ViewCtx<'_>) {
    let km = v.keymap;
    let id = pane.id;
    let sel = pane.selected_entries();
    let single = sel.len() == 1;
    let all_dirs = sel.iter().all(|e| e.is_dir_like());
    let first = sel.first().map(|e| (e.path.clone(), e.is_dir_like()));
    let has_clip = !v.fileops.clipboard.is_empty() || crate::services::os_clipboard::has_files();
    let dual = v.dual;
    let cmds = &mut *v.cmds;

    action_item(
        ui,
        km,
        cmds,
        id,
        if all_dirs { ph::FOLDER_OPEN } else { ph::ARROW_SQUARE_OUT },
        tr!("app-ctx-open"),
        Action::Open,
    );
    if all_dirs {
        action_item(ui, km, cmds, id, ph::BROWSERS, tr!("app-ctx-open-in-new-tab"), Action::OpenInNewTab);
        if single
            && let Some((path, _)) = &first
            && menu_item(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), tr!("app-ctx-open-in-other-pane"), None, true).clicked()
        {
            cmds.push(Cmd::OpenInOtherPane(Location::Dir(path.clone())));
            ui.close();
        }
    }
    if single
        && !is_hidden(&Action::OpenWith)
        && let Some((path, false)) = &first
    {
        let path = path.clone();
        // Warm the per-extension cache as soon as the menu opens, so the submenu is
        // usually filled by the time the pointer reaches it.
        let _ = apps_for_cached(ui.ctx(), &path);
        submenu(ui, Some(ph::APP_WINDOW), tr!("app-ctx-open-with"), |ui| {
            ui.set_width(220.0);
            let Some(apps) = apps_for_cached(ui.ctx(), &path) else {
                menu_item(ui, Some(ph::HOURGLASS), tr!("app-ctx-open-with-looking"), None, false);
                return;
            };
            if apps.is_empty() {
                menu_item(ui, Some(ph::INFO), tr!("app-ctx-open-with-none"), None, false);
            }
            for app in apps {
                if menu_item(ui, Some(ph::APP_WINDOW), &app.name, None, app.launchable).clicked() {
                    cmds.push(Cmd::OpenWith { path: path.clone(), app: app.clone() });
                    ui.close();
                }
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::DOTS_THREE), tr!("app-ctx-open-with-choose"), None, true).clicked() {
                cmds.push(Cmd::ChooseOpenWith(path.clone()));
                ui.close();
            }
        });
    }
    if single && let Some((_, false)) = &first {
        action_item(ui, km, cmds, id, ph::NOTE_PENCIL, tr!("app-ctx-edit-in-soyuz"), Action::OpenInEditor);
    }
    menu_separator(ui);
    action_item(ui, km, cmds, id, ph::SCISSORS, tr!("app-ctx-cut"), Action::Cut);
    action_item(ui, km, cmds, id, ph::COPY, tr!("app-ctx-copy"), Action::Copy);
    if single
        && has_clip
        && let Some((path, true)) = &first
        && menu_item(ui, Some(ph::CLIPBOARD), tr!("app-ctx-paste-into-folder"), None, true).clicked()
    {
        cmds.push(Cmd::FileOp(crate::ops::fileops::FileOpCmd::PasteInto(path.clone())));
        ui.close();
    }
    if single
        && v.config.sync.previous_versions
        && crate::ui::tools::versions::supported()
        && matches!(pane.location, Location::Dir(_))
    {
        action_item(
            ui,
            km,
            cmds,
            id,
            ph::CLOCK_COUNTER_CLOCKWISE,
            tr!("app-ctx-previous-versions"),
            Action::PreviousVersions,
        );
    }
    if dual {
        action_item(ui, km, cmds, id, ph::COPY_SIMPLE, tr!("app-ctx-copy-to-other-pane"), Action::CopyToOtherPane);
        action_item(
            ui,
            km,
            cmds,
            id,
            ph::ARROW_BEND_DOWN_RIGHT,
            tr!("app-ctx-move-to-other-pane"),
            Action::MoveToOtherPane,
        );
    }
    action_item(ui, km, cmds, id, ph::PENCIL_SIMPLE, tr!("app-ctx-rename"), Action::Rename);
    action_item(ui, km, cmds, id, ph::COPY_SIMPLE, tr!("app-ctx-duplicate"), Action::Duplicate);
    action_item(ui, km, cmds, id, ph::LINK, tr!("app-ctx-create-link"), Action::NewSymlink);
    action_item(ui, km, cmds, id, ph::TRASH, tr!("app-ctx-move-to-trash"), Action::Delete);
    action_item(ui, km, cmds, id, ph::X_CIRCLE, tr!("app-ctx-delete-permanently"), Action::DeletePermanent);
    menu_separator(ui);
    action_item(
        ui,
        km,
        cmds,
        id,
        ph::CLIPBOARD_TEXT,
        if single { tr!("app-ctx-copy-path") } else { tr!("app-ctx-copy-paths") },
        Action::CopyPath,
    );
    action_item(
        ui,
        km,
        cmds,
        id,
        ph::TEXT_AA,
        if single { tr!("app-ctx-copy-name") } else { tr!("app-ctx-copy-names") },
        Action::CopyName,
    );
    if single {
        action_item(
            ui,
            km,
            cmds,
            id,
            ph::ARROW_SQUARE_UP_RIGHT,
            tr!("app-ctx-reveal-in-system"),
            Action::RevealInSystem,
        );
    }
    if single && let Some((path, true)) = &first {
        action_item(ui, km, cmds, id, ph::TERMINAL_WINDOW, tr!("app-ctx-open-terminal-here"), Action::OpenInTerminal);
        if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("app-ctx-open-external-terminal"), None, true).clicked() {
            cmds.push(Cmd::TerminalAt(path.clone()));
            ui.close();
        }
        if menu_item(ui, Some(ph::STAR), tr!("app-ctx-add-to-favorites"), None, true).clicked() {
            cmds.push(Cmd::AddFavorite(path.clone()));
            ui.close();
        }
    }
    let any_archive = sel.iter().any(|e| !e.is_dir_like() && crate::services::archive::is_browsable(&e.path));
    crate::ui::archive::context_items(ui, cmds, id, &pane.location, any_archive, !sel.is_empty());
    let sel_paths: Vec<(std::path::PathBuf, bool)> = sel.iter().map(|e| (e.path.clone(), e.is_dir_like())).collect();
    crate::ui::tools::tags_ui::items_menu(ui, km, cmds, id, &pane.location, &sel_paths);
    crate::ui::ai::context_items(ui, km, cmds, id, true, single && all_dirs);
    if all_dirs && matches!(pane.location, Location::Dir(_)) {
        let dirs: Vec<std::path::PathBuf> = sel_paths.iter().map(|(p, _)| p.clone()).collect();
        crate::ui::looks::folder_color_menu(ui, &dirs);
    }
    if all_dirs {
        action_item(ui, km, cmds, id, ph::CHART_PIE, tr!("app-ctx-folder-sizes"), Action::FolderSize);
    }
    let kinds = crate::commands::kinds_for(&pane.location, !all_dirs, sel.iter().any(|e| e.is_dir_like()), false);
    crate::ui::commands::context_items(ui, v.config, cmds, id, &kinds);
    let paths: Vec<std::path::PathBuf> = sel_paths.iter().map(|(p, _)| p.clone()).collect();
    crate::ui::automation::menu::context_items(
        ui,
        cmds,
        id,
        &kinds,
        pane.location.as_fs_path().map(|p| p.to_path_buf()),
        &paths,
    );
    menu_separator(ui);
    let single_file = single && first.as_ref().is_some_and(|(_, dir)| !dir);
    crate::ui::dialogs::properties::context_items(ui, km, cmds, id, sel.len() > 1, single_file);
    action_item(ui, km, cmds, id, ph::INFO, tr!("app-ctx-properties"), Action::Properties);
}

fn background_menu(ui: &mut Ui, pane: &Pane, v: &mut ViewCtx<'_>) {
    let km = v.keymap;
    let id = pane.id;
    let has_clip = !v.fileops.clipboard.is_empty() || crate::services::os_clipboard::has_files();
    let cmds = &mut *v.cmds;
    submenu(ui, Some(ph::SQUARES_FOUR), tr!("app-ctx-view"), |ui| {
        ui.set_width(220.0);
        view_mode_items(ui, km, cmds, pane);
    });
    submenu(ui, Some(ph::SORT_ASCENDING), tr!("app-ctx-sort-by"), |ui| {
        ui.set_width(220.0);
        sort_items(ui, cmds, pane);
    });
    submenu(ui, Some(ph::ROWS), tr!("app-ctx-group-by"), |ui| {
        ui.set_width(200.0);
        group_items(ui, cmds, pane);
    });
    menu_separator(ui);
    action_item(ui, km, cmds, id, ph::ARROW_CLOCKWISE, tr!("app-ctx-refresh"), Action::Refresh);
    if has_clip {
        action_item(ui, km, cmds, id, ph::CLIPBOARD, tr!("app-ctx-paste"), Action::Paste);
        action_item(ui, km, cmds, id, ph::LINK, tr!("app-ctx-paste-as-link"), Action::PasteAsLink);
        action_item(ui, km, cmds, id, ph::SEAL_CHECK, tr!("app-ctx-paste-and-verify"), Action::PasteAndVerify);
    } else {
        menu_item(ui, Some(ph::CLIPBOARD), tr!("app-ctx-paste"), chord_label(km, &Action::Paste).as_deref(), false);
    }
    action_item(ui, km, cmds, id, ph::FOLDER_PLUS, tr!("app-ctx-new-folder"), Action::NewFolder);
    action_item(ui, km, cmds, id, ph::FILE_PLUS, tr!("app-ctx-new-file"), Action::NewFile);
    action_item(ui, km, cmds, id, ph::LINK, tr!("app-ctx-new-link"), Action::NewSymlink);
    menu_separator(ui);
    if let Location::Dir(dir) = &pane.location {
        action_item(ui, km, cmds, id, ph::TERMINAL_WINDOW, tr!("app-ctx-open-terminal-here"), Action::OpenTerminalHere);
        if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("app-ctx-open-external-terminal"), None, true).clicked() {
            cmds.push(Cmd::TerminalAt(dir.clone()));
            ui.close();
        }
        action_item(
            ui,
            km,
            cmds,
            id,
            ph::ARROW_SQUARE_UP_RIGHT,
            tr!("app-ctx-reveal-in-system"),
            Action::RevealInSystem,
        );
        action_item(ui, km, cmds, id, ph::CLIPBOARD_TEXT, tr!("app-ctx-copy-folder-path"), Action::CopyPath);
        if menu_item(ui, Some(ph::STAR), tr!("app-ctx-add-to-favorites"), None, true).clicked() {
            cmds.push(Cmd::AddFavorite(dir.clone()));
            ui.close();
        }
        crate::ui::ai::context_items(ui, km, cmds, id, false, true);
        menu_separator(ui);
    }
    if matches!(pane.location, Location::Archive { .. }) {
        crate::ui::archive::context_items(ui, cmds, id, &pane.location, false, false);
        menu_separator(ui);
    }
    crate::ui::tools::tags_ui::background_menu(ui, km, cmds, id, &pane.location, v.dual);
    if menu_check(ui, v.show_hidden, tr!("app-ctx-show-hidden"), chord_label(km, &Action::ToggleHidden).as_deref())
        .clicked()
    {
        cmds.push(Cmd::PaneAction(id, Action::ToggleHidden));
        ui.close();
    }
    if menu_check(ui, v.checkbox_mode, tr!("app-ctx-item-checkboxes"), None).clicked() {
        cmds.push(Cmd::PaneAction(id, Action::ToggleCheckboxes));
        ui.close();
    }
    action_item(ui, km, cmds, id, ph::SELECTION_ALL, tr!("app-ctx-select-all"), Action::SelectAll);
    action_item(ui, km, cmds, id, ph::CHART_PIE, tr!("app-ctx-folder-sizes"), Action::FolderSize);
    crate::ui::commands::context_items(ui, v.config, cmds, id, &[soyuz_core::config::CommandContext::Background]);
    crate::ui::automation::menu::context_items(
        ui,
        cmds,
        id,
        &[soyuz_core::config::CommandContext::Background],
        pane.location.as_fs_path().map(|p| p.to_path_buf()),
        &[],
    );
    menu_separator(ui);
    action_item(ui, km, cmds, id, ph::INFO, tr!("app-ctx-properties"), Action::Properties);
}

pub const MODES: [(ViewMode, &str, Action); 5] = [
    (ViewMode::Details, ph::LIST, Action::ViewDetails),
    (ViewMode::List, ph::LIST_DASHES, Action::ViewList),
    (ViewMode::Grid, ph::SQUARES_FOUR, Action::ViewGrid),
    (ViewMode::Gallery, ph::IMAGES, Action::ViewGallery),
    (ViewMode::Miller, ph::COLUMNS, Action::ViewMiller),
];

/// Translated name of a view mode (menus, status bar switcher).
pub fn mode_label(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Details => tr!("app-view-details"),
        ViewMode::List => tr!("app-view-list"),
        ViewMode::Grid => tr!("app-view-icons"),
        ViewMode::Gallery => tr!("app-view-gallery"),
        ViewMode::Miller => tr!("app-view-columns"),
    }
}

pub fn view_mode_items(ui: &mut Ui, km: &Keymap, cmds: &mut Vec<Cmd>, pane: &Pane) {
    for (mode, icon, action) in MODES {
        let checked = pane.view.mode == mode;
        let sc = chord_label(km, &action);
        let r = menu_item(ui, Some(if checked { ph::CHECK } else { icon }), mode_label(mode), sc.as_deref(), true);
        if r.clicked() {
            cmds.push(Cmd::PaneAction(pane.id, action));
            ui.close();
        }
    }
}

const SORT_COLUMNS: [Column; 7] =
    [Column::Name, Column::Modified, Column::Size, Column::Kind, Column::Extension, Column::Created, Column::Accessed];

pub fn sort_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, pane: &Pane) {
    for col in SORT_COLUMNS {
        if menu_check(ui, pane.view.sort.column == col, crate::i18n::labels::column(col), None).clicked() {
            if pane.view.sort.column != col {
                cmds.push(Cmd::PaneAction(pane.id, Action::SortBy(col)));
            }
            ui.close();
        }
    }
    menu_separator(ui);
    for (dir, label) in [(SortDir::Asc, tr!("app-sort-ascending")), (SortDir::Desc, tr!("app-sort-descending"))] {
        if menu_check(ui, pane.view.sort.dir == dir, label, None).clicked() {
            if pane.view.sort.dir != dir {
                cmds.push(Cmd::PaneAction(pane.id, Action::ToggleSortDir));
            }
            ui.close();
        }
    }
}

pub fn group_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, pane: &Pane) {
    for (g, _) in GROUPS {
        if menu_check(ui, pane.view.group == g, crate::palette::catalog::group_label(g), None).clicked() {
            cmds.push(Cmd::PaneAction(pane.id, Action::SetGroupBy(g)));
            ui.close();
        }
    }
}

/// Column chooser (details header right-click).
pub fn column_menu(ui: &mut Ui, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    ui.set_width(220.0);
    menu_caption(ui, tr!("app-ctx-columns"));
    let visible: Vec<Column> = pane.view.visible_columns().iter().map(|c| c.column).collect();
    for col in Column::ALL {
        if matches!(
            col,
            Column::Dimensions | Column::Duration | Column::Checksum | Column::ItemCount | Column::LinkTarget
        ) {
            continue; // no data source yet
        }
        if col.script_slot().is_some() && crate::automation::scripts::column_label(col).is_none() {
            continue; // no script provides this column
        }
        let on = visible.contains(&col);
        let enabled = col != Column::Name;
        let label = crate::ui::views::columns::header_label(col);
        let r = crate::ui::widgets::menu_item(ui, Some(if on { ph::CHECK } else { "" }), &label, None, enabled);
        if r.clicked() {
            pane.view.set_column_visible(col, !on);
            pane.layout_sig = None;
            v.cmds.push(Cmd::ViewChanged(pane.id));
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("app-ctx-reset-columns"), None, true).clicked() {
        pane.view.columns = crate::state::view_memory::ViewState::from_config(v.config).columns;
        pane.layout_sig = None;
        v.cmds.push(Cmd::ViewChanged(pane.id));
        ui.close();
    }
}

/// "Open with" candidates, cached per extension. The lookup (registry / desktop-file
/// scan, possibly slow) runs on a background thread; `None` while it is in flight, and
/// the context repaints when the result lands.
pub fn apps_for_cached(ctx: &egui::Context, path: &std::path::Path) -> Option<Vec<soyuz_fs::AppInfo>> {
    type Slot = std::sync::Arc<parking_lot::Mutex<Option<Vec<soyuz_fs::AppInfo>>>>;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let id = egui::Id::new(("soyuz.openwith", ext));
    if let Some(slot) = ctx.data(|d| d.get_temp::<Slot>(id)) {
        return slot.lock().clone();
    }
    let slot: Slot = Default::default();
    ctx.data_mut(|d| d.insert_temp(id, slot.clone()));
    let (path, ctx2) = (path.to_path_buf(), ctx.clone());
    let spawned = std::thread::Builder::new().name("soyuz-apps-for".into()).spawn(move || {
        let apps = soyuz_fs::launch::apps_for(&path);
        *slot.lock() = Some(apps);
        ctx2.request_repaint();
    });
    if spawned.is_err() {
        ctx.data_mut(|d| d.remove::<Slot>(id));
        return Some(Vec::new());
    }
    None
}
