//! "Create link" dialog (symbolic link / hard link / junction) with a built-in target
//! picker: a small folder browser under the target field (no native file dialog, which
//! would need shell APIs), live validation of the target (exists, file vs folder, same
//! volume for hard links) and an optional relative target for symbolic links. The link
//! is created by a `Create` job (undoable).

use std::path::{Component, Path, PathBuf};

use egui::{Align, CornerRadius, FontId, Key, Layout, Margin, RichText, Sense, Stroke, TextEdit, Ui, vec2};
use soyuz_core::EntryInfo;
use soyuz_fs::{LinkKind, ListOptions};
use soyuz_jobs::CreateKind;

use super::conflict::{Btn, button};
use super::new_item::LinkChoice;
use super::properties::{Bg, MetaCmd};
use super::properties_widgets::{badge, caption, check, small_icon_button};
use super::{Dialog, DialogCtx};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::toasts::Toast;

pub fn open(app: &mut SoyuzApp, kind: LinkChoice) {
    let pane = app.ws.pane();
    let Some(dir) = pane.location.as_fs_path().map(Path::to_path_buf) else {
        app.ui.toasts.push(Toast::info("Links can only be created in a folder on disk"));
        return;
    };
    if !matches!(pane.location, soyuz_core::Location::Dir(_)) {
        app.ui.toasts.push(Toast::info("Links can only be created in a folder on disk"));
        return;
    }
    let target = if pane.selection.is_empty() { None } else { pane.selected_entries().first().map(|e| e.path.clone()) };
    let names: Vec<String> = pane.entries.iter().map(|e| e.name.clone()).collect();
    app.ui.dialogs.push(LinkDialog::new(dir, target, kind, names));
}

/// `target` relative to `base` (`None` when they share no root, e.g. other drive).
pub fn relative_to(target: &Path, base: &Path) -> Option<PathBuf> {
    let t: Vec<Component<'_>> = target.components().collect();
    let b: Vec<Component<'_>> = base.components().collect();
    let eq = |x: &Component<'_>, y: &Component<'_>| {
        if cfg!(windows) {
            x.as_os_str().to_string_lossy().eq_ignore_ascii_case(&y.as_os_str().to_string_lossy())
        } else {
            x == y
        }
    };
    if t.is_empty() || b.is_empty() || !eq(&t[0], &b[0]) {
        return None;
    }
    let common = t.iter().zip(&b).take_while(|(x, y)| eq(x, y)).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    Some(out)
}

/// Background listing of the picker folder.
type Listing = Bg<Result<Vec<EntryInfo>, String>>;

/// What the typed target currently refers to.
#[derive(Clone, Debug, PartialEq)]
enum TargetState {
    Empty,
    Missing,
    File,
    Dir,
}

pub struct LinkDialog {
    dir: PathBuf,
    target: String,
    name: String,
    kind: LinkChoice,
    existing: Vec<String>,
    name_touched: bool,
    relative: bool,
    focus: bool,
    /// Folder shown in the picker and its listing.
    browse: Option<PathBuf>,
    listing: Option<(PathBuf, Listing)>,
    picker_open: bool,
}

impl LinkDialog {
    pub fn new(dir: PathBuf, target: Option<PathBuf>, kind: LinkChoice, existing: Vec<String>) -> Self {
        let target_s = target.as_ref().map(|t| soyuz_fs::strip_verbatim(t).display().to_string()).unwrap_or_default();
        let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
        let name = target.as_ref().map(|t| crate::ops::fileops::names::link_name(t, &refs)).unwrap_or_default();
        LinkDialog {
            browse: Some(
                target.as_ref().and_then(|t| t.parent().map(Path::to_path_buf)).unwrap_or_else(|| dir.clone()),
            ),
            dir,
            target: target_s,
            name,
            kind,
            existing,
            name_touched: false,
            relative: false,
            focus: true,
            listing: None,
            picker_open: target.is_none(),
        }
    }

    fn target_path(&self) -> PathBuf {
        let t = PathBuf::from(soyuz_fs::expand_env(self.target.trim()));
        if t.is_relative() { self.dir.join(t) } else { t }
    }

    fn state(&self) -> TargetState {
        if self.target.trim().is_empty() {
            return TargetState::Empty;
        }
        let p = self.target_path();
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() => TargetState::Dir,
            Ok(_) => TargetState::File,
            Err(_) => TargetState::Missing,
        }
    }

    fn set_target(&mut self, p: &Path) {
        self.target = soyuz_fs::strip_verbatim(p).display().to_string();
        if !self.name_touched {
            let refs: Vec<&str> = self.existing.iter().map(String::as_str).collect();
            self.name = crate::ops::fileops::names::link_name(p, &refs);
        }
    }

    fn picker(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let Some(dir) = self.browse.clone() else { return };
        if self.listing.as_ref().is_none_or(|(d, _)| *d != dir) {
            let d2 = dir.clone();
            let bg = Bg::spawn(ui.ctx(), move || {
                let opts = ListOptions { include_hidden: false, ..ListOptions::default() };
                soyuz_fs::read_dir(&d2, &opts).map_err(|e| e.to_string()).map(|mut v| {
                    v.sort_by(|a, b| {
                        b.is_dir_like()
                            .cmp(&a.is_dir_like())
                            .then_with(|| soyuz_core::natural_cmp(&a.name, &b.name, true))
                    });
                    v
                })
            });
            self.listing = Some((dir.clone(), bg));
        }
        egui::Frame::new()
            .fill(p.bg_sunken)
            .stroke(Stroke::new(1.0, p.border_subtle))
            .corner_radius(CornerRadius::same(radius::CARD))
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let mut nav: Option<PathBuf> = None;
                let mut pick: Option<PathBuf> = None;
                ui.horizontal(|ui| {
                    if small_icon_button(ui, ph::ARROW_UP, "Parent folder").clicked()
                        && let Some(parent) = dir.parent()
                    {
                        nav = Some(parent.to_path_buf());
                    }
                    if small_icon_button(ui, ph::HOUSE, "Home").clicked() {
                        nav = Some(soyuz_fs::home());
                    }
                    let shown = soyuz_fs::strip_verbatim(&dir).display().to_string();
                    let w = (ui.available_width() - 112.0).max(40.0);
                    ui.allocate_ui(vec2(w, 20.0), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&shown).font(FontId::proportional(11.5)).color(p.text_secondary),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&shown);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Use this folder").clicked() {
                            pick = Some(dir.clone());
                        }
                    });
                });
                ui.add_space(2.0);
                if let Some((_, bg)) = &self.listing {
                    bg.with(|l| match l {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Loading\u{2026}").color(p.text_tertiary));
                            });
                        }
                        Some(Err(e)) => {
                            ui.label(RichText::new(e).color(p.danger));
                        }
                        Some(Ok(entries)) => {
                            egui::ScrollArea::vertical()
                                .id_salt("link_picker")
                                .max_height(170.0)
                                .auto_shrink([false, true])
                                .show_rows(ui, 24.0, entries.len(), |ui, range| {
                                    for e in &entries[range] {
                                        let (r, resp) =
                                            ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                                        let selected = self.target_path() == e.path;
                                        if selected {
                                            ui.painter().rect_filled(
                                                r,
                                                CornerRadius::same(radius::ROW),
                                                p.accent_subtle,
                                            );
                                        } else if resp.hovered() {
                                            ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2);
                                        }
                                        ui.painter().text(
                                            egui::pos2(r.left() + 14.0, r.center().y),
                                            egui::Align2::CENTER_CENTER,
                                            crate::ui::views::icons::glyph(e),
                                            icons::font(14.0),
                                            crate::ui::views::icons::color(e, &p),
                                        );
                                        let g = crate::ui::widgets::trunc_galley(
                                            ui.painter(),
                                            &e.name,
                                            FontId::proportional(12.5),
                                            p.text_primary,
                                            r.width() - 40.0,
                                        );
                                        ui.painter().galley(
                                            egui::pos2(r.left() + 30.0, r.center().y - g.size().y / 2.0),
                                            g,
                                            p.text_primary,
                                        );
                                        if resp.clicked() {
                                            pick = Some(e.path.clone());
                                        }
                                        if resp.double_clicked() && e.is_dir_like() {
                                            nav = Some(e.path.clone());
                                        }
                                    }
                                });
                            if entries.is_empty() {
                                ui.label(RichText::new("Empty folder").color(p.text_tertiary));
                            }
                        }
                    });
                }
                if let Some(pk) = pick {
                    self.set_target(&pk);
                }
                if let Some(n) = nav {
                    self.browse = Some(n);
                }
            });
        ui.label(
            RichText::new("Click to choose, double-click a folder to open it.")
                .font(FontId::proportional(10.5))
                .color(p.text_tertiary),
        );
    }
}

impl Dialog for LinkDialog {
    fn title(&self) -> &str {
        "Create link"
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::LINK)
    }

    fn width(&self) -> f32 {
        540.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            for (k, label, glyph) in [
                (LinkChoice::Symlink, "Symbolic link", ph::LINK),
                (LinkChoice::Hardlink, "Hard link", ph::LINK_SIMPLE),
                (LinkChoice::Junction, "Junction", ph::TREE_STRUCTURE),
            ] {
                if k == LinkChoice::Junction && !cfg!(windows) {
                    continue;
                }
                let on = self.kind == k;
                if ui.selectable_label(on, format!("{glyph}  {label}")).clicked() {
                    self.kind = k;
                }
            }
        });
        let hint = match self.kind {
            LinkChoice::Symlink => {
                "Points to a file or folder by path and may cross volumes. On Windows this needs Developer Mode or elevation."
            }
            LinkChoice::Hardlink => "A second name for the same file data. Files only, on the same volume.",
            LinkChoice::Junction => "A Windows folder link that needs no special rights. Folders only, local volumes.",
        };
        ui.label(RichText::new(hint).font(FontId::proportional(11.5)).color(p.text_tertiary));
        caption(ui, "Target");
        ui.horizontal(|ui| {
            let r = ui.add(
                TextEdit::singleline(&mut self.target)
                    .hint_text("Path of the file or folder to link to")
                    .desired_width(ui.available_width() - 30.0)
                    .margin(Margin::symmetric(10, 6)),
            );
            if self.focus {
                r.request_focus();
                self.focus = false;
            }
            if r.changed() {
                let tp = self.target_path();
                if !self.name_touched {
                    let refs: Vec<&str> = self.existing.iter().map(String::as_str).collect();
                    self.name = crate::ops::fileops::names::link_name(&tp, &refs);
                }
                if tp.is_dir() {
                    self.browse = Some(tp);
                } else if let Some(parent) = tp.parent().filter(|pp| pp.is_dir()) {
                    self.browse = Some(parent.to_path_buf());
                }
            }
            let glyph = if self.picker_open { ph::CARET_UP } else { ph::FOLDER_OPEN };
            if small_icon_button(ui, glyph, "Browse").clicked() {
                self.picker_open = !self.picker_open;
            }
        });
        if self.picker_open {
            ui.add_space(4.0);
            self.picker(ui);
        }
        let state = self.state();
        ui.add_space(4.0);
        ui.horizontal(|ui| match state {
            TargetState::Empty => {}
            TargetState::Missing => {
                badge(ui, Some(ph::WARNING), "Target does not exist", p.warning);
            }
            TargetState::File => {
                badge(ui, Some(ph::FILE), "File", p.success);
            }
            TargetState::Dir => {
                badge(ui, Some(ph::FOLDER), "Folder", p.success);
            }
        });
        // Always laid out (disabled for other kinds) so the dialog doesn't jump.
        ui.add_enabled_ui(self.kind == LinkChoice::Symlink, |ui| {
            check(ui, &mut self.relative, "Store a relative path (the link keeps working when both move together)");
        });
        caption(ui, "Link name");
        let n = ui.add(
            TextEdit::singleline(&mut self.name)
                .hint_text("Name of the new link")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        if n.changed() {
            self.name_touched = true;
        }
        ui.label(
            RichText::new(format!("Created in {}", soyuz_fs::strip_verbatim(&self.dir).display()))
                .font(FontId::proportional(11.0))
                .color(p.text_tertiary),
        );

        let name = self.name.trim().to_string();
        let target = self.target_path();
        let err: Option<String> = if self.target.trim().is_empty() {
            Some("Choose a target.".into())
        } else if let Err(e) = soyuz_core::validate_name(&name) {
            Some(format!("Invalid name: {e}"))
        } else if self.existing.iter().any(|e| e.eq_ignore_ascii_case(&name)) {
            Some(format!("\u{201c}{name}\u{201d} already exists here."))
        } else {
            match (self.kind, &state) {
                (LinkChoice::Hardlink, TargetState::Dir) => Some("Hard links can only point to files.".into()),
                (LinkChoice::Hardlink, TargetState::Missing) => Some("A hard link needs an existing file.".into()),
                (LinkChoice::Hardlink, _)
                    if soyuz_fs::device_of(&target).ok() != soyuz_fs::device_of(&self.dir).ok() =>
                {
                    Some("Hard links must be on the same volume as the target.".into())
                }
                (LinkChoice::Junction, TargetState::File) => Some("Junctions can only point to folders.".into()),
                (LinkChoice::Junction, TargetState::Missing) => Some("A junction needs an existing folder.".into()),
                _ => None,
            }
        };
        if let Some(e) = &err
            && !self.target.trim().is_empty()
        {
            ui.add_space(4.0);
            ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.danger));
        }
        ui.add_space(14.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let enabled = err.is_none();
            ok = ui.add_enabled_ui(enabled, |ui| button(ui, "Create", Btn::Primary)).inner.clicked();
            cancel = button(ui, "Cancel", Btn::Secondary).clicked();
        });
        let text_focus = n.has_focus();
        if text_focus && ui.input(|i| i.key_pressed(Key::Enter)) {
            ok = true;
        }
        if ok && err.is_none() {
            let path = self.dir.join(&name);
            let kind = match self.kind {
                LinkChoice::Symlink => {
                    let lk = if state == TargetState::Dir { LinkKind::Dir } else { LinkKind::File };
                    let stored = if self.relative {
                        relative_to(&target, &self.dir).unwrap_or(target.clone())
                    } else {
                        target.clone()
                    };
                    CreateKind::Symlink { target: stored, kind: lk }
                }
                LinkChoice::Hardlink => CreateKind::Hardlink { src: target.clone() },
                LinkChoice::Junction => CreateKind::Junction { target: target.clone() },
            };
            ctx.cmds.push(Cmd::Meta(MetaCmd::Create { dir: self.dir.clone(), path, kind }));
            return false;
        }
        !cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let (t, b) = if cfg!(windows) {
            (PathBuf::from(r"C:\data\photos\a.jpg"), PathBuf::from(r"C:\data\links"))
        } else {
            (PathBuf::from("/data/photos/a.jpg"), PathBuf::from("/data/links"))
        };
        assert_eq!(relative_to(&t, &b).unwrap(), Path::new("..").join("photos").join("a.jpg"));
        assert_eq!(relative_to(&b, &b).unwrap(), PathBuf::from("."));
        let child = b.join("x");
        assert_eq!(relative_to(&child, &b).unwrap(), PathBuf::from("x"));
        if cfg!(windows) {
            assert!(relative_to(Path::new(r"D:\x"), Path::new(r"C:\y")).is_none(), "different drives");
            assert!(relative_to(Path::new(r"c:\data\x"), Path::new(r"C:\data")).is_some(), "drive letter case");
        }
    }
}
