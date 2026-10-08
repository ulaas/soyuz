//! File-type glyphs and colors (PLAN §1.2 `glyph.*` tokens).

use egui::Color32;
use soyuz_core::{EntryInfo, EntryKind, LinkTarget};

use crate::theme::Palette;
use crate::theme::icons::ph;

/// Broad file category used for glyph + color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileClass {
    Folder,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    Pdf,
    Doc,
    Sheet,
    Slides,
    Text,
    Exe,
    Font,
    Link,
    Other,
}

pub fn classify(e: &EntryInfo) -> FileClass {
    if e.is_dir_like() {
        return FileClass::Folder;
    }
    if let EntryKind::Symlink { target: LinkTarget::Missing } = e.kind {
        return FileClass::Link;
    }
    let Some(ext) = e.extension() else { return FileClass::Other };
    classify_ext(&ext.to_ascii_lowercase())
}

pub fn classify_ext(ext: &str) -> FileClass {
    match ext {
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tif" | "tiff" | "heic" | "heif"
        | "avif" | "jxl" | "qoi" | "tga" | "exr" | "hdr" | "raw" | "dng" | "cr2" | "cr3" | "nef" | "arw" | "orf"
        | "raf" | "rw2" | "psd" | "xcf" | "ai" | "eps" | "kra" | "sketch" | "fig" => FileClass::Image,
        "mp4" | "m4v" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp" | "mts" | "m2ts"
        | "ogv" | "vob" => FileClass::Video,
        "mp3" | "flac" | "wav" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "wma" | "aiff" | "aif" | "mid" | "midi"
        | "ape" | "alac" | "mka" | "amr" | "au" => FileClass::Audio,
        "zip" | "zipx" | "7z" | "rar" | "tar" | "gz" | "tgz" | "xz" | "txz" | "bz2" | "tbz2" | "zst" | "iso"
        | "img" | "cab" | "lz" | "lzma" | "lz4" | "br" | "jar" | "war" | "apk" | "dmg" | "vhd" | "vhdx" | "wim" => {
            FileClass::Archive
        }
        "rs" | "py" | "pyw" | "js" | "mjs" | "cjs" | "ts" | "cts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "cc"
        | "cxx" | "hpp" | "hh" | "cs" | "go" | "java" | "kt" | "kts" | "scala" | "swift" | "m" | "mm" | "rb"
        | "php" | "lua" | "pl" | "pm" | "r" | "jl" | "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "json"
        | "jsonc" | "json5" | "toml" | "yaml" | "yml" | "xml" | "html" | "htm" | "css" | "scss" | "sass" | "less"
        | "sql" | "vue" | "svelte" | "astro" | "dart" | "zig" | "nim" | "hs" | "ml" | "fs" | "fsx" | "clj" | "erl"
        | "ex" | "exs" | "elm" | "groovy" | "gradle" | "cmake" | "mk" | "proto" | "graphql" | "gql" | "tf" | "wasm"
        | "wat" | "asm" | "s" | "ipynb" | "lock" | "env" | "editorconfig" | "gitignore" | "dockerfile" => {
            FileClass::Code
        }
        "pdf" => FileClass::Pdf,
        "doc" | "docx" | "odt" | "rtf" | "pages" | "tex" | "epub" | "mobi" | "azw3" | "djvu" | "xps" => FileClass::Doc,
        "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "tsv" | "numbers" | "parquet" => FileClass::Sheet,
        "ppt" | "pptx" | "odp" | "key" => FileClass::Slides,
        "txt" | "md" | "markdown" | "rst" | "adoc" | "org" | "log" | "ini" | "cfg" | "conf" | "nfo" | "srt" | "vtt"
        | "ass" | "diff" | "patch" | "eml" | "msg" | "ics" | "vcf" => FileClass::Text,
        "exe" | "msi" | "msix" | "appx" | "bat" | "cmd" | "com" | "scr" | "app" | "appimage" | "deb" | "rpm"
        | "dll" | "sys" | "so" | "dylib" | "efi" => FileClass::Exe,
        "ttf" | "otf" | "ttc" | "woff" | "woff2" | "eot" | "fon" => FileClass::Font,
        "lnk" | "url" | "webloc" | "desktop" => FileClass::Link,
        _ => FileClass::Other,
    }
}

/// An extension-specific glyph (curated Phosphor mapping), when there is one.
pub fn ext_glyph(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => ph::FILE_RS,
        "py" | "pyw" => ph::FILE_PY,
        "js" | "mjs" | "cjs" => ph::FILE_JS,
        "ts" | "cts" => ph::FILE_TS,
        "tsx" => ph::FILE_TSX,
        "jsx" => ph::FILE_JSX,
        "html" | "htm" => ph::FILE_HTML,
        "css" | "scss" | "sass" | "less" => ph::FILE_CSS,
        "c" | "h" => ph::FILE_C,
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => ph::FILE_CPP,
        "cs" => ph::FILE_C_SHARP,
        "sql" => ph::FILE_SQL,
        "vue" => ph::FILE_VUE,
        "md" | "markdown" => ph::FILE_MD,
        "csv" | "tsv" => ph::FILE_CSV,
        "ini" | "cfg" | "conf" | "editorconfig" => ph::FILE_INI,
        "svg" => ph::FILE_SVG,
        "txt" | "log" | "nfo" => ph::FILE_TXT,
        "lock" => ph::FILE_LOCK,
        "png" => ph::FILE_PNG,
        "jpg" | "jpeg" | "jfif" => ph::FILE_JPG,
        "psd" | "xcf" | "kra" | "ai" | "eps" => ph::PALETTE,
        "sketch" | "fig" => ph::PEN_NIB,
        "json" | "jsonc" | "json5" | "toml" | "yaml" | "yml" => ph::BRACKETS_CURLY,
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "bat" | "cmd" => ph::TERMINAL_WINDOW,
        "ipynb" => ph::NOTEBOOK,
        "gitignore" => ph::GIT_BRANCH,
        "env" | "pem" | "crt" | "cer" | "p12" | "pfx" => ph::CERTIFICATE,
        "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "parquet" => ph::DATABASE,
        "iso" | "img" | "vhd" | "vhdx" | "wim" => ph::DISC,
        "dmg" => ph::APPLE_LOGO,
        "apk" => ph::ANDROID_LOGO,
        "appimage" | "deb" | "rpm" => ph::PACKAGE,
        "msi" | "msix" | "appx" => ph::PACKAGE,
        "jar" | "war" | "java" => ph::COFFEE,
        "epub" | "mobi" | "azw3" | "djvu" => ph::BOOK_OPEN,
        "srt" | "vtt" | "ass" => ph::SUBTITLES,
        "eml" | "msg" => ph::ENVELOPE,
        "ics" => ph::CALENDAR,
        "vcf" => ph::ADDRESS_BOOK,
        "torrent" => ph::MAGNET,
        "diff" | "patch" => ph::GIT_BRANCH,
        "mid" | "midi" => ph::MUSIC_NOTES,
        "dll" | "sys" | "so" | "dylib" | "efi" => ph::GEAR,
        "obj" | "fbx" | "stl" | "gltf" | "glb" | "blend" | "3mf" => ph::CUBE,
        "url" | "webloc" => ph::GLOBE,
        "tex" => ph::FILE_TEXT,
        _ => return None,
    })
}

/// The letters some glyphs already spell (`FILE_ZIP` says "ZIP").
fn glyph_letters(g: &str) -> Option<&'static str> {
    const L: [(&str, &str); 25] = [
        (ph::FILE_RS, "rs"),
        (ph::FILE_PY, "py"),
        (ph::FILE_JS, "js"),
        (ph::FILE_TS, "ts"),
        (ph::FILE_TSX, "tsx"),
        (ph::FILE_JSX, "jsx"),
        (ph::FILE_HTML, "html"),
        (ph::FILE_CSS, "css"),
        (ph::FILE_C, "c"),
        (ph::FILE_CPP, "cpp"),
        (ph::FILE_C_SHARP, "cs"),
        (ph::FILE_SQL, "sql"),
        (ph::FILE_VUE, "vue"),
        (ph::FILE_MD, "md"),
        (ph::FILE_CSV, "csv"),
        (ph::FILE_INI, "ini"),
        (ph::FILE_SVG, "svg"),
        (ph::FILE_TXT, "txt"),
        (ph::FILE_PNG, "png"),
        (ph::FILE_JPG, "jpg"),
        (ph::FILE_PDF, "pdf"),
        (ph::FILE_DOC, "doc"),
        (ph::FILE_XLS, "xls"),
        (ph::FILE_PPT, "ppt"),
        (ph::FILE_ZIP, "zip"),
    ];
    L.iter().find(|(gl, _)| *gl == g).map(|(_, l)| *l)
}

/// Glyph for an entry.
pub fn glyph(e: &EntryInfo) -> &'static str {
    let class = classify(e);
    if class == FileClass::Folder {
        return match e.kind {
            EntryKind::Symlink { .. } | EntryKind::Junction => ph::FOLDER_NOTCH,
            _ => ph::FOLDER,
        };
    }
    let ext = e.extension().map(str::to_ascii_lowercase).unwrap_or_default();
    if let Some(g) = ext_glyph(&ext) {
        return g;
    }
    class_glyph(class)
}

/// The generic glyph of a class.
pub fn class_glyph(class: FileClass) -> &'static str {
    match class {
        FileClass::Image => ph::FILE_IMAGE,
        FileClass::Video => ph::FILE_VIDEO,
        FileClass::Audio => ph::FILE_AUDIO,
        FileClass::Archive => ph::FILE_ZIP,
        FileClass::Code => ph::FILE_CODE,
        FileClass::Pdf => ph::FILE_PDF,
        FileClass::Doc => ph::FILE_DOC,
        FileClass::Sheet => ph::FILE_XLS,
        FileClass::Slides => ph::FILE_PPT,
        FileClass::Text => ph::FILE_TEXT,
        FileClass::Exe => ph::APP_WINDOW,
        FileClass::Font => ph::TEXT_AA,
        FileClass::Link => ph::LINK,
        FileClass::Folder | FileClass::Other => ph::FILE,
    }
}

/// The extension label for the icon badge (`MP4`, `ZIP`), or `None` for folders, files
/// without an extension, long extensions and glyphs that already spell it.
pub fn badge_label(e: &EntryInfo) -> Option<String> {
    if e.is_dir_like() {
        return None;
    }
    let ext = e.extension()?;
    if ext.is_empty() || ext.chars().count() > 4 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let lower = ext.to_ascii_lowercase();
    // The glyph already spells it (or a close variant: `docx` on DOC, `htm` on HTML).
    if glyph_letters(glyph(e)).is_some_and(|l| lower.starts_with(l) || l.starts_with(lower.as_str())) {
        return None;
    }
    Some(ext.to_ascii_uppercase())
}

/// `[appearance] icon_style = "mono"`: every glyph in the neutral document color.
static MONO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_mono(on: bool) {
    MONO.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Glyph color for an entry.
pub fn color(e: &EntryInfo, p: &Palette) -> Color32 {
    let class = classify(e);
    if class == FileClass::Folder
        && let Some(c) = crate::services::folder_colors::color_of(&e.path, p)
    {
        // A color the user picked for this folder wins over the mono style.
        return c;
    }
    if MONO.load(std::sync::atomic::Ordering::Relaxed) {
        return p.glyph_doc;
    }
    class_color(class, p)
}

/// The color of a file class (`glyph.*` tokens).
pub fn class_color(class: FileClass, p: &Palette) -> Color32 {
    match class {
        FileClass::Folder => p.glyph_folder,
        FileClass::Image => p.glyph_image,
        FileClass::Video => p.glyph_video,
        FileClass::Audio => p.glyph_audio,
        FileClass::Archive => p.glyph_archive,
        FileClass::Code => p.glyph_code,
        FileClass::Pdf => p.danger,
        FileClass::Doc => p.accent,
        FileClass::Sheet => p.success,
        FileClass::Slides => p.warning,
        FileClass::Exe => p.glyph_exe,
        FileClass::Link => p.info,
        FileClass::Text | FileClass::Font | FileClass::Other => p.glyph_doc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classes() {
        let f = |n: &str| EntryInfo::minimal(PathBuf::from(n), EntryKind::File);
        assert_eq!(classify(&f("a.JPG")), FileClass::Image);
        assert_eq!(classify(&f("a.tar")), FileClass::Archive);
        assert_eq!(classify(&f("Makefile")), FileClass::Other);
        assert_eq!(classify(&EntryInfo::minimal(PathBuf::from("d"), EntryKind::Dir)), FileClass::Folder);
        assert_eq!(glyph(&f("main.rs")), ph::FILE_RS);
        assert_eq!(glyph(&f("x.zip")), ph::FILE_ZIP);
    }

    #[test]
    fn curated_glyphs_and_badges() {
        let f = |n: &str| EntryInfo::minimal(PathBuf::from(n), EntryKind::File);
        assert_eq!(glyph(&f("notes.ipynb")), ph::NOTEBOOK);
        assert_eq!(glyph(&f("data.SQLITE")), ph::DATABASE);
        assert_eq!(glyph(&f("win.iso")), ph::DISC);
        assert_eq!(glyph(&f("cfg.yaml")), ph::BRACKETS_CURLY);
        assert_eq!(glyph(&f("clip.mkv")), ph::FILE_VIDEO);
        assert_eq!(classify(&f("photo.CR3")), FileClass::Image);
        assert_eq!(classify(&f("show.m2ts")), FileClass::Video);
        assert_eq!(classify(&f("book.epub")), FileClass::Doc);
        // Badges: short alphanumeric extensions whose glyph does not spell them.
        assert_eq!(badge_label(&f("clip.mkv")).as_deref(), Some("MKV"));
        assert_eq!(badge_label(&f("a.zip")), None, "FILE_ZIP spells ZIP");
        assert_eq!(badge_label(&f("a.tar.gz")).as_deref(), Some("GZ"), "other archives say what they are");
        assert_eq!(badge_label(&f("a.docx")), None, "close variant of DOC");
        assert_eq!(badge_label(&f("page.htm")), None);
        assert_eq!(badge_label(&f("old.rtf")).as_deref(), Some("RTF"));
        assert_eq!(badge_label(&f("main.rs")), None, "FILE_RS already says RS");
        assert_eq!(badge_label(&f("photo.png")), None);
        assert_eq!(badge_label(&f("archive.backup")), None, "too long");
        assert_eq!(badge_label(&f("Makefile")), None);
        assert_eq!(badge_label(&EntryInfo::minimal(PathBuf::from("d.png"), EntryKind::Dir)), None);
        // Every class has a distinct generic glyph except Folder/Other.
        let classes = [
            FileClass::Image,
            FileClass::Video,
            FileClass::Audio,
            FileClass::Archive,
            FileClass::Code,
            FileClass::Pdf,
            FileClass::Doc,
            FileClass::Sheet,
            FileClass::Slides,
            FileClass::Text,
            FileClass::Exe,
            FileClass::Font,
            FileClass::Link,
        ];
        let mut seen = std::collections::HashSet::new();
        assert!(classes.iter().all(|c| seen.insert(class_glyph(*c))));
    }
}
