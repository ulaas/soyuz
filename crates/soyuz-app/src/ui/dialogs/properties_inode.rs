//! Linux inode flags in the Properties dialog (Attributes tab): append-only, no-dump,
//! no-atime, no copy-on-write, compress and immutable (`chattr` / `lsattr`). The flags
//! are read on a worker thread when the tab is first shown; Apply changes them on a
//! worker thread too (quick ioctls, but on a slow or hung mount any call can block),
//! ordered around the rename / meta jobs of the same Apply (`properties::FlagPlan`),
//! reports failures as a toast with a plain-language reason (root for immutable /
//! append-only, unsupported file systems) and then re-reads the flags from disk.
//! Flag changes are not undoable (no job kind; like chown).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{FontId, RichText, Ui};
use soyuz_core::{FsError, FsErrorKind};
use soyuz_fs::attrs::InodeFlags;

use super::properties::Bg;
use super::properties_widgets::{Tri, caption, tri_check};
use crate::theme::palette;

/// The editable flags with their labels and hints, in display order.
pub fn flags() -> [(InodeFlags, &'static str, &'static str); 6] {
    [
        (InodeFlags::APPEND, tr!("dialogs-inode-append"), tr!("dialogs-inode-append-hint")),
        (InodeFlags::NODUMP, tr!("dialogs-inode-nodump"), tr!("dialogs-inode-nodump-hint")),
        (InodeFlags::NOATIME, tr!("dialogs-inode-noatime"), tr!("dialogs-inode-noatime-hint")),
        (InodeFlags::NOCOW, tr!("dialogs-inode-nocow"), tr!("dialogs-inode-nocow-hint")),
        (InodeFlags::COMPRESS, tr!("dialogs-inode-compress"), tr!("dialogs-inode-compress-hint")),
        (InodeFlags::IMMUTABLE, tr!("dialogs-attr-immutable"), tr!("dialogs-attr-immutable-hint")),
    ]
}

/// The flags of every entry (`Err` per entry that can't be read).
type Loaded = Vec<Result<InodeFlags, FsError>>;

/// Tri-state inode flag editor over a set of entries.
pub struct InodeEdit {
    paths: Vec<PathBuf>,
    load: Option<Bg<Loaded>>,
    /// `(flag, now, original)`; empty until the flags are read.
    rows: Vec<(InodeFlags, Tri, Tri)>,
    /// Why nothing can be edited (no entry's flags could be read).
    unavailable: Option<String>,
    /// An Apply is running: `(finished, the paths afterwards)`; the flags are re-read
    /// from disk once it finished.
    applying: Option<(Arc<AtomicBool>, Vec<PathBuf>)>,
}

impl InodeEdit {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        InodeEdit { paths, load: None, rows: Vec::new(), unavailable: None, applying: None }
    }

    /// An Apply was queued: hold the edits until it finished, then re-read the flags of
    /// `paths` (the items as they are after a rename).
    pub fn applying(&mut self, done: Arc<AtomicBool>, paths: Vec<PathBuf>) {
        for r in &mut self.rows {
            r.2 = r.1;
        }
        self.applying = Some((done, paths));
    }

    /// An Apply is still running.
    pub fn busy(&self) -> bool {
        self.applying.is_some()
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some((done, _)) = &self.applying {
            if !done.load(Ordering::Relaxed) {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
                return;
            }
            if let Some((_, paths)) = self.applying.take() {
                *self = InodeEdit::new(paths);
            }
        }
        if self.load.is_none() {
            let paths = self.paths.clone();
            self.load = Some(Bg::spawn(ctx, move || paths.iter().map(|p| soyuz_fs::attrs::inode_flags(p)).collect()));
        }
        if !self.rows.is_empty() || self.unavailable.is_some() {
            return;
        }
        let Some(loaded) = self.load.as_ref().and_then(|bg| bg.with(|l| l.cloned())) else { return };
        let ok: Vec<InodeFlags> = loaded.iter().filter_map(|r| r.as_ref().ok().copied()).collect();
        if ok.is_empty() {
            let reason = loaded.iter().find_map(|r| r.as_ref().err()).map(load_reason).unwrap_or_default();
            self.unavailable = Some(reason);
            return;
        }
        self.rows = flags()
            .iter()
            .map(|(f, _, _)| {
                let t = Tri::from_counts(ok.iter().filter(|x| x.contains(*f)).count(), ok.len());
                (*f, t, t)
            })
            .collect();
    }

    /// Flags switched on / off relative to the loaded state (`None` = unchanged).
    pub fn changes(&self) -> Option<(InodeFlags, InodeFlags)> {
        if self.busy() {
            return None;
        }
        let mut set = InodeFlags::empty();
        let mut clear = InodeFlags::empty();
        for (f, now, orig) in &self.rows {
            if now == orig {
                continue;
            }
            match now {
                Tri::On => set |= *f,
                Tri::Off => clear |= *f,
                Tri::Mixed => {}
            }
        }
        (!set.is_empty() || !clear.is_empty()).then_some((set, clear))
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn show(&mut self, ui: &mut Ui) {
        self.poll(ui.ctx());
        let p = palette(ui.ctx());
        caption(ui, tr!("dialogs-inode-title"));
        if let Some(why) = &self.unavailable {
            ui.add(egui::Label::new(RichText::new(why).font(FontId::proportional(11.5)).color(p.text_tertiary)).wrap());
            return;
        }
        if self.rows.is_empty() || self.busy() {
            let text = if self.busy() { tr!("dialogs-inode-applying") } else { tr!("dialogs-props-reading") };
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(text).color(p.text_tertiary));
            });
            return;
        }
        let labels = flags();
        for (row, (_, label, hint)) in self.rows.iter_mut().zip(labels.iter()) {
            ui.horizontal(|ui| {
                let mut t = row.1;
                if tri_check(ui, &mut t, label, true).on_hover_text(*hint).clicked() {
                    row.1 = t;
                }
                if row.1 != row.2 {
                    ui.label(
                        RichText::new(tr!("dialogs-props-changed")).font(FontId::proportional(10.5)).color(p.accent),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(*hint).font(FontId::proportional(11.0)).color(p.text_tertiary));
                });
            });
        }
        if self.changes().is_some_and(|(s, c)| (s | c).intersects(InodeFlags::IMMUTABLE | InodeFlags::APPEND)) {
            ui.label(
                RichText::new(tr!("dialogs-inode-root-note")).font(FontId::proportional(11.0)).color(p.text_tertiary),
            );
        }
    }
}

/// Why the flags of a file can't be read.
fn load_reason(e: &FsError) -> String {
    match e.kind {
        FsErrorKind::Unsupported => tr!("dialogs-inode-unsupported").to_string(),
        _ => tr!("dialogs-inode-unreadable", error = e.to_string()),
    }
}

/// Plain-language reason for a failed change.
pub fn apply_reason(e: &FsError, touched: InodeFlags) -> String {
    match e.kind {
        FsErrorKind::PermissionDenied if touched.intersects(InodeFlags::IMMUTABLE | InodeFlags::APPEND) => {
            tr!("dialogs-inode-needs-root").to_string()
        }
        FsErrorKind::PermissionDenied => tr!("dialogs-inode-denied").to_string(),
        FsErrorKind::Unsupported => tr!("dialogs-inode-unsupported").to_string(),
        FsErrorKind::ReadOnlyFs => tr!("dialogs-inode-read-only-fs").to_string(),
        _ => e.to_string(),
    }
}
