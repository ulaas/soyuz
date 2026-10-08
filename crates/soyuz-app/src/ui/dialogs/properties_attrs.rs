//! Attribute and permission editing shared by the Properties dialog and the bulk
//! "Attributes & timestamps" dialog: tri-state attribute flags (Windows attributes,
//! immutable / hidden flags on unix), the unix mode grid (rwx × owner/group/other plus
//! setuid / setgid / sticky) with a two-way octal field, and the conversion of the
//! edits into `SetMeta` jobs (grouped by resulting mode, since a job carries one mode).

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Align2, FontId, Margin, RichText, Sense, TextEdit, Ui, pos2, vec2};
use soyuz_core::{Attributes, EntryInfo};
use soyuz_fs::{AttrSet, TimesSet};

use super::properties_widgets::{Tri, caption, tri_check};
use crate::theme::palette;

/// Attribute flags editable on this platform, with labels and hints.
pub fn editable_attrs() -> Vec<(Attributes, &'static str, &'static str)> {
    if cfg!(windows) {
        vec![
            (Attributes::READONLY, "Read-only", "Programs can read but not change the file"),
            (Attributes::HIDDEN, "Hidden", "Not shown unless hidden files are visible"),
            (Attributes::SYSTEM, "System", "Used by the operating system"),
            (Attributes::ARCHIVE, "Archive", "Changed since the last backup"),
            (Attributes::NOT_CONTENT_INDEXED, "Not content indexed", "Excluded from the search index"),
            (Attributes::TEMPORARY, "Temporary", "Kept in memory when possible"),
            (Attributes::OFFLINE, "Offline", "Data is not immediately available"),
            (Attributes::COMPRESSED, "Compressed (NTFS)", "Stored compressed by the file system"),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            (Attributes::IMMUTABLE, "Locked (immutable)", "Cannot be changed, renamed or deleted"),
            (Attributes::HIDDEN, "Hidden", "Hidden in Finder"),
        ]
    } else {
        vec![(Attributes::IMMUTABLE, "Immutable", "Cannot be changed, renamed or deleted (needs root)")]
    }
}

/// Read-only flags shown for information.
pub fn info_attrs() -> Vec<(Attributes, &'static str)> {
    vec![
        (Attributes::ENCRYPTED, "Encrypted"),
        (Attributes::SPARSE, "Sparse"),
        (Attributes::REPARSE, "Reparse point"),
        (Attributes::CLOUD_PLACEHOLDER, "Cloud placeholder"),
    ]
}

/// Tri-state attribute editor over a set of entries.
#[derive(Clone, Debug)]
pub struct AttrEdit {
    pub rows: Vec<(Attributes, Tri, Tri)>,
}

impl AttrEdit {
    pub fn new(entries: &[EntryInfo]) -> Self {
        let n = entries.len().max(1);
        let rows = editable_attrs()
            .into_iter()
            .map(|(a, _, _)| {
                let on = entries.iter().filter(|e| e.attrs.contains(a)).count();
                let t = Tri::from_counts(on, n);
                (a, t, t)
            })
            .collect();
        AttrEdit { rows }
    }

    /// Bits switched on / off relative to the original state (`None` = unchanged).
    pub fn changes(&self) -> Option<AttrSet> {
        let mut set = Attributes::empty();
        let mut clear = Attributes::empty();
        for (a, now, orig) in &self.rows {
            if now == orig {
                continue;
            }
            match now {
                Tri::On => set |= *a,
                Tri::Off => clear |= *a,
                Tri::Mixed => {}
            }
        }
        (!set.is_empty() || !clear.is_empty()).then_some(AttrSet { set, clear })
    }

    pub fn commit(&mut self) {
        for r in &mut self.rows {
            r.2 = r.1;
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let labels = editable_attrs();
        let p = palette(ui.ctx());
        for (row, (_, label, hint)) in self.rows.iter_mut().zip(labels.iter()) {
            ui.horizontal(|ui| {
                let mut t = row.1;
                if tri_check(ui, &mut t, label, true).on_hover_text(*hint).clicked() {
                    row.1 = t;
                }
                if row.1 != row.2 {
                    ui.label(RichText::new("changed").font(FontId::proportional(10.5)).color(p.accent));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(*hint).font(FontId::proportional(11.0)).color(p.text_tertiary));
                });
            });
        }
    }
}

// ---------------------------------------------------------------------------------------
// Unix mode
// ---------------------------------------------------------------------------------------

/// Mode bits in grid order: setuid, setgid, sticky, then r/w/x for owner, group, other.
pub const MODE_BITS: [u32; 12] = [0o4000, 0o2000, 0o1000, 0o400, 0o200, 0o100, 0o40, 0o20, 0o10, 0o4, 0o2, 0o1];

/// `0755` style octal text (4 digits when a special bit is set, else 3).
pub fn mode_to_octal(mode: u32) -> String {
    let m = mode & 0o7777;
    if m & 0o7000 != 0 { format!("{m:04o}") } else { format!("{m:03o}") }
}

/// Parse `755`, `0755`, `4755` (1–4 octal digits).
pub fn parse_octal(s: &str) -> Option<u32> {
    let s = s.trim();
    let s = s.strip_prefix("0o").unwrap_or(s);
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return None;
    }
    u32::from_str_radix(s, 8).ok()
}

/// `rwxr-sr-t` symbolic form (with `s`/`S`/`t`/`T` for the special bits).
pub fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    let trip = [(0o400, 0o200, 0o100, 0o4000, 's'), (0o40, 0o20, 0o10, 0o2000, 's'), (0o4, 0o2, 0o1, 0o1000, 't')];
    for (r, w, x, special, ch) in trip {
        s.push(if mode & r != 0 { 'r' } else { '-' });
        s.push(if mode & w != 0 { 'w' } else { '-' });
        s.push(match (mode & x != 0, mode & special != 0) {
            (true, true) => ch,
            (false, true) => ch.to_ascii_uppercase(),
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    s
}

/// Tri-state mode editor over a set of entries.
#[derive(Clone, Debug)]
pub struct ModeEdit {
    pub bits: [Tri; 12],
    orig: [Tri; 12],
    pub octal: String,
    octal_focus: bool,
}

impl ModeEdit {
    /// `None` when no entry has a mode (Windows).
    pub fn new(entries: &[EntryInfo]) -> Option<Self> {
        let modes: Vec<u32> = entries.iter().filter_map(|e| e.mode).collect();
        if modes.is_empty() {
            return None;
        }
        let mut bits = [Tri::Off; 12];
        for (i, b) in MODE_BITS.iter().enumerate() {
            bits[i] = Tri::from_counts(modes.iter().filter(|m| *m & b != 0).count(), modes.len());
        }
        let mut e = ModeEdit { bits, orig: bits, octal: String::new(), octal_focus: false };
        e.sync_octal();
        Some(e)
    }

    /// The mode if no bit is mixed.
    pub fn uniform(&self) -> Option<u32> {
        let mut m = 0;
        for (i, t) in self.bits.iter().enumerate() {
            match t {
                Tri::On => m |= MODE_BITS[i],
                Tri::Off => {}
                Tri::Mixed => return None,
            }
        }
        Some(m)
    }

    fn sync_octal(&mut self) {
        self.octal = self.uniform().map(mode_to_octal).unwrap_or_default();
    }

    pub fn changed(&self) -> bool {
        self.bits != self.orig
    }

    pub fn commit(&mut self) {
        self.orig = self.bits;
    }

    /// New mode for an entry whose current mode is `old` (`None` = unchanged).
    pub fn apply_to(&self, old: u32) -> Option<u32> {
        let mut m = old & 0o7777;
        for (i, (now, orig)) in self.bits.iter().zip(&self.orig).enumerate() {
            if now == orig {
                continue;
            }
            match now {
                Tri::On => m |= MODE_BITS[i],
                Tri::Off => m &= !MODE_BITS[i],
                Tri::Mixed => {}
            }
        }
        (m != old & 0o7777).then_some(m | (old & !0o7777))
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        caption(ui, "Permissions");
        let col_w = 64.0;
        let label_w = 70.0;
        let (head, _) = ui.allocate_exact_size(vec2(label_w + col_w * 3.0, 18.0), Sense::hover());
        for (i, h) in ["Read", "Write", "Execute"].iter().enumerate() {
            ui.painter().text(
                pos2(head.left() + label_w + col_w * i as f32 + 8.0, head.center().y),
                Align2::LEFT_CENTER,
                *h,
                FontId::proportional(11.0),
                p.text_tertiary,
            );
        }
        let mut changed = false;
        for (row, who) in ["Owner", "Group", "Others"].iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (r, _) = ui.allocate_exact_size(vec2(label_w, 22.0), Sense::hover());
                ui.painter().text(
                    pos2(r.left(), r.center().y),
                    Align2::LEFT_CENTER,
                    *who,
                    FontId::proportional(12.5),
                    p.text_secondary,
                );
                for col in 0..3 {
                    let idx = 3 + row * 3 + col;
                    ui.allocate_ui(vec2(col_w, 22.0), |ui| {
                        let mut t = self.bits[idx];
                        if tri_check(ui, &mut t, "", true).clicked() {
                            self.bits[idx] = t;
                            changed = true;
                        }
                    });
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (i, label) in [(0, "Set user ID"), (1, "Set group ID"), (2, "Sticky")] {
                let mut t = self.bits[i];
                if tri_check(ui, &mut t, label, true).clicked() {
                    self.bits[i] = t;
                    changed = true;
                }
                ui.add_space(10.0);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Octal").color(p.text_tertiary));
            let r = ui.add(
                TextEdit::singleline(&mut self.octal)
                    .desired_width(64.0)
                    .font(FontId::monospace(12.5))
                    .hint_text("mixed")
                    .margin(Margin::symmetric(8, 4)),
            );
            self.octal_focus = r.has_focus();
            if r.changed()
                && let Some(m) = parse_octal(&self.octal)
            {
                for (i, b) in MODE_BITS.iter().enumerate() {
                    self.bits[i] = if m & b != 0 { Tri::On } else { Tri::Off };
                }
            }
            if let Some(m) = self.uniform() {
                ui.label(RichText::new(mode_string(m)).font(FontId::monospace(12.5)).color(p.text_secondary));
            }
            if self.changed() {
                ui.label(RichText::new("changed").font(FontId::proportional(10.5)).color(p.accent));
            }
        });
        if changed && !self.octal_focus {
            self.sync_octal();
        }
    }
}

/// One `SetMeta` job.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaJob {
    pub paths: Vec<PathBuf>,
    pub times: Option<TimesSet>,
    pub attrs: Option<AttrSet>,
    pub mode: Option<u32>,
}

/// Turn edits into jobs: one per resulting mode (a job carries a single mode), plus one
/// for entries whose mode does not change. Empty when nothing changed.
pub fn plan_jobs(
    entries: &[EntryInfo],
    times: Option<TimesSet>,
    attrs: Option<AttrSet>,
    mode: Option<&ModeEdit>,
) -> Vec<MetaJob> {
    let mut by_mode: BTreeMap<Option<u32>, Vec<PathBuf>> = BTreeMap::new();
    for e in entries {
        let m = match (mode, e.mode) {
            (Some(me), Some(old)) if me.changed() => me.apply_to(old),
            _ => None,
        };
        by_mode.entry(m).or_default().push(e.path.clone());
    }
    by_mode
        .into_iter()
        .filter(|(m, _)| m.is_some() || times.is_some() || attrs.is_some())
        .map(|(mode, paths)| MetaJob { paths, times, attrs, mode })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::EntryKind;

    fn entry(p: &str, mode: Option<u32>, attrs: Attributes) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from(p), EntryKind::File);
        e.mode = mode;
        e.attrs = attrs;
        e
    }

    #[test]
    fn octal_conversion() {
        assert_eq!(mode_to_octal(0o100755), "755");
        assert_eq!(mode_to_octal(0o4755), "4755");
        assert_eq!(mode_to_octal(0o1777), "1777");
        assert_eq!(parse_octal("755"), Some(0o755));
        assert_eq!(parse_octal("0755"), Some(0o755));
        assert_eq!(parse_octal("0o644"), Some(0o644));
        assert_eq!(parse_octal("4755"), Some(0o4755));
        assert_eq!(parse_octal("7"), Some(0o7));
        assert_eq!(parse_octal("789"), None);
        assert_eq!(parse_octal("17777"), None);
        assert_eq!(parse_octal(""), None);
        for m in [0o644, 0o755, 0o4711, 0o2750, 0o1777, 0o0] {
            assert_eq!(parse_octal(&mode_to_octal(m)), Some(m));
        }
    }

    #[test]
    fn symbolic_mode() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o644), "rw-r--r--");
        assert_eq!(mode_string(0o4755), "rwsr-xr-x");
        assert_eq!(mode_string(0o4644), "rwSr--r--");
        assert_eq!(mode_string(0o2750), "rwxr-s---");
        assert_eq!(mode_string(0o1777), "rwxrwxrwt");
        assert_eq!(mode_string(0o1776), "rwxrwxrwT");
    }

    #[test]
    fn mode_edit_tri_state() {
        let es = [entry("/a", Some(0o100644), Attributes::empty()), entry("/b", Some(0o100755), Attributes::empty())];
        let mut me = ModeEdit::new(&es).unwrap();
        assert_eq!(me.uniform(), None, "owner/group/other x differ");
        assert_eq!(me.octal, "");
        assert!(!me.changed());
        // Turn group write on for everything; the mixed execute bits stay per file.
        me.bits[7] = Tri::On;
        assert!(me.changed());
        assert_eq!(me.apply_to(0o100644), Some(0o100664));
        assert_eq!(me.apply_to(0o100755), Some(0o100775));
        let jobs = plan_jobs(&es, None, None, Some(&me));
        assert_eq!(jobs.len(), 2, "one job per resulting mode");
        assert!(jobs.iter().any(|j| j.mode == Some(0o100664) && j.paths == [PathBuf::from("/a")]));
        me.commit();
        assert!(!me.changed());
        assert_eq!(me.apply_to(0o644), None);
        assert!(ModeEdit::new(&[entry("/w", None, Attributes::empty())]).is_none());
    }

    #[test]
    fn attr_changes_and_plan() {
        let es = [entry("/a", None, Attributes::HIDDEN), entry("/b", None, Attributes::empty())];
        let mut ae = AttrEdit::new(&es);
        assert!(ae.changes().is_none());
        let first = ae.rows[0].0;
        for r in &mut ae.rows {
            if r.0 == first {
                r.1 = Tri::On;
            }
        }
        let c = ae.changes().unwrap();
        assert_eq!(c.set, first);
        assert!(c.clear.is_empty());
        let jobs = plan_jobs(&es, None, Some(c), None);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].paths.len(), 2);
        assert!(plan_jobs(&es, None, None, None).is_empty());
        let t = TimesSet { modified: Some(std::time::SystemTime::now()), ..TimesSet::default() };
        assert_eq!(plan_jobs(&es, Some(t), None, None)[0].times, Some(t));
        ae.commit();
        assert!(ae.changes().is_none());
    }
}
