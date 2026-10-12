//! Broad file categories by extension, shared by every frontend: the GUI draws a Phosphor
//! glyph and a `glyph.*` colour per class (`soyuz-app` `ui::views::icons`), the terminal UI
//! its own glyph and colour role. Only the classification lives here.

use soyuz_core::{EntryInfo, EntryKind, LinkTarget};

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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn classes() {
        let f = |n: &str| EntryInfo::minimal(PathBuf::from(n), EntryKind::File);
        assert_eq!(classify(&f("a.JPG")), FileClass::Image);
        assert_eq!(classify(&f("a.tar")), FileClass::Archive);
        assert_eq!(classify(&f("Makefile")), FileClass::Other);
        assert_eq!(classify(&f("photo.CR3")), FileClass::Image);
        assert_eq!(classify(&f("show.m2ts")), FileClass::Video);
        assert_eq!(classify(&f("book.epub")), FileClass::Doc);
        assert_eq!(classify(&EntryInfo::minimal(PathBuf::from("d"), EntryKind::Dir)), FileClass::Folder);
        assert_eq!(classify_ext("rs"), FileClass::Code);
    }
}
