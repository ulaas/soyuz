//! Left sidebar: a mini icon bar on top switches between the places sections and the
//! full-height disk tree (`[sidebar] mode`). The places view has Favorites (user-ordered,
//! drag to reorder, plus config bookmarks), Devices (volumes with free-space bars), Portal
//! (folders pinned by dropping them, see `ui::portal`), Tags, Other (Recent, Timeline,
//! Trash), Network, Cloud folders and Smart folders, in collapsible sections ordered by
//! `[sidebar] sections`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::{Align2, CursorIcon, FontId, Frame, Margin, Panel, Rect, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::config::{Bookmark, PortalPin, SidebarMode, SidebarSection};
use soyuz_core::{Location, SizeStyle, format_size};
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::state::session::FavoriteEntry;
use crate::theme::tokens::size;
use crate::theme::{icons::ph, palette};
use crate::ui::views::computer::{volume_glyph, volume_label};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator, trunc_galley};

/// A standard place under the home directory.
#[derive(Clone, Debug)]
pub struct Place {
    pub label: String,
    pub icon: &'static str,
    pub path: PathBuf,
}

const STANDARD: [(&str, &str); 6] = [
    ("Desktop", ph::DESKTOP),
    ("Documents", ph::FILE_TEXT),
    ("Downloads", ph::DOWNLOAD_SIMPLE),
    ("Pictures", ph::IMAGE),
    ("Music", ph::MUSIC_NOTES),
    ("Videos", ph::FILM_STRIP),
];

static PROBED: std::sync::OnceLock<Vec<Place>> = std::sync::OnceLock::new();
static PROBE_STARTED: std::sync::Once = std::sync::Once::new();

/// Home plus the standard folders: `~/<English name>`, or (Linux / BSD) the folder
/// `user-dirs.dirs` names for that role (`~/Belgeler` on a Turkish desktop). A role
/// set to the home folder itself (the XDG way of disabling it) is left out.
fn candidate_places(xdg: Option<&soyuz_fs::xdg_dirs::UserDirs>) -> Vec<Place> {
    let home = soyuz_fs::home();
    let mut out = vec![Place { label: "Home".into(), icon: ph::HOUSE, path: home.clone() }];
    for &(name, icon) in &STANDARD {
        let path = match xdg.and_then(|d| xdg_role(d, name)) {
            Some(p) if p == home => continue,
            Some(p) => p,
            None => home.join(name),
        };
        out.push(Place { label: name.into(), icon, path });
    }
    out
}

/// The XDG user folder for a standard place (English role name).
fn xdg_role(d: &soyuz_fs::xdg_dirs::UserDirs, name: &str) -> Option<PathBuf> {
    match name {
        "Desktop" => d.desktop.clone(),
        "Documents" => d.documents.clone(),
        "Downloads" => d.downloads.clone(),
        "Pictures" => d.pictures.clone(),
        "Music" => d.music.clone(),
        "Videos" => d.videos.clone(),
        _ => None,
    }
}

/// Translated label of a standard place (`label` is the English folder name, which is
/// also what the probe stores).
fn place_label(label: &str) -> String {
    match label {
        "Home" => tr!("app-place-home"),
        "Desktop" => tr!("app-place-desktop"),
        "Documents" => tr!("app-place-documents"),
        "Downloads" => tr!("app-place-downloads"),
        "Pictures" => tr!("app-place-pictures"),
        "Music" => tr!("app-place-music"),
        "Videos" => tr!("app-place-videos"),
        other => other,
    }
    .to_string()
}

fn localized(mut places: Vec<Place>) -> Vec<Place> {
    for p in &mut places {
        p.label = place_label(&p.label);
    }
    places
}

/// Home plus the standard user folders that exist. The existence checks run once on a
/// worker thread (never on the UI thread); until they finish every candidate is listed.
/// Labels are in the current UI language.
pub fn standard_places() -> Vec<Place> {
    if let Some(p) = PROBED.get() {
        return localized(p.clone());
    }
    PROBE_STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("soyuz-places".into()).spawn(|| {
            // `user-dirs.dirs` (Linux / BSD; empty elsewhere): read here, never on the UI thread.
            let xdg = soyuz_fs::xdg_dirs::user_dirs();
            let places: Vec<Place> = candidate_places(Some(&xdg))
                .into_iter()
                .enumerate()
                .filter(|(i, p)| *i == 0 || p.path.is_dir())
                .map(|(_, p)| p)
                .collect();
            let _ = PROBED.set(places);
        });
    });
    localized(candidate_places(None))
}

/// The standard places probe has finished.
pub fn places_probed() -> bool {
    PROBED.get().is_some()
}

/// Glyph for a favorite path (standard places get their own icons).
pub fn favorite_glyph(path: &Path) -> &'static str {
    let home = soyuz_fs::home();
    if path == home {
        return ph::HOUSE;
    }
    // XDG folders with localized names (`~/Belgeler`).
    if let Some(p) = PROBED.get().and_then(|v| v.iter().find(|p| p.path == path)) {
        return p.icon;
    }
    if path.parent() == Some(home.as_path())
        && let Some(n) = path.file_name().and_then(|n| n.to_str())
        && let Some((_, g)) = STANDARD.iter().find(|(s, _)| s.eq_ignore_ascii_case(n))
    {
        return g;
    }
    ph::FOLDER_SIMPLE
}

fn bookmark_glyph(name: &str) -> &'static str {
    match name.trim().to_ascii_lowercase().as_str() {
        "code" | "source" | "dev" => ph::CODE,
        "star" => ph::STAR,
        "home" => ph::HOUSE,
        "download" | "downloads" => ph::DOWNLOAD_SIMPLE,
        "music" => ph::MUSIC_NOTES,
        "image" | "pictures" | "photos" => ph::IMAGE,
        "video" | "film" => ph::FILM_STRIP,
        "cloud" => ph::CLOUD,
        "terminal" => ph::TERMINAL_WINDOW,
        "archive" => ph::PACKAGE,
        "database" => ph::DATABASE,
        _ => ph::FOLDER_SIMPLE_STAR,
    }
}

pub struct SidebarState {
    pub favorites: Vec<FavoriteEntry>,
    /// Favorites were edited by the user (persist them in the session).
    pub favorites_customized: bool,
    /// Collapsed section names.
    pub collapsed: BTreeSet<String>,
    /// Favorite being dragged (index).
    drag: Option<usize>,
    /// Default favorites were refreshed from the background places probe.
    places_applied: bool,
    /// `i18n::generation()` the default favorites were labelled in.
    lang_gen: u64,
    /// The Portal section (pinned folders).
    pub portal: crate::ui::portal::PortalState,
    /// Unmounted devices of the Devices section (Linux).
    devices: DevicesCache,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self::new()
    }
}

impl SidebarState {
    pub fn new() -> Self {
        SidebarState {
            favorites: crate::app::default_favorites(),
            favorites_customized: false,
            collapsed: BTreeSet::new(),
            drag: None,
            places_applied: places_probed(),
            lang_gen: crate::i18n::generation(),
            portal: Default::default(),
            devices: Default::default(),
        }
    }
}

/// Inputs for one sidebar frame.
pub struct SidebarCtx<'a> {
    pub bookmarks: &'a [Bookmark],
    pub volumes: &'a [VolumeInfo],
    pub cloud: &'a [(String, PathBuf)],
    pub sections: &'a [SidebarSection],
    pub current: &'a Location,
    pub pane: PaneId,
    pub cmds: &'a mut Vec<Cmd>,
    pub width: f32,
    /// Saved search names (config ones first) and how many come from the config.
    pub saved_searches: &'a [String],
    pub saved_config: usize,
    /// Draw the logo row at the top (off when the custom title bar shows it).
    pub brand: bool,
    /// The settings window is open (highlights the footer row).
    pub settings_open: bool,
    /// Shortcut of `Action::Settings`, shown in the footer row.
    pub settings_key: Option<String>,
    /// Folder tree state and options (`[sidebar] mode` decides where it shows).
    pub tree: crate::ui::tree::TreeArgs<'a>,
    /// Folders pinned to the Portal section (`[[sidebar.portal]]`).
    pub portal: &'a [PortalPin],
    /// Shortcut of `Action::ToggleTreeSidebar`, shown in the icon bar's tooltips.
    pub tree_key: Option<String>,
    /// `[sidebar]` (unmounted devices, hidden devices, GTK bookmarks).
    pub config: &'a soyuz_core::config::Sidebar,
}

/// Section heading shown to the user.
fn section_name(s: SidebarSection) -> &'static str {
    match s {
        SidebarSection::Favorites => tr!("app-sidebar-favorites"),
        SidebarSection::Devices => tr!("app-sidebar-devices"),
        SidebarSection::Portal => tr!("app-sidebar-portal"),
        SidebarSection::Cloud => tr!("app-sidebar-cloud"),
        SidebarSection::Tags => tr!("app-sidebar-tags"),
        SidebarSection::Network => tr!("app-sidebar-network"),
        SidebarSection::Trash => tr!("app-sidebar-other"),
        SidebarSection::Tree => tr!("app-sidebar-folders"),
        SidebarSection::Smart => tr!("app-sidebar-smart"),
    }
}

/// Stable section key (stored in the session's collapsed set; never translated).
fn section_key(s: SidebarSection) -> &'static str {
    match s {
        SidebarSection::Favorites => "Favorites",
        SidebarSection::Devices => "Devices",
        SidebarSection::Portal => "Portal",
        SidebarSection::Cloud => "Cloud",
        SidebarSection::Tags => "Tags",
        SidebarSection::Network => "Network",
        SidebarSection::Trash => "Other",
        SidebarSection::Tree => "Folders",
        SidebarSection::Smart => "Smart folders",
    }
}

pub fn show(root: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    let p = palette(root.ctx());
    desk_pump(root.ctx(), c);
    if !s.places_applied {
        if places_probed() {
            s.places_applied = true;
            if !s.favorites_customized {
                s.favorites = crate::app::default_favorites();
            }
        } else {
            root.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    // Language switched: relabel the default favorites (customized ones keep their names).
    let gen_now = crate::i18n::generation();
    if s.lang_gen != gen_now {
        s.lang_gen = gen_now;
        if !s.favorites_customized {
            s.favorites = crate::app::default_favorites();
        }
    }
    Panel::left("soyuz.sidebar")
        .resizable(true)
        .default_size(c.width.clamp(size::SIDEBAR_MIN, size::SIDEBAR_MAX))
        .size_range(size::SIDEBAR_MIN..=size::SIDEBAR_MAX)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 8, right: 4, top: 0, bottom: 6 }))
        .show(root, |ui| {
            if c.brand {
                brand(ui);
            } else {
                ui.add_space(2.0);
            }
            // Set again while the Portal section is drawn (OS drops over it pin).
            s.portal.rect = None;
            view_bar(ui, c);
            Panel::bottom("soyuz.sidebar.footer")
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 0 }))
                .show(ui, |ui| footer(ui, c));
            if c.tree.mode == SidebarMode::Tree {
                // Tree only: the filter stays on top, the rows scroll.
                crate::ui::tree::filter_box(ui, c.tree.state);
                ScrollArea::vertical().id_salt("soyuz.sidebar.tree").auto_shrink([false, false]).show(ui, |ui| {
                    tree_rows(ui, c);
                    ui.add_space(8.0);
                });
                return;
            }
            let tree_listed = c.sections.contains(&SidebarSection::Tree);
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for &sec in c.sections {
                    match sec {
                        SidebarSection::Tree if c.tree.mode == SidebarMode::Both => tree_section(ui, s, c),
                        SidebarSection::Favorites => favorites(ui, s, c),
                        SidebarSection::Devices => devices(ui, s, c),
                        SidebarSection::Portal => portal(ui, s, c),
                        SidebarSection::Cloud if !c.cloud.is_empty() => cloud(ui, s, c),
                        SidebarSection::Trash => other(ui, s, c),
                        SidebarSection::Network => {
                            crate::ui::remote::sidebar_section(ui, &mut s.collapsed, c.current, c.pane, c.cmds)
                        }
                        SidebarSection::Tags => {
                            crate::ui::tools::tags_ui::sidebar_section(ui, &mut s.collapsed, c.current, c.pane, c.cmds)
                        }
                        SidebarSection::Smart => {
                            crate::ui::smart::sidebar_section(ui, &mut s.collapsed, c.current, c.cmds)
                        }
                        _ => {}
                    }
                }
                if !tree_listed && c.tree.mode == SidebarMode::Both {
                    tree_section(ui, s, c);
                }
                if !c.sections.contains(&SidebarSection::Smart) {
                    crate::ui::smart::sidebar_section(ui, &mut s.collapsed, c.current, c.cmds);
                }
                crate::ui::search::sidebar_section(ui, c.saved_searches, &mut s.collapsed, c.cmds, c.saved_config);
                ui.add_space(8.0);
            });
        });
    crate::ui::a11y::panel_splitter(root.ctx(), "soyuz.sidebar", crate::tr!("app-a11y-resize-sidebar"));
}

/// The mini icon bar above the sections: Places (sections) | Disk tree (full height).
/// `both` counts as Places (the tree is then a section at the bottom).
fn view_bar(ui: &mut Ui, c: &mut SidebarCtx<'_>) {
    let tree = c.tree.mode == SidebarMode::Tree;
    let items = [
        (false, ph::BOOKMARKS_SIMPLE, tr!("app-sidebar-view-places"), tr!("app-sidebar-view-places-tip")),
        (true, ph::TREE_STRUCTURE, tr!("app-sidebar-view-tree"), tr!("app-sidebar-view-tree-tip")),
    ];
    let bar = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(4.0);
        for (is_tree, glyph, name, tip) in items {
            let on = is_tree == tree;
            let tip = match &c.tree_key {
                Some(k) => tr!("app-sidebar-view-tip-key", tip = tip, key = k),
                None => tip.to_string(),
            };
            if view_button(ui, glyph, name, &tip, on).clicked() && !on {
                let mode = if is_tree { SidebarMode::Tree } else { SidebarMode::Places };
                c.cmds.push(Cmd::SetSidebarMode(mode));
            }
        }
    });
    crate::ui::a11y::container(ui.ctx(), bar.response.id, egui::accesskit::Role::Toolbar, tr!("app-sidebar-view-bar"));
    ui.add_space(2.0);
}

/// One compact icon toggle of the view bar (subtle accent fill when active).
fn view_button(ui: &mut Ui, glyph: &str, name: &str, tip: &str, on: bool) -> egui::Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    crate::ui::a11y::toggle(&resp, name, on);
    if ui.is_rect_visible(rect) {
        let rounding = egui::CornerRadius::same(crate::theme::tokens::radius::CONTROL);
        let hover =
            ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), crate::theme::anim(ui.ctx(), 0.10));
        if on {
            ui.painter().rect_filled(rect, rounding, p.accent_subtle);
        } else if hover > 0.0 {
            ui.painter().rect_filled(rect, rounding, p.bg_surface2.gamma_multiply(hover));
        }
        let color = if on {
            p.accent
        } else if hover > 0.5 {
            p.text_primary
        } else {
            p.text_tertiary
        };
        let font = if on { crate::theme::icons::font_fill(15.0) } else { crate::theme::icons::font(15.0) };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, font, color);
        if resp.has_focus() {
            crate::ui::widgets::focus_ring(ui, rect);
        }
    }
    resp.on_hover_text(tip)
}

/// The Portal section: folders pinned by dropping them on it (see `ui::portal`).
fn portal(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    let key = section_key(SidebarSection::Portal);
    let mut open = !s.collapsed.contains(key);
    let resp = collapsible_header(ui, section_name(SidebarSection::Portal), &mut open);
    if open {
        s.collapsed.remove(key);
    } else {
        s.collapsed.insert(key.to_string());
    }
    let mut pc = crate::ui::portal::PortalCtx { pins: c.portal, current: c.current, pane: c.pane, cmds: &mut *c.cmds };
    crate::ui::portal::section(ui, &mut s.portal, resp.rect, open, &mut pc);
}

/// The "Disk tree" section (`mode = "both"`): filter box and tree rows.
fn tree_section(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Tree) {
        return;
    }
    crate::ui::tree::filter_box(ui, c.tree.state);
    tree_rows(ui, c);
}

fn tree_rows(ui: &mut Ui, c: &mut SidebarCtx<'_>) {
    let mut io = crate::ui::tree::TreeIo { volumes: c.volumes, current: c.current, pane: c.pane, cmds: &mut *c.cmds };
    crate::ui::tree::show(ui, &mut c.tree, &mut io);
}

fn brand(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::TAB_STRIP), Sense::hover());
    let c = pos2(r.left() + 18.0, r.center().y);
    ui.painter().circle_stroke(c, 7.0, Stroke::new(2.0, p.accent));
    ui.painter().circle_filled(c, 2.5, p.accent);
    ui.painter().circle_filled(c + vec2(5.2, -5.2), 2.6, p.glyph_folder);
    ui.painter().text(
        pos2(r.left() + 34.0, r.center().y),
        Align2::LEFT_CENTER,
        "Soyuz",
        semibold(14.0),
        p.text_primary,
    );
}

/// Bottom of the sidebar: a separator and the Settings row (gear + shortcut).
fn footer(ui: &mut Ui, c: &mut SidebarCtx<'_>) {
    let p = palette(ui.ctx());
    let top = ui.max_rect().top() - 3.0;
    let x = ui.max_rect().x_range();
    ui.painter().hline(x.min + 4.0..=x.max - 4.0, top, Stroke::new(1.0, p.border_subtle));
    let resp = NavRow {
        icon: ph::GEAR_SIX,
        label: tr!("app-sidebar-settings"),
        active: c.settings_open,
        icon_color: None,
        usage: None,
    }
    .show(ui);
    if let Some(k) = &c.settings_key
        && ui.is_rect_visible(resp.rect)
    {
        ui.painter().text(
            pos2(resp.rect.right() - 10.0, resp.rect.center().y),
            Align2::RIGHT_CENTER,
            k,
            FontId::proportional(11.0),
            p.text_tertiary,
        );
    }
    let tip = match &c.settings_key {
        Some(k) => tr!("app-sidebar-settings-tip", key = k),
        None => tr!("app-sidebar-settings").to_string(),
    };
    if resp.on_hover_text(tip).clicked() {
        c.cmds.push(Cmd::Action(soyuz_core::config::Action::Settings));
    }
}

/// Section header; returns whether the section is open.
fn header(ui: &mut Ui, s: &mut SidebarState, sec: SidebarSection) -> bool {
    let key = section_key(sec);
    let mut open = !s.collapsed.contains(key);
    collapsible_header(ui, section_name(sec), &mut open);
    if open {
        s.collapsed.remove(key);
    } else {
        s.collapsed.insert(key.to_string());
    }
    open
}

fn is_current(c: &SidebarCtx<'_>, path: &Path) -> bool {
    matches!(c.current, Location::Dir(p) if p == path)
}

fn place_menu(ui: &mut Ui, cmds: &mut Vec<Cmd>, path: &Path, removable: bool) {
    ui.set_width(220.0);
    if menu_item(ui, Some(ph::BROWSERS), tr!("app-ctx-open-in-new-tab"), None, true).clicked() {
        cmds.push(Cmd::OpenInNewTab { location: Location::Dir(path.to_path_buf()), activate: true });
        ui.close();
    }
    if menu_item(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), tr!("app-ctx-open-in-other-pane"), None, true).clicked() {
        cmds.push(Cmd::OpenInOtherPane(Location::Dir(path.to_path_buf())));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROW_SQUARE_UP_RIGHT), tr!("app-ctx-reveal-in-system"), None, true).clicked() {
        cmds.push(Cmd::RevealInSystem(path.to_path_buf()));
        ui.close();
    }
    if menu_item(ui, Some(ph::CLIPBOARD_TEXT), tr!("app-ctx-copy-path"), None, true).clicked() {
        cmds.push(Cmd::CopyText(soyuz_fs::strip_verbatim(path).to_string_lossy().into_owned()));
        ui.close();
    }
    if removable {
        menu_separator(ui);
        if menu_item(ui, Some(ph::STAR), tr!("app-sidebar-remove-favorite"), None, true).clicked() {
            cmds.push(Cmd::RemoveFavorite(path.to_path_buf()));
            ui.close();
        }
    } else {
        menu_separator(ui);
        if menu_item(ui, Some(ph::STAR), tr!("app-ctx-add-to-favorites"), None, true).clicked() {
            cmds.push(Cmd::AddFavorite(path.to_path_buf()));
            ui.close();
        }
    }
}

fn row_clicks(resp: &egui::Response, c: &mut SidebarCtx<'_>, path: &Path) {
    if resp.clicked() {
        c.cmds.push(Cmd::FocusPane(c.pane));
        c.cmds.push(Cmd::Navigate { pane: c.pane, location: Location::Dir(path.to_path_buf()), select: None });
    }
    if resp.middle_clicked() {
        c.cmds.push(Cmd::OpenInNewTab { location: Location::Dir(path.to_path_buf()), activate: false });
    }
}

fn favorites(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Favorites) {
        return;
    }
    let p = palette(ui.ctx());
    let mut rects: Vec<Rect> = Vec::with_capacity(s.favorites.len());
    let favs = s.favorites.clone();
    for (i, f) in favs.iter().enumerate() {
        let r = NavRow {
            icon: favorite_glyph(&f.path),
            label: &f.name,
            active: is_current(c, &f.path),
            // A folder color tints the favorite's icon.
            icon_color: crate::services::folder_colors::color_of(&f.path, &p),
            usage: None,
        }
        .show(ui);
        rects.push(r.rect);
        let r = r.on_hover_text(soyuz_fs::strip_verbatim(&f.path).display().to_string());
        row_clicks(&r, c, &f.path);
        crate::ops::fileops::dnd::target_open(ui, &r, &f.path, c.pane, c.cmds);
        if r.drag_started() {
            s.drag = Some(i);
        }
        let cmds = &mut *c.cmds;
        crate::ui::native::popup::context_menu(&r, |ui| place_menu(ui, cmds, &f.path, true));
    }
    // Drag to reorder.
    if let Some(from) = s.drag {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let down = ui.input(|i| i.pointer.primary_down());
        let to = pointer.map(|pos| rects.iter().position(|r| pos.y < r.center().y).unwrap_or(rects.len()));
        if let (Some(to), Some(first), Some(last)) = (to, rects.first(), rects.last()) {
            let y = if to < rects.len() { rects[to].top() - 0.5 } else { last.bottom() + 0.5 };
            let y = y.clamp(first.top() - 1.0, last.bottom() + 1.0);
            ui.painter()
                .line_segment([pos2(first.left() + 6.0, y), pos2(first.right() - 6.0, y)], Stroke::new(2.0, p.accent));
            ui.painter().circle_filled(pos2(first.left() + 6.0, y), 3.0, p.accent);
        }
        if !down {
            if let Some(to) = to
                && to != from
                && to != from + 1
            {
                c.cmds.push(Cmd::MoveFavorite { from, to });
            }
            s.drag = None;
        }
    }
    for b in c.bookmarks {
        let label = if b.name.is_empty() { soyuz_core::entry::display_name(&b.path) } else { b.name.clone() };
        let r = NavRow {
            icon: bookmark_glyph(&b.icon),
            label: &label,
            active: is_current(c, &b.path),
            icon_color: Some(p.accent),
            usage: None,
        }
        .show(ui);
        let tip = if b.hotkey.is_empty() {
            b.path.display().to_string()
        } else {
            format!("{}  ·  {}", b.path.display(), b.hotkey)
        };
        let r = r.on_hover_text(tip);
        row_clicks(&r, c, &b.path);
        crate::ops::fileops::dnd::target_open(ui, &r, &b.path, c.pane, c.cmds);
        let cmds = &mut *c.cmds;
        crate::ui::native::popup::context_menu(&r, |ui| place_menu(ui, cmds, &b.path, false));
    }
    gtk_bookmarks(ui, s, c);
}

fn devices(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Devices) {
        return;
    }
    let active = matches!(c.current, Location::Computer);
    let r = NavRow {
        icon: ph::DESKTOP_TOWER,
        label: if cfg!(windows) { tr!("app-sidebar-this-pc") } else { tr!("app-sidebar-computer") },
        active,
        icon_color: None,
        usage: None,
    }
    .show(ui);
    if r.clicked() {
        c.cmds.push(Cmd::FocusPane(c.pane));
        c.cmds.push(Cmd::Navigate { pane: c.pane, location: Location::Computer, select: None });
    }
    let vols: Vec<VolumeInfo> = c.volumes.to_vec();
    let desk_devices = if cfg!(target_os = "linux") { desk().lock().devices.clone() } else { Vec::new() };
    for v in &vols {
        let usage = (v.total > 0).then(|| 1.0 - v.free as f32 / v.total as f32);
        let label = volume_label(v);
        let active = matches!(c.current, Location::Dir(p) if p == &v.mount);
        let r = NavRow { icon: volume_glyph(v), label: &label, active, icon_color: None, usage }.show(ui);
        let tip = if v.total > 0 {
            tr!(
                "app-sidebar-volume-tip",
                path = v.mount.display().to_string(),
                free = format_size(v.free, SizeStyle::Decimal),
                total = format_size(v.total, SizeStyle::Decimal)
            )
        } else {
            v.mount.display().to_string()
        };
        let r = r.on_hover_text(tip);
        let dev = if desk_devices.is_empty() { None } else { device_at(&desk_devices, &v.mount) };
        let removable_dev = dev.as_ref().filter(|d| d.drive.is_some() && (d.ejectable || d.can_power_off));
        // Removable media: a small eject button on the right of the row.
        let eject_hit = removable_dev.and_then(|d| {
            let br = Rect::from_center_size(pos2(r.rect.right() - 14.0, r.rect.top() + 14.0), vec2(20.0, 20.0));
            let er = ui.interact(br, r.id.with("eject"), Sense::click());
            if ui.is_rect_visible(br) && (r.hovered() || er.hovered()) {
                let p = palette(ui.ctx());
                let color = if er.hovered() { p.text_primary } else { p.text_tertiary };
                ui.painter().text(
                    br.center(),
                    Align2::CENTER_CENTER,
                    ph::EJECT_SIMPLE,
                    crate::theme::icons::font(13.0),
                    color,
                );
            }
            let er = er.on_hover_text(tr!("app-device-eject"));
            er.clicked().then(|| d.clone())
        });
        if let Some(d) = eject_hit {
            eject_device(&d, &desk_devices);
        } else {
            row_clicks(&r, c, &v.mount);
        }
        crate::ops::fileops::dnd::target_open(ui, &r, &v.mount, c.pane, c.cmds);
        let cmds = &mut *c.cmds;
        crate::ui::native::popup::context_menu(&r, |ui| {
            place_menu(ui, cmds, &v.mount, false);
            if let Some(d) = &dev {
                device_menu(ui, d, &desk_devices, &label);
            }
        });
    }
    if cfg!(target_os = "linux") {
        unmounted_devices(ui, s, c);
    }
    if vols.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let p = palette(ui.ctx());
        ui.painter().text(
            pos2(rect.left() + 34.0, rect.center().y),
            Align2::LEFT_CENTER,
            tr!("app-sidebar-scanning"),
            FontId::proportional(12.0),
            p.text_tertiary,
        );
    }
}

// ---- Linux desktop: GTK bookmarks and UDisks2 devices ------------------------------------
//
// One background thread ("soyuz-desktop-places", Linux only) re-reads the GTK bookmarks
// file and the UDisks2 block devices every `DESKTOP_REFRESH` and whenever the volume list
// changes. Mount / unmount / eject run on their own threads (polkit may prompt); their
// results come back through `Desk::events` and are turned into toasts / navigation on the
// next frame. Nothing here touches D-Bus or the disk on the UI thread.

/// Background refresh of the GTK bookmarks and UDisks2 devices.
const DESKTOP_REFRESH: std::time::Duration = std::time::Duration::from_secs(10);

/// `[sidebar] gtk_bookmarks`: show the GTK bookmarks (`~/.config/gtk-3.0/bookmarks`)
/// under Favorites (Linux; default on).
static SHOW_GTK_BOOKMARKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Show / hide the GTK bookmarks in the Favorites section.
pub fn set_show_gtk_bookmarks(on: bool) {
    SHOW_GTK_BOOKMARKS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Result of a device operation, applied on the UI thread.
enum DeskEvent {
    Mounted {
        pane: PaneId,
        path: PathBuf,
    },
    Unmounted {
        name: String,
    },
    Ejected {
        name: String,
    },
    Locked {
        name: String,
    },
    Failed {
        title: String,
        body: String,
    },
    /// NTFS left hibernated / dirty by Windows: offer a read-only mount.
    OfferReadOnly {
        dev: Box<soyuz_fs::udisks::BlockDevice>,
        name: String,
    },
}

/// What a busy device is doing (the row shows it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Work {
    Mounting,
    Unlocking,
    Unmounting,
    Locking,
}

#[derive(Default)]
struct Desk {
    started: bool,
    /// UDisks2 answered (devices are listed); `false` hides the device extras.
    available: bool,
    /// Every block device UDisks2 knows (mounted or not).
    devices: Vec<soyuz_fs::udisks::BlockDevice>,
    /// Bumped whenever `devices` changes (the sidebar re-classifies then).
    generation: u64,
    gtk: Vec<soyuz_fs::xdg_dirs::Bookmark>,
    /// UDisks object paths with an operation running.
    busy: std::collections::BTreeMap<String, Work>,
    events: Vec<DeskEvent>,
    trigger: Option<crossbeam_channel::Sender<()>>,
    /// Signature of the volume list last seen (a change triggers a device rescan).
    vol_sig: u64,
}

fn desk() -> &'static parking_lot::Mutex<Desk> {
    static D: std::sync::OnceLock<parking_lot::Mutex<Desk>> = std::sync::OnceLock::new();
    D.get_or_init(Default::default)
}

/// Start the refresher once (Linux only).
fn desk_start(ctx: &egui::Context) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let mut d = desk().lock();
    if d.started {
        return;
    }
    d.started = true;
    let (tx, rx) = crossbeam_channel::bounded::<()>(1);
    d.trigger = Some(tx);
    drop(d);
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("soyuz-desktop-places".into()).spawn(move || {
        let udisks = soyuz_fs::udisks::available();
        loop {
            let gtk = soyuz_fs::xdg_dirs::gtk_bookmarks();
            let devices = if udisks {
                match soyuz_fs::udisks::block_devices() {
                    Ok(v) => Some(v),
                    Err(e) => {
                        log::debug!("udisks: {e}");
                        None
                    }
                }
            } else {
                None
            };
            let changed = {
                let mut d = desk().lock();
                let mut changed = d.gtk != gtk;
                d.gtk = gtk;
                if let Some(devices) = devices {
                    if d.devices != devices || !d.available {
                        changed = true;
                        d.generation += 1;
                    }
                    d.devices = devices;
                    d.available = true;
                }
                changed
            };
            if changed {
                ctx.request_repaint();
            }
            match rx.recv_timeout(DESKTOP_REFRESH) {
                Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("cannot spawn the desktop places thread: {e}");
    }
}

/// Ask the refresher for a rescan now (after a device operation, or new volumes).
fn desk_rescan() {
    if let Some(t) = &desk().lock().trigger {
        let _ = t.try_send(());
    }
}

/// The volume list should be re-read now (a device was mounted / unmounted from the
/// sidebar); taken by the frame loop, which asks the places service for a refresh.
static VOLUMES_STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A device was mounted / unmounted from the sidebar since the last call.
pub fn take_volumes_stale() -> bool {
    VOLUMES_STALE.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Per frame (sidebar shown): start the refresher, rescan when the volume list changed,
/// turn finished device operations into toasts / navigation.
fn desk_pump(ctx: &egui::Context, c: &mut SidebarCtx<'_>) {
    if !cfg!(target_os = "linux") {
        return;
    }
    desk_start(ctx);
    let sig = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for v in c.volumes {
            v.mount.hash(&mut h);
        }
        h.finish()
    };
    let events = {
        let mut d = desk().lock();
        let rescan = d.vol_sig != sig;
        d.vol_sig = sig;
        let ev = std::mem::take(&mut d.events);
        drop(d);
        if rescan {
            desk_rescan();
        }
        ev
    };
    for ev in events {
        use crate::ui::toasts::Toast;
        VOLUMES_STALE.store(true, std::sync::atomic::Ordering::Relaxed);
        match ev {
            DeskEvent::Mounted { pane, path } => {
                c.cmds.push(Cmd::FocusPane(pane));
                c.cmds.push(Cmd::Navigate { pane, location: Location::Dir(path), select: None });
            }
            DeskEvent::Unmounted { name } => {
                c.cmds.push(Cmd::Toast(Toast::info(tr!("app-device-unmounted", name = name))))
            }
            DeskEvent::Ejected { name } => c.cmds.push(Cmd::Toast(Toast::info(tr!("app-device-ejected", name = name)))),
            DeskEvent::Locked { name } => {
                c.cmds.push(Cmd::Toast(Toast::info(tr!("app-device-locked-done", name = name))))
            }
            DeskEvent::Failed { title, body } => c.cmds.push(Cmd::Toast(Toast::error(title).body(body))),
            DeskEvent::OfferReadOnly { dev, name } => c.cmds.push(Cmd::Device(DeviceCmd::OfferReadOnly { dev, name })),
        }
    }
}

/// A short, human reason for a failed UDisks2 call (`fs_type`: the device's filesystem).
fn udisks_reason(e: &std::io::Error, fs_type: &str) -> String {
    use soyuz_fs::udisks::Failure;
    match soyuz_fs::udisks::classify_error(e, fs_type) {
        Failure::NotAuthorized => tr!("app-device-not-authorized").to_string(),
        Failure::Dismissed => tr!("app-device-auth-dismissed").to_string(),
        Failure::Busy => tr!("app-device-busy").to_string(),
        Failure::WindowsHibernated => tr!("app-device-hibernated").to_string(),
        Failure::UnsupportedFs => tr!("app-device-unsupported-fs", fs = soyuz_fs::udisks::fs_display_name(fs_type)),
        Failure::WrongPassphrase => tr!("app-device-wrong-passphrase").to_string(),
        Failure::NoUdisks => tr!("app-device-no-udisks").to_string(),
        Failure::Other => e.to_string().trim_start_matches("UDisks2: ").to_string(),
    }
}

/// Run a device operation on its own thread; `object` is marked busy meanwhile.
fn desk_op(object: &str, work: Work, op: impl FnOnce() -> DeskEvent + Send + 'static) {
    let object = object.to_string();
    {
        let mut d = desk().lock();
        if d.busy.contains_key(&object) {
            return;
        }
        d.busy.insert(object.clone(), work);
    }
    let key = object.clone();
    let spawned = std::thread::Builder::new().name("soyuz-udisks-op".into()).spawn(move || {
        let ev = op();
        let mut d = desk().lock();
        d.busy.remove(&key);
        d.events.push(ev);
        drop(d);
        desk_rescan();
    });
    if let Err(e) = spawned {
        desk().lock().busy.remove(&object);
        log::warn!("cannot spawn a device thread: {e}");
    }
}

/// Mount `dev` (polkit may ask; the OS picks the driver), then open it in `pane`.
/// `read_only` mounts with `ro` (NTFS that Windows left hibernated).
fn mount_device(dev: &soyuz_fs::udisks::BlockDevice, name: String, pane: PaneId, read_only: bool) {
    let d = dev.clone();
    desk_op(&dev.object, Work::Mounting, move || mount_now(&d, name, pane, read_only, false));
}

/// The blocking part of a mount (on a device thread). `fresh`: the device was just
/// unlocked, so UDisks may need a moment to probe its filesystem.
fn mount_now(d: &soyuz_fs::udisks::BlockDevice, name: String, pane: PaneId, read_only: bool, fresh: bool) -> DeskEvent {
    let o = soyuz_fs::udisks::MountOptions {
        interactive: true,
        options: if read_only { "ro".into() } else { String::new() },
    };
    let mut tries = if fresh { 10 } else { 1 };
    loop {
        tries -= 1;
        match soyuz_fs::udisks::mount(&d.object, &o) {
            Ok(path) => return DeskEvent::Mounted { pane, path },
            Err(e) => {
                let text = e.to_string();
                // The cleartext device's Filesystem interface appears after the probe.
                if tries > 0 && (text.contains("Unknown") || text.contains("interface")) {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    continue;
                }
                if !read_only
                    && soyuz_fs::udisks::classify_error(&e, &d.fs_type) == soyuz_fs::udisks::Failure::WindowsHibernated
                {
                    return DeskEvent::OfferReadOnly { dev: Box::new(d.clone()), name };
                }
                return DeskEvent::Failed {
                    title: tr!("app-device-mount-failed", name = name),
                    body: udisks_reason(&e, &d.fs_type),
                };
            }
        }
    }
}

fn unmount_device(dev: &soyuz_fs::udisks::BlockDevice, name: String) {
    let object = dev.object.clone();
    let fs = dev.fs_type.clone();
    desk_op(&dev.object, Work::Unmounting, move || match soyuz_fs::udisks::unmount(&object, false, true) {
        Ok(()) => DeskEvent::Unmounted { name },
        Err(e) => {
            DeskEvent::Failed { title: tr!("app-device-unmount-failed", name = name), body: udisks_reason(&e, &fs) }
        }
    });
}

/// Unmount the cleartext filesystem (when mounted), then lock its container.
fn lock_device(cleartext: &soyuz_fs::udisks::BlockDevice, container: String, name: String) {
    let object = cleartext.object.clone();
    let mounted = !cleartext.mount_points.is_empty();
    let fs = cleartext.fs_type.clone();
    desk_op(&container.clone(), Work::Locking, move || {
        let failed = |e: std::io::Error| DeskEvent::Failed {
            title: tr!("app-device-lock-failed", name = name.clone()),
            body: udisks_reason(&e, &fs),
        };
        if mounted && let Err(e) = soyuz_fs::udisks::unmount(&object, false, true) {
            return failed(e);
        }
        match soyuz_fs::udisks::lock(&container, true) {
            Ok(()) => DeskEvent::Locked { name: name.clone() },
            Err(e) => failed(e),
        }
    });
}

/// Unlock a BitLocker / LUKS container with `secret` (zeroized afterwards), then mount
/// its filesystem and open it in `pane`. `reply` gets `Ok(())` once unlocked, or the
/// reason it failed (the unlock dialog shows it and stays open).
pub fn unlock_device(
    object: &str,
    name: String,
    mut secret: zeroize::Zeroizing<String>,
    pane: PaneId,
    reply: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let object = object.to_string();
    {
        let mut d = desk().lock();
        if d.busy.contains_key(&object) {
            return;
        }
        d.busy.insert(object.clone(), Work::Unlocking);
    }
    let key = object.clone();
    let target = object.clone();
    let spawned = std::thread::Builder::new().name("soyuz-udisks-unlock".into()).spawn(move || {
        let r = soyuz_fs::udisks::unlock(&target, secret.as_str(), true);
        zeroize::Zeroize::zeroize(&mut *secret);
        drop(secret);
        desk().lock().busy.remove(&key);
        match r {
            Ok(cleartext) => {
                let _ = reply.send(Ok(()));
                let dev = soyuz_fs::udisks::BlockDevice { object: cleartext.clone(), ..Default::default() };
                desk_op(&cleartext, Work::Mounting, move || mount_now(&dev, name, pane, false, true));
            }
            Err(e) => {
                let _ = reply.send(Err(udisks_reason(&e, "")));
                desk_rescan();
            }
        }
    });
    if let Err(e) = spawned {
        desk().lock().busy.remove(&object);
        log::warn!("cannot spawn a device thread: {e}");
    }
}

/// Unmount every mounted file system of the drive, then eject it (or power it off when
/// it can't eject, e.g. a USB disk).
fn eject_device(dev: &soyuz_fs::udisks::BlockDevice, all: &[soyuz_fs::udisks::BlockDevice]) {
    let Some(drive) = dev.drive.clone() else { return };
    let mounted: Vec<String> = all
        .iter()
        .filter(|b| b.drive.as_deref() == Some(drive.as_str()) && !b.mount_points.is_empty())
        .map(|b| b.object.clone())
        .collect();
    let (ejectable, power_off) = (dev.ejectable, dev.can_power_off);
    let name = dev.display_name();
    let fs = dev.fs_type.clone();
    desk_op(&dev.object, Work::Unmounting, move || {
        let failed = |e: std::io::Error| DeskEvent::Failed {
            title: tr!("app-device-eject-failed", name = name.clone()),
            body: udisks_reason(&e, &fs),
        };
        for o in &mounted {
            if let Err(e) = soyuz_fs::udisks::unmount(o, false, true) {
                return failed(e);
            }
        }
        let r = if ejectable {
            soyuz_fs::udisks::eject(&drive, true)
        } else if power_off {
            soyuz_fs::udisks::power_off(&drive, true)
        } else {
            Ok(())
        };
        match r {
            Ok(()) => DeskEvent::Ejected { name: name.clone() },
            Err(e) => failed(e),
        }
    });
}

/// Mount points of the running system (`/`, `/boot`, `/home`, `/var` …): never offered
/// for unmounting from the sidebar. Mounts below the home folder or under `/mnt`,
/// `/media`, `/run/media` are the user's.
fn is_system_mount(m: &Path) -> bool {
    const EXACT: [&str; 4] = ["/", "/home", "/mnt", "/media"];
    const TREES: [&str; 13] =
        ["/boot", "/efi", "/usr", "/var", "/opt", "/srv", "/nix", "/etc", "/root", "/sysroot", "/gnu", "/tmp", "/snap"];
    EXACT.iter().any(|e| m == Path::new(e)) || TREES.iter().any(|t| m.starts_with(t))
}

/// The UDisks2 device mounted at `mount`, if it may be unmounted from the sidebar
/// (anything but a mount of the running system).
fn device_at(devices: &[soyuz_fs::udisks::BlockDevice], mount: &Path) -> Option<soyuz_fs::udisks::BlockDevice> {
    if is_system_mount(mount) {
        return None;
    }
    devices
        .iter()
        .find(|b| b.mount_points.iter().any(|m| m == mount) && b.mount_points.iter().all(|m| !is_system_mount(m)))
        .cloned()
}

/// Unmount / Lock / Eject entries of a mounted device's context menu.
fn device_menu(ui: &mut Ui, dev: &soyuz_fs::udisks::BlockDevice, all: &[soyuz_fs::udisks::BlockDevice], name: &str) {
    menu_separator(ui);
    if menu_item(ui, Some(ph::ARROW_LINE_UP), tr!("app-device-unmount"), None, true).clicked() {
        unmount_device(dev, name.to_string());
        ui.close();
    }
    if let Some(container) = &dev.crypto_backing
        && menu_item(ui, Some(ph::LOCK_SIMPLE), tr!("app-device-lock"), None, true).clicked()
    {
        lock_device(dev, container.clone(), name.to_string());
        ui.close();
    }
    if dev.drive.is_some()
        && (dev.ejectable || dev.can_power_off)
        && menu_item(ui, Some(ph::EJECT_SIMPLE), tr!("app-device-eject"), None, true).clicked()
    {
        eject_device(dev, all);
        ui.close();
    }
}

/// Sidebar effects that need the app (dialogs, config): `Cmd::Device`.
#[derive(Clone, Debug)]
pub enum DeviceCmd {
    /// Ask for the passphrase / recovery key of a locked container.
    Unlock { object: String, name: String, kind: String },
    /// A mount failed because Windows left the NTFS volume hibernated: explain and
    /// offer a read-only mount.
    OfferReadOnly { dev: Box<soyuz_fs::udisks::BlockDevice>, name: String },
    /// Mount (read-only or not) into the focused pane.
    Mount { dev: Box<soyuz_fs::udisks::BlockDevice>, name: String, read_only: bool },
    /// "Hide from sidebar": add the key to `[sidebar] hidden_devices`.
    Hide(String),
}

/// Apply a [`DeviceCmd`].
pub fn apply_device(app: &mut crate::app::SoyuzApp, ctx: &egui::Context, cmd: DeviceCmd) {
    let pane = app.ws.pane().id;
    match cmd {
        DeviceCmd::Unlock { object, name, kind } => {
            app.ui.dialogs.push(crate::ui::dialogs::unlock_volume::UnlockVolumeDialog::new(object, name, kind, pane))
        }
        DeviceCmd::OfferReadOnly { dev, name } => app.ui.dialogs.push(crate::ui::dialogs::ConfirmDialog::new(
            &tr!("app-device-hibernated-title", name = name.clone()),
            tr!("app-device-hibernated-body"),
            tr!("app-device-mount-ro"),
            Cmd::Device(DeviceCmd::Mount { dev, name, read_only: true }),
        )),
        DeviceCmd::Mount { dev, name, read_only } => mount_device(&dev, name, pane, read_only),
        DeviceCmd::Hide(key) => {
            if !app.config.sidebar.hidden_devices.contains(&key) {
                crate::ui::settings::update_config(app, ctx, |c| c.sidebar.hidden_devices.push(key));
                app.ui
                    .toasts
                    .push(crate::ui::toasts::Toast::info(tr!("app-device-hidden")).body(tr!("app-device-hidden-body")));
            }
        }
    }
}

/// Name of an unmounted device: its label, else "<size> <filesystem> volume".
fn candidate_name(c: &soyuz_fs::udisks::Candidate) -> String {
    let d = &c.dev;
    for s in [&d.label, &d.hint_name] {
        if !s.trim().is_empty() {
            return s.trim().to_string();
        }
    }
    let fs = soyuz_fs::udisks::fs_display_name(&d.fs_type);
    if d.size > 0 {
        tr!("app-device-volume-name", size = format_size(d.size, SizeStyle::Decimal), fs = fs)
    } else {
        tr!("app-device-volume-name-nosize", fs = fs)
    }
}

/// Badge text: the filesystem, or the container type for a locked device.
fn candidate_badge(c: &soyuz_fs::udisks::Candidate) -> String {
    soyuz_fs::udisks::fs_display_name(&c.dev.fs_type)
}

fn candidate_tip(c: &soyuz_fs::udisks::Candidate, name: &str) -> String {
    use soyuz_fs::udisks::CandidateState as S;
    let d = &c.dev;
    let mut fs = soyuz_fs::udisks::fs_display_name(&d.fs_type);
    if !c.container_type.is_empty() {
        fs = format!("{fs} · {}", soyuz_fs::udisks::fs_display_name(&c.container_type));
    }
    let (vendor, model) = (d.drive_vendor.trim(), d.drive_model.trim());
    let drive =
        if vendor.is_empty() || model.starts_with(vendor) { model.to_string() } else { format!("{vendor} {model}") };
    let kind = if d.is_removable_media() { tr!("app-device-removable") } else { tr!("app-device-internal") };
    let hint = match c.state {
        S::Unmounted => tr!("app-device-click-mount"),
        S::Locked => tr!("app-device-click-unlock"),
        S::Unsupported => tr!("app-device-unsupported-hint"),
    };
    tr!(
        "app-device-candidate-tip",
        name = name,
        device = d.device.display().to_string(),
        fs = fs,
        size = if d.size > 0 { format_size(d.size, SizeStyle::Decimal) } else { "?".into() },
        drive = if drive.is_empty() { kind.to_string() } else { format!("{drive} · {kind}") },
        hint = hint
    )
}

/// Unmounted devices for the current config, re-classified only when the device list
/// or the config changed.
#[derive(Default)]
pub struct DevicesCache {
    key: Option<(u64, soyuz_core::config::UnmountedDevices, Vec<String>)>,
    list: Vec<soyuz_fs::udisks::Candidate>,
}

fn candidates(s: &mut SidebarState, cfg: &soyuz_core::config::Sidebar) -> Vec<soyuz_fs::udisks::Candidate> {
    use soyuz_core::config::UnmountedDevices;
    let d = desk().lock();
    if !d.available || cfg.unmounted_devices == UnmountedDevices::Off {
        return Vec::new();
    }
    let key = (d.generation, cfg.unmounted_devices, cfg.hidden_devices.clone());
    if s.devices.key.as_ref() != Some(&key) {
        let removable = cfg.unmounted_devices == UnmountedDevices::Removable;
        s.devices.list = soyuz_fs::udisks::sidebar_candidates(&d.devices, removable, &cfg.hidden_devices);
        s.devices.key = Some(key);
    }
    s.devices.list.clone()
}

/// Rows for devices that are not mounted (or locked): click mounts (and opens) or asks
/// for the passphrase. They look unlike mounted volumes: a dimmed outline glyph, a muted
/// label, a filesystem badge and a "Not mounted" / "Locked" line where mounted volumes
/// show their usage bar.
fn unmounted_devices(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    use soyuz_fs::udisks::CandidateState as S;
    let list = candidates(s, c.config);
    if list.is_empty() {
        return;
    }
    let busy = desk().lock().busy.clone();
    for cand in &list {
        let name = candidate_name(cand);
        let work = busy.get(&cand.dev.object).copied();
        let (r, button) = candidate_row(ui, cand, &name, work);
        let r = r.on_hover_text(candidate_tip(cand, &name));
        let activate = (r.clicked() || button) && work.is_none();
        if activate {
            match cand.state {
                S::Unmounted => mount_device(&cand.dev, name.clone(), c.pane, false),
                S::Locked => c.cmds.push(Cmd::Device(DeviceCmd::Unlock {
                    object: cand.dev.object.clone(),
                    name: name.clone(),
                    kind: cand.dev.fs_type.clone(),
                })),
                S::Unsupported => {}
            }
        }
        let pane = c.pane;
        let cmds = &mut *c.cmds;
        crate::ui::native::popup::context_menu(&r, |ui| candidate_menu(ui, cand, &name, work.is_none(), pane, cmds));
    }
}

fn candidate_menu(
    ui: &mut Ui,
    cand: &soyuz_fs::udisks::Candidate,
    name: &str,
    idle: bool,
    pane: PaneId,
    cmds: &mut Vec<Cmd>,
) {
    use soyuz_fs::udisks::CandidateState as S;
    ui.set_width(220.0);
    match cand.state {
        S::Unmounted => {
            if menu_item(ui, Some(ph::ARROW_LINE_DOWN), tr!("app-device-mount"), None, idle).clicked() {
                mount_device(&cand.dev, name.to_string(), pane, false);
                ui.close();
            }
            if menu_item(ui, Some(ph::LOCK_SIMPLE), tr!("app-device-mount-ro"), None, idle).clicked() {
                mount_device(&cand.dev, name.to_string(), pane, true);
                ui.close();
            }
            if let Some(container) = &cand.container
                && menu_item(ui, Some(ph::LOCK_SIMPLE), tr!("app-device-lock"), None, idle).clicked()
            {
                lock_device(&cand.dev, container.clone(), name.to_string());
                ui.close();
            }
        }
        S::Locked => {
            if menu_item(ui, Some(ph::LOCK_SIMPLE_OPEN), tr!("app-device-unlock"), None, idle).clicked() {
                cmds.push(Cmd::Device(DeviceCmd::Unlock {
                    object: cand.dev.object.clone(),
                    name: name.to_string(),
                    kind: cand.dev.fs_type.clone(),
                }));
                ui.close();
            }
        }
        S::Unsupported => {
            let _ = menu_item(ui, Some(ph::PROHIBIT), tr!("app-device-mount"), None, false);
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::EYE_SLASH), tr!("app-device-hide"), None, true).clicked() {
        cmds.push(Cmd::Device(DeviceCmd::Hide(cand.hide_key())));
        ui.close();
    }
    if menu_item(ui, Some(ph::CLIPBOARD_TEXT), tr!("app-device-copy-path"), None, true).clicked() {
        cmds.push(Cmd::CopyText(cand.dev.device.display().to_string()));
        ui.close();
    }
}

/// One unmounted / locked device row. Returns the row response and whether the hover
/// button (mount / unlock) was clicked.
fn candidate_row(
    ui: &mut Ui,
    cand: &soyuz_fs::udisks::Candidate,
    name: &str,
    work: Option<Work>,
) -> (egui::Response, bool) {
    use soyuz_fs::udisks::CandidateState as S;
    let p = palette(ui.ctx());
    let base = size::ROW_COZY.max(ui.spacing().interact_size.y + 4.0);
    let height = base + 8.0;
    // Unsupported rows still take clicks: the context menu hides / copies them.
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    crate::ui::a11y::button(&resp, name);
    let mut button = false;
    let show_button = cand.state != S::Unsupported && work.is_none();
    let br = Rect::from_center_size(pos2(rect.right() - 14.0, rect.top() + base / 2.0), vec2(22.0, 22.0));
    let bresp = show_button.then(|| ui.interact(br, resp.id.with("mount"), Sense::click()));
    let hovered = resp.hovered() || bresp.as_ref().is_some_and(|b| b.hovered()) || resp.has_focus();
    if let Some(b) = &bresp {
        button = b.clicked();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect.expand(3.0));
        let rounding = egui::CornerRadius::same(crate::theme::tokens::radius::ROW);
        let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), hovered, crate::theme::anim(ui.ctx(), 0.10));
        if hover > 0.0 && cand.state != S::Unsupported {
            painter.rect_filled(rect, rounding, p.bg_surface2.gamma_multiply(hover));
        }
        let text_y = rect.top() + base / 2.0;
        // Glyph: the drive outline, dimmed; a lock badge on locked containers.
        let glyph = if cand.dev.is_removable_media() { ph::USB } else { ph::HARD_DRIVE };
        let dim = if cand.state == S::Unsupported { p.text_tertiary.gamma_multiply(0.6) } else { p.text_tertiary };
        let icon_pos = pos2(rect.left() + 10.0, text_y);
        painter.text(icon_pos, Align2::LEFT_CENTER, glyph, crate::theme::icons::font(size::ICON), dim);
        if cand.state == S::Locked {
            let c = pos2(icon_pos.x + 14.0, text_y + 5.0);
            painter.circle_filled(c, 5.5, p.bg_window);
            painter.text(
                c,
                Align2::CENTER_CENTER,
                egui_phosphor::fill::LOCK_SIMPLE,
                crate::theme::icons::font_fill(9.5),
                p.warning,
            );
        }
        // Right side: the filesystem badge, or (hovered) the mount / unlock button.
        let mut right = rect.right() - 6.0;
        if let Some(b) = &bresp
            && hover > 0.5
        {
            let g = if cand.state == S::Locked { ph::LOCK_SIMPLE_OPEN } else { ph::ARROW_LINE_DOWN };
            if b.hovered() {
                painter.rect_filled(br, egui::CornerRadius::same(crate::theme::tokens::radius::CONTROL), p.bg_surface3);
            }
            let color = if b.hovered() { p.accent } else { p.text_secondary };
            painter.text(br.center(), Align2::CENTER_CENTER, g, crate::theme::icons::font(14.0), color);
            right = br.left() - 4.0;
        } else {
            let badge = candidate_badge(cand);
            let font = FontId::proportional(9.5);
            let g = painter.layout_no_wrap(badge, font, p.text_tertiary);
            let w = g.size().x + 10.0;
            let r = Rect::from_min_size(pos2(right - w, text_y - 7.5), vec2(w, 15.0));
            painter.rect_stroke(
                r,
                egui::CornerRadius::same(4),
                Stroke::new(1.0, p.border_strong),
                egui::StrokeKind::Inside,
            );
            painter.galley(pos2(r.left() + 5.0, r.center().y - g.size().y / 2.0), g, p.text_tertiary);
            right = r.left() - 6.0;
        }
        let label_color = if cand.state == S::Unsupported { p.text_tertiary } else { p.text_secondary };
        let max_w = right - (rect.left() + 34.0);
        let galley = trunc_galley(&painter, name, FontId::proportional(13.0), label_color, max_w);
        painter.galley(pos2(rect.left() + 34.0, text_y - galley.size().y / 2.0), galley, label_color);
        // Second line, where mounted volumes show their usage bar.
        let size_txt = if cand.dev.size > 0 { format_size(cand.dev.size, SizeStyle::Decimal) } else { String::new() };
        let status = match (work, cand.state) {
            (Some(Work::Mounting), _) => tr!("app-device-status-mounting").to_string(),
            (Some(Work::Unlocking), _) => tr!("app-device-status-unlocking").to_string(),
            (Some(Work::Locking), _) => tr!("app-device-status-locking").to_string(),
            (Some(Work::Unmounting), _) => tr!("app-device-status-unmounting").to_string(),
            (None, S::Unmounted) => tr!("app-device-status-unmounted", size = size_txt),
            (None, S::Locked) => tr!("app-device-status-locked", size = size_txt),
            (None, S::Unsupported) => tr!("app-device-status-unsupported", size = size_txt),
        };
        let line_y = rect.bottom() - 8.0;
        let mut x = rect.left() + 34.0;
        if work.is_some() {
            crate::ui::widgets::spinner(ui, pos2(x + 4.0, line_y), 3.5, p.accent);
            x += 12.0;
        }
        let status_color = if work.is_some() { p.text_secondary } else { p.text_tertiary };
        let g = trunc_galley(&painter, &status, FontId::proportional(10.5), status_color, rect.right() - 8.0 - x);
        painter.galley(pos2(x, line_y - g.size().y / 2.0), g, status_color);
        if resp.has_focus() {
            crate::ui::widgets::focus_ring(ui, rect);
        }
    }
    (resp, button)
}

/// GTK bookmarks not already shown (favorites, config bookmarks).
fn gtk_bookmarks(ui: &mut Ui, s: &SidebarState, c: &mut SidebarCtx<'_>) {
    if !cfg!(target_os = "linux")
        || !c.config.gtk_bookmarks
        || !SHOW_GTK_BOOKMARKS.load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    let marks = desk().lock().gtk.clone();
    let p = palette(ui.ctx());
    for b in &marks {
        let loc = match &b.path {
            Some(path) => {
                if s.favorites.iter().any(|f| &f.path == path) || c.bookmarks.iter().any(|k| &k.path == path) {
                    continue;
                }
                Location::Dir(path.clone())
            }
            None if soyuz_core::location::looks_remote(&b.uri) => Location::Remote(b.uri.clone()),
            None => continue,
        };
        let label = b.display_name();
        let active = c.current == &loc;
        let icon = if b.path.is_some() { ph::BOOKMARK_SIMPLE } else { ph::GLOBE };
        let r = NavRow { icon, label: &label, active, icon_color: Some(p.text_secondary), usage: None }.show(ui);
        let tip = b.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| b.uri.clone());
        let r = r.on_hover_text(tip);
        if r.clicked() {
            c.cmds.push(Cmd::FocusPane(c.pane));
            c.cmds.push(Cmd::Navigate { pane: c.pane, location: loc.clone(), select: None });
        }
        if r.middle_clicked() {
            c.cmds.push(Cmd::OpenInNewTab { location: loc.clone(), activate: false });
        }
        if let Some(path) = &b.path {
            crate::ops::fileops::dnd::target_open(ui, &r, path, c.pane, c.cmds);
            let cmds = &mut *c.cmds;
            crate::ui::native::popup::context_menu(&r, |ui| place_menu(ui, cmds, path, false));
        }
    }
}

fn cloud(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Cloud) {
        return;
    }
    let p = palette(ui.ctx());
    let items: Vec<(String, PathBuf)> = c.cloud.to_vec();
    for (name, path) in &items {
        let r =
            NavRow { icon: ph::CLOUD, label: name, active: is_current(c, path), icon_color: Some(p.info), usage: None }
                .show(ui);
        let r = r.on_hover_text(path.display().to_string());
        row_clicks(&r, c, path);
        crate::ops::fileops::dnd::target_open(ui, &r, path, c.pane, c.cmds);
        let cmds = &mut *c.cmds;
        crate::ui::native::popup::context_menu(&r, |ui| place_menu(ui, cmds, path, false));
    }
}

fn other(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Trash) {
        return;
    }
    crate::ui::tools::tags_ui::recent_row(ui, c.current, c.pane, c.cmds);
    crate::ui::workspaces::timeline_row(ui, c.current, c.pane, c.cmds);
    let active = matches!(c.current, Location::Trash);
    let r = NavRow { icon: ph::TRASH, label: tr!("app-sidebar-trash"), active, icon_color: None, usage: None }.show(ui);
    crate::ops::fileops::dnd::trash_target(ui, &r, c.cmds);
    if r.clicked() {
        c.cmds.push(Cmd::FocusPane(c.pane));
        c.cmds.push(Cmd::Navigate { pane: c.pane, location: Location::Trash, select: None });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_folders_replace_the_english_defaults() {
        let home = soyuz_fs::home();
        let xdg = soyuz_fs::xdg_dirs::UserDirs {
            documents: Some(home.join("Belgeler")),
            desktop: Some(home.clone()), // disabled the XDG way
            ..Default::default()
        };
        let places = candidate_places(Some(&xdg));
        assert_eq!(places[0].path, home);
        let docs = places.iter().find(|p| p.label == "Documents").unwrap();
        assert_eq!(docs.path, home.join("Belgeler"));
        assert!(places.iter().all(|p| p.label != "Desktop"), "a role pointing at home is left out");
        assert_eq!(places.iter().find(|p| p.label == "Music").unwrap().path, home.join("Music"), "fallback");
        assert_eq!(candidate_places(None).len(), 1 + STANDARD.len());
    }

    #[test]
    fn devices_match_volumes_by_mount_point() {
        let dev = |mount: &str, removable: bool, system: bool| soyuz_fs::udisks::BlockDevice {
            object: format!("/org/freedesktop/UDisks2/block_devices/{}", mount.len()),
            mount_points: if mount.is_empty() { vec![] } else { vec![PathBuf::from(mount)] },
            removable,
            hint_system: system,
            has_filesystem: true,
            ..Default::default()
        };
        let all = [
            dev("/run/media/u/STICK", true, true),
            dev("/", false, true),
            dev("/mnt/data", false, false),
            dev("/run/media/u/Games", false, true),
        ];
        assert!(device_at(&all, Path::new("/run/media/u/STICK")).is_some());
        assert!(device_at(&all, Path::new("/run/media/u/Games")).is_some(), "internal disks mounted for the user");
        assert!(device_at(&all, Path::new("/")).is_none(), "system disks are not offered for unmount");
        assert!(device_at(&all, Path::new("/mnt/data")).is_some());
        assert!(device_at(&all, Path::new("/nowhere")).is_none());
    }

    #[test]
    fn udisks_errors_read_well() {
        let e = std::io::Error::other("org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain: Not authorized");
        assert!(udisks_reason(&e, "ntfs").contains("polkit"));
        let e = std::io::Error::other("org.freedesktop.UDisks2.Error.DeviceBusy: target is busy");
        assert!(udisks_reason(&e, "ext4").contains("in use"));
        let e = std::io::Error::new(std::io::ErrorKind::Unsupported, "no bus");
        assert!(udisks_reason(&e, "").contains("UDisks2"));
        assert_eq!(udisks_reason(&std::io::Error::other("UDisks2: odd"), ""), "odd");
        let e = std::io::Error::other("Error mounting /dev/sdb1: Windows is hibernated, refused to mount.");
        assert!(udisks_reason(&e, "ntfs").contains("Fast Startup"));
        let e = std::io::Error::other("Error unlocking /dev/sda1: Failed to activate device: Operation not permitted");
        assert!(udisks_reason(&e, "BitLocker").contains("passphrase"));
        let e = std::io::Error::other("UDisks2: org.freedesktop.UDisks2.Error.NotAuthorizedDismissed: dismissed");
        assert!(udisks_reason(&e, "ntfs").contains("cancelled"));
    }

    fn cand(label: &str, fs: &str, size: u64) -> soyuz_fs::udisks::Candidate {
        soyuz_fs::udisks::Candidate {
            dev: soyuz_fs::udisks::BlockDevice {
                label: label.into(),
                fs_type: fs.into(),
                size,
                device: "/dev/sdb1".into(),
                drive_model: "Samsung SSD 860 EVO 1TB".into(),
                ..Default::default()
            },
            state: soyuz_fs::udisks::CandidateState::Unmounted,
            container: None,
            container_type: String::new(),
        }
    }

    #[test]
    fn device_labels() {
        assert_eq!(candidate_name(&cand("Games", "ntfs", 1_000_202_043_392)), "Games");
        assert_eq!(candidate_name(&cand("  ", "ntfs", 1_000_202_043_392)), "1 TB NTFS volume");
        assert_eq!(candidate_name(&cand("", "exfat", 0)), "exFAT volume");
        assert_eq!(candidate_badge(&cand("", "BitLocker", 1)), "BitLocker");
        let tip = candidate_tip(&cand("Games", "ntfs", 1_000_202_043_392), "Games");
        for part in ["Games", "/dev/sdb1", "NTFS", "1 TB", "Samsung SSD 860 EVO 1TB", "internal"] {
            assert!(tip.contains(part), "{part} missing in {tip}");
        }
    }

    #[test]
    fn system_mounts_are_not_unmountable() {
        for m in ["/", "/boot", "/boot/efi", "/home", "/var", "/var/lib/x", "/usr", "/mnt"] {
            assert!(is_system_mount(Path::new(m)), "{m}");
        }
        for m in ["/run/media/u/Games", "/mnt/data", "/media/stick", "/home/u/disks/x"] {
            assert!(!is_system_mount(Path::new(m)), "{m}");
        }
    }
}
