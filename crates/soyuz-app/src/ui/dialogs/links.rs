//! "Create link" dialog (symbolic link / hard link / junction) with a built-in target
//! picker: a small folder browser under the target field (no native file dialog, which
//! would need shell APIs), live validation of the target (exists, file vs folder, same
//! volume for hard links) and an optional relative target for symbolic links. The link
//! is created by a `Create` job (undoable). The form, the probe, the listing and the
//! validation are the core's (`models::link_dialog`, shared with the terminal UI).

use std::path::{Path, PathBuf};

use egui::{Align, CornerRadius, FontId, Key, Layout, Margin, RichText, Sense, Stroke, TextEdit, Ui, vec2};
use soyuz_core::EntryInfo;

use super::conflict::{Btn, button};
use super::new_item::LinkChoice;
use super::properties::Bg;
use super::properties_widgets::{badge, caption, check, small_icon_button};
use super::{Dialog, DialogCtx};
use crate::app::SoyuzApp;
use crate::models::link_dialog::{LinkForm, PROBE_DEBOUNCE, Probe, TargetState, list_folder, probe_target};
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::toasts::Toast;

pub fn open(app: &mut SoyuzApp, kind: LinkChoice) {
    match LinkForm::open(&app.core, kind) {
        Ok(form) => app.gui.dialogs.push(LinkDialog::from_form(form)),
        Err(e) => {
            app.core.ui.toasts.push(Toast::info(e));
        }
    }
}

/// Background listing of the picker folder.
type Listing = Bg<Result<Vec<EntryInfo>, String>>;

pub struct LinkDialog {
    form: LinkForm,
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
}

impl LinkDialog {
    pub fn new(dir: PathBuf, target: Option<PathBuf>, kind: LinkChoice, existing: Vec<String>) -> Self {
        LinkDialog::from_form(LinkForm::new(dir, target, kind, existing))
    }

    fn from_form(form: LinkForm) -> Self {
        let picker_open = form.target.trim().is_empty();
        LinkDialog {
            browse: Some(form.start_folder()),
            form,
            focus: true,
            listing: None,
            picker_open,
            probe: None,
            edited_at: None,
            follow_browse: false,
        }
    }

    /// Validate names with the Windows rules (Windows-family volume or strict names).
    pub fn windows_rules(mut self, on: bool) -> Self {
        self.form.windows_rules = on;
        self
    }

    /// Current probe result for the typed target, starting a background probe once
    /// the text has settled (debounced) and differs from the last probed path.
    fn poll_probe(&mut self, ctx: &egui::Context) -> Option<Probe> {
        if self.form.target.trim().is_empty() {
            return Some(Probe::empty());
        }
        let tp = self.form.target_path();
        let fresh = self.probe.as_ref().is_some_and(|(p, _)| *p == tp);
        if !fresh {
            if self.edited_at.is_none_or(|t| t.elapsed() >= PROBE_DEBOUNCE) {
                let (t2, d2) = (tp.clone(), self.form.dir.clone());
                self.probe = Some((tp, Bg::spawn(&crate::host::EguiHost::new(ctx), move || probe_target(&t2, &d2))));
            } else {
                ctx.request_repaint_after(PROBE_DEBOUNCE);
            }
            return None;
        }
        self.probe.as_ref().and_then(|(_, bg)| bg.with(|r| r.cloned()))
    }

    fn set_target(&mut self, p: &Path) {
        self.form.set_target(p);
    }

    fn picker(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let Some(dir) = self.browse.clone() else { return };
        if self.listing.as_ref().is_none_or(|(d, _)| *d != dir) {
            let d2 = dir.clone();
            let bg = Bg::spawn(&crate::host::EguiHost::new(ui.ctx()), move || list_folder(&d2));
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
                                        let selected = self.form.target_path() == e.path;
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
                let on = self.form.kind == k;
                if ui.selectable_label(on, format!("{glyph}  {label}")).clicked() {
                    self.form.kind = k;
                }
            }
        });
        let hint = match self.form.kind {
            LinkChoice::Symlink if cfg!(windows) => tr!("dialogs-link-symlink-hint"),
            LinkChoice::Symlink => tr!("dialogs-link-symlink-hint-unix"),
            LinkChoice::Hardlink => tr!("dialogs-link-hardlink-hint"),
            LinkChoice::Junction => tr!("dialogs-link-junction-hint"),
        };
        ui.label(RichText::new(hint).font(FontId::proportional(11.5)).color(p.text_tertiary));
        caption(ui, tr!("dialogs-link-target"));
        ui.horizontal(|ui| {
            let r = ui.add(
                TextEdit::singleline(&mut self.form.target)
                    .hint_text(tr!("dialogs-link-target-hint"))
                    .desired_width(ui.available_width() - 30.0)
                    .margin(Margin::symmetric(10, 6)),
            );
            if self.focus {
                r.request_focus();
                self.focus = false;
            }
            if r.changed() {
                self.form.target_typed();
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
        ui.add_enabled_ui(self.form.kind == LinkChoice::Symlink, |ui| {
            check(ui, &mut self.form.relative, tr!("dialogs-link-relative"));
        });
        caption(ui, tr!("dialogs-link-name"));
        let n = ui.add(
            TextEdit::singleline(&mut self.form.name)
                .hint_text(tr!("dialogs-link-name-hint"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        if n.changed() {
            self.form.name_edited();
        }
        ui.label(
            RichText::new(tr!(
                "dialogs-link-created-in",
                dir = soyuz_fs::strip_verbatim(&self.form.dir).display().to_string()
            ))
            .font(FontId::proportional(11.0))
            .color(p.text_tertiary),
        );

        let err = self.form.error(&state, same_volume);
        if let Some(e) = &err
            && !self.form.target.trim().is_empty()
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
        if ok && let Some(c) = self.form.create(&state, same_volume) {
            ctx.cmds.push(c);
            return false;
        }
        !cancel
    }
}
