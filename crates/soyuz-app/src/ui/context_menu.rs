//! Context menus (items / background / column header) and the reusable View, Sort and
//! Group-by menu sections shared with the toolbar. The entries come from the core's
//! catalogue (`models::context_menu`, shared with the terminal UI); this module draws
//! them. Every action entry dispatches through `Cmd::PaneAction`, so menus, keys and
//! toolbar behave identically.

use egui::{Align2, CornerRadius, Response, Sense, Stroke, Ui, vec2};
use soyuz_core::ViewMode;
use soyuz_core::config::Keymap;

use crate::cmd::Cmd;
use crate::models::context_menu::{
    self as catalogue, Entry, Item, ItemStyle, MenuCtx, MenuIcon, StripKind, SubmenuKind,
};
use crate::state::pane::{ContextTarget, Pane, PaneId};
use crate::theme::icons::ph;
use crate::theme::tokens::radius;
use crate::theme::{icons, palette};
use crate::ui::views::ViewCtx;
use crate::ui::widgets::{menu_caption, menu_check, menu_item, menu_separator, submenu};

/// Check `[context_menu] hide` and `extended` (startup and every config change); returns
/// warnings for unknown names. The catalogue reads the lists from the config when it
/// builds a menu.
pub fn set_hidden(cm: &soyuz_core::config::ContextMenu) -> Vec<String> {
    catalogue::config_warnings(cm)
}

/// Call where a pane's context menu opens (`resp` is what it opens on): a
/// Shift+right-click opens the full menu (`[context_menu] extended` rows inline). The
/// modifiers are read here, in the window that got the click: once the menu has its own
/// native window, its input no longer sees them.
pub fn note_open(ui: &Ui, resp: &Response, pane: &mut Pane) {
    if resp.secondary_clicked() {
        pane.context_full = ui.input(|i| i.modifiers.shift);
    }
}

fn keyboard_key() -> egui::Id {
    egui::Id::unique("soyuz.ctx_menu.keyboard")
}

fn focus_first_key() -> egui::Id {
    egui::Id::unique("soyuz.ctx_menu.focus_first")
}

/// `ContextMenu` (Shift+F10, the palette): open `pane`'s context menu at its cursor on the
/// next frame it is drawn.
pub fn request_keyboard_open(ctx: &egui::Context, pane: PaneId) {
    ctx.data_mut(|d| d.insert_temp(keyboard_key(), pane.0));
}

/// Draw `pane`'s context menu on `bg`: opened by a right click there (at the pointer), or
/// by the keyboard ([`request_keyboard_open`]) at `anchor(pane)`, a point of the window
/// near the cursor item. A keyboard open targets the selection, else the cursor item
/// (selected first, as in the terminal UI), else the folder; its first row takes the
/// keyboard focus so the arrows, Enter and Esc work at once.
pub fn show_pane_menu(
    ui: &mut Ui,
    bg: &Response,
    pane: &mut Pane,
    v: &mut ViewCtx<'_>,
    anchor: impl FnOnce(&Pane) -> egui::Pos2,
) {
    note_open(ui, bg, pane);
    let ctx = ui.ctx().clone();
    let keyboard = ctx.data(|d| d.get_temp::<u64>(keyboard_key())) == Some(pane.id.0);
    let open_at = keyboard.then(|| {
        ctx.data_mut(|d| d.remove::<u64>(keyboard_key()));
        if pane.selection.is_empty()
            && let Some(key) = pane.cursor.clone()
        {
            pane.selection.select_only(key);
        }
        pane.context = if pane.selection.is_empty() { ContextTarget::Background } else { ContextTarget::Items };
        pane.context_full = false;
        ctx.data_mut(|d| d.insert_temp(focus_first_key(), true));
        anchor(pane)
    });
    if crate::ui::native::popup::context_menu_at(bg, open_at, |ui| pane_menu(ui, pane, v)).is_some() {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| d.insert_temp(open_key(), pass));
    }
}

fn open_key() -> egui::Id {
    egui::Id::unique("soyuz.ctx_menu.open")
}

/// A pane's context menu was shown this pass or the last: it owns the keyboard (arrows,
/// Enter and Esc go to the menu, not the file list).
pub fn owns_keyboard(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<u64>(open_key())).is_some_and(|pass| ctx.cumulative_pass_nr().saturating_sub(pass) <= 1)
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
                // A keyboard-opened menu: the first row takes the focus (not during egui's
                // invisible sizing pass).
                if it.enabled
                    && !ui.is_sizing_pass()
                    && ui.ctx().data_mut(|d| d.remove_temp::<bool>(focus_first_key())).is_some()
                {
                    resp.request_focus();
                }
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
                        SubmenuKind::CopyAs | SubmenuKind::Terminal | SubmenuKind::PasteSpecial => ui.set_width(220.0),
                        SubmenuKind::SendTo | SubmenuKind::Tools | SubmenuKind::Media | SubmenuKind::More => {
                            ui.set_width(240.0)
                        }
                        SubmenuKind::Commands => ui.set_min_width(220.0),
                        SubmenuKind::TabGroups | SubmenuKind::Windows => {}
                    }
                    draw_entries(ui, s.items, cmds);
                });
            }
            Entry::OpenWith(path) => open_with_menu(ui, cmds, path),
            Entry::FolderColor(dirs) => crate::ui::looks::folder_color_menu(ui, &dirs),
            Entry::Strip(kind, items) => strip(ui, kind, items, cmds),
        }
    }
}

/// A row of icon buttons (the quick bar) or tag dots, side by side; an activated one
/// runs and closes the menu. Arrow keys / Tab reach each button (egui focus).
fn strip(ui: &mut Ui, kind: StripKind, items: Vec<Item>, cmds: &mut Vec<Cmd>) {
    let p = palette(ui.ctx());
    let size = match kind {
        StripKind::QuickBar => vec2(32.0, 30.0),
        StripKind::Tags => vec2(22.0, 26.0),
    };
    if kind == StripKind::Tags {
        menu_separator(ui);
    }
    ui.horizontal(|ui| {
        ui.add_space(if kind == StripKind::Tags { 6.0 } else { 4.0 });
        ui.spacing_mut().item_spacing.x = 2.0;
        for it in items {
            // Disabled buttons are announced as disabled.
            let resp = ui.add_enabled_ui(it.enabled, |ui| ui.allocate_exact_size(size, Sense::click()).1).inner;
            let r = resp.rect;
            let hot = it.enabled && (resp.hovered() || resp.has_focus());
            match it.icon {
                MenuIcon::Tag { tag, checked } => {
                    let c = p.tags[tag as usize % p.tags.len()];
                    ui.painter().circle_filled(r.center(), 7.5, c);
                    if checked {
                        ui.painter().circle_stroke(r.center(), 10.0, Stroke::new(2.0, c));
                    } else if hot {
                        ui.painter().circle_stroke(r.center(), 10.0, Stroke::new(1.0, p.border_strong));
                    }
                    crate::ui::a11y::toggle(&resp, &it.label, checked);
                }
                icon => {
                    if hot {
                        ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.accent_subtle);
                    }
                    let color = if !it.enabled {
                        p.text_tertiary
                    } else if hot {
                        p.accent
                    } else {
                        p.text_secondary
                    };
                    let size = if kind == StripKind::Tags { 14.0 } else { 16.0 };
                    let g = glyph(icon).unwrap_or(ph::DOTS_THREE);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, g, icons::font(size), color);
                    crate::ui::a11y::button(&resp, &it.label);
                }
            }
            if resp.has_focus() {
                crate::ui::widgets::focus_ring(ui, r);
            }
            let tip = match &it.chord {
                Some(c) => format!("{}  {c}", it.label),
                None => it.label.clone(),
            };
            if resp.on_hover_text(tip).clicked() {
                if let Some(r) = it.run {
                    catalogue::run(r, cmds);
                }
                ui.close();
            }
        }
    });
    if kind == StripKind::QuickBar {
        menu_separator(ui);
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
        can_elevate: cfg!(any(windows, target_os = "linux")),
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
