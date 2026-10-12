//! Properties dialog (Alt+Enter) and the dispatcher for the metadata actions
//! (Properties, checksums, batch rename, bulk attributes / timestamps, touch, links).
//!
//! Tabs: **General** (icon or thumbnail, editable name, type, location, size / size on
//! disk / item counts computed in the background, editable created / modified / accessed
//! times), **Attributes** (Windows attributes or unix mode bits + octal), **Security**
//! (owner and a read-only ACL view; unix chown), **Checksums**, **Details** (file id,
//! links, EXIF / media metadata). Multi-selection shows a summary with tri-state
//! editing. Changes are applied as `SetMeta` / `Rename` jobs, so they show up in the
//! activity panel and are undoable.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use egui::{Align, Align2, CornerRadius, FontId, Key, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use parking_lot::Mutex;
use soyuz_core::config::Action;
use soyuz_core::{Control, EntryInfo, EntryKind, OpControl, SizeStyle, format_size};
use soyuz_fs::{TimesSet, WalkItem, WalkOptions};

use super::conflict::{Btn, button};
use super::properties_attrs::{AttrEdit, ModeEdit, info_attrs, plan_jobs};
use super::properties_checksum::Checksums;
use super::properties_security::OwnerEdit;
use super::properties_widgets::{
    badge, caption, datetime_field, kv_row, kv_row_with, rule, small_icon_button, tab_strip,
};
use super::{Dialog, DialogCtx};
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::state::pane::PaneId;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::views::flow::semibold;

pub use crate::models::properties::{
    Bg, FlagPlan, MetaCmd, Start, Tab, apply, apply_reason, execute, handles, open, pump, targets,
};

/// Icon and label of a tab.
fn tab_label(tab: Tab) -> (&'static str, &'static str) {
    match tab {
        Tab::General => (ph::INFO, tr!("dialogs-props-tab-general")),
        Tab::Attributes => (ph::SLIDERS_HORIZONTAL, tr!("dialogs-props-tab-attributes")),
        Tab::Security => (ph::SHIELD_CHECK, tr!("dialogs-props-tab-security")),
        Tab::Checksums => (ph::FINGERPRINT, tr!("dialogs-props-tab-checksums")),
        Tab::Details => (ph::LIST_BULLETS, tr!("dialogs-props-tab-details")),
        Tab::Versions => (ph::CLOCK_COUNTER_CLOCKWISE, tr!("dialogs-props-tab-versions")),
    }
}

/// Extra rows for the items context menu (batch rename, attributes, checksums).
pub fn context_items(
    ui: &mut Ui,
    km: &soyuz_core::config::Keymap,
    cmds: &mut Vec<Cmd>,
    pane: PaneId,
    multi: bool,
    single_file: bool,
) {
    use crate::ui::widgets::{menu_item, submenu};
    let item = |ui: &mut Ui, cmds: &mut Vec<Cmd>, icon: &str, label: &str, a: Action| {
        let sc = crate::keys::chord_label(km, &a);
        if menu_item(ui, Some(icon), label, sc.as_deref(), true).clicked() {
            cmds.push(Cmd::PaneAction(pane, a));
            ui.close();
        }
    };
    if multi {
        item(ui, cmds, ph::TEXTBOX, tr!("dialogs-menu-batch-rename"), Action::BatchRename);
    }
    item(ui, cmds, ph::SLIDERS_HORIZONTAL, tr!("dialogs-menu-attributes"), Action::SetAttributes);
    if single_file {
        submenu(ui, Some(ph::FINGERPRINT), tr!("dialogs-menu-checksum"), |ui| {
            ui.set_min_width(200.0);
            for a in super::properties_checksum::ALGOS {
                item(ui, cmds, ph::HASH, a.label(), Action::Checksum(a));
            }
            item(ui, cmds, ph::CHECK_CIRCLE, tr!("dialogs-menu-compare"), Action::CompareChecksum);
        });
    }
}

/// Folder statistics accumulated by the background walk.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub alloc: u64,
    pub alloc_known: bool,
    pub errors: u64,
    pub done: bool,
}

fn spawn_stats(ctx: &egui::Context, entries: &[EntryInfo], ctl: Arc<OpControl>) -> Arc<Mutex<Stats>> {
    let stats = Arc::new(Mutex::new(Stats::default()));
    let s2 = stats.clone();
    let roots: Vec<(PathBuf, bool)> = entries.iter().map(|e| (e.path.clone(), e.kind == EntryKind::Dir)).collect();
    let single_dir = roots.len() == 1 && roots[0].1;
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("soyuz-props-size".into()).spawn(move || {
        let mut last = Instant::now();
        let mut local = Stats::default();
        for (root, _) in &roots {
            let r = soyuz_fs::walk(root, &WalkOptions::default(), &ctl, &mut |item| {
                match item {
                    WalkItem::Entry { entry, depth } => {
                        if !(single_dir && depth == 0) {
                            match entry.kind {
                                EntryKind::Dir => local.dirs += 1,
                                _ => {
                                    local.files += 1;
                                    local.bytes += entry.size;
                                    match entry.alloc_size {
                                        Some(a) => {
                                            local.alloc += a;
                                            local.alloc_known = true;
                                        }
                                        None => local.alloc += entry.size.div_ceil(4096) * 4096,
                                    }
                                }
                            }
                        }
                    }
                    WalkItem::Error { .. } => local.errors += 1,
                    WalkItem::DirDone { .. } => {}
                }
                if last.elapsed() > Duration::from_millis(120) {
                    last = Instant::now();
                    *s2.lock() = local.clone();
                    ctx.request_repaint();
                }
                if ctl.is_cancelled() { Control::Stop } else { Control::Continue }
            });
            if r.is_err() {
                local.errors += 1;
            }
            if ctl.is_cancelled() {
                return;
            }
        }
        local.done = true;
        *s2.lock() = local;
        ctx.request_repaint();
    });
    stats
}

struct TimeEdit {
    label: &'static str,
    orig: Option<SystemTime>,
    value: Option<SystemTime>,
    mixed: bool,
}

impl TimeEdit {
    fn new(label: &'static str, vals: Vec<Option<SystemTime>>) -> Self {
        let first = vals.first().copied().flatten();
        let mixed = vals.iter().any(|v| *v != first);
        TimeEdit { label, orig: if mixed { None } else { first }, value: if mixed { None } else { first }, mixed }
    }

    fn changed(&self) -> bool {
        self.value != self.orig && self.value.is_some()
    }
}

pub struct PropertiesDialog {
    title: String,
    pane: PaneId,
    entries: Vec<EntryInfo>,
    size_style: SizeStyle,
    tabs: Vec<Tab>,
    tab: usize,
    name: String,
    focus_name: bool,
    times: [TimeEdit; 3],
    attrs: AttrEdit,
    mode: Option<ModeEdit>,
    owner: Option<OwnerEdit>,
    ctl: Arc<OpControl>,
    stats: Option<Arc<Mutex<Stats>>>,
    security: Option<Bg<super::properties_security::Loaded>>,
    details: Option<Bg<super::properties_details::Sections>>,
    checksums: Option<Checksums>,
    start: Option<Start>,
    error: Option<String>,
    /// Validate a new name with the Windows rules (see `rename::windows_rules_for`).
    windows_rules: bool,
    thumb: Option<Bg<Option<egui::ColorImage>>>,
    thumb_tex: Option<egui::TextureHandle>,
    /// Link target of a single symlink / junction, read once in the background (it used
    /// to be a `read_link` per frame on the UI thread).
    link_target: Option<Bg<Option<String>>>,
    /// A fresh `stat` of a single item (listings may omit allocation size / link count /
    /// file id), taken on a worker thread and merged once when it lands.
    fresh: Option<Bg<Option<EntryInfo>>>,
    fresh_merged: bool,
    /// Previous versions tab (created when first shown).
    versions: Option<crate::ui::tools::versions::VersionsPanel>,
    /// Linux inode flags (Attributes tab; they replace the generic attribute rows there).
    inode: Option<super::properties_inode::InodeEdit>,
}

/// Decode a small thumbnail (worker thread).
fn load_thumb(path: &std::path::Path) -> Option<egui::ColorImage> {
    let img = image::open(path).ok()?.thumbnail(112, 112).to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()))
}

impl PropertiesDialog {
    pub fn new(pane: PaneId, entries: Vec<EntryInfo>, size_style: SizeStyle, tab: Tab, start: Option<Start>) -> Self {
        let single = entries.len() == 1;
        let first = entries[0].clone();
        let file = single && first.kind == EntryKind::File;
        let mut tabs = vec![Tab::General, Tab::Attributes];
        if single {
            tabs.push(Tab::Security);
        }
        if file {
            tabs.push(Tab::Checksums);
        }
        if single {
            tabs.push(Tab::Details);
        }
        if single && crate::ui::tools::versions::enabled() {
            tabs.push(Tab::Versions);
        }
        let tab = tabs.iter().position(|t| *t == tab).unwrap_or(0);
        let title = if single {
            tr!("dialogs-props-title-one", name = crate::ui::widgets::shown_name(&first.name))
        } else {
            tr!("common-items", count = entries.len())
        };
        let times = [
            TimeEdit::new(tr!("dialogs-props-created"), entries.iter().map(|e| e.created).collect()),
            TimeEdit::new(tr!("dialogs-props-modified"), entries.iter().map(|e| e.modified).collect()),
            TimeEdit::new(tr!("dialogs-props-accessed"), entries.iter().map(|e| e.accessed).collect()),
        ];
        let owner = (!cfg!(windows)).then(|| {
            let same = |f: fn(&EntryInfo) -> Option<&str>| {
                let v = f(&entries[0]);
                entries.iter().all(|e| f(e) == v).then_some(v).flatten()
            };
            OwnerEdit::new(same(|e| e.owner.as_deref()), same(|e| e.group.as_deref()))
        });
        PropertiesDialog {
            title,
            pane,
            size_style,
            tabs,
            tab,
            name: first.name.clone(),
            focus_name: false,
            times,
            attrs: AttrEdit::new(&entries),
            mode: ModeEdit::new(&entries),
            owner,
            ctl: OpControl::new(),
            stats: None,
            security: None,
            details: None,
            checksums: file.then(|| Checksums::new(first.path.clone())),
            start,
            error: None,
            windows_rules: cfg!(windows),
            thumb: None,
            thumb_tex: None,
            link_target: None,
            fresh: None,
            fresh_merged: false,
            versions: None,
            inode: cfg!(target_os = "linux")
                .then(|| super::properties_inode::InodeEdit::new(entries.iter().map(|e| e.path.clone()).collect())),
            entries,
        }
    }

    /// Name rules of the item's folder (Windows rules on FAT / exFAT / NTFS volumes
    /// or with `[general] strict_windows_names`; cached volume list, no I/O).
    pub fn naming(mut self, app: &crate::app::SoyuzApp) -> Self {
        let dir = self.entries.first().and_then(|e| e.path.parent());
        self.windows_rules = crate::ops::fileops::rename::windows_rules_for(
            app.core.config.general.strict_windows_names,
            &app.core.ui.volumes,
            dir,
        );
        self
    }

    fn single(&self) -> Option<&EntryInfo> {
        (self.entries.len() == 1).then(|| &self.entries[0])
    }

    fn size(&self, b: u64) -> String {
        format_size(b, self.size_style)
    }

    fn exact(&self, b: u64) -> String {
        let s = self.size(b);
        if b < 1024 { s } else { tr!("dialogs-props-exact-size", size = s, bytes = b) }
    }

    /// Start / merge the background re-stat of a single item.
    fn refresh_single(&mut self, ctx: &egui::Context) {
        if self.entries.len() != 1 || self.fresh_merged {
            return;
        }
        let path = self.entries[0].path.clone();
        let bg = self.fresh.get_or_insert_with(|| Bg::spawn(ctx, move || soyuz_fs::stat(&path, false).ok()));
        let Some(fresh) = bg.with(|v| v.cloned()) else { return };
        self.fresh_merged = true;
        if let Some(f) = fresh {
            let e = &mut self.entries[0];
            e.alloc_size = f.alloc_size.or(e.alloc_size);
            e.nlink = f.nlink.or(e.nlink);
            e.inode = f.inode.or(e.inode);
            e.dev = f.dev.or(e.dev);
            e.owner = e.owner.take().or(f.owner);
            e.group = e.group.take().or(f.group);
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        self.refresh_single(ui.ctx());
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(crate::theme::tokens::radius::CARD), p.bg_surface2);
            let e = &self.entries[0];
            let thumb = self.entries.len() == 1
                && e.kind == EntryKind::File
                && e.size < 64 * 1024 * 1024
                && crate::ui::views::icons::classify(e) == crate::ui::views::icons::FileClass::Image;
            if thumb && self.thumb.is_none() {
                let path = e.path.clone();
                self.thumb = Some(Bg::spawn(ui.ctx(), move || load_thumb(&path)));
            }
            if self.thumb_tex.is_none()
                && let Some(img) = self.thumb.as_ref().and_then(|b| b.with(|v| v.cloned().flatten()))
            {
                self.thumb_tex = Some(ui.ctx().load_texture("soyuz.props.thumb", img, egui::TextureOptions::LINEAR));
            }
            let e = &self.entries[0];
            if let Some(tex) = &self.thumb_tex {
                let size = tex.size_vec2();
                let scale = (52.0 / size.x).min(52.0 / size.y);
                let rect = egui::Rect::from_center_size(r.center(), size * scale);
                egui::Image::new(tex).corner_radius(CornerRadius::same(4)).paint_at(ui, rect);
            } else {
                let (glyph, color) = if self.entries.len() > 1 {
                    (ph::STACK, p.accent)
                } else {
                    (crate::ui::views::icons::glyph(e), crate::ui::views::icons::color(e, &p))
                };
                ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, icons::font(30.0), color);
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                if let Some(e) = self.single().cloned() {
                    let resp = ui.add(
                        TextEdit::singleline(&mut self.name)
                            .font(semibold(15.0))
                            .desired_width(ui.available_width())
                            .margin(Margin::symmetric(8, 5)),
                    );
                    if self.focus_name {
                        resp.request_focus();
                        self.focus_name = false;
                    }
                    // The edit field holds the real name; spell out hidden characters.
                    crate::ui::widgets::hidden_note(ui, &e.name);
                    let mut line = crate::state::listing::kind_label(&e);
                    if e.kind == EntryKind::File {
                        line = tr!("dialogs-props-kind-size", kind = line, size = self.size(e.size));
                    }
                    ui.label(RichText::new(line).font(FontId::proportional(11.5)).color(p.text_tertiary));
                } else {
                    let files = self.entries.iter().filter(|e| !e.is_dir_like()).count();
                    let dirs = self.entries.len() - files;
                    ui.label(
                        RichText::new(tr!("dialogs-props-selected", count = self.entries.len()))
                            .font(semibold(15.0))
                            .color(p.text_primary),
                    );
                    let mut parts = Vec::new();
                    if files > 0 {
                        parts.push(tr!("common-files", count = files));
                    }
                    if dirs > 0 {
                        parts.push(tr!("common-folders", count = dirs));
                    }
                    let kinds: std::collections::BTreeSet<String> =
                        self.entries.iter().map(crate::state::listing::kind_label).collect();
                    let kinds = if kinds.len() == 1 {
                        tr!("dialogs-props-all-kind", kind = kinds.into_iter().next().unwrap_or_default())
                    } else {
                        tr!("dialogs-props-mixed-types").into()
                    };
                    ui.label(
                        RichText::new(tr!("dialogs-props-summary", counts = parts.join(", "), kinds = kinds))
                            .font(FontId::proportional(11.5))
                            .color(p.text_tertiary),
                    );
                }
            });
        });
    }

    fn general(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let p = palette(ui.ctx());
        let needs_walk = self.entries.iter().any(|e| e.kind == EntryKind::Dir) || self.entries.len() > 1;
        if needs_walk && self.stats.is_none() {
            self.stats = Some(spawn_stats(ui.ctx(), &self.entries, self.ctl.clone()));
        }
        caption(ui, tr!("dialogs-props-tab-general"));
        if let Some(e) = self.single().cloned() {
            kv_row(ui, tr!("dialogs-props-type"), &crate::state::listing::kind_label(&e));
        }
        let parent = common_parent(&self.entries);
        if let Some(parent) = &parent {
            kv_row_with(ui, tr!("dialogs-props-location"), |ui| {
                let s = soyuz_fs::strip_verbatim(parent).display().to_string();
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 30.0;
                    ui.allocate_ui(vec2(w, 20.0), |ui| {
                        ui.add(egui::Label::new(RichText::new(&s).color(p.text_primary)).truncate().selectable(true))
                            .on_hover_text(&s);
                    });
                    super::properties_widgets::copy_button(ui, &s, cmds);
                });
            });
        }
        match (&self.stats, self.single()) {
            (None, Some(e)) => {
                kv_row(ui, tr!("dialogs-props-size"), &self.exact(e.size));
                match e.alloc_size {
                    Some(a) => kv_row(ui, tr!("dialogs-props-size-on-disk"), &self.exact(a)),
                    None => kv_row(
                        ui,
                        tr!("dialogs-props-size-on-disk"),
                        &format!("\u{2248} {}", self.size(e.size.div_ceil(4096) * 4096)),
                    ),
                };
            }
            (Some(stats), _) => {
                let s = stats.lock().clone();
                let busy = !s.done;
                kv_row_with(ui, tr!("dialogs-props-size"), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(self.exact(s.bytes)).color(p.text_primary));
                        if busy {
                            ui.spinner();
                        }
                    });
                });
                let approx = if s.alloc_known { "" } else { "\u{2248} " };
                kv_row(ui, tr!("dialogs-props-size-on-disk"), &format!("{approx}{}", self.exact(s.alloc)));
                let contains = if s.errors > 0 {
                    tr!("dialogs-props-contains-errors", files = s.files, dirs = s.dirs, errors = s.errors)
                } else {
                    tr!("dialogs-props-contains", files = s.files, dirs = s.dirs)
                };
                let label = if self.entries.len() == 1 {
                    tr!("dialogs-props-contains-label")
                } else {
                    tr!("dialogs-props-total")
                };
                kv_row(ui, label, &contains);
            }
            (None, None) => {}
        }
        if let Some(e) = self.single()
            && let EntryKind::Symlink { .. } | EntryKind::Junction = e.kind
        {
            let path = e.path.clone();
            let bg = self.link_target.get_or_insert_with(|| {
                Bg::spawn(ui.ctx(), move || {
                    soyuz_fs::read_link(&path).ok().map(|t| soyuz_fs::strip_verbatim(&t).display().to_string())
                })
            });
            if let Some(t) = bg.with(|t| t.cloned().flatten()) {
                kv_row(ui, tr!("dialogs-props-link-target"), &t);
            }
        }
        if let Some(e) = self.single().filter(|e| e.kind == EntryKind::Dir) {
            crate::ui::ai::properties_row(ui, &e.path.clone(), cmds);
        }

        rule(ui);
        ui.horizontal(|ui| {
            caption(ui, tr!("dialogs-props-dates"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if small_icon_button(ui, ph::CLOCK_COUNTER_CLOCKWISE, tr!("dialogs-props-touch-tip")).clicked() {
                    let now = SystemTime::now();
                    self.times[1].value = Some(now);
                    self.times[2].value = Some(now);
                }
            });
        });
        let id = ui.id().with("times");
        for (i, t) in self.times.iter_mut().enumerate() {
            kv_row_with(ui, t.label, |ui| {
                ui.horizontal(|ui| {
                    let ph_text = if t.mixed { tr!("dialogs-props-multiple") } else { tr!("dialogs-props-unknown") };
                    datetime_field(ui, id.with(i), &mut t.value, ph_text, true);
                    if t.changed() {
                        badge(ui, None, tr!("dialogs-props-changed"), p.accent);
                        if small_icon_button(ui, ph::ARROW_COUNTER_CLOCKWISE, tr!("dialogs-props-revert")).clicked() {
                            t.value = t.orig;
                        }
                    }
                });
            });
        }
        if cfg!(not(any(windows, target_os = "macos"))) {
            ui.label(
                RichText::new(tr!("dialogs-props-no-created-linux"))
                    .font(FontId::proportional(11.0))
                    .color(p.text_tertiary),
            );
        }
    }

    fn attributes(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        match &mut self.inode {
            Some(ie) => ie.show(ui),
            None => {
                caption(ui, tr!("dialogs-props-attributes"));
                self.attrs.show(ui);
            }
        }
        let info: Vec<&str> = info_attrs()
            .into_iter()
            .filter(|(a, _)| self.entries.iter().any(|e| e.attrs.contains(*a)))
            .map(|(_, l)| l)
            .collect();
        if !info.is_empty() {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for l in info {
                    badge(ui, Some(ph::INFO), l, p.text_secondary);
                }
            });
        }
        if let Some(m) = &mut self.mode {
            rule(ui);
            m.show(ui);
        }
    }

    fn security(&mut self, ui: &mut Ui) {
        if self.security.is_none()
            && let Some(e) = self.single()
        {
            let path = e.path.clone();
            self.security = Some(Bg::spawn(ui.ctx(), move || super::properties_security::load(&path)));
        }
        let owner = self.owner.as_mut();
        match &self.security {
            Some(bg) => bg.with(|l| super::properties_security::show(ui, l, owner)),
            None => super::properties_security::show(ui, None, owner),
        }
    }

    fn details(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let p = palette(ui.ctx());
        if self.details.is_none()
            && let Some(e) = self.single().cloned()
        {
            self.details = Some(Bg::spawn(ui.ctx(), move || super::properties_details::load(&e)));
        }
        let Some(bg) = &self.details else { return };
        bg.with(|d| match d {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tr!("dialogs-props-reading")).color(p.text_tertiary));
                });
            }
            Some(sections) => {
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (title, rows) in sections {
                        caption(ui, title);
                        for (k, v) in rows {
                            kv_row_with(ui, k, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(v).color(p.text_primary))
                                            .wrap()
                                            .selectable(true),
                                    );
                                });
                            });
                        }
                    }
                    if let Some(e) = self.entries.first() {
                        ui.add_space(6.0);
                        let s = soyuz_fs::strip_verbatim(&e.path).display().to_string();
                        if ui.add(egui::Button::new(RichText::new(tr!("dialogs-props-copy-path")).size(12.0))).clicked()
                        {
                            cmds.push(Cmd::CopyText(s));
                        }
                    }
                });
            }
        });
    }

    fn dirty(&self) -> bool {
        let renamed = self.single().is_some_and(|e| self.name.trim() != e.name);
        renamed
            || self.times.iter().any(TimeEdit::changed)
            || self.attrs.changes().is_some()
            || self.inode.as_ref().is_some_and(|i| i.changes().is_some())
            || self.mode.as_ref().is_some_and(ModeEdit::changed)
            || self.owner.as_ref().is_some_and(|o| matches!(o.changes(), Ok(Some(_))))
    }

    /// Validate and queue the changes. Returns `false` when something is invalid.
    fn commit(&mut self, cmds: &mut Vec<Cmd>) -> bool {
        self.error = None;
        let rename = match self.single() {
            Some(e) if self.name.trim() != e.name => {
                let n = self.name.trim().to_string();
                if let Err(err) = soyuz_core::validate_name_with(&n, self.windows_rules) {
                    self.error = Some(tr!("dialogs-props-invalid-name", error = err.to_string()));
                    return false;
                }
                Some((e.path.clone(), n))
            }
            _ => None,
        };
        let owner = match self.owner.as_ref().map(OwnerEdit::changes) {
            Some(Err(e)) => {
                self.error = Some(e);
                return false;
            }
            Some(Ok(c)) => c,
            None => None,
        };
        let times = {
            let t = TimesSet {
                created: self.times[0].changed().then_some(self.times[0].value).flatten(),
                modified: self.times[1].changed().then_some(self.times[1].value).flatten(),
                accessed: self.times[2].changed().then_some(self.times[2].value).flatten(),
            };
            (t != TimesSet::default()).then_some(t)
        };
        let jobs = plan_jobs(&self.entries, times, self.attrs.changes(), self.mode.as_ref());
        let chown = owner.map(|(uid, gid)| (self.entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>(), uid, gid));
        let flags = self.inode.as_ref().and_then(|ie| ie.changes().map(|c| (ie.paths().to_vec(), c)));
        if let Some((paths, (set, clear))) = flags {
            // Inode flags are ordered around the jobs of this Apply (see `FlagPlan`).
            let plan = FlagPlan {
                paths,
                set,
                clear,
                meta: jobs,
                chown,
                rename: rename.clone().map(|(from, to)| (self.pane, from, to)),
                done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            };
            if let Some(ie) = &mut self.inode {
                ie.applying(plan.done.clone(), plan.renamed_paths());
            }
            cmds.push(Cmd::Meta(MetaCmd::InodeFlags(Box::new(plan))));
        } else {
            if !jobs.is_empty() {
                cmds.push(Cmd::Meta(MetaCmd::SetMeta(jobs)));
            }
            if let Some((paths, uid, gid)) = chown {
                cmds.push(Cmd::Meta(MetaCmd::Chown { paths, uid, gid }));
            }
            if let Some((from, to)) = &rename {
                cmds.push(Cmd::FileOp(FileOpCmd::Rename { pane: self.pane, from: from.clone(), to: to.clone() }));
            }
        }
        if let Some((from, to)) = rename {
            let new_path = from.with_file_name(&to);
            self.entries[0].path = new_path.clone();
            self.entries[0].name = to.clone();
            self.title = tr!("dialogs-props-title-one", name = &to);
            if let Some(c) = &mut self.checksums {
                c.cancel_all();
                *c = Checksums::new(new_path);
            }
        }
        // The edits are now the baseline.
        for t in &mut self.times {
            if t.changed() {
                t.orig = t.value;
                t.mixed = false;
            }
        }
        self.attrs.commit();
        if let Some(m) = &mut self.mode {
            m.commit();
        }
        if let Some(o) = &mut self.owner {
            o.commit();
        }
        true
    }
}

impl Drop for PropertiesDialog {
    fn drop(&mut self) {
        self.ctl.cancel();
    }
}

/// The folder all entries live in (`None` when they differ, e.g. search results).
fn common_parent(entries: &[EntryInfo]) -> Option<PathBuf> {
    let first = entries.first()?.path.parent()?.to_path_buf();
    entries.iter().all(|e| e.path.parent() == Some(first.as_path())).then_some(first)
}

impl Dialog for PropertiesDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::INFO)
    }

    fn width(&self) -> f32 {
        560.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        if let Some(start) = self.start.take()
            && let Some(c) = &mut self.checksums
        {
            match start {
                Start::Hash(a) => c.start(a, ui.ctx()),
                Start::Compare => c.focus_compare = true,
            }
        }
        self.header(ui);
        ui.add_space(10.0);
        let labels: Vec<(&str, &str)> = self.tabs.iter().map(|t| tab_label(*t)).collect();
        tab_strip(ui, &labels, &mut self.tab);
        let tab = self.tabs[self.tab];
        // Fixed body height so switching tabs doesn't make the modal jump.
        let body_h = 380.0;
        let (body, _) = ui.allocate_exact_size(vec2(ui.available_width(), body_h), Sense::hover());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
        child.set_clip_rect(body.expand(4.0));
        egui::ScrollArea::vertical().id_salt(("props", tab as u8)).auto_shrink([false, false]).show(&mut child, |ui| {
            match tab {
                Tab::General => self.general(ui, ctx.cmds),
                Tab::Attributes => self.attributes(ui),
                Tab::Security => self.security(ui),
                Tab::Checksums => {
                    if let Some(c) = &mut self.checksums {
                        c.show(ui, ctx.cmds);
                    }
                }
                Tab::Details => self.details(ui, ctx.cmds),
                Tab::Versions => {
                    let e = &self.entries[0];
                    let panel = self.versions.get_or_insert_with(|| {
                        crate::ui::tools::versions::VersionsPanel::new(
                            e.path.clone(),
                            e.is_dir_like(),
                            Some((e.size, e.modified)),
                        )
                    });
                    if let Some(cmd) = panel.show(ui) {
                        crate::ui::tools::queue(crate::ui::tools::ToolCmd::VersionAction(cmd));
                    }
                }
            }
        });

        ui.add_space(12.0);
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.danger));
            ui.add_space(4.0);
        }
        let dirty = self.dirty();
        let mut ok = false;
        let mut cancel = false;
        let mut apply = false;
        ui.horizontal(|ui| {
            if dirty {
                badge(ui, Some(ph::PENCIL_SIMPLE), tr!("dialogs-props-unsaved"), p.accent);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ok = button(ui, tr!("common-ok"), Btn::Primary).clicked();
                cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
                ui.add_enabled_ui(dirty, |ui| {
                    apply = button(ui, tr!("common-apply"), Btn::Ghost).clicked() && dirty;
                });
            });
        });
        let text_focused =
            ui.ctx().memory(|m| m.focused()).is_some_and(|id| TextEdit::load_state(ui.ctx(), id).is_some());
        if ui.input(|i| i.key_pressed(Key::Enter)) && (!text_focused || self.single().is_some()) {
            ok = true;
        }
        if apply {
            self.commit(ctx.cmds);
        }
        if ok {
            return !self.commit(ctx.cmds);
        }
        !cancel
    }
}
