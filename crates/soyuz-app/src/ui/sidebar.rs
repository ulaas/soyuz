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
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator};

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

fn candidate_places() -> Vec<Place> {
    let home = soyuz_fs::home();
    let mut out = vec![Place { label: "Home".into(), icon: ph::HOUSE, path: home.clone() }];
    out.extend(STANDARD.iter().map(|&(name, icon)| Place { label: name.into(), icon, path: home.join(name) }));
    out
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
            let places: Vec<Place> = candidate_places()
                .into_iter()
                .enumerate()
                .filter(|(i, p)| *i == 0 || p.path.is_dir())
                .map(|(_, p)| p)
                .collect();
            let _ = PROBED.set(places);
        });
    });
    localized(candidate_places())
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
            // A folder color (B2) tints the favorite's icon.
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
}

fn devices(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Devices) {
        return;
    }
    let active = matches!(c.current, Location::Computer);
    let r =
        NavRow { icon: ph::DESKTOP_TOWER, label: tr!("app-sidebar-this-pc"), active, icon_color: None, usage: None }
            .show(ui);
    if r.clicked() {
        c.cmds.push(Cmd::FocusPane(c.pane));
        c.cmds.push(Cmd::Navigate { pane: c.pane, location: Location::Computer, select: None });
    }
    let vols: Vec<VolumeInfo> = c.volumes.to_vec();
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
        row_clicks(&r, c, &v.mount);
        crate::ops::fileops::dnd::target_open(ui, &r, &v.mount, c.pane, c.cmds);
        let cmds = &mut *c.cmds;
        crate::ui::native::popup::context_menu(&r, |ui| place_menu(ui, cmds, &v.mount, false));
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
