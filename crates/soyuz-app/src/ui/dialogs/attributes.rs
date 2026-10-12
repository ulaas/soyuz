//! Bulk "Attributes & timestamps" dialog (Action::SetAttributes): tri-state attribute
//! flags / unix mode for the whole selection, optional new created / modified /
//! accessed times, and an option to include everything inside selected folders. The
//! result is one or more undoable `SetMeta` jobs.

use std::sync::Arc;
use std::time::SystemTime;

use egui::{Align, FontId, Layout, RichText, Ui};
use parking_lot::Mutex;
use soyuz_core::{Control, EntryInfo, EntryKind, OpControl};
use soyuz_fs::{TimesSet, WalkItem, WalkOptions};

use super::conflict::{Btn, button};
use super::properties::MetaCmd;
use super::properties_attrs::{AttrEdit, AttrEditUi as _, ModeEdit, ModeEditUi as _, plan_jobs};
use super::properties_widgets::{caption, check, datetime_field, kv_row_with, rule};
use super::{Dialog, DialogCtx};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::theme::{icons::ph, palette};
use crate::ui::toasts::Toast;

pub fn open(app: &mut SoyuzApp) {
    match super::properties::targets(&app.core) {
        Ok((_, entries)) => app.gui.dialogs.push(AttributesDialog::new(entries)),
        Err(e) => {
            app.core.ui.toasts.push(Toast::info(e));
        }
    }
}

/// Cancel handle + result slot of the recursive walk.
type Collecting = (Arc<OpControl>, Arc<Mutex<Option<Vec<EntryInfo>>>>);

struct TimeSlot {
    label: &'static str,
    on: bool,
    value: Option<SystemTime>,
}

pub struct AttributesDialog {
    title: String,
    entries: Vec<EntryInfo>,
    attrs: AttrEdit,
    mode: Option<ModeEdit>,
    times: [TimeSlot; 3],
    recursive: bool,
    collecting: Option<Collecting>,
}

impl AttributesDialog {
    pub fn new(entries: Vec<EntryInfo>) -> Self {
        let now = Some(SystemTime::now());
        AttributesDialog {
            title: if entries.len() == 1 {
                tr!("dialogs-attrs-title-one", name = crate::ui::widgets::shown_name(&entries[0].name))
            } else {
                tr!("dialogs-attrs-title-many", count = entries.len())
            },
            attrs: AttrEdit::new(&entries),
            mode: ModeEdit::new(&entries),
            times: [
                TimeSlot { label: tr!("dialogs-props-created"), on: false, value: now },
                TimeSlot { label: tr!("dialogs-props-modified"), on: false, value: now },
                TimeSlot { label: tr!("dialogs-props-accessed"), on: false, value: now },
            ],
            recursive: false,
            collecting: None,
            entries,
        }
    }

    fn times(&self) -> Option<TimesSet> {
        let pick = |i: usize| self.times[i].on.then_some(self.times[i].value).flatten();
        let t = TimesSet { created: pick(0), modified: pick(1), accessed: pick(2) };
        (t != TimesSet::default()).then_some(t)
    }

    fn has_changes(&self) -> bool {
        self.times().is_some() || self.attrs.changes().is_some() || self.mode.as_ref().is_some_and(ModeEdit::changed)
    }

    fn jobs_for(&self, entries: &[EntryInfo]) -> Vec<super::properties_attrs::MetaJob> {
        plan_jobs(entries, self.times(), self.attrs.changes(), self.mode.as_ref())
    }

    fn start_collect(&mut self, ctx: &egui::Context) {
        let ctl = OpControl::new();
        let slot = Arc::new(Mutex::new(None));
        let roots = self.entries.clone();
        let (c2, s2, ctx) = (ctl.clone(), slot.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("soyuz-attr-walk".into()).spawn(move || {
            let mut all: Vec<EntryInfo> = Vec::new();
            for root in roots {
                if root.kind != EntryKind::Dir {
                    all.push(root);
                    continue;
                }
                let _ = soyuz_fs::walk(&root.path, &WalkOptions::default(), &c2, &mut |item| {
                    if let WalkItem::Entry { entry, .. } = item {
                        all.push(entry);
                    }
                    if c2.is_cancelled() { Control::Stop } else { Control::Continue }
                });
            }
            if !c2.is_cancelled() {
                *s2.lock() = Some(all);
                ctx.request_repaint();
            }
        });
        self.collecting = Some((ctl, slot));
    }
}

impl Drop for AttributesDialog {
    fn drop(&mut self) {
        if let Some((c, _)) = &self.collecting {
            c.cancel();
        }
    }
}

impl Dialog for AttributesDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::SLIDERS_HORIZONTAL)
    }

    fn width(&self) -> f32 {
        500.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        if let Some(slot) = self.collecting.as_ref().map(|(_, s)| s.clone()) {
            if let Some(all) = slot.lock().take() {
                let jobs = self.jobs_for(&all);
                ctx.cmds.push(Cmd::Meta(MetaCmd::SetMeta(jobs)));
                self.collecting = None;
                return false;
            }
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr!("dialogs-attrs-collecting")).color(p.text_secondary));
            });
            let cancel = super::close_footer(ui, tr!("common-cancel"));
            return !cancel;
        }
        caption(ui, tr!("dialogs-props-attributes"));
        self.attrs.show(ui);
        if let Some(m) = &mut self.mode {
            rule(ui);
            m.show(ui);
        }
        rule(ui);
        ui.horizontal(|ui| {
            caption(ui, tr!("dialogs-props-timestamps"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(tr!("dialogs-attrs-all-now")).clicked() {
                    let now = Some(SystemTime::now());
                    for t in &mut self.times {
                        t.value = now;
                        t.on = true;
                    }
                }
            });
        });
        let id = ui.id().with("bulk_times");
        for (i, t) in self.times.iter_mut().enumerate() {
            kv_row_with(ui, t.label, |ui| {
                ui.horizontal(|ui| {
                    check(ui, &mut t.on, "");
                    if datetime_field(ui, id.with(i), &mut t.value, "", t.on) {
                        t.on = true;
                    }
                });
            });
        }
        let any_dir = self.entries.iter().any(|e| e.kind == EntryKind::Dir);
        if any_dir {
            rule(ui);
            check(ui, &mut self.recursive, tr!("dialogs-attrs-recursive"));
        }
        ui.add_space(4.0);
        ui.label(RichText::new(tr!("dialogs-props-undo-note")).font(FontId::proportional(11.0)).color(p.text_tertiary));
        ui.add_space(14.0);
        let changes = self.has_changes();
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = ui.add_enabled_ui(changes, |ui| button(ui, tr!("common-apply"), Btn::Primary)).inner.clicked()
                && changes;
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
        });
        if ok {
            if self.recursive && any_dir {
                self.start_collect(ui.ctx());
                return true;
            }
            let jobs = self.jobs_for(&self.entries);
            ctx.cmds.push(Cmd::Meta(MetaCmd::SetMeta(jobs)));
            return false;
        }
        !cancel
    }
}
