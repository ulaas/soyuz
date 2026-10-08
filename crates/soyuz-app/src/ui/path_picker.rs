//! A small in-app file picker (no native dialog / COM): browse folders, pick a file to
//! open or type a file name to save. Listings run on a worker thread, so a slow or dead
//! network folder never blocks the UI. Used by settings export / import.
//!
//! The picker is drawn as its own [`egui::Modal`], so it stacks on top of whatever modal
//! opened it (draw it after that modal in the frame).

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, unbounded};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modal, RichText, ScrollArea, Sense,
    Stroke, TextEdit, Ui, vec2,
};
use soyuz_core::{EntryInfo, FsError};
use soyuz_fs::ListOptions;

use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{icon_button, pill_button, trunc_galley};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickMode {
    /// Choose an existing file.
    Open,
    /// Choose a folder and type a file name (asks before replacing).
    Save,
}

/// Outcome of one frame of [`PathPicker::show`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickResult {
    Pending,
    Cancelled,
    Picked(PathBuf),
}

type Listing = (PathBuf, Result<Vec<EntryInfo>, FsError>);

pub struct PathPicker {
    id: Id,
    title: String,
    mode: PickMode,
    /// Lower-case extensions shown as candidates (`["toml"]`); empty = all files.
    exts: Vec<String>,
    dir: PathBuf,
    path_text: String,
    name: String,
    entries: Vec<EntryInfo>,
    error: Option<String>,
    loading: bool,
    rx: Option<Receiver<Listing>>,
    selected: Option<usize>,
    confirm_replace: Option<PathBuf>,
    places: Vec<(String, &'static str, PathBuf)>,
    focus_name: bool,
}

impl PathPicker {
    pub fn new(title: &str, mode: PickMode, dir: PathBuf, name: &str, exts: &[&str]) -> Self {
        // Home (always first) plus Desktop / Documents / Downloads (matched on the
        // folder name: labels are translated).
        let places = crate::ui::sidebar::standard_places()
            .into_iter()
            .enumerate()
            .filter(|(i, p)| {
                *i == 0
                    || p.path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        ["Desktop", "Documents", "Downloads"].iter().any(|s| s.eq_ignore_ascii_case(n))
                    })
            })
            .map(|(_, p)| (p.label, p.icon, p.path))
            .collect();
        let mut picker = PathPicker {
            id: Id::new(("soyuz.picker", title.to_string())),
            title: title.into(),
            mode,
            exts: exts.iter().map(|e| e.to_ascii_lowercase()).collect(),
            dir: PathBuf::new(),
            path_text: String::new(),
            name: name.into(),
            entries: Vec::new(),
            error: None,
            loading: false,
            rx: None,
            selected: None,
            confirm_replace: None,
            places,
            focus_name: mode == PickMode::Save,
        };
        picker.navigate(dir);
        picker
    }

    /// The folder currently shown.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir.clone();
        self.path_text = soyuz_fs::strip_verbatim(&dir).display().to_string();
        self.entries.clear();
        self.selected = None;
        self.error = None;
        self.loading = true;
        self.confirm_replace = None;
        let (tx, rx) = unbounded();
        self.rx = Some(rx);
        let _ = std::thread::Builder::new().name("soyuz-picker-list".into()).spawn(move || {
            let opts = ListOptions { include_hidden: false, ..ListOptions::default() };
            let _ = tx.send((dir.clone(), soyuz_fs::read_dir(&dir, &opts)));
        });
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        if let Ok((dir, result)) = rx.try_recv() {
            if dir == self.dir {
                self.loading = false;
                match result {
                    Ok(mut entries) => {
                        entries.retain(|e| e.is_dir_like() || self.matches(e));
                        soyuz_core::sort_entries(&mut entries, &soyuz_core::SortSpec::default());
                        self.entries = entries;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            self.rx = None;
        }
    }

    fn matches(&self, e: &EntryInfo) -> bool {
        self.exts.is_empty()
            || Path::new(&e.name)
                .extension()
                .is_some_and(|x| self.exts.iter().any(|w| x.to_string_lossy().eq_ignore_ascii_case(w)))
    }

    /// The path the primary button would return now.
    fn target(&self) -> Option<PathBuf> {
        match self.mode {
            PickMode::Open => {
                let e = self.selected.and_then(|i| self.entries.get(i)).filter(|e| !e.is_dir_like())?;
                Some(e.path.clone())
            }
            PickMode::Save => {
                let name = self.name.trim();
                if name.is_empty() || soyuz_core::validate_name(name).is_err() {
                    return None;
                }
                Some(self.dir.join(name))
            }
        }
    }

    /// Draw the picker (call every frame while it is open).
    pub fn show(&mut self, ctx: &egui::Context) -> PickResult {
        self.poll();
        if self.loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let p = palette(ctx);
        let frame = Frame::new()
            .fill(p.bg_surface3)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(CornerRadius::same(radius::MODAL))
            .inner_margin(Margin::same(18))
            .shadow(egui::epaint::Shadow {
                offset: [0, 12],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
            });
        let mut result = PickResult::Pending;
        let mut go: Option<PathBuf> = None;
        // Above the modal that opened it (both are new in the same frame, so pin the order).
        let area = Modal::default_area(self.id).order(egui::Order::Tooltip);
        let resp = Modal::new(self.id).area(area).frame(frame).show(ctx, |ui| {
            ui.set_width(520.0);
            crate::ui::a11y::container(ui.ctx(), ui.unique_id(), egui::accesskit::Role::Dialog, &self.title);
            ui.horizontal(|ui| {
                let glyph = if self.mode == PickMode::Save { ph::FLOPPY_DISK } else { ph::FOLDER_OPEN };
                ui.label(icons::icon(glyph, 18.0).color(p.accent));
                ui.label(RichText::new(&self.title).font(semibold(16.0)).color(p.text_primary));
            });
            ui.add_space(10.0);
            // Location row: up + editable path.
            ui.horizontal(|ui| {
                let parent = self.dir.parent().map(Path::to_path_buf);
                if icon_button(ui, ph::ARROW_UP, tr!("app-picker-parent"), parent.is_some()).clicked() {
                    go = parent;
                }
                let r = ui.add(
                    TextEdit::singleline(&mut self.path_text)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(8, 6))
                        .hint_text(tr!("app-picker-folder-path")),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    let typed = PathBuf::from(self.path_text.trim().trim_matches('"'));
                    go = Some(typed);
                }
            });
            ui.add_space(6.0);
            // Quick places.
            ui.horizontal_wrapped(|ui| {
                for (label, glyph, path) in &self.places {
                    let on = &self.dir == path;
                    // Glyphs live in the icon font family, the label in the UI font.
                    let color = if on { p.accent } else { p.text_secondary };
                    let mut job = egui::text::LayoutJob::default();
                    job.append(glyph, 0.0, egui::TextFormat::simple(icons::font(13.0), color));
                    job.append(label, 6.0, egui::TextFormat::simple(FontId::proportional(12.5), color));
                    if ui.add(egui::Button::selectable(on, job)).clicked() {
                        go = Some(path.clone());
                    }
                }
            });
            ui.add_space(6.0);
            self.list(ui, &mut go, &mut result);
            ui.add_space(10.0);
            if self.mode == PickMode::Save {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr!("app-picker-file-name")).color(p.text_secondary));
                    let r = ui.add(
                        TextEdit::singleline(&mut self.name)
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(8, 6)),
                    );
                    if self.focus_name {
                        r.request_focus();
                        self.focus_name = false;
                    }
                    if r.changed() {
                        self.confirm_replace = None;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        result = self.confirm();
                    }
                });
            }
            if let Some(path) = &self.confirm_replace {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(tr!("app-picker-replace", name = soyuz_core::entry::display_name(path)))
                        .color(p.warning),
                );
            }
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if self.mode == PickMode::Save { tr!("common-save") } else { tr!("app-picker-open") };
                let ok = self.target().is_some();
                let resp = ui.add_enabled_ui(ok, |ui| pill_button(ui, None, label, true)).inner;
                if resp.clicked() && ok {
                    result = self.confirm();
                }
                ui.add_space(6.0);
                if pill_button(ui, None, tr!("common-cancel"), false).clicked() {
                    result = PickResult::Cancelled;
                }
            });
        });
        if let Some(dir) = go {
            self.navigate(dir);
        }
        if resp.should_close() && result == PickResult::Pending {
            result = PickResult::Cancelled;
        }
        result
    }

    fn confirm(&mut self) -> PickResult {
        let Some(path) = self.target() else { return PickResult::Pending };
        if self.mode == PickMode::Save
            && self.confirm_replace.as_ref() != Some(&path)
            && soyuz_fs::exists_nofollow(&path)
        {
            self.confirm_replace = Some(path);
            return PickResult::Pending;
        }
        PickResult::Picked(path)
    }

    fn list(&mut self, ui: &mut Ui, go: &mut Option<PathBuf>, result: &mut PickResult) {
        let p = palette(ui.ctx());
        let row_h = (ui.spacing().interact_size.y + 4.0).max(28.0);
        Frame::new()
            .fill(p.bg_sunken)
            .stroke(Stroke::new(1.0, p.border_subtle))
            .corner_radius(CornerRadius::same(radius::CARD))
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.set_min_height(280.0);
                ui.set_width(ui.available_width());
                if self.loading {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(tr!("common-loading")).color(p.text_tertiary)));
                    return;
                }
                if let Some(e) = &self.error {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(e).color(p.danger)));
                    return;
                }
                if self.entries.is_empty() {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(tr!("app-picker-no-items")).color(p.text_tertiary))
                    });
                    return;
                }
                ScrollArea::vertical().max_height(280.0).auto_shrink([false, false]).show_rows(
                    ui,
                    row_h,
                    self.entries.len(),
                    |ui, range| {
                        for i in range {
                            let e = &self.entries[i];
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                            let kind = if e.is_dir_like() { "folder" } else { "file" };
                            crate::ui::a11y::toggle(
                                &resp,
                                &tr!("app-picker-row", name = &e.name, kind = kind),
                                self.selected == Some(i),
                            );
                            let sel = self.selected == Some(i);
                            if sel {
                                ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), p.accent_subtle);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), p.bg_surface2);
                            }
                            if resp.has_focus() {
                                crate::ui::widgets::focus_ring(ui, rect.shrink(1.0));
                            }
                            let (glyph, color) = if e.is_dir_like() {
                                (ph::FOLDER, p.glyph_folder)
                            } else {
                                (ph::FILE_TEXT, p.text_secondary)
                            };
                            ui.painter().text(
                                rect.left_center() + vec2(14.0, 0.0),
                                Align2::CENTER_CENTER,
                                glyph,
                                icons::font_fill(15.0),
                                color,
                            );
                            let g = trunc_galley(
                                ui.painter(),
                                &e.name,
                                FontId::proportional(13.0),
                                p.text_primary,
                                rect.width() - 40.0,
                            );
                            ui.painter().galley(rect.left_center() + vec2(30.0, -g.size().y / 2.0), g, p.text_primary);
                            if resp.clicked() {
                                self.selected = Some(i);
                                if !e.is_dir_like() && self.mode == PickMode::Save {
                                    self.name = e.name.clone();
                                    self.confirm_replace = None;
                                }
                            }
                            if resp.double_clicked() {
                                if e.is_dir_like() {
                                    *go = Some(e.path.clone());
                                } else {
                                    self.selected = Some(i);
                                    *result = self.confirm();
                                }
                            }
                        }
                    },
                );
            });
        // Enter on a selected folder opens it (when no text field has the keyboard).
        if ui.memory(|m| m.focused().is_none())
            && ui.input(|i| i.key_pressed(Key::Enter))
            && let Some(e) = self.selected.and_then(|i| self.entries.get(i))
        {
            if e.is_dir_like() {
                *go = Some(e.path.clone());
            } else {
                *result = self.confirm();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(p: &mut PathPicker) {
        for _ in 0..200 {
            p.poll();
            if !p.loading {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("listing never arrived");
    }

    #[test]
    fn lists_folders_and_matching_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("a.toml"), "").unwrap();
        std::fs::write(d.path().join("b.txt"), "").unwrap();
        let mut p = PathPicker::new("Import", PickMode::Open, d.path().to_path_buf(), "", &["toml"]);
        wait(&mut p);
        let names: Vec<_> = p.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "a.toml"]);
        assert_eq!(p.target(), None, "nothing selected");
        p.selected = Some(1);
        assert_eq!(p.target(), Some(d.path().join("a.toml")));
        p.selected = Some(0);
        assert_eq!(p.target(), None, "folders are not files");
    }

    #[test]
    fn save_asks_before_replacing() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("x.toml"), "").unwrap();
        let mut p = PathPicker::new("Export", PickMode::Save, d.path().to_path_buf(), "x.toml", &["toml"]);
        wait(&mut p);
        assert_eq!(p.confirm(), PickResult::Pending, "first press warns");
        assert_eq!(p.confirm(), PickResult::Picked(d.path().join("x.toml")));
        p.name = "new.toml".into();
        assert_eq!(p.confirm(), PickResult::Picked(d.path().join("new.toml")));
        p.name = "bad/name".into();
        assert_eq!(p.target(), None);
    }
}
