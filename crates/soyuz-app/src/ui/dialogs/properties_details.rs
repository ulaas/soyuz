//! "Details" tab: file-system identity (link target, hard-link count, file id / inode,
//! device, reparse tag) plus cheap content metadata — image dimensions and EXIF, audio /
//! video header fields, `.lnk` targets — gathered on a worker thread.

use std::path::Path;

use soyuz_core::{EntryInfo, EntryKind};

/// `(section, rows)`.
pub type Sections = Vec<(String, Vec<(String, String)>)>;

fn yes_no(b: bool) -> String {
    if b { "Yes".into() } else { "No".into() }
}

/// Gather the details of one entry (blocking; run off the UI thread).
pub fn load(e: &EntryInfo) -> Sections {
    // A direct stat has fields listings may skip (link count, file id).
    let fresh = soyuz_fs::stat(&e.path, false).ok();
    let e = fresh.as_ref().unwrap_or(e);
    let mut out: Sections = Vec::new();
    let path = &e.path;
    let mut fs = vec![("Full path".to_string(), soyuz_fs::strip_verbatim(path).display().to_string())];
    match e.kind {
        EntryKind::Symlink { .. } | EntryKind::Junction => {
            if let Ok(t) = soyuz_fs::read_link(path) {
                fs.push(("Link target".into(), soyuz_fs::strip_verbatim(&t).display().to_string()));
            }
            match soyuz_fs::resolve_link(path) {
                Ok(t) => fs.push(("Resolves to".into(), soyuz_fs::strip_verbatim(&t).display().to_string())),
                Err(_) => fs.push(("Resolves to".into(), "Target missing".into())),
            }
        }
        _ => {}
    }
    if let Some(n) = e.nlink {
        fs.push(("Hard links".into(), n.to_string()));
    }
    if let Some(i) = e.inode {
        fs.push((if cfg!(windows) { "File ID" } else { "Inode" }.into(), format!("{i} (0x{i:X})")));
    }
    if let Some(d) = e.dev {
        fs.push((if cfg!(windows) { "Volume serial" } else { "Device" }.into(), format!("0x{:X}", d.0)));
    }
    if let Some(t) = e.reparse_tag {
        fs.push(("Reparse tag".into(), format!("0x{t:08X}")));
    }
    if let Some(m) = e.mode {
        fs.push((
            "Mode".into(),
            format!("{} ({})", super::properties_attrs::mode_string(m), super::properties_attrs::mode_to_octal(m)),
        ));
    }
    if let Some(o) = &e.owner {
        fs.push(("Owner".into(), o.to_string()));
    }
    if let Some(g) = &e.group {
        fs.push(("Group".into(), g.to_string()));
    }
    fs.push(("Hidden".into(), yes_no(e.is_hidden())));
    out.push(("File system".into(), fs));

    if e.kind == EntryKind::File {
        let ext = e.extension().map(str::to_ascii_lowercase).unwrap_or_default();
        if ext == "lnk"
            && let Ok(Some(t)) = soyuz_fs::launch::shortcut_target(path)
        {
            out.push(("Shortcut".into(), vec![("Target".into(), soyuz_fs::strip_verbatim(&t).display().to_string())]));
        }
        let mut img = Vec::new();
        if let Ok((w, h)) = image::image_dimensions(path) {
            img.push(("Dimensions".into(), format!("{w} \u{00d7} {h} px")));
            img.push(("Megapixels".into(), format!("{:.1} MP", w as f64 * h as f64 / 1e6)));
        }
        if let Some(x) = crate::rename::exif::read(path) {
            if img.is_empty()
                && let (Some(w), Some(h)) = (x.width, x.height)
            {
                img.push(("Dimensions".into(), format!("{w} \u{00d7} {h} px")));
            }
            img.extend(x.fields());
        }
        if !img.is_empty() {
            out.push(("Image".into(), img));
        }
        if let Some(media) = media(path, e.size, &ext)
            && !media.is_empty()
        {
            out.push(("Media".into(), media));
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
