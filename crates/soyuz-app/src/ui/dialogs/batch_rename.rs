//! Batch rename dialog (F2 on a multiple selection, Ctrl+Shift+R; PLAN M10).
//!
//! Left: the rule stack (find/replace literal or regex, case, insert, remove, numbering,
//! template, trim) with the numbering counter, the "edit the part" target (name /
//! extension / both), presets, and an alternative "text editor" mode with one name per
//! line. Right: a live before → after table with invalid / duplicate / existing-name
//! highlighting and a summary (including how many rename cycles need temporary names).
//! The engine lives in `crate::rename`; the result is one undoable `Rename` job.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::NaiveDateTime;
use egui::{Align, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, vec2};
use parking_lot::Mutex;

use super::conflict::{Btn, button};
use super::properties::MetaCmd;
use super::properties_widgets::{badge, caption, check, small_icon_button};
use super::{Dialog, DialogCtx};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::rename::presets::{self, Preset};
use crate::rename::{self, CaseMode, Counter, Item, Position, Row, Rule, RuleEntry, Spec, Status, Target};
use crate::state::pane::PaneId;
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons, icons::ph, palette};
use crate::ui::toasts::Toast;
use crate::ui::widgets::menu_item;

pub fn open(app: &mut SoyuzApp) {
    let pane = app.ws.pane();
    let (soyuz_core::Location::Dir(_) | soyuz_core::Location::Search(_)) = &pane.location else {
        app.ui.toasts.push(Toast::info(tr!("dialogs-rename-unavailable")));
        return;
    };
    let entries: Vec<&soyuz_core::EntryInfo> = if pane.selection.is_empty() {
        pane.visible.iter().map(|&i| &pane.entries[i]).collect()
    } else {
        pane.selected_entries()
    };
    if entries.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("dialogs-rename-nothing")));
        return;
    }
    let items: Vec<Item> = entries.iter().map(|e| Item::new(e.path.clone(), e.is_dir_like(), e.modified)).collect();
    let known: HashSet<PathBuf> = pane.entries.iter().map(|e| e.path.clone()).collect();
    let listed_dir = pane.location.as_fs_path().map(Path::to_path_buf);
    let windows_rules = cfg!(windows) || app.config.general.strict_windows_names;
    let d = BatchRenameDialog::new(pane.id, items, known, listed_dir, windows_rules);
    app.ui.dialogs.push(d);
}

/// EXIF capture dates of the items, filled by a worker thread.
type ExifSlot = Arc<Mutex<Option<Vec<Option<NaiveDateTime>>>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Rules,
    Editor,
}

pub struct BatchRenameDialog {
    title: String,
    pane: PaneId,
    items: Vec<Item>,
    known: HashSet<PathBuf>,
    /// Lowercased `known`, for O(1) case-insensitive collision checks.
    known_lower: HashSet<String>,
    listed_dir: Option<PathBuf>,
    windows_rules: bool,
    spec: Spec,
    mode: Mode,
    editor: String,
    /// Cache key of the last preview.
    computed: Option<(Spec, Mode, String, bool)>,
    rows: Vec<Row>,
    compile_error: Option<String>,
    cycles: usize,
    builtin: Vec<Preset>,
    user: Vec<Preset>,
    preset_name: String,
    exif: Option<ExifSlot>,
    exif_loaded: bool,
}

impl BatchRenameDialog {
    pub fn new(
        pane: PaneId,
        items: Vec<Item>,
        known: HashSet<PathBuf>,
        listed_dir: Option<PathBuf>,
        windows_rules: bool,
    ) -> Self {
        let spec = Spec {
            rules: vec![RuleEntry::new(Rule::Replace {
                find: String::new(),
                with: String::new(),
                regex: false,
                case_sensitive: false,
                first_only: false,
            })],
            ..Spec::default()
        };
        let editor = items.iter().map(Item::name).collect::<Vec<_>>().join("\n");
        BatchRenameDialog {
            title: tr!("dialogs-rename-title", count = items.len()),
            pane,
            items,
            known_lower: known.iter().map(|k| k.to_string_lossy().to_lowercase()).collect(),
            known,
            listed_dir,
            windows_rules,
            spec,
            mode: Mode::Rules,
            editor,
            computed: None,
            rows: Vec::new(),
            compile_error: None,
            cycles: 0,
            builtin: presets::builtin(),
            user: presets::load(&presets::default_path()),
            preset_name: String::new(),
            exif: None,
            exif_loaded: false,
        }
    }

    fn exists(&self, p: &Path) -> bool {
        if self.known.contains(p) {
            return true;
        }
        // Outside the listed folder (search results) ask the file system.
        if self.listed_dir.as_deref() != p.parent() {
            return soyuz_fs::exists_nofollow(p);
        }
        // Case-insensitive file systems: compare names case-insensitively.
        crate::ops::fileops::names::case_insensitive_fs()
            && self.known_lower.contains(&p.to_string_lossy().to_lowercase())
    }

    fn ensure_exif(&mut self, ctx: &egui::Context) {
        if self.exif.is_some() || !self.spec.uses_exif() {
            return;
        }
        let slot = Arc::new(Mutex::new(None));
        self.exif = Some(slot.clone());
        let paths: Vec<(PathBuf, bool)> = self.items.iter().map(|i| (i.path.clone(), i.is_dir)).collect();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("soyuz-exif".into()).spawn(move || {
            let dates: Vec<Option<NaiveDateTime>> = paths
                .iter()
                .map(|(p, dir)| if *dir { None } else { rename::exif::read(p).and_then(|x| x.taken) })
                .collect();
            *slot.lock() = Some(dates);
            ctx.request_repaint();
        });
    }

    fn recompute(&mut self, ctx: &egui::Context) {
        self.ensure_exif(ctx);
        if !self.exif_loaded
            && let Some(slot) = &self.exif
            && let Some(dates) = slot.lock().take()
        {
            for (it, d) in self.items.iter_mut().zip(dates) {
                it.exif_date = d;
            }
            self.exif_loaded = true;
            self.computed = None;
        }
        let key = (self.spec.clone(), self.mode, self.editor.clone(), self.exif_loaded);
        if self.computed.as_ref() == Some(&key) {
            return;
        }
        self.compile_error = None;
        let names: Vec<Result<String, String>> = match self.mode {
            Mode::Rules => match rename::compile(&self.spec) {
                Ok(c) => c.apply(&self.items),
                Err(e) => {
                    self.compile_error = Some(e.clone());
                    self.items.iter().map(|_| Err(e.clone())).collect()
                }
            },
            Mode::Editor => match rename::parse_editor(&self.editor, self.items.len()) {
                Ok(lines) => lines.into_iter().map(Ok).collect(),
                Err(e) => {
                    self.compile_error = Some(e.clone());
                    self.items.iter().map(|it| Ok(it.name())).collect()
                }
            },
        };
        let ci = crate::ops::fileops::names::case_insensitive_fs();
        let rows = rename::validate(&self.items, &names, &|p| self.exists(p), self.windows_rules, ci);
        self.cycles = rename::count_cycles(&rename::pairs(&self.items, &rows), ci);
        self.rows = rows;
        self.computed = Some(key);
    }

    fn counts(&self) -> (usize, usize) {
        let ok = self.rows.iter().filter(|r| r.status == Status::Ok).count();
        let bad = self.rows.iter().filter(|r| r.status.is_problem()).count();
        (ok, bad)
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            for (m, label, glyph) in [
                (Mode::Rules, tr!("dialogs-rename-rules"), ph::LIST_CHECKS),
                (Mode::Editor, tr!("dialogs-rename-editor"), ph::NOTE_PENCIL),
            ] {
                let on = self.mode == m;
                let r = chip(ui, glyph, label, on, &p);
                if r.clicked() && !on {
                    if m == Mode::Editor {
                        // Start from the current preview so rules + manual edits combine.
                        self.editor = self.rows.iter().map(|r| r.new_name.clone()).collect::<Vec<_>>().join("\n");
                    }
                    self.mode = m;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let presets_btn = button(ui, tr!("dialogs-rename-presets"), Btn::Secondary);
                crate::ui::native::popup::Menu::menu(&presets_btn).show(|ui| {
                    ui.set_min_width(260.0);
                    crate::ui::widgets::menu_caption(ui, tr!("dialogs-rename-builtin"));
                    for pr in self.builtin.clone() {
                        if menu_item(ui, Some(ph::MAGIC_WAND), &pr.name, None, true).clicked() {
                            self.spec = pr.spec.clone();
                            self.mode = Mode::Rules;
                        }
                    }
                    if !self.user.is_empty() {
                        crate::ui::widgets::menu_separator(ui);
                        crate::ui::widgets::menu_caption(ui, tr!("dialogs-rename-saved"));
                    }
                    let mut delete = None;
                    for pr in self.user.clone() {
                        if menu_item(ui, Some(ph::BOOKMARK_SIMPLE), &pr.name, None, true).clicked() {
                            self.spec = pr.spec.clone();
                            self.mode = Mode::Rules;
                        }
                    }
                    crate::ui::widgets::menu_separator(ui);
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut self.preset_name)
                                .hint_text(tr!("dialogs-rename-preset-name"))
                                .desired_width(150.0)
                                .margin(Margin::symmetric(8, 4)),
                        );
                        let ok = !self.preset_name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new(tr!("common-save"))).clicked() {
                            let name = self.preset_name.trim().to_string();
                            self.user.retain(|p| p.name != name);
                            self.user.push(Preset { name, spec: self.spec.clone() });
                            self.preset_name.clear();
                            if let Err(e) = presets::save(&presets::default_path(), &self.user) {
                                log::warn!("cannot save rename presets: {e}");
                            }
                        }
                    });
                    if !self.user.is_empty() {
                        crate::ui::widgets::submenu(ui, Some(ph::TRASH), tr!("dialogs-rename-delete-preset"), |ui| {
                            for (i, pr) in self.user.iter().enumerate() {
                                if menu_item(ui, Some(ph::X), &pr.name, None, true).clicked() {
                                    delete = Some(i);
                                }
                            }
                        });
                    }
                    if let Some(i) = delete {
                        self.user.remove(i);
                        if let Err(e) = presets::save(&presets::default_path(), &self.user) {
                            log::warn!("cannot save rename presets: {e}");
                        }
                    }
                });
            });
        });
    }

    fn rules_panel(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        caption(ui, tr!("dialogs-rename-edit"));
        let targets: Vec<(Target, &str)> = Target::ALL.iter().map(|t| (*t, t.label())).collect();
        crate::ui::settings::widgets::segmented(ui, &mut self.spec.target, &targets);
        caption(ui, tr!("dialogs-rename-rules"));
        let mut remove = None;
        let mut swap = None;
        let n = self.spec.rules.len();
        for (i, entry) in self.spec.rules.iter_mut().enumerate() {
            egui::Frame::new()
                .fill(egui::Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, p.border_subtle))
                .corner_radius(CornerRadius::same(radius::CARD))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        check(ui, &mut entry.enabled, "");
                        ui.label(
                            RichText::new(format!("{}. {}", i + 1, entry.rule.title()))
                                .font(crate::ui::views::flow::semibold(12.5))
                                .color(if entry.enabled { p.text_primary } else { p.text_tertiary }),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if small_icon_button(ui, ph::X, tr!("dialogs-rename-remove-rule")).clicked() {
                                remove = Some(i);
                            }
                            if i + 1 < n
                                && small_icon_button(ui, ph::ARROW_DOWN, tr!("dialogs-rename-move-down")).clicked()
                            {
                                swap = Some((i, i + 1));
                            }
                            if i > 0 && small_icon_button(ui, ph::ARROW_UP, tr!("dialogs-rename-move-up")).clicked() {
                                swap = Some((i - 1, i));
                            }
                        });
                    });
                    ui.add_enabled_ui(entry.enabled, |ui| rule_editor(ui, &mut entry.rule, i));
                });
            ui.add_space(6.0);
        }
        if let Some(i) = remove {
            self.spec.rules.remove(i);
        }
        if let Some((a, b)) = swap {
            self.spec.rules.swap(a, b);
        }
        let add = button(ui, tr!("dialogs-rename-add-rule"), Btn::Secondary);
        crate::ui::native::popup::Menu::menu(&add).show(|ui| {
            ui.set_min_width(220.0);
            for r in Rule::templates() {
                let label = match &r {
                    Rule::Insert { at: Position::Start, .. } => tr!("dialogs-rename-insert-prefix").to_string(),
                    Rule::Insert { at: Position::End, .. } => tr!("dialogs-rename-insert-suffix").to_string(),
                    other => other.title().to_string(),
                };
                if menu_item(ui, Some(rule_icon(&r)), &label, None, true).clicked() {
                    self.spec.rules.push(RuleEntry::new(r));
                }
            }
        });
        caption(ui, &tr!("dialogs-rename-counter", token = "{n}"));
        counter_editor(ui, &mut self.spec.counter);
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr!(
                "dialogs-rename-tokens-hint",
                tokens = "{n} {n:3} {name} {ext} {parent} {date} {date:%Y-%m-%d} {exif:%Y%m%d}",
                group = "$1",
                named = "${name}"
            ))
            .font(FontId::proportional(11.0))
            .color(p.text_tertiary),
        );
    }

    fn editor_panel(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        caption(ui, tr!("dialogs-rename-one-per-line"));
        ui.label(
            RichText::new(tr!("dialogs-rename-editor-hint")).font(FontId::proportional(11.0)).color(p.text_tertiary),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("br_editor").max_height(420.0).show(ui, |ui| {
            ui.add(
                TextEdit::multiline(&mut self.editor)
                    .font(FontId::monospace(12.0))
                    .desired_width(f32::INFINITY)
                    .desired_rows(20)
                    .code_editor(),
            );
        });
        if ui.button(tr!("dialogs-rename-reset-names")).clicked() {
            self.editor = self.items.iter().map(Item::name).collect::<Vec<_>>().join("\n");
        }
    }

    fn table(&self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let row_h = 24.0;
        let w = ui.available_width();
        // Header.
        let (h, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        let col = (w - 30.0) / 2.0;
        ui.painter().text(
            egui::pos2(h.left() + 26.0, h.center().y),
            egui::Align2::LEFT_CENTER,
            tr!("dialogs-rename-current-name"),
            FontId::proportional(11.0),
            p.text_tertiary,
        );
        ui.painter().text(
            egui::pos2(h.left() + 26.0 + col + 4.0, h.center().y),
            egui::Align2::LEFT_CENTER,
            tr!("dialogs-conflict-new-name"),
            FontId::proportional(11.0),
            p.text_tertiary,
        );
        egui::Frame::new()
            .fill(p.bg_sunken)
            .corner_radius(CornerRadius::same(radius::CARD))
            .stroke(Stroke::new(1.0, p.border_subtle))
            .inner_margin(Margin::same(2))
            .show(ui, |ui| {
                ui.set_width(w - 4.0);
                egui::ScrollArea::vertical()
                    .id_salt("br_table")
                    .max_height(470.0)
                    .auto_shrink([false, false])
                    .show_rows(ui, row_h, self.items.len(), |ui, range| {
                        for i in range {
                            let (Some(it), Some(row)) = (self.items.get(i), self.rows.get(i)) else { continue };
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                            if i % 2 == 1 {
                                ui.painter().rect_filled(r, 0.0, p.bg_surface1.gamma_multiply(0.5));
                            }
                            let (glyph, color, tip) = status_style(&row.status, &p);
                            if row.status.is_problem() {
                                ui.painter().rect_filled(r, CornerRadius::same(4), color.gamma_multiply(0.10));
                            }
                            ui.painter().text(
                                egui::pos2(r.left() + 12.0, r.center().y),
                                egui::Align2::CENTER_CENTER,
                                glyph,
                                icons::font(13.0),
                                color,
                            );
                            let old = it.name();
                            let cw = (r.width() - 30.0) / 2.0;
                            let left =
                                egui::Rect::from_min_size(egui::pos2(r.left() + 26.0, r.top()), vec2(cw - 6.0, row_h));
                            let right = egui::Rect::from_min_size(
                                egui::pos2(left.right() + 10.0, r.top()),
                                vec2(cw - 6.0, row_h),
                            );
                            let old_c =
                                if row.status == Status::Unchanged { p.text_tertiary } else { p.text_secondary };
                            let g = crate::ui::widgets::trunc_galley(
                                ui.painter(),
                                &old,
                                FontId::proportional(12.5),
                                old_c,
                                left.width(),
                            );
                            ui.painter().galley(egui::pos2(left.left(), left.center().y - g.size().y / 2.0), g, old_c);
                            let new_c = match row.status {
                                Status::Unchanged => p.text_tertiary,
                                Status::Ok => p.text_primary,
                                _ => color,
                            };
                            let shown =
                                if row.new_name.is_empty() { "\u{2014}".to_string() } else { row.new_name.clone() };
                            let g = crate::ui::widgets::trunc_galley(
                                ui.painter(),
                                &shown,
                                FontId::proportional(12.5),
                                new_c,
                                right.width(),
                            );
                            ui.painter().galley(
                                egui::pos2(right.left(), right.center().y - g.size().y / 2.0),
                                g,
                                new_c,
                            );
                            if !tip.is_empty() || old.len() > 40 || row.new_name.len() > 40 {
                                resp.on_hover_text(format!(
                                    "{old}\n\u{2192} {}{}{tip}",
                                    row.new_name,
                                    if tip.is_empty() { "" } else { "\n\n" }
                                ));
                            }
                        }
                    });
            });
    }
}

fn status_style(s: &Status, p: &Palette) -> (&'static str, egui::Color32, String) {
    match s {
        Status::Unchanged => (ph::MINUS, p.text_tertiary, String::new()),
        Status::Ok => (ph::ARROW_RIGHT, p.accent, String::new()),
        Status::Invalid(e) => (ph::WARNING_CIRCLE, p.danger, tr!("dialogs-props-invalid-name", error = e)),
        Status::Duplicate => (ph::COPY, p.danger, tr!("dialogs-rename-duplicate").into()),
        Status::Exists => (ph::WARNING, p.warning, tr!("dialogs-rename-exists").into()),
        Status::Error(e) => (ph::BUG, p.danger, e.clone()),
    }
}

fn chip(ui: &mut Ui, glyph: &str, label: &str, on: bool, p: &Palette) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text_primary);
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 40.0, 28.0), Sense::click());
    let (bg, fg) = if on {
        (p.accent_subtle, p.accent)
    } else if resp.hovered() {
        (p.bg_surface2, p.text_primary)
    } else {
        (egui::Color32::TRANSPARENT, p.text_secondary)
    };
    ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), bg);
    if !on {
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(radius::ROW),
            Stroke::new(1.0, p.border_subtle),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        egui::pos2(r.left() + 12.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        glyph,
        icons::font(14.0),
        fg,
    );
    let tg = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), fg);
    ui.painter().galley(egui::pos2(r.left() + 30.0, r.center().y - tg.size().y / 2.0), tg, fg);
    resp
}

fn rule_icon(r: &Rule) -> &'static str {
    match r {
        Rule::Replace { regex: true, .. } => ph::BRACKETS_CURLY,
        Rule::Replace { .. } => ph::SWAP,
        Rule::Case { .. } => ph::TEXT_AA,
        Rule::Insert { .. } => ph::TEXT_T,
        Rule::Remove { .. } => ph::ERASER,
        Rule::Number { .. } => ph::LIST_NUMBERS,
        Rule::Template { .. } => ph::BRACKETS_SQUARE,
        Rule::Trim { .. } => ph::SCISSORS,
    }
}

/// Width of the label column of the rule editors: fits the longest label of the
/// current language (Turkish labels are longer than the English ones).
fn label_w(ui: &Ui) -> f32 {
    let font = FontId::proportional(11.5);
    let labels = [
        tr!("dialogs-rename-find"),
        tr!("dialogs-rename-replace"),
        tr!("dialogs-rename-text"),
        tr!("dialogs-rename-separator"),
        tr!("dialogs-rename-pattern"),
        tr!("dialogs-rename-position"),
    ];
    let widest = labels
        .iter()
        .map(|l| ui.painter().layout_no_wrap(l.to_string(), font.clone(), egui::Color32::WHITE).size().x)
        .fold(0.0, f32::max);
    (widest + 8.0).max(70.0)
}

fn field(ui: &mut Ui, label: &str, text: &mut String, hint: &str, mono: bool) {
    let p = palette(ui.ctx());
    let lw = label_w(ui);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(lw, 22.0), Sense::hover());
        ui.painter().text(
            egui::pos2(r.left(), r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(11.5),
            p.text_tertiary,
        );
        let mut te =
            TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY).margin(Margin::symmetric(8, 4));
        if mono {
            te = te.font(FontId::monospace(12.0));
        }
        ui.add(te);
    });
}

fn position_editor(ui: &mut Ui, at: &mut Position) {
    let p = palette(ui.ctx());
    let lw = label_w(ui);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(lw, 22.0), Sense::hover());
        ui.painter().text(
            egui::pos2(r.left(), r.center().y),
            egui::Align2::LEFT_CENTER,
            tr!("dialogs-rename-position"),
            FontId::proportional(11.5),
            p.text_tertiary,
        );
        let is_idx = matches!(at, Position::Index(_));
        if ui.selectable_label(*at == Position::Start, tr!("dialogs-rename-pos-start")).clicked() {
            *at = Position::Start;
        }
        if ui.selectable_label(*at == Position::End, tr!("dialogs-rename-pos-end")).clicked() {
            *at = Position::End;
        }
        if ui.selectable_label(is_idx, tr!("dialogs-rename-pos-at")).clicked() && !is_idx {
            *at = Position::Index(1);
        }
        if let Position::Index(i) = at {
            ui.add(egui::DragValue::new(i).range(-255..=255)).on_hover_text(tr!("dialogs-rename-pos-index-tip"));
        }
    });
}

fn rule_editor(ui: &mut Ui, rule: &mut Rule, idx: usize) {
    ui.push_id(idx, |ui| match rule {
        Rule::Replace { find, with, regex, case_sensitive, first_only } => {
            let find_hint =
                if *regex { tr!("dialogs-rename-find-regex-hint") } else { tr!("dialogs-rename-find-hint") };
            field(ui, tr!("dialogs-rename-find"), find, find_hint, *regex);
            let with_hint =
                if *regex { tr!("dialogs-rename-replace-regex-hint") } else { tr!("dialogs-rename-replace-hint") };
            field(ui, tr!("dialogs-rename-replace"), with, with_hint, *regex);
            let indent = label_w(ui) + 4.0;
            ui.horizontal_wrapped(|ui| {
                ui.add_space(indent);
                check(ui, regex, tr!("dialogs-rename-regex"));
                check(ui, case_sensitive, tr!("dialogs-rename-match-case"));
                check(ui, first_only, tr!("dialogs-rename-first-only"));
            });
        }
        Rule::Case { mode } => {
            let items: Vec<_> =
                CaseMode::ALL.iter().map(|m| crate::ui::widgets::dropdown::Item::new(*m, m.label())).collect();
            crate::ui::widgets::dropdown::Dropdown::new(format!("rename.case.{idx}"))
                .width(200.0)
                .name(tr!("dialogs-rename-rule-case"))
                .show(ui, mode, &items);
        }
        Rule::Insert { text, at } => {
            field(
                ui,
                tr!("dialogs-rename-text"),
                text,
                &tr!("dialogs-rename-text-hint", token = "{date:%Y%m%d}_"),
                true,
            );
            position_editor(ui, at);
        }
        Rule::Remove { start, count, from_end } => {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr!("dialogs-rename-remove-count"));
                ui.add(egui::DragValue::new(count).range(0..=255));
                ui.label(tr!("dialogs-rename-remove-start"));
                ui.add(egui::DragValue::new(start).range(0..=255));
                check(ui, from_end, tr!("dialogs-rename-from-end"));
            });
        }
        Rule::Number { at, separator } => {
            position_editor(ui, at);
            field(ui, tr!("dialogs-rename-separator"), separator, tr!("dialogs-rename-separator-hint"), false);
        }
        Rule::Template { pattern } => {
            field(ui, tr!("dialogs-rename-pattern"), pattern, "{parent} {n}", true);
        }
        Rule::Trim { collapse } => {
            check(ui, collapse, tr!("dialogs-rename-collapse"));
        }
    });
}

fn counter_editor(ui: &mut Ui, c: &mut Counter) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr!("dialogs-rename-counter-start"));
        ui.add(egui::DragValue::new(&mut c.start).range(-1_000_000..=1_000_000));
        ui.label(tr!("dialogs-rename-counter-step"));
        ui.add(egui::DragValue::new(&mut c.step).range(-1000..=1000));
        ui.label(tr!("dialogs-rename-counter-digits"));
        ui.add(egui::DragValue::new(&mut c.pad).range(0..=12));
        check(ui, &mut c.per_folder, tr!("dialogs-rename-per-folder"));
    });
}

impl Dialog for BatchRenameDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::TEXTBOX)
    }

    fn width(&self) -> f32 {
        900.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        self.recompute(ui.ctx());
        self.toolbar(ui);
        ui.add_space(8.0);
        let total_w = ui.available_width();
        let left_w = 360.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(left_w, 520.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(left_w);
                egui::ScrollArea::vertical().id_salt("br_rules").max_height(520.0).auto_shrink([false, false]).show(
                    ui,
                    |ui| match self.mode {
                        Mode::Rules => self.rules_panel(ui),
                        Mode::Editor => self.editor_panel(ui),
                    },
                );
            });
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(vec2(total_w - left_w - 12.0, 520.0), Layout::top_down(Align::Min), |ui| {
                self.table(ui);
            });
        });
        ui.add_space(10.0);
        let (ok, bad) = self.counts();
        let mut go = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            if let Some(e) = &self.compile_error {
                badge(ui, Some(ph::WARNING_CIRCLE), e, p.danger);
            } else {
                badge(ui, Some(ph::ARROW_RIGHT), &tr!("dialogs-rename-to-rename", count = ok), p.accent);
                if bad > 0 {
                    badge(ui, Some(ph::WARNING), &tr!("dialogs-rename-problems", count = bad), p.danger);
                }
                if self.cycles > 0 {
                    badge(ui, Some(ph::ARROWS_CLOCKWISE), &tr!("dialogs-rename-cycles", count = self.cycles), p.info);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let enabled = ok > 0 && bad == 0 && self.compile_error.is_none();
                let label = tr!("dialogs-rename-go", count = ok);
                let r = ui.add_enabled_ui(enabled, |ui| button(ui, &label, Btn::Primary)).inner;
                go = r.clicked() && enabled;
                cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
            });
        });
        if go {
            ctx.cmds.push(Cmd::Meta(MetaCmd::BatchRename {
                pane: self.pane,
                pairs: rename::pairs(&self.items, &self.rows),
            }));
            return false;
        }
        !cancel
    }
}
