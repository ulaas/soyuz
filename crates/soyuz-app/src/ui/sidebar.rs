//! Left sidebar: a mini icon bar on top switches between the places sections and the
//! full-height disk tree (`[sidebar] mode`). The places view has Favorites (user-ordered,
//! drag to reorder, plus config bookmarks), Devices (volumes with free-space bars), Portal
//! (folders pinned by dropping them, see `ui::portal`), Tags, Other (Recent, Timeline,
//! Trash), Network, Cloud folders and Smart folders, in collapsible sections ordered by
//! `[sidebar] sections`.

use std::path::{Path, PathBuf};

use egui::{Align2, CursorIcon, FontId, Frame, Margin, Panel, Rect, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::config::{Bookmark, PortalPin, SidebarMode, SidebarSection};
use soyuz_core::{Location, SizeStyle, format_size};
use soyuz_fs::VolumeInfo;

pub use crate::models::devices::{DeviceCmd, unlock_device};
pub use crate::models::sidebar::{Place, SidebarState, standard_places};

use crate::cmd::Cmd;
use crate::models::devices::{
    self, Work, candidate_badge, candidate_name, candidate_tip, device_at, eject_device, eject_plan, lock_device,
    mount_device, unmount_device,
};
use crate::models::sidebar::{PlaceRole, section_name};
use crate::state::pane::PaneId;
use crate::theme::tokens::size;
use crate::theme::{icons::ph, palette};
use crate::ui::icons::glyph;
use crate::ui::views::computer::{volume_glyph, volume_label};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator, trunc_galley};

/// Glyph of a standard place.
pub fn place_glyph(role: PlaceRole) -> &'static str {
    glyph(crate::models::sidebar::place_icon(role))
}

/// Glyph for a favorite path (standard places get their own icons).
pub fn favorite_glyph(path: &Path) -> &'static str {
    glyph(crate::models::sidebar::favorite_icon(path))
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

pub fn show(root: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    let p = palette(root.ctx());
    devices::start(&crate::host::EguiHost::new(root.ctx()));
    if s.refresh_favorites() {
        root.ctx().request_repaint_after(std::time::Duration::from_millis(50));
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
    crate::ui::a11y::panel_splitter(root.ctx(), "soyuz.sidebar", tr!("app-a11y-resize-sidebar"));
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
    let mut open = s.is_open(SidebarSection::Portal);
    let resp = collapsible_header(ui, section_name(SidebarSection::Portal), &mut open);
    s.set_open(SidebarSection::Portal, open);
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
    let mut open = s.is_open(sec);
    collapsible_header(ui, section_name(sec), &mut open);
    s.set_open(sec, open);
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
            icon_color: crate::ui::looks::folder_color_of(&f.path, &p),
            usage: None,
        }
        .show(ui);
        rects.push(r.rect);
        let r = r.on_hover_text(soyuz_fs::strip_verbatim(&f.path).display().to_string());
        row_clicks(&r, c, &f.path);
        crate::ui::dnd::target_open(ui, &r, &f.path, c.pane, c.cmds);
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
        crate::ui::dnd::target_open(ui, &r, &b.path, c.pane, c.cmds);
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
    let desk_devices = devices::block_devices();
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
        let removable_dev = dev.as_ref().filter(|d| eject_plan(d, &desk_devices).is_some());
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
        crate::ui::dnd::target_open(ui, &r, &v.mount, c.pane, c.cmds);
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
// The refresher thread, the device operations and their results live in
// `models::devices`; here are the rows and menus.

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
    if eject_plan(dev, all).is_some()
        && menu_item(ui, Some(ph::EJECT_SIMPLE), tr!("app-device-eject"), None, true).clicked()
    {
        eject_device(dev, all);
        ui.close();
    }
}

/// `FrontendCmd::UnlockDevice`: ask for the passphrase / recovery key of a locked container.
pub fn unlock_dialog(app: &mut crate::app::SoyuzApp, object: String, name: String, kind: String, pane: PaneId) {
    app.gui.dialogs.push(crate::ui::dialogs::unlock_volume::UnlockVolumeDialog::new(object, name, kind, pane))
}

/// `FrontendCmd::OfferReadOnlyMount`: explain the hibernated NTFS volume and offer a
/// read-only mount.
pub fn offer_read_only(app: &mut crate::app::SoyuzApp, dev: Box<soyuz_fs::udisks::BlockDevice>, name: String) {
    app.gui.dialogs.push(crate::ui::dialogs::ConfirmDialog::new(
        &tr!("app-device-hibernated-title", name = name.clone()),
        tr!("app-device-hibernated-body"),
        tr!("app-device-mount-ro"),
        Cmd::Device(DeviceCmd::Mount { dev, name, read_only: true }),
    ))
}

/// Rows for devices that are not mounted (or locked): click mounts (and opens) or asks
/// for the passphrase. They look unlike mounted volumes: a dimmed outline glyph, a muted
/// label, a filesystem badge and a "Not mounted" / "Locked" line where mounted volumes
/// show their usage bar.
fn unmounted_devices(ui: &mut Ui, s: &mut SidebarState, c: &mut SidebarCtx<'_>) {
    use soyuz_fs::udisks::CandidateState as S;
    let list = devices::candidates(&mut s.devices, c.config);
    if list.is_empty() {
        return;
    }
    let busy = devices::busy();
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
    let marks = devices::gtk_bookmarks(c.config);
    if marks.is_empty() {
        return;
    }
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
            crate::ui::dnd::target_open(ui, &r, path, c.pane, c.cmds);
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
        crate::ui::dnd::target_open(ui, &r, path, c.pane, c.cmds);
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
    crate::ui::dnd::trash_target(ui, &r, c.cmds);
    if r.clicked() {
        c.cmds.push(Cmd::FocusPane(c.pane));
        c.cmds.push(Cmd::Navigate { pane: c.pane, location: Location::Trash, select: None });
    }
}
