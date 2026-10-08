//! Left sidebar: Favorites (user-ordered, drag to reorder, plus config bookmarks),
//! Devices (volumes with free-space bars), Cloud folders and Trash, in collapsible
//! sections ordered by `[sidebar] sections`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::{Align2, CursorIcon, FontId, Frame, Margin, Panel, Rect, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::config::{Bookmark, SidebarSection};
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

/// Home plus the standard user folders that exist.
pub fn standard_places() -> Vec<Place> {
    let home = soyuz_fs::home();
    let mut out = vec![Place { label: "Home".into(), icon: ph::HOUSE, path: home.clone() }];
    for (name, icon) in STANDARD {
        let p = home.join(name);
        if p.is_dir() {
            out.push(Place { label: name.into(), icon, path: p });
        }
    }
    out
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
}

fn section_name(s: SidebarSection) -> &'static str {
    match s {
        SidebarSection::Favorites => "Favorites",
        SidebarSection::Devices => "Devices",
        SidebarSection::Cloud => "Cloud",
        SidebarSection::Tags => "Tags",
        SidebarSection::Network => "Network",
        SidebarSection::Trash => "Other",
        SidebarSection::Tree => "Folders",
    }
}

pub fn show(root: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    let p = palette(root.ctx());
    Panel::left("soyuz.sidebar")
        .resizable(true)
        .default_size(c.width.clamp(size::SIDEBAR_MIN, size::SIDEBAR_MAX))
        .size_range(size::SIDEBAR_MIN..=size::SIDEBAR_MAX)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 8, right: 4, top: 0, bottom: 6 }))
        .show(root, |ui| {
            brand(ui);
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for &sec in c.sections {
                    match sec {
                        SidebarSection::Favorites => favorites(ui, s, c),
                        SidebarSection::Devices => devices(ui, s, c),
                        SidebarSection::Cloud if !c.cloud.is_empty() => cloud(ui, s, c),
                        SidebarSection::Trash => other(ui, s, c),
                        SidebarSection::Tags => {
                            crate::ui::tools::tags_ui::sidebar_section(ui, &mut s.collapsed, c.current, c.pane, c.cmds)
                        }
                        _ => {}
                    }
                }
                crate::ui::search::sidebar_section(ui, c.saved_searches, &mut s.collapsed, c.cmds, c.saved_config);
                ui.add_space(8.0);
            });
        });
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

/// Section header; returns whether the section is open.
fn header(ui: &mut Ui, s: &mut SidebarState, sec: SidebarSection) -> bool {
    let name = section_name(sec);
    let mut open = !s.collapsed.contains(name);
    collapsible_header(ui, name, &mut open);
    if open {
        s.collapsed.remove(name);
    } else {
        s.collapsed.insert(name.to_string());
    }
    open
}

fn is_current(c: &SidebarCtx<'_>, path: &Path) -> bool {
    matches!(c.current, Location::Dir(p) if p == path)
}

fn place_menu(ui: &mut Ui, cmds: &mut Vec<Cmd>, path: &Path, removable: bool) {
    ui.set_width(220.0);
    if menu_item(ui, Some(ph::BROWSERS), "Open in new tab", None, true).clicked() {
        cmds.push(Cmd::OpenInNewTab { location: Location::Dir(path.to_path_buf()), activate: true });
        ui.close();
    }
    if menu_item(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), "Open in other pane", None, true).clicked() {
        cmds.push(Cmd::OpenInOtherPane(Location::Dir(path.to_path_buf())));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROW_SQUARE_UP_RIGHT), "Reveal in system file manager", None, true).clicked() {
        cmds.push(Cmd::RevealInSystem(path.to_path_buf()));
        ui.close();
    }
    if menu_item(ui, Some(ph::CLIPBOARD_TEXT), "Copy path", None, true).clicked() {
        cmds.push(Cmd::CopyText(soyuz_fs::strip_verbatim(path).to_string_lossy().into_owned()));
        ui.close();
    }
    if removable {
        menu_separator(ui);
        if menu_item(ui, Some(ph::STAR), "Remove from favorites", None, true).clicked() {
            cmds.push(Cmd::RemoveFavorite(path.to_path_buf()));
            ui.close();
        }
    } else {
        menu_separator(ui);
        if menu_item(ui, Some(ph::STAR), "Add to favorites", None, true).clicked() {
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
            icon_color: None,
            usage: None,
        }
        .show(ui);
        rects.push(r.rect);
        let r = r.on_hover_text(soyuz_fs::strip_verbatim(&f.path).display().to_string());
        row_clicks(&r, c, &f.path);
        crate::ops::fileops::dnd::target(ui, &r, &f.path, c.cmds);
        if r.drag_started() {
            s.drag = Some(i);
        }
        let cmds = &mut *c.cmds;
        r.context_menu(|ui| place_menu(ui, cmds, &f.path, true));
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
        crate::ops::fileops::dnd::target(ui, &r, &b.path, c.cmds);
        let cmds = &mut *c.cmds;
        r.context_menu(|ui| place_menu(ui, cmds, &b.path, false));
    }
}

fn devices(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Devices) {
        return;
    }
    let active = matches!(c.current, Location::Computer);
    let r = NavRow { icon: ph::DESKTOP_TOWER, label: "This PC", active, icon_color: None, usage: None }.show(ui);
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
            format!(
                "{}\n{} free of {}",
                v.mount.display(),
                format_size(v.free, SizeStyle::Decimal),
                format_size(v.total, SizeStyle::Decimal)
            )
        } else {
            v.mount.display().to_string()
        };
        let r = r.on_hover_text(tip);
        row_clicks(&r, c, &v.mount);
        crate::ops::fileops::dnd::target(ui, &r, &v.mount, c.cmds);
        let cmds = &mut *c.cmds;
        r.context_menu(|ui| place_menu(ui, cmds, &v.mount, false));
    }
    if vols.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let p = palette(ui.ctx());
        ui.painter().text(
            pos2(rect.left() + 34.0, rect.center().y),
            Align2::LEFT_CENTER,
            "Scanning…",
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
        crate::ops::fileops::dnd::target(ui, &r, path, c.cmds);
        let cmds = &mut *c.cmds;
        r.context_menu(|ui| place_menu(ui, cmds, path, false));
    }
}

fn other(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    if !header(ui, s, SidebarSection::Trash) {
        return;
    }
    crate::ui::tools::tags_ui::recent_row(ui, c.current, c.pane, c.cmds);
    let active = matches!(c.current, Location::Trash);
    let r = NavRow { icon: ph::TRASH, label: "Trash", active, icon_color: None, usage: None }.show(ui);
    crate::ops::fileops::dnd::trash_target(ui, &r, c.cmds);
    if r.clicked() {
        c.cmds.push(Cmd::FocusPane(c.pane));
        c.cmds.push(Cmd::Navigate { pane: c.pane, location: Location::Trash, select: None });
    }
}
