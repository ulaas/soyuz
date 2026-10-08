//! File-type glyphs and colors (PLAN §1.2 `glyph.*` tokens).

use egui::Color32;
use soyuz_core::{EntryInfo, EntryKind, LinkTarget};

use crate::theme::Palette;
use crate::theme::icons::ph;

/// Broad file category used for glyph + color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tif" | "tiff" | "heic" | "heif" | "avif"
        | "raw" | "cr2" | "nef" | "psd" => FileClass::Image,
        "mp4" | "m4v" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" => FileClass::Video,
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "wma" | "aiff" | "mid" => FileClass::Audio,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "iso" | "cab" | "lz" | "lzma" => {
            FileClass::Archive
        }
        "rs" | "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "cc" | "hpp" | "cs" | "go" | "java"
        | "kt" | "swift" | "rb" | "php" | "lua" | "sh" | "bash" | "zsh" | "ps1" | "json" | "toml" | "yaml" | "yml"
        | "xml" | "html" | "htm" | "css" | "scss" | "sql" | "vue" | "dart" | "zig" | "nim" | "hs" | "ex" | "exs"
        | "lock" => FileClass::Code,
        "pdf" => FileClass::Pdf,
        "doc" | "docx" | "odt" | "rtf" | "pages" => FileClass::Doc,
        "xls" | "xlsx" | "ods" | "csv" | "numbers" => FileClass::Sheet,
        "ppt" | "pptx" | "odp" | "key" => FileClass::Slides,
        "txt" | "md" | "log" | "ini" | "cfg" | "conf" | "nfo" => FileClass::Text,
        "exe" | "msi" | "bat" | "cmd" | "com" | "app" | "appimage" | "deb" | "rpm" | "dll" | "so" | "dylib" => {
            FileClass::Exe
        }
        "ttf" | "otf" | "woff" | "woff2" => FileClass::Font,
        "lnk" | "url" | "desktop" => FileClass::Link,
        _ => FileClass::Other,
    }
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
    match ext.as_str() {
        "rs" => return ph::FILE_RS,
        "py" => return ph::FILE_PY,
        "js" | "mjs" => return ph::FILE_JS,
        "ts" => return ph::FILE_TS,
        "tsx" => return ph::FILE_TSX,
        "jsx" => return ph::FILE_JSX,
        "html" | "htm" => return ph::FILE_HTML,
        "css" | "scss" => return ph::FILE_CSS,
        "c" | "h" => return ph::FILE_C,
        "cpp" | "cc" | "hpp" => return ph::FILE_CPP,
        "cs" => return ph::FILE_C_SHARP,
        "sql" => return ph::FILE_SQL,
        "vue" => return ph::FILE_VUE,
        "md" => return ph::FILE_MD,
        "csv" => return ph::FILE_CSV,
        "ini" | "cfg" | "conf" => return ph::FILE_INI,
        "svg" => return ph::FILE_SVG,
        "txt" | "log" => return ph::FILE_TXT,
        "lock" => return ph::FILE_LOCK,
        _ => {}
    }
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

/// Glyph color for an entry.
pub fn color(e: &EntryInfo, p: &Palette) -> Color32 {
    match classify(e) {
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
}
