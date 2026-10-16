//! Context menus (items / background / column header) and the reusable View, Sort and
//! Group-by menu sections shared with the toolbar. The entries come from the core's
//! catalogue (`models::context_menu`, shared with the terminal UI); this module draws
//! them. Every action entry dispatches through `Cmd::PaneAction`, so menus, keys and
//! toolbar behave identically.

use egui::Ui;
use soyuz_core::ViewMode;
use soyuz_core::config::Keymap;

use crate::cmd::Cmd;
use crate::models::context_menu::{self as catalogue, Entry, ItemStyle, MenuCtx, MenuIcon, SubmenuKind};
use crate::state::pane::Pane;
use crate::theme::icons::ph;
use crate::ui::views::ViewCtx;
use crate::ui::widgets::{menu_caption, menu_check, menu_item, menu_separator, submenu};

/// Check `[context_menu] hide` (startup and every config change); returns warnings for
/// unknown names. The catalogue reads the list from the config when it builds a menu.
pub fn set_hidden(names: &[String]) -> Vec<String> {
    catalogue::hide_warnings(names)
}

/// The Phosphor glyph of a catalogue icon (`None`: no icon).
fn glyph(icon: MenuIcon) -> Option<&'static str> {
    Some(match icon {
        MenuIcon::None | MenuIcon::Tag { .. } | MenuIcon::Dot(_) => return None,
        MenuIcon::Blank => "",
        MenuIcon::Id(id) => crate::ui::icons::glyph(id),
        MenuIcon::ChartPieSlice => ph::CHART_PIE_SLICE,
        MenuIcon::Check => ph::CHECK,
        MenuIcon::CheckCircle => ph::CHECK_CIRCLE,
        MenuIcon::DotsThree => ph::DOTS_THREE,
        MenuIcon::Files => ph::FILES,
        MenuIcon::Hash => ph::HASH,
        MenuIcon::ArrowsMerge => ph::ARROWS_MERGE,
        MenuIcon::ArrowsSplit => ph::ARROWS_SPLIT,
        MenuIcon::Package => ph::PACKAGE,
        MenuIcon::Prohibit => ph::PROHIBIT,
        MenuIcon::SlidersHorizontal => ph::SLIDERS_HORIZONTAL,
        MenuIcon::TrayArrowDown => ph::TRAY_ARROW_DOWN,
        MenuIcon::Plus => ph::PLUS,
        MenuIcon::PushPinSlash => ph::PUSH_PIN_SLASH,
        MenuIcon::X => ph::X,
        MenuIcon::ArrowLineRight => ph::ARROW_LINE_RIGHT,
        MenuIcon::Plugs => ph::PLUGS,
        MenuIcon::Eject => ph::EJECT_SIMPLE,
        MenuIcon::LockOpen => ph::LOCK_SIMPLE_OPEN,
    })
}

/// Draw catalogue `entries`; an activated entry runs and closes the menu (unless it
/// keeps it open).
pub fn draw_entries(ui: &mut Ui, entries: Vec<Entry>, cmds: &mut Vec<Cmd>) {
    for e in entries {
        match e {
            Entry::Separator => menu_separator(ui),
            Entry::Caption(c) => menu_caption(ui, &c),
            Entry::Item(it) => {
                let chord = it.chord.as_deref();
                let resp = match (it.style, it.icon) {
                    (ItemStyle::Check(on), _) => menu_check(ui, on, &it.label, chord),
                    (ItemStyle::Plain, MenuIcon::Tag { tag, checked }) => {
                        crate::ui::tools::tags_ui::tag_row(ui, tag, checked, chord)
                    }
                    (ItemStyle::Plain, MenuIcon::Dot(color)) => crate::ui::workspaces::dot_item(ui, color, &it.label),
                    (ItemStyle::Plain, icon) => menu_item(ui, glyph(icon), &it.label, chord, it.enabled),
                };
                let resp = match it.tip {
                    Some(tip) => resp.on_hover_text(tip),
                    None => resp,
                };
                if resp.clicked() {
                    if let Some(r) = it.run {
                        catalogue::run(r, cmds);
                    }
                    if !it.keep_open {
                        ui.close();
                    }
                }
            }
            Entry::Submenu(s) => {
                submenu(ui, glyph(s.icon), &s.label, |ui| {
                    match s.kind {
                        SubmenuKind::View | SubmenuKind::SortBy | SubmenuKind::Extract => ui.set_width(220.0),
                        SubmenuKind::GroupBy | SubmenuKind::Tags => ui.set_width(200.0),
                        SubmenuKind::Ai => ui.set_width(230.0),
                        SubmenuKind::Checksum => ui.set_min_width(200.0),
                        SubmenuKind::TabGroups | SubmenuKind::Windows => {}
                    }
                    draw_entries(ui, s.items, cmds);
                });
            }
            Entry::OpenWith(path) => open_with_menu(ui, cmds, path),
            Entry::FolderColor(dirs) => crate::ui::looks::folder_color_menu(ui, &dirs),
        }
    }
}

/// "Open with" for a file: the apps found for its type (looked up in the background).
fn open_with_menu(ui: &mut Ui, cmds: &mut Vec<Cmd>, path: std::path::PathBuf) {
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

pub fn pane_menu(ui: &mut Ui, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    ui.set_width(260.0);
    let cx = MenuCtx {
        config: v.config,
        keymap: v.keymap,
        has_clip: catalogue::has_clip(v.fileops),
        dual: v.dual,
        show_hidden: v.show_hidden,
        checkbox_mode: v.checkbox_mode,
    };
    let entries = catalogue::pane_menu(&cx, pane);
    draw_entries(ui, entries, v.cmds);
}

/// Translated name of a view mode (menus, status bar switcher).
pub fn mode_label(mode: ViewMode) -> &'static str {
    catalogue::mode_label(mode)
}

pub fn view_mode_items(ui: &mut Ui, km: &Keymap, cmds: &mut Vec<Cmd>, pane: &Pane) {
    draw_entries(ui, catalogue::view_mode_entries(km, pane), cmds);
}

pub fn sort_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, pane: &Pane) {
    draw_entries(ui, catalogue::sort_entries(pane), cmds);
}

pub fn group_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, pane: &Pane) {
    draw_entries(ui, catalogue::group_entries(pane), cmds);
}

/// Column chooser (details header right-click).
pub fn column_menu(ui: &mut Ui, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    ui.set_width(220.0);
    draw_entries(ui, catalogue::column_menu(pane), v.cmds);
}

/// "Open with" candidates, cached per extension. The lookup (registry / desktop-file
/// scan, possibly slow) runs on a background thread; `None` while it is in flight, and
/// the context repaints when the result lands.
pub fn apps_for_cached(ctx: &egui::Context, path: &std::path::Path) -> Option<Vec<soyuz_fs::AppInfo>> {
    type Slot = std::sync::Arc<parking_lot::Mutex<Option<Vec<soyuz_fs::AppInfo>>>>;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let id = egui::Id::unique(("soyuz.openwith", ext));
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
