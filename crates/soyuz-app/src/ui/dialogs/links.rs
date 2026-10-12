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
    // Junctions are NTFS-only: a bound key elsewhere opens the symbolic link dialog.
    let kind = if kind == LinkChoice::Junction && !cfg!(windows) { LinkChoice::Symlink } else { kind };
    let pane = app.core.ws.pane();
    let Some(dir) = pane.location.as_fs_path().map(Path::to_path_buf) else {
        app.core.ui.toasts.push(Toast::info(tr!("dialogs-link-need-folder")));
        return;
    };
    if !matches!(pane.location, soyuz_core::Location::Dir(_)) {
        app.core.ui.toasts.push(Toast::info(tr!("dialogs-link-need-folder")));
        return;
    }
    let target = if pane.selection.is_empty() { None } else { pane.selected_entries().first().map(|e| e.path.clone()) };
    let names: Vec<String> = pane.entries.iter().map(|e| e.name.clone()).collect();
    let windows_rules = crate::ops::fileops::rename::windows_rules_for(
        app.core.config.general.strict_windows_names,
        &app.core.ui.volumes,
        Some(&dir),
    );
    app.gui.dialogs.push(LinkDialog::new(dir, target, kind, names).windows_rules(windows_rules));
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
    /// The background probe of the typed target has not finished yet.
    Checking,
    Missing,
    File,
    Dir,
}

/// Result of probing a target off the UI thread (`metadata` on a dead UNC share can
/// block for tens of seconds, so it must never run per frame on the UI thread).
#[derive(Clone, Debug)]
struct Probe {
    state: TargetState,
    /// Target and link folder are on the same volume (for hard links).
    same_volume: bool,
    /// Folder the picker should switch to after a typed edit (target or its parent).
    browse: Option<PathBuf>,
}

fn probe_target(target: &Path, dir: &Path) -> Probe {
    let state = match std::fs::metadata(target) {
        Ok(m) if m.is_dir() => TargetState::Dir,
        Ok(_) => TargetState::File,
        Err(_) => TargetState::Missing,
    };
    let same_volume = match state {
        TargetState::File => soyuz_fs::device_of(target).ok() == soyuz_fs::device_of(dir).ok(),
        _ => true,
    };
    let browse = if state == TargetState::Dir {
        Some(target.to_path_buf())
    } else {
        target.parent().filter(|pp| pp.is_dir()).map(Path::to_path_buf)
    };
    Probe { state, same_volume, browse }
}

/// Typing pause before the target is probed.
const PROBE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

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
    /// Probe of the target path `.0` (debounced, background).
    probe: Option<(PathBuf, Bg<Probe>)>,
    /// When the target text was last edited by typing (debounce start).
    edited_at: Option<std::time::Instant>,
    /// Apply the probe's `browse` suggestion once it arrives (typed edits only).
    follow_browse: bool,
    /// Validate the name with the Windows rules (see `rename::windows_rules_for`).
    windows_rules: bool,
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
            probe: None,
            edited_at: None,
            follow_browse: false,
            windows_rules: cfg!(windows),
        }
    }

    /// Validate names with the Windows rules (Windows-family volume or strict names).
    pub fn windows_rules(mut self, on: bool) -> Self {
        self.windows_rules = on;
        self
    }

    fn target_path(&self) -> PathBuf {
        let t = PathBuf::from(soyuz_fs::expand_env(self.target.trim()));
        if t.is_relative() { self.dir.join(t) } else { t }
    }

    /// Current probe result for the typed target, starting a background probe once
    /// the text has settled (debounced) and differs from the last probed path.
    fn poll_probe(&mut self, ctx: &egui::Context) -> Option<Probe> {
        if self.target.trim().is_empty() {
            return Some(Probe { state: TargetState::Empty, same_volume: true, browse: None });
        }
        let tp = self.target_path();
        let fresh = self.probe.as_ref().is_some_and(|(p, _)| *p == tp);
        if !fresh {
            if self.edited_at.is_none_or(|t| t.elapsed() >= PROBE_DEBOUNCE) {
                let (t2, d2) = (tp.clone(), self.dir.clone());
                self.probe = Some((tp, Bg::spawn(ctx, move || probe_target(&t2, &d2))));
            } else {
                ctx.request_repaint_after(PROBE_DEBOUNCE);
            }
            return None;
        }
        self.probe.as_ref().and_then(|(_, bg)| bg.with(|r| r.cloned()))
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
                let dot_hidden = crate::services::lister::dot_hidden();
                let opts = ListOptions { include_hidden: false, dot_hidden, ..ListOptions::default() };
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
                    if small_icon_button(ui, ph::ARROW_UP, tr!("dialogs-link-parent")).clicked()
                        && let Some(parent) = dir.parent()
                    {
                        nav = Some(parent.to_path_buf());
                    }
                    if small_icon_button(ui, ph::HOUSE, tr!("dialogs-link-home")).clicked() {
                        nav = Some(soyuz_fs::home());
                    }
                    let shown = soyuz_fs::strip_verbatim(&dir).display().to_string();
                    let use_label = tr!("dialogs-link-use-folder");
                    let use_w = ui
                        .painter()
                        .layout_no_wrap(
                            use_label.to_string(),
                            egui::TextStyle::Small.resolve(ui.style()),
                            p.text_primary,
                        )
                        .size()
                        .x;
                    let w = (ui.available_width() - use_w - 24.0).max(40.0);
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
                        if ui.small_button(use_label).clicked() {
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
                                ui.label(RichText::new(tr!("common-loading")).color(p.text_tertiary));
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
                                ui.label(RichText::new(tr!("dialogs-link-empty-folder")).color(p.text_tertiary));
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
            RichText::new(tr!("dialogs-link-picker-hint")).font(FontId::proportional(10.5)).color(p.text_tertiary),
        );
    }
}

impl Dialog for LinkDialog {
    fn title(&self) -> &str {
        tr!("dialogs-link-title")
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
                (LinkChoice::Symlink, tr!("dialogs-link-symlink"), ph::LINK),
                (LinkChoice::Hardlink, tr!("dialogs-link-hardlink"), ph::LINK_SIMPLE),
                (LinkChoice::Junction, tr!("dialogs-link-junction"), ph::TREE_STRUCTURE),
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
            LinkChoice::Symlink if cfg!(windows) => tr!("dialogs-link-symlink-hint"),
            LinkChoice::Symlink => tr!("dialogs-link-symlink-hint-unix"),
            LinkChoice::Hardlink => tr!("dialogs-link-hardlink-hint"),
            LinkChoice::Junction => tr!("dialogs-link-junction-hint"),
        };
        ui.label(RichText::new(hint).font(FontId::proportional(11.5)).color(p.text_tertiary));
        caption(ui, tr!("dialogs-link-target"));
        ui.horizontal(|ui| {
            let r = ui.add(
                TextEdit::singleline(&mut self.target)
                    .hint_text(tr!("dialogs-link-target-hint"))
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
                self.edited_at = Some(std::time::Instant::now());
                self.follow_browse = true;
            }
            let glyph = if self.picker_open { ph::CARET_UP } else { ph::FOLDER_OPEN };
            if small_icon_button(ui, glyph, tr!("dialogs-link-browse")).clicked() {
                self.picker_open = !self.picker_open;
            }
        });
        if self.picker_open {
            ui.add_space(4.0);
            self.picker(ui);
        }
        let probe = self.poll_probe(ui.ctx());
        if self.follow_browse
            && let Some(pr) = &probe
        {
            self.follow_browse = false;
            if let Some(b) = pr.browse.clone() {
                self.browse = Some(b);
            }
        }
        let state = probe.as_ref().map_or(TargetState::Checking, |pr| pr.state.clone());
        let same_volume = probe.as_ref().is_none_or(|pr| pr.same_volume);
        ui.add_space(4.0);
        ui.horizontal(|ui| match state {
            TargetState::Empty => {}
            TargetState::Checking => {
                ui.spinner();
                ui.label(
                    RichText::new(tr!("dialogs-link-checking")).font(FontId::proportional(11.5)).color(p.text_tertiary),
                );
            }
            TargetState::Missing => {
                badge(ui, Some(ph::WARNING), tr!("dialogs-link-missing"), p.warning);
            }
            TargetState::File => {
                badge(ui, Some(ph::FILE), tr!("dialogs-link-file"), p.success);
            }
            TargetState::Dir => {
                badge(ui, Some(ph::FOLDER), tr!("dialogs-conflict-folder"), p.success);
            }
        });
        // Always laid out (disabled for other kinds) so the dialog doesn't jump.
        ui.add_enabled_ui(self.kind == LinkChoice::Symlink, |ui| {
            check(ui, &mut self.relative, tr!("dialogs-link-relative"));
        });
        caption(ui, tr!("dialogs-link-name"));
        let n = ui.add(
            TextEdit::singleline(&mut self.name)
                .hint_text(tr!("dialogs-link-name-hint"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        if n.changed() {
            self.name_touched = true;
        }
        ui.label(
            RichText::new(tr!(
                "dialogs-link-created-in",
                dir = soyuz_fs::strip_verbatim(&self.dir).display().to_string()
            ))
            .font(FontId::proportional(11.0))
            .color(p.text_tertiary),
        );

        let name = self.name.trim().to_string();
        let target = self.target_path();
        let err: Option<String> = if self.target.trim().is_empty() {
            Some(tr!("dialogs-link-choose-target").into())
        } else if let Err(e) = soyuz_core::validate_name_with(&name, self.windows_rules) {
            Some(tr!("dialogs-props-invalid-name", error = e.to_string()))
        } else if self.existing.iter().any(|e| e.eq_ignore_ascii_case(&name)) {
            Some(tr!("dialogs-link-exists", name = &name))
        } else {
            match (self.kind, &state) {
                (_, TargetState::Checking) => Some(tr!("dialogs-link-checking").into()),
                (LinkChoice::Hardlink, TargetState::Dir) => Some(tr!("dialogs-link-hard-files-only").into()),
                (LinkChoice::Hardlink, TargetState::Missing) => Some(tr!("dialogs-link-hard-needs-file").into()),
                (LinkChoice::Hardlink, _) if !same_volume => Some(tr!("dialogs-link-hard-same-volume").into()),
                (LinkChoice::Junction, TargetState::File) => Some(tr!("dialogs-link-junction-folders-only").into()),
                (LinkChoice::Junction, TargetState::Missing) => Some(tr!("dialogs-link-junction-needs-folder").into()),
                _ => None,
            }
        };
        if let Some(e) = &err
            && !self.target.trim().is_empty()
            && state != TargetState::Checking
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
            ok = ui.add_enabled_ui(enabled, |ui| button(ui, tr!("dialogs-link-create"), Btn::Primary)).inner.clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
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
