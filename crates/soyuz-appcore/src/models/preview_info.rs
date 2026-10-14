//! The preview's info section as data: rows of `(key, value)` for an entry (type, size,
//! dates, attributes, owner, plus what the loader found), their translated labels, and
//! the attribute / mode strings. Both frontends draw these rows.

use std::time::SystemTime;

use soyuz_core::config::Config;
use soyuz_core::{Attributes, EntryInfo, format_size, format_time};

use crate::services::preview::{Kind, Loaded, Source};

/// `RHSA…` attribute letters.
pub fn attr_letters(a: Attributes) -> String {
    [
        (Attributes::READONLY, 'R'),
        (Attributes::HIDDEN, 'H'),
        (Attributes::SYSTEM, 'S'),
        (Attributes::ARCHIVE, 'A'),
        (Attributes::COMPRESSED, 'C'),
        (Attributes::ENCRYPTED, 'E'),
        (Attributes::REPARSE, 'L'),
        (Attributes::OFFLINE, 'O'),
        (Attributes::CLOUD_PLACEHOLDER, 'P'),
    ]
    .into_iter()
    .filter(|(f, _)| a.contains(*f))
    .map(|(_, c)| c)
    .collect()
}

/// `rwxr-xr-x` from a unix mode.
pub fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// Info rows for the selection: type, size, dates, attributes, owner + loader facts.
pub fn info_rows(
    entry: &EntryInfo,
    loaded: Option<&Loaded>,
    folder_size: Option<(u64, u64, u64, bool)>,
    cfg: &Config,
) -> Vec<(String, String)> {
    let style = cfg.general.size_style;
    let fmt_t = |t: SystemTime| format_time(t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Long));
    let meta = loaded.and_then(|l| l.meta.as_ref());
    let mut rows = vec![("Type".to_string(), crate::state::listing::kind_label(entry))];
    if entry.is_dir_like() {
        if let Some((bytes, files, dirs, done)) = folder_size {
            let v = format!("{}{}", format_size(bytes, style), if done { "" } else { "…" });
            rows.push(("Size".into(), v));
            rows.push(("Contains".into(), tr!("views-info-contains", files = files, dirs = dirs)));
        }
    } else {
        let size = if entry.size == 0 { meta.map_or(0, |m| m.size) } else { entry.size };
        rows.push(("Size".into(), tr!("views-info-size-bytes", size = format_size(size, style), bytes = size)));
        if let Some(a) = entry.alloc_size.filter(|&a| a != entry.size && Source::from_path(&entry.path).is_entry()) {
            rows.push(("Compressed".into(), format_size(a, style)));
        }
    }
    let parent = match Source::from_path(&entry.path) {
        Source::Entry { archive, inner } => {
            let dir = inner.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            format!("{}!/{dir}", soyuz_fs::strip_verbatim(&archive).display())
        }
        Source::File(p) => p.parent().map(|d| soyuz_fs::strip_verbatim(d).display().to_string()).unwrap_or_default(),
    };
    rows.push(("Where".into(), parent));
    let pick = |a: Option<SystemTime>, b: Option<SystemTime>| a.or(b);
    if let Some(t) = pick(entry.modified, meta.and_then(|m| m.modified)) {
        rows.push(("Modified".into(), fmt_t(t)));
    }
    if let Some(t) = pick(entry.created, meta.and_then(|m| m.created)) {
        rows.push(("Created".into(), fmt_t(t)));
    }
    if let Some(t) = pick(entry.accessed, meta.and_then(|m| m.accessed)) {
        rows.push(("Accessed".into(), fmt_t(t)));
    }
    let attrs = if entry.attrs.is_empty() { meta.map(|m| m.attrs).unwrap_or_default() } else { entry.attrs };
    if !attrs.is_empty() {
        rows.push(("Attributes".into(), attr_letters(attrs)));
    }
    if let Some(m) = entry.mode.or(meta.and_then(|m| m.mode)) {
        rows.push(("Mode".into(), format!("{} ({:o})", mode_string(m), m & 0o7777)));
    }
    if let Some(o) = entry.owner.as_deref().or(meta.and_then(|m| m.owner.as_deref())) {
        rows.push(("Owner".into(), o.to_string()));
    }
    if let Some(l) = loaded {
        for (k, v) in &l.facts {
            if !rows.iter().any(|(rk, _)| rk == k) {
                rows.push((k.clone(), v.clone()));
            }
        }
    }
    rows
}

/// `1234567` → `1,234,567` (`1.234.567` in Turkish).
pub fn group_digits(n: u64) -> String {
    soyuz_core::locale::group_int(n)
}

/// Translated label of an info row key. [`info_rows`] and the preview loaders key rows
/// by their English name (rows are deduplicated and looked up by it); unknown keys are
/// shown as they are.
pub fn info_label(key: &str) -> &str {
    match key {
        "Type" => tr!("views-info-type"),
        "Size" => tr!("views-info-size"),
        "Contains" => tr!("views-info-contains-label"),
        "Compressed" => tr!("views-info-compressed"),
        "Where" => tr!("views-info-where"),
        "Modified" => tr!("views-info-modified"),
        "Created" => tr!("views-info-created"),
        "Accessed" => tr!("views-info-accessed"),
        "Attributes" => tr!("views-info-attributes"),
        "Mode" => tr!("views-info-mode"),
        "Owner" => tr!("views-info-owner"),
        "Title" => tr!("views-info-title"),
        "Artist" => tr!("views-info-artist"),
        "Album" => tr!("views-info-album"),
        "Album artist" => tr!("views-info-album-artist"),
        "Year" => tr!("views-info-year"),
        "Genre" => tr!("views-info-genre"),
        "Track" => tr!("views-info-track"),
        "Composer" => tr!("views-info-composer"),
        "Container" => tr!("views-info-container"),
        "Format" => tr!("views-info-format"),
        "Sample rate" => tr!("views-info-sample-rate"),
        "Channels" => tr!("views-info-channels"),
        "Bit depth" => tr!("views-info-bit-depth"),
        "Duration" => tr!("views-info-duration"),
        "Dimensions" => tr!("views-info-dimensions"),
        "Frame rate" => tr!("views-info-frame-rate"),
        "Bitrate" => tr!("views-info-bitrate"),
        "Tracks" => tr!("views-info-tracks"),
        "Color" => tr!("views-info-color"),
        "Compression" => tr!("views-info-compression"),
        "Encoding" => tr!("views-info-encoding"),
        "Encrypted" => tr!("views-info-encrypted"),
        "Entries" => tr!("views-info-entries"),
        "Language" => tr!("views-info-language"),
        "Lines" => tr!("views-info-lines"),
        "Note" => tr!("views-info-note"),
        "PDF version" => tr!("views-info-pdf-version"),
        "Previewer" => tr!("views-info-previewer"),
        "Pages" => tr!("views-info-pages"),
        "Author" => tr!("views-info-author"),
        "Producer" => tr!("views-info-producer"),
        "Publisher" => tr!("views-info-publisher"),
        "Published" => tr!("views-info-published"),
        "Version" => tr!("views-info-version"),
        "EPUB version" => tr!("views-info-epub-version"),
        "Page size" => tr!("views-info-page-size"),
        "Paragraphs" => tr!("views-info-paragraphs"),
        "Codec" => tr!("views-info-codec"),
        "Copyright" => tr!("views-info-copyright"),
        "Designer" => tr!("views-info-designer"),
        "Weight" => tr!("views-info-weight"),
        "Spacing" => tr!("views-info-spacing"),
        "Variable axes" => tr!("views-info-variable-axes"),
        "Bounds" => tr!("views-info-bounds"),
        "Faces" => tr!("views-info-faces"),
        "Meshes" => tr!("views-info-meshes"),
        "Drawn" => tr!("views-info-drawn"),
        "Shown" => tr!("views-info-shown"),
        "Renderer" => tr!("views-info-renderer"),
        "Waveform" => tr!("views-info-waveform"),
        "Video" => tr!("views-info-video"),
        "Audio" => tr!("views-info-audio"),
        "Subtitles" => tr!("views-info-subtitles"),
        "Pixel format" => tr!("views-info-pixel-format"),
        "HDR" => tr!("views-info-hdr"),
        "Rotation" => tr!("views-info-rotation"),
        "Video bitrate" => tr!("views-info-video-bitrate"),
        "Audio bitrate" => tr!("views-info-audio-bitrate"),
        "Comment" => tr!("views-info-comment"),
        "Encoder" => tr!("views-info-encoder"),
        "Words" => tr!("views-info-words"),
        "Sheets" => tr!("views-info-sheets"),
        "Slides" => tr!("views-info-slides"),
        "Chapters" => tr!("views-info-chapters"),
        "Family" => tr!("views-info-family"),
        "Style" => tr!("views-info-style"),
        "Glyphs" => tr!("views-info-glyphs"),
        "Triangles" => tr!("views-info-triangles"),
        "Vertices" => tr!("views-info-vertices"),
        other => other,
    }
}

/// Translated label of a preview category (`Kind::label` stays English for logs).
pub fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Folder => tr!("views-preview-kind-folder"),
        Kind::Image => tr!("views-preview-kind-image"),
        Kind::Svg => tr!("views-preview-kind-svg"),
        Kind::Text => tr!("views-preview-kind-text"),
        Kind::Markdown => "Markdown",
        Kind::Archive => tr!("views-preview-kind-archive"),
        Kind::Audio => tr!("views-preview-kind-audio"),
        Kind::Video => tr!("views-preview-kind-video"),
        Kind::Pdf => tr!("views-preview-kind-pdf"),
        Kind::Binary => tr!("views-preview-kind-binary"),
        Kind::Document => tr!("views-preview-kind-document"),
        Kind::Spreadsheet => tr!("views-preview-kind-spreadsheet"),
        Kind::Presentation => tr!("views-preview-kind-presentation"),
        Kind::Ebook => tr!("views-preview-kind-ebook"),
        Kind::Font => tr!("views-preview-kind-font"),
        Kind::Model => tr!("views-preview-kind-model"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use soyuz_core::EntryKind;

    use super::*;

    #[test]
    fn digit_grouping() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }

    #[test]
    fn multi_plural_messages_format_on_one_line() {
        use crate::i18n::{FluentArgs, t_args_in};
        use soyuz_core::Locale;
        let mut a = FluentArgs::new();
        a.set("dirs", 2);
        a.set("files", 1);
        assert_eq!(t_args_in("views-preview-folder-counts", &a, Locale::EnUs), "2 folders · 1 file");
        assert_eq!(t_args_in("views-preview-folder-counts", &a, Locale::TrTr), "2 klasör · 1 dosya");
        let mut a = FluentArgs::new();
        a.set("files", 1234);
        a.set("folders", 1);
        assert_eq!(t_args_in("views-index-idle", &a, Locale::EnUs), "Indexed · 1,234 files, 1 folder");
        assert_eq!(t_args_in("views-index-idle", &a, Locale::TrTr), "Dizine eklendi · 1.234 dosya, 1 klasör");
    }

    #[test]
    fn info_rows_for_file_and_folder() {
        let cfg = Config::default();
        let mut e = EntryInfo::minimal(PathBuf::from("/data/report.pdf"), EntryKind::File);
        e.size = 2048;
        e.attrs = Attributes::READONLY;
        e.mode = Some(0o100644);
        e.modified = Some(SystemTime::UNIX_EPOCH);
        let rows = info_rows(&e, None, None, &cfg);
        let get = |k: &str| rows.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        assert!(get("Size").unwrap().contains("2,048 bytes"));
        assert!(get("Mode").unwrap().contains("644"));
        assert!(get("Attributes").is_some());
        assert!(get("Modified").is_some());
        assert_eq!(get("Type").unwrap(), crate::state::listing::kind_label(&e));

        let d = EntryInfo::minimal(PathBuf::from("/data/sub"), EntryKind::Dir);
        let rows = info_rows(&d, None, Some((4096, 3, 1, false)), &cfg);
        let size = rows.iter().find(|(a, _)| a == "Size").unwrap();
        assert!(size.1.ends_with('…'), "still computing");
        assert!(rows.iter().any(|(a, v)| a == "Contains" && v == "3 files, 1 folder"));

        let a = EntryInfo::minimal(PathBuf::from("/x/a.zip!/dir/f.txt"), EntryKind::File);
        let rows = info_rows(&a, None, None, &cfg);
        assert!(rows.iter().any(|(k, v)| k == "Where" && v.ends_with("a.zip!/dir")));
    }
}
