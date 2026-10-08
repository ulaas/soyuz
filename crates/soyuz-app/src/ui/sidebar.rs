//! Left sidebar: Favorites and Devices (M0: static places + baseline volumes).

use std::path::PathBuf;

use egui::{Frame, Margin, Panel, ScrollArea, Ui};
use soyuz_fs::VolumeInfo;

use crate::theme::tokens::size;
use crate::theme::{icons::ph, palette};
use crate::ui::widgets::{NavRow, section_header};

/// A sidebar destination.
#[derive(Clone, Debug)]
pub struct Place {
    pub label: String,
    pub icon: &'static str,
    pub path: PathBuf,
}

/// Standard user folders that exist under the home directory.
pub fn default_places() -> Vec<Place> {
    let home = soyuz_fs::home();
    let mut out = vec![Place { label: "Home".into(), icon: ph::HOUSE, path: home.clone() }];
    for (name, icon) in [
        ("Desktop", ph::DESKTOP),
        ("Documents", ph::FILE_TEXT),
        ("Downloads", ph::DOWNLOAD_SIMPLE),
        ("Pictures", ph::IMAGE),
        ("Music", ph::MUSIC_NOTES),
        ("Videos", ph::FILM_STRIP),
    ] {
        let p = home.join(name);
        if p.is_dir() {
            out.push(Place { label: name.into(), icon, path: p });
        }
    }
    out
}

/// Which sidebar row is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarSel {
    Place(usize),
    Volume(usize),
    Trash,
}

pub struct SidebarState {
    pub places: Vec<Place>,
    pub volumes: Vec<VolumeInfo>,
    pub selected: Option<SidebarSel>,
    pub width: f32,
}

impl SidebarState {
    pub fn new(width: f32) -> Self {
        SidebarState {
            places: default_places(),
            volumes: soyuz_fs::volumes(),
            selected: Some(SidebarSel::Place(0)),
            width,
        }
    }

    /// Label + path of the selection (for the toolbar / central area).
    pub fn selection(&self) -> Option<(String, Option<PathBuf>)> {
        match self.selected? {
            SidebarSel::Place(i) => self.places.get(i).map(|p| (p.label.clone(), Some(p.path.clone()))),
            SidebarSel::Volume(i) => self.volumes.get(i).map(|v| (volume_label(v), Some(v.mount.clone()))),
            SidebarSel::Trash => Some(("Trash".into(), None)),
        }
    }
}

fn volume_label(v: &VolumeInfo) -> String {
    if v.label.is_empty() { v.mount.display().to_string() } else { v.label.clone() }
}

pub fn show(root: &mut Ui, state: &mut SidebarState) {
    let p = palette(root.ctx());
    Panel::left("soyuz.sidebar")
        .resizable(true)
        .default_size(state.width.clamp(size::SIDEBAR_MIN, size::SIDEBAR_MAX))
        .size_range(size::SIDEBAR_MIN..=size::SIDEBAR_MAX)
        .frame(Frame::new().fill(p.bg_surface1).inner_margin(Margin::symmetric(8, 6)))
        .show(root, |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                section_header(ui, "Favorites");
                for (i, place) in state.places.iter().enumerate() {
                    let active = state.selected == Some(SidebarSel::Place(i));
                    let r = NavRow { icon: place.icon, label: &place.label, active, icon_color: None, usage: None }
                        .show(ui);
                    if r.on_hover_text(place.path.display().to_string()).clicked() {
                        state.selected = Some(SidebarSel::Place(i));
                    }
                }

                section_header(ui, "Devices");
                for (i, v) in state.volumes.iter().enumerate() {
                    let active = state.selected == Some(SidebarSel::Volume(i));
                    let usage = (v.total > 0).then(|| 1.0 - v.free as f32 / v.total as f32);
                    let label = volume_label(v);
                    let r = NavRow { icon: ph::HARD_DRIVES, label: &label, active, icon_color: None, usage }.show(ui);
                    let tip = if v.total > 0 {
                        format!(
                            "{}\n{} free of {}",
                            v.mount.display(),
                            soyuz_core::format_size(v.free, soyuz_core::SizeStyle::Binary),
                            soyuz_core::format_size(v.total, soyuz_core::SizeStyle::Binary)
                        )
                    } else {
                        v.mount.display().to_string()
                    };
                    if r.on_hover_text(tip).clicked() {
                        state.selected = Some(SidebarSel::Volume(i));
                    }
                }

                section_header(ui, "Other");
                let active = state.selected == Some(SidebarSel::Trash);
                let r = NavRow { icon: ph::TRASH, label: "Trash", active, icon_color: None, usage: None }.show(ui);
                if r.clicked() {
                    state.selected = Some(SidebarSel::Trash);
                }
            });
        });
}
