//! "Security" tab: owner / group and a read-only view of the access-control entries
//! (`soyuz_fs::security_info`, `effective_access`), loaded on a worker thread. On unix
//! the owner and group can be changed (`chown`, applied by the dialog's Apply) when the
//! user is permitted; names are resolved through `/etc/passwd` and `/etc/group`.

use egui::{Align, CornerRadius, FontId, Layout, Margin, RichText, TextEdit, Ui};
use soyuz_fs::{AccessRights, SecurityInfo};

use super::properties_widgets::{badge, caption, kv_row, kv_row_with};
use crate::theme::{icons::ph, palette};

pub type Loaded = Result<(SecurityInfo, Option<AccessRights>), String>;

/// Blocking load (worker thread).
pub fn load(path: &std::path::Path) -> Loaded {
    let info = soyuz_fs::security_info(path).map_err(|e| e.to_string())?;
    let eff = soyuz_fs::effective_access(path).ok();
    Ok((info, eff))
}

pub fn rights_labels(r: AccessRights) -> Vec<&'static str> {
    if r.contains(AccessRights::FULL) {
        return vec![tr!("dialogs-sec-full-control")];
    }
    let mut v = Vec::new();
    for (bit, l) in [
        (AccessRights::READ, tr!("dialogs-mode-read")),
        (AccessRights::WRITE, tr!("dialogs-mode-write")),
        (AccessRights::EXECUTE, tr!("dialogs-mode-execute")),
        (AccessRights::DELETE, tr!("common-delete")),
    ] {
        if r.contains(bit) {
            v.push(l);
        }
    }
    if v.is_empty() {
        v.push(tr!("dialogs-sec-special"));
    }
    v
}

/// `(uid, gid)` to set (`None` = keep).
pub type IdChange = (Option<u32>, Option<u32>);

/// Unix owner / group edit state.
#[derive(Clone, Debug, Default)]
pub struct OwnerEdit {
    pub owner: String,
    pub group: String,
    orig_owner: String,
    orig_group: String,
}

impl OwnerEdit {
    pub fn new(owner: Option<&str>, group: Option<&str>) -> Self {
        let o = owner.unwrap_or_default().to_string();
        let g = group.unwrap_or_default().to_string();
        OwnerEdit { owner: o.clone(), group: g.clone(), orig_owner: o, orig_group: g }
    }

    /// `(uid, gid)` to apply; `Err` when a name cannot be resolved.
    pub fn changes(&self) -> Result<Option<IdChange>, String> {
        let o = self.owner.trim();
        let g = self.group.trim();
        let uid = if o != self.orig_owner && !o.is_empty() {
            Some(resolve_id(o, "/etc/passwd").ok_or_else(|| tr!("dialogs-sec-unknown-user", name = o))?)
        } else {
            None
        };
        let gid = if g != self.orig_group && !g.is_empty() {
            Some(resolve_id(g, "/etc/group").ok_or_else(|| tr!("dialogs-sec-unknown-group", name = g))?)
        } else {
            None
        };
        Ok((uid.is_some() || gid.is_some()).then_some((uid, gid)))
    }

    pub fn commit(&mut self) {
        self.orig_owner = self.owner.trim().to_string();
        self.orig_group = self.group.trim().to_string();
    }
}

/// A numeric id, or the id of `name` in a passwd/group style file.
pub fn resolve_id(name: &str, db: &str) -> Option<u32> {
    if let Ok(n) = name.parse::<u32>() {
        return Some(n);
    }
    let text = std::fs::read_to_string(db).ok()?;
    lookup_id(&text, name)
}

/// Find `name` in `name:x:id:...` lines.
pub fn lookup_id(db: &str, name: &str) -> Option<u32> {
    db.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let mut f = l.split(':');
        (f.next()? == name).then_some(())?;
        f.next()?;
        f.next()?.parse().ok()
    })
}

pub fn show(ui: &mut Ui, loaded: Option<&Loaded>, owner: Option<&mut OwnerEdit>) {
    let p = palette(ui.ctx());
    if let Some(edit) = owner {
        caption(ui, tr!("dialogs-sec-ownership"));
        for (label, text) in
            [(tr!("dialogs-mode-owner"), &mut edit.owner), (tr!("dialogs-mode-group"), &mut edit.group)]
        {
            kv_row_with(ui, label, |ui| {
                ui.add(TextEdit::singleline(text).desired_width(200.0).margin(Margin::symmetric(8, 4)));
            });
        }
        match edit.changes() {
            Err(e) => {
                ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.danger));
            }
            Ok(Some(_)) => {
                ui.label(
                    RichText::new(tr!("dialogs-sec-owner-admin"))
                        .font(FontId::proportional(11.5))
                        .color(p.text_tertiary),
                );
            }
            Ok(None) => {}
        }
    }
    let Some(loaded) = loaded else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(tr!("dialogs-sec-reading")).color(p.text_tertiary));
        });
        return;
    };
    match loaded {
        Err(e) => {
            badge(ui, Some(ph::WARNING), tr!("dialogs-sec-unavailable"), p.warning);
            ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.text_tertiary));
        }
        Ok((info, eff)) => {
            caption(ui, tr!("dialogs-mode-owner"));
            kv_row(ui, tr!("dialogs-mode-owner"), &info.owner);
            if let Some(g) = &info.group {
                kv_row(ui, tr!("dialogs-mode-group"), g);
            }
            if let Some(eff) = eff {
                kv_row_with(ui, tr!("dialogs-sec-your-access"), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for l in rights_labels(*eff) {
                            badge(ui, None, l, p.accent);
                        }
                    });
                });
            }
            caption(ui, tr!("dialogs-sec-entries"));
            egui::Frame::new()
                .fill(p.bg_surface2)
                .corner_radius(CornerRadius::same(crate::theme::tokens::radius::CARD))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if info.entries.is_empty() {
                        ui.label(RichText::new(tr!("dialogs-sec-no-entries")).color(p.text_tertiary));
                    }
                    egui::ScrollArea::vertical().max_height(190.0).show(ui, |ui| {
                        for ace in &info.entries {
                            ui.horizontal(|ui| {
                                let (g, c) =
                                    if ace.allow { (ph::CHECK_CIRCLE, p.success) } else { (ph::PROHIBIT, p.danger) };
                                ui.label(crate::theme::icons::icon(g, 14.0).color(c));
                                // Reserve room for the right-hand badges before the name.
                                let labels = rights_labels(ace.rights);
                                let badges_w: f32 = labels
                                    .iter()
                                    .map(|l| {
                                        ui.painter()
                                            .layout_no_wrap(l.to_string(), FontId::proportional(11.0), p.text_primary)
                                            .size()
                                            .x
                                            + 22.0
                                    })
                                    .sum::<f32>()
                                    + if ace.inherited {
                                        ui.painter()
                                            .layout_no_wrap(
                                                tr!("dialogs-sec-inherited").to_string(),
                                                FontId::proportional(10.5),
                                                p.text_tertiary,
                                            )
                                            .size()
                                            .x
                                            + 12.0
                                    } else {
                                        0.0
                                    };
                                let name_w = (ui.available_width() - badges_w - 8.0).max(60.0);
                                ui.allocate_ui(egui::vec2(name_w, 20.0), |ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(&ace.principal).color(p.text_primary))
                                            .truncate(),
                                    )
                                    .on_hover_text(&ace.principal);
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ace.inherited {
                                        ui.label(
                                            RichText::new(tr!("dialogs-sec-inherited"))
                                                .font(FontId::proportional(10.5))
                                                .color(p.text_tertiary),
                                        );
                                    }
                                    for l in rights_labels(ace.rights).into_iter().rev() {
                                        badge(ui, None, l, if ace.allow { p.text_secondary } else { p.danger });
                                    }
                                });
                            });
                        }
                    });
                });
            if cfg!(windows) {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(tr!("dialogs-sec-read-only-view"))
                        .font(FontId::proportional(11.0))
                        .color(p.text_tertiary),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_lookup() {
        let db = "# comment\nroot:x:0:0:root:/root:/bin/sh\nalice:x:1000:1000::/home/alice:/bin/bash\nstaff:x:20:\n";
        assert_eq!(lookup_id(db, "root"), Some(0));
        assert_eq!(lookup_id(db, "alice"), Some(1000));
        assert_eq!(lookup_id(db, "staff"), Some(20));
        assert_eq!(lookup_id(db, "bob"), None);
        assert_eq!(resolve_id("42", "/nonexistent"), Some(42));
        assert_eq!(rights_labels(AccessRights::READ | AccessRights::EXECUTE), ["Read", "Execute"]);
        assert_eq!(rights_labels(AccessRights::FULL | AccessRights::READ), ["Full control"]);
        let mut o = OwnerEdit::new(Some("alice"), Some("staff"));
        assert_eq!(o.changes(), Ok(None));
        o.owner = "7".into();
        assert_eq!(o.changes(), Ok(Some((Some(7), None))));
        o.commit();
        assert_eq!(o.changes(), Ok(None));
    }
}
