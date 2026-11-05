//! A small in-app file picker (no native dialog / COM): browse folders, pick a file to
//! open or type a file name to save. Listings run on a worker thread
//! ([`FileBrowser`]), so a slow or dead network folder never blocks the UI. Used by
//! settings export / import and, on programs ([`PathPicker::programs`]), by Open with….
//!
//! Keyboard: when picking a file, typing filters the list; ↑ / ↓ choose, Enter opens the
//! folder or picks the file, Backspace on an empty filter goes up, Esc cancels.
//!
//! The picker is drawn as its own [`egui::Modal`], so it stacks on top of whatever modal
//! opened it (draw it after that modal in the frame).

use std::path::{Path, PathBuf};

use egui::{
    Align, Align2, Frame, Id, Key, Layout, Margin, Modal, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, vec2,
};

use crate::models::file_browser::{BrowseFilter, FileBrowser};
use crate::theme::font::semibold;
use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::widgets::{Btn, chip, fit, icon_button, pill_button};

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

/// The folders above the list: filled once, off the UI thread, for programs.
type PlacesSlot = std::sync::Arc<parking_lot::Mutex<Option<Vec<(String, PathBuf)>>>>;

pub struct PathPicker {
    id: Id,
    title: String,
    mode: PickMode,
    browser: FileBrowser,
    path_text: String,
    name: String,
    /// Typed text narrowing the list (mirrors the browser's query).
    query: String,
    selected: Option<usize>,
    /// The next frame scrolls the selected row into view.
    scroll_to_selected: bool,
    confirm_replace: Option<PathBuf>,
    places: Vec<(String, &'static str, PathBuf)>,
    /// The program folders on their way ([`PathPicker::programs`]).
    places_slot: Option<PlacesSlot>,
    /// Picking programs: rows and places say so, and "All files" is offered.
    programs: bool,
    /// The name of the row last picked.
    picked_name: Option<String>,
    /// Saving with one extension on offer: a name typed without one gets it.
    save_ext: Option<String>,
    focus_name: bool,
    focus_query: bool,
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
            .map(|(_, p)| (p.label, crate::ui::sidebar::place_glyph(p.role), p.path))
            .collect();
        let filter = if exts.is_empty() {
            BrowseFilter::All
        } else {
            BrowseFilter::Exts(exts.iter().map(|e| e.to_ascii_lowercase()).collect())
        };
        let mut picker = PathPicker {
            id: Id::unique(("soyuz.picker", title.to_string())),
            title: title.into(),
            mode,
            browser: FileBrowser::polled(filter),
            path_text: String::new(),
            name: name.into(),
            query: String::new(),
            selected: None,
            scroll_to_selected: false,
            confirm_replace: None,
            places,
            places_slot: None,
            programs: false,
            picked_name: None,
            save_ext: match exts {
                [ext] if mode == PickMode::Save => Some(ext.trim_start_matches('.').to_string()),
                _ => None,
            },
            focus_name: mode == PickMode::Save,
            focus_query: mode == PickMode::Open,
        };
        picker.navigate(dir);
        picker
    }

    /// A picker on programs (Windows: shortcuts to programs count), starting in `dir`,
    /// with the usual program folders as its places.
    pub fn programs(title: &str, dir: PathBuf) -> Self {
        let mut picker = PathPicker::new(title, PickMode::Open, dir.clone(), "", &[]);
        picker.browser = FileBrowser::polled(BrowseFilter::Programs);
        picker.programs = true;
        picker.navigate(dir);
        let slot = PlacesSlot::default();
        let filled = slot.clone();
        let _ = std::thread::Builder::new().name("soyuz-picker-places".into()).spawn(move || {
            *filled.lock() = Some(crate::models::requests::open_with::existing_program_places());
        });
        picker.places_slot = Some(slot);
        picker
    }

    /// The folder currently shown.
    pub fn dir(&self) -> &Path {
        self.browser.dir()
    }

    /// The name shown for the row last picked: a shortcut's name, without the extension.
    pub fn picked_name(&self) -> Option<String> {
        self.picked_name.clone()
    }

    fn navigate(&mut self, dir: PathBuf) {
        self.path_text = soyuz_fs::strip_verbatim(&dir).display().to_string();
        self.browser.navigate(dir);
        self.query.clear();
        self.selected = None;
        self.confirm_replace = None;
        self.focus_query = self.mode == PickMode::Open;
    }

    fn poll(&mut self) {
        self.browser.poll();
        let places = self.places_slot.as_ref().and_then(|slot| slot.lock().take());
        if let Some(places) = places {
            self.places_slot = None;
            self.places.retain(|(_, _, path)| *path == soyuz_fs::home());
            self.places.extend(places.into_iter().map(|(label, path)| (label, ph::APP_WINDOW, path)));
        }
    }

    /// Move the selection by `delta` rows.
    fn step(&mut self, delta: isize) {
        let n = self.browser.len();
        if n == 0 {
            return;
        }
        let next = match self.selected {
            None if delta < 0 => n - 1,
            None => 0,
            Some(i) => i.saturating_add_signed(delta).min(n - 1),
        };
        self.selected = Some(next);
        self.scroll_to_selected = true;
    }

    /// The keys of the list (when picking a file; a text field has no use for them).
    fn list_keys(&mut self, ui: &Ui, go: &mut Option<PathBuf>) {
        use egui::Modifiers as M;
        let (up, down, page_up, page_down) = ui.input_mut(|i| {
            (
                i.consume_key(M::NONE, Key::ArrowUp),
                i.consume_key(M::NONE, Key::ArrowDown),
                i.consume_key(M::NONE, Key::PageUp),
                i.consume_key(M::NONE, Key::PageDown),
            )
        });
        for (pressed, delta) in [(up, -1), (down, 1), (page_up, -10), (page_down, 10)] {
            if pressed {
                self.step(delta);
            }
        }
        // Runs before the filter field: the Backspace that empties it stays in the folder.
        let focus = ui.memory(|m| m.focused());
        if self.query.is_empty()
            && (focus.is_none() || focus == Some(self.id.with("query")))
            && ui.input(|i| i.key_pressed(Key::Backspace))
        {
            *go = self.browser.parent();
        }
    }

    /// The name a save will really use when it differs from the typed one: the only
    /// extension on offer is added to a name without any.
    fn final_name(&self) -> Option<String> {
        let name = self.name.trim();
        let ext = self.save_ext.as_deref()?;
        let bare = Path::new(name).extension().is_none() && !name.eq_ignore_ascii_case(&format!(".{ext}"));
        (bare && !name.is_empty()).then(|| format!("{}.{ext}", name.trim_end_matches('.')))
    }

    /// The path the primary button would return now.
    fn target(&self) -> Option<PathBuf> {
        match self.mode {
            PickMode::Open => {
                let e = self.selected.and_then(|i| self.browser.row(i)).filter(|e| !e.is_dir)?;
                Some(e.path.clone())
            }
            PickMode::Save => {
                let name = self.name.trim();
                if name.is_empty() || soyuz_core::validate_name(name).is_err() {
                    return None;
                }
                Some(self.browser.dir().join(self.final_name().unwrap_or_else(|| name.to_string())))
            }
        }
    }

    /// Draw the picker (call every frame while it is open).
    pub fn show(&mut self, ctx: &egui::Context) -> PickResult {
        self.poll();
        if self.browser.loading() || self.places_slot.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let p = palette(ctx);
        let frame = Frame::new()
            .fill(p.bg_surface3)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(crate::theme::tokens::radius::same(radius::MODAL))
            .inner_margin(Margin::same(18))
            .shadow(p.shadow(crate::theme::tokens::shadow::WINDOW));
        let mut result = PickResult::Pending;
        let mut go: Option<PathBuf> = None;
        let title = self.title.clone();
        let picker_id = self.id;
        let glyph = if self.mode == PickMode::Save { ph::FLOPPY_DISK } else { ph::FOLDER_OPEN };
        let mut body = |ui: &mut Ui, native: bool| {
            if !native {
                crate::ui::a11y::container(ui.ctx(), ui.unique_id(), egui::accesskit::Role::Dialog, &self.title);
                ui.horizontal(|ui| {
                    ui.label(icons::icon(glyph, 18.0).color(p.accent));
                    ui.label(RichText::new(&self.title).font(semibold(16.0)).color(p.text_primary));
                });
                ui.add_space(10.0);
            }
            // Location row: up + editable path.
            ui.horizontal(|ui| {
                let parent = self.browser.parent();
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
                    let on = self.browser.dir() == path;
                    if chip(ui, Some(*glyph), label, on).clicked() {
                        go = Some(path.clone());
                    }
                }
            });
            ui.add_space(6.0);
            if self.mode == PickMode::Open {
                self.list_keys(ui, &mut go);
                ui.horizontal(|ui| {
                    if self.programs {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut all = self.browser.show_all();
                            if ui.checkbox(&mut all, tr!("dialogs-openwith-all-files")).changed() {
                                self.browser.set_show_all(all);
                                self.selected = None;
                            }
                            self.query_field(ui);
                        });
                    } else {
                        self.query_field(ui);
                    }
                });
                ui.add_space(6.0);
            }
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
                if let Some(name) = self.final_name() {
                    ui.add_space(4.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.label(
                            RichText::new(tr!("app-picker-saves-as", name = name))
                                .size(crate::theme::font::px(11.5))
                                .color(p.text_tertiary),
                        );
                    });
                }
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
                let resp = ui.add_enabled_ui(ok, |ui| pill_button(ui, None, label, Btn::Primary)).inner;
                if resp.clicked() && ok {
                    result = self.confirm();
                }
                ui.add_space(6.0);
                if pill_button(ui, None, tr!("common-cancel"), Btn::Secondary).clicked() {
                    result = PickResult::Cancelled;
                }
            });
        };
        let close = if crate::ui::native::active(ctx) {
            let mut w = crate::ui::native::dialog::DialogWindow::new(
                picker_id.with("native"),
                Id::unique("soyuz.path_picker"),
                &title,
            );
            w.icon = Some(glyph);
            w.width = 520.0;
            w.margin = 18.0;
            w.show(ctx, |ui| body(ui, true)).close
        } else {
            // Above the modal that opened it (both are new in the same frame, so pin the order).
            let area = Modal::default_area(picker_id).order(egui::Order::Tooltip);
            Modal::new(picker_id)
                .area(area)
                .frame(frame)
                .show(ctx, |ui| {
                    ui.set_width(520.0);
                    body(ui, false)
                })
                .should_close()
        };
        if let Some(dir) = go {
            self.navigate(dir);
        }
        if close && result == PickResult::Pending {
            result = PickResult::Cancelled;
        }
        result
    }

    /// The filter field: it has the keyboard from the start, and selects the first match.
    fn query_field(&mut self, ui: &mut Ui) {
        let r = ui.add(
            TextEdit::singleline(&mut self.query)
                .id(self.id.with("query"))
                .hint_text(tr!("dialogs-openwith-filter"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(8, 6)),
        );
        if self.focus_query {
            self.focus_query = false;
            r.request_focus();
        }
        if r.changed() {
            self.browser.set_query(&self.query);
            self.selected = (!self.query.trim().is_empty() && !self.browser.is_empty()).then_some(0);
            self.scroll_to_selected = true;
        }
    }

    fn confirm(&mut self) -> PickResult {
        let Some(path) = self.target() else { return PickResult::Pending };
        if let Some(row) = self.selected.and_then(|i| self.browser.row(i)) {
            let name = Path::new(&row.name).file_stem().map(|n| n.to_string_lossy().into_owned());
            self.picked_name = name;
        }
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
            .corner_radius(crate::theme::tokens::radius::same(radius::CARD))
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.set_min_height(280.0);
                ui.set_width(ui.available_width());
                if self.browser.loading() {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(tr!("common-loading")).color(p.text_tertiary)));
                    return;
                }
                if let Some(e) = self.browser.error() {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(e).color(p.danger)));
                    return;
                }
                if self.browser.is_empty() {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(tr!("app-picker-no-items")).color(p.text_tertiary))
                    });
                    return;
                }
                let mut area = ScrollArea::vertical().max_height(280.0).auto_shrink([false, false]);
                if std::mem::take(&mut self.scroll_to_selected)
                    && let Some(i) = self.selected
                {
                    // Keeps the row near the middle (only rows in view exist to scroll to).
                    let pitch = row_h + ui.spacing().item_spacing.y;
                    area = area.vertical_scroll_offset((i as f32 * pitch - 126.0).max(0.0));
                }
                let row_count = self.browser.len();
                area.show_rows(ui, row_h, row_count, |ui, range| {
                    for i in range {
                        let Some(e) = self.browser.row(i) else { continue };
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                        let kind = if e.is_dir { "folder" } else { "file" };
                        crate::ui::a11y::toggle(
                            &resp,
                            &tr!("app-picker-row", name = &e.name, kind = kind),
                            self.selected == Some(i),
                        );
                        let sel = self.selected == Some(i);
                        if sel {
                            ui.painter().rect_filled(
                                rect,
                                crate::theme::tokens::radius::same(radius::ROW),
                                p.accent_subtle,
                            );
                        } else if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                crate::theme::tokens::radius::same(radius::ROW),
                                p.bg_surface2,
                            );
                        }
                        if resp.has_focus() {
                            crate::ui::widgets::focus_ring(ui, rect.shrink(1.0));
                        }
                        let resp = if self.programs && !e.is_dir {
                            resp.on_hover_text(e.path.display().to_string())
                        } else {
                            resp
                        };
                        let (glyph, color) = if e.is_dir {
                            (ph::FOLDER, p.glyph_folder)
                        } else if self.programs {
                            (ph::APP_WINDOW, p.text_secondary)
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
                        let g = fit::entry_name(
                            ui.painter(),
                            &e.name,
                            e.is_dir,
                            crate::theme::font::ui(13.0),
                            p.text_primary,
                            rect.width() - 40.0,
                        )
                        .galley;
                        ui.painter().galley(rect.left_center() + vec2(30.0, -g.size().y / 2.0), g, p.text_primary);
                        if resp.clicked() {
                            self.selected = Some(i);
                            if !e.is_dir && self.mode == PickMode::Save {
                                self.name = e.name.clone();
                                self.confirm_replace = None;
                            }
                        }
                        if resp.double_clicked() {
                            if e.is_dir {
                                *go = Some(e.path.clone());
                            } else {
                                self.selected = Some(i);
                                *result = self.confirm();
                            }
                        }
                    }
                });
            });
        // Enter on the selected row (a text field gives up the keyboard on Enter). The
        // path and name fields already acted on theirs.
        if go.is_none()
            && *result == PickResult::Pending
            && ui.memory(|m| m.focused().is_none())
            && ui.input(|i| i.key_pressed(Key::Enter))
            && let Some(e) = self.selected.and_then(|i| self.browser.row(i))
        {
            if e.is_dir {
                *go = Some(e.path.clone());
            } else {
                *result = self.confirm();
            }
        } else if self.mode == PickMode::Open && go.is_none() && ui.input(|i| i.key_pressed(Key::Enter)) {
            // Nothing to act on: the filter keeps the keyboard.
            self.focus_query = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(p: &mut PathPicker) {
        for _ in 0..200 {
            p.poll();
            if !p.browser.loading() {
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
        let names: Vec<_> = p.browser.rows().map(|e| e.name.as_str()).collect();
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

    #[test]
    fn saving_adds_the_only_extension_on_offer() {
        let d = tempfile::tempdir().unwrap();
        let mut p = PathPicker::new("Export", PickMode::Save, d.path().to_path_buf(), "settings", &["toml"]);
        wait(&mut p);
        assert_eq!(p.final_name().as_deref(), Some("settings.toml"));
        assert_eq!(p.target(), Some(d.path().join("settings.toml")));
        p.name = "settings.toml".into();
        assert_eq!(p.final_name(), None);
        p.name = "settings.bak".into();
        assert_eq!(p.final_name(), None, "another extension is the user's choice");
        assert_eq!(p.target(), Some(d.path().join("settings.bak")));
        p.name = ".toml".into();
        assert_eq!(p.final_name(), None);
        let any = PathPicker::new("Export", PickMode::Save, d.path().to_path_buf(), "x", &["toml", "json"]);
        assert_eq!(any.final_name(), None, "two extensions on offer: no guess");
    }

    #[test]
    fn the_keyboard_walks_the_filtered_rows() {
        let d = tempfile::tempdir().unwrap();
        for f in ["alpha.toml", "beta.toml", "gamma.toml"] {
            std::fs::write(d.path().join(f), "").unwrap();
        }
        let mut p = PathPicker::new("Import", PickMode::Open, d.path().to_path_buf(), "", &["toml"]);
        wait(&mut p);
        p.step(1);
        assert_eq!(p.target(), Some(d.path().join("alpha.toml")));
        p.step(10);
        assert_eq!(p.target(), Some(d.path().join("gamma.toml")), "stops at the end");
        p.step(-1);
        assert_eq!(p.target(), Some(d.path().join("beta.toml")));
        p.browser.set_query("gam");
        p.selected = Some(0);
        assert_eq!(p.target(), Some(d.path().join("gamma.toml")));
        assert_eq!(p.confirm(), PickResult::Picked(d.path().join("gamma.toml")));
        assert_eq!(p.picked_name().as_deref(), Some("gamma"));
    }
}
