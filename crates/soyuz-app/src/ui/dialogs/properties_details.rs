//! "Details" tab: file-system identity (link target, hard-link count, file id / inode,
//! device, reparse tag) plus cheap content metadata — image dimensions and EXIF, audio /
//! video header fields, `.lnk` targets — gathered on a worker thread.

use std::path::Path;

use soyuz_core::{EntryInfo, EntryKind};

/// `(section, rows)`.
pub type Sections = Vec<(String, Vec<(String, String)>)>;

fn yes_no(b: bool) -> String {
    if b { tr!("common-yes").into() } else { tr!("common-no").into() }
}

/// Gather the details of one entry (blocking; run off the UI thread).
pub fn load(e: &EntryInfo) -> Sections {
    // A direct stat has fields listings may skip (link count, file id).
    let fresh = soyuz_fs::stat(&e.path, false).ok();
    let e = fresh.as_ref().unwrap_or(e);
    let mut out: Sections = Vec::new();
    let path = &e.path;
    let mut fs =
        vec![(tr!("dialogs-details-full-path").to_string(), soyuz_fs::strip_verbatim(path).display().to_string())];
    match e.kind {
        EntryKind::Symlink { .. } | EntryKind::Junction => {
            if let Ok(t) = soyuz_fs::read_link(path) {
                fs.push((tr!("dialogs-props-link-target").into(), soyuz_fs::strip_verbatim(&t).display().to_string()));
            }
            match soyuz_fs::resolve_link(path) {
                Ok(t) => fs.push((
                    tr!("dialogs-details-resolves-to").into(),
                    soyuz_fs::strip_verbatim(&t).display().to_string(),
                )),
                Err(_) => {
                    fs.push((tr!("dialogs-details-resolves-to").into(), tr!("dialogs-details-target-missing").into()))
                }
            }
        }
        _ => {}
    }
    if let Some(n) = e.nlink {
        fs.push((tr!("dialogs-details-hard-links").into(), n.to_string()));
    }
    if let Some(i) = e.inode {
        let label = if cfg!(windows) { tr!("dialogs-details-file-id") } else { tr!("dialogs-details-inode") };
        fs.push((label.into(), format!("{i} (0x{i:X})")));
    }
    if let Some(d) = e.dev {
        let label = if cfg!(windows) { tr!("dialogs-details-volume-serial") } else { tr!("dialogs-details-device") };
        fs.push((label.into(), format!("0x{:X}", d.0)));
    }
    if let Some(t) = e.reparse_tag {
        fs.push((tr!("dialogs-details-reparse-tag").into(), format!("0x{t:08X}")));
    }
    if let Some(m) = e.mode {
        fs.push((
            tr!("dialogs-details-mode").into(),
            format!("{} ({})", super::properties_attrs::mode_string(m), super::properties_attrs::mode_to_octal(m)),
        ));
    }
    if let Some(o) = &e.owner {
        fs.push((tr!("dialogs-mode-owner").into(), o.to_string()));
    }
    if let Some(g) = &e.group {
        fs.push((tr!("dialogs-mode-group").into(), g.to_string()));
    }
    fs.push((tr!("dialogs-attr-hidden").into(), yes_no(e.is_hidden())));
    out.push((tr!("dialogs-details-file-system").into(), fs));

    if e.kind == EntryKind::File {
        let ext = e.extension().map(str::to_ascii_lowercase).unwrap_or_default();
        if ext == "lnk"
            && let Ok(Some(t)) = soyuz_fs::launch::shortcut_target(path)
        {
            let target = soyuz_fs::strip_verbatim(&t).display().to_string();
            out.push((tr!("dialogs-details-shortcut").into(), vec![(tr!("dialogs-details-target").into(), target)]));
        }
        let mut img = Vec::new();
        if let Ok((w, h)) = image::image_dimensions(path) {
            img.push((tr!("dialogs-details-dimensions").into(), format!("{w} \u{00d7} {h} px")));
            let mp = soyuz_core::locale::format_decimal(w as f64 * h as f64 / 1e6, 1);
            img.push((tr!("dialogs-details-megapixels").into(), format!("{mp} MP")));
        }
        if let Some(x) = crate::rename::exif::read(path) {
            if img.is_empty()
                && let (Some(w), Some(h)) = (x.width, x.height)
            {
                img.push((tr!("dialogs-details-dimensions").into(), format!("{w} \u{00d7} {h} px")));
            }
            img.extend(x.fields());
        }
        if !img.is_empty() {
            out.push((tr!("dialogs-details-image").into(), img));
        }
        if let Some(media) = media(path, e.size, &ext)
            && !media.is_empty()
        {
            out.push((tr!("dialogs-details-media").into(), media));
        }
    }
    out
}

fn media(path: &Path, len: u64, ext: &str) -> Option<Vec<(String, String)>> {
    use crate::ui::views::icons::{FileClass, classify_ext};
    if !matches!(classify_ext(ext), FileClass::Audio | FileClass::Video) {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    Some(crate::services::preview_media::probe(&mut f, len, ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_for_a_jpeg_with_exif() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.jpg");
        std::fs::write(&p, crate::rename::exif::tests::sample_jpeg()).unwrap();
        let e = soyuz_fs::stat(&p, false).unwrap();
        let s = load(&e);
        assert_eq!(s[0].0, "File system");
        let img = s.iter().find(|(n, _)| n == "Image").expect("image section");
        assert!(img.1.iter().any(|(k, v)| k == "Camera" && v == "Canon EOS R5"), "{img:?}");
    }
}
