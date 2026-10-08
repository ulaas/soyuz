//! Settings page for `[editor]`: the built-in text, hex and image editors ("Edit in
//! Soyuz", F4).

use egui::Ui;
use soyuz_core::config::EditorDock;

use super::widgets::{group, number, segmented, switch};
use crate::app::SoyuzApp;

pub fn editor(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft.editor;
    let ctx = "Editor edit in soyuz text hex image F4";
    let mut any = false;
    any |= group(ui, q, ctx, "Panel", |r| {
        r.row_kw(
            "Dock",
            "Where the editor panel opens. It can also be maximized over the file panes.",
            "position layout",
            |ui| {
                segmented(ui, &mut d.dock, &[(EditorDock::Right, "Right"), (EditorDock::Bottom, "Bottom")]);
            },
        );
    });
    any |= group(ui, q, ctx, "Text", |r| {
        r.row_kw("Font size", "Text and hex editor font.", "zoom monospace", |ui| {
            number(ui, &mut d.font_size, 8.0..=32.0, " pt");
        });
        r.row_kw("Tab width", "Columns per tab stop.", "indent spaces", |ui| {
            number(ui, &mut d.tab_width, 1..=16, "");
        });
        r.row_kw("Wrap long lines", "Soft-wrap at the panel edge instead of scrolling sideways.", "word wrap", |ui| {
            switch(ui, &mut d.wrap);
        });
        r.row_kw("Show whitespace", "Draw spaces as dots and tabs as arrows.", "invisible characters", |ui| {
            switch(ui, &mut d.show_whitespace);
        });
        r.row_kw("Largest text file", "Larger files open read-only in the hex editor.", "size limit max", |ui| {
            number(ui, &mut d.max_text_mib, 1..=512, " MiB");
        });
    });
    any |= group(ui, q, ctx, "Saving", |r| {
        r.row_kw(
            "Keep the previous version in the trash",
            "Before a save overwrites a file, a copy (<name>.bak) goes to the trash so you can restore it.",
            "backup undo bak",
            |ui| {
                switch(ui, &mut d.backup_to_trash);
            },
        );
        r.row_kw(
            "JPEG quality",
            "Rotated or cropped JPEGs are re-encoded (there is no lossless path); other formats save losslessly.",
            "image compression",
            |ui| {
                number(ui, &mut d.jpeg_quality, 50..=100, "");
            },
        );
        r.row_kw("Largest image", "Bigger images aren't decoded by the image editor.", "megapixels limit", |ui| {
            number(ui, &mut d.max_image_mp, 10..=1000, " MP");
        });
    });
    any
}
