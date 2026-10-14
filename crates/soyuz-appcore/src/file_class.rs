//! Broad file categories by extension, shared by every frontend: the GUI draws a Phosphor
//! glyph and a `glyph.*` colour per class (`soyuz-app` `ui::views::icons`), the terminal UI
//! its own glyph and colour role. Only the classification lives here, plus the one table
//! of video / audio extensions the previews and thumbnails share ([`is_video_ext`],
//! [`is_audio_ext`]): the base lists work without the FFmpeg libraries (pure-Rust
//! metadata, Symphonia audio), the extended ones only when they are loaded
//! (`services::media::runtime`).

use soyuz_core::{EntryInfo, EntryKind, LinkTarget};

/// Video containers previewed and thumbnailed by every build (metadata from
/// `preview_media` without the libraries).
pub const VIDEO_EXT: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "3gp", "m2ts"];
/// More containers the FFmpeg libraries demux; previewed / thumbnailed only when loaded.
/// (`ts` is left out on purpose: it is TypeScript far more often than MPEG-TS.)
pub const VIDEO_EXT_FFMPEG: &[&str] =
    &["mts", "ogv", "vob", "mxf", "wtv", "3g2", "rm", "rmvb", "asf", "f4v", "divx", "m2v", "mpv", "y4m", "nut", "dv"];
/// Audio files previewed by every build (Symphonia decodes most of them).
pub const AUDIO_EXT: &[&str] = &["mp3", "wav", "flac", "ogg", "oga", "opus", "m4a", "aac", "wma", "aiff", "aif"];
/// More audio the FFmpeg libraries decode; previewed only when loaded.
pub const AUDIO_EXT_FFMPEG: &[&str] =
    &["ape", "mka", "amr", "au", "dsf", "dff", "ac3", "eac3", "dts", "tta", "wv", "mpc", "spx", "caf", "alac"];

/// `ext` (lower-case, no dot) is a video container: the base table, plus the FFmpeg one
/// when `ffmpeg` (the libraries are loaded).
pub fn is_video_ext(ext: &str, ffmpeg: bool) -> bool {
    VIDEO_EXT.contains(&ext) || (ffmpeg && VIDEO_EXT_FFMPEG.contains(&ext))
}

/// `ext` (lower-case, no dot) is an audio file; see [`is_video_ext`].
pub fn is_audio_ext(ext: &str, ffmpeg: bool) -> bool {
    AUDIO_EXT.contains(&ext) || (ffmpeg && AUDIO_EXT_FFMPEG.contains(&ext))
}

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
        // Icons are static: every container the previews may handle gets the video glyph.
        _ if is_video_ext(ext, true) => FileClass::Video,
        "mid" | "midi" => FileClass::Audio,
        _ if is_audio_ext(ext, true) => FileClass::Audio,
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
        assert_eq!(classify_ext("mxf"), FileClass::Video, "FFmpeg-only containers still get the video glyph");
        assert_eq!(classify_ext("ape"), FileClass::Audio);
        assert_eq!(classify_ext("mid"), FileClass::Audio);
        assert_eq!(classify_ext("ts"), FileClass::Code, "never a video by extension");
    }

    #[test]
    fn extension_tables() {
        for e in VIDEO_EXT {
            assert!(is_video_ext(e, false) && is_video_ext(e, true), "{e}");
            assert!(!is_audio_ext(e, true), "{e} is not audio");
        }
        for e in VIDEO_EXT_FFMPEG {
            assert!(!is_video_ext(e, false), "{e} needs the libraries");
            assert!(is_video_ext(e, true), "{e}");
            assert!(!VIDEO_EXT.contains(e), "{e} listed twice");
        }
        for e in AUDIO_EXT_FFMPEG {
            assert!(!is_audio_ext(e, false) && is_audio_ext(e, true), "{e}");
            assert!(!AUDIO_EXT.contains(e), "{e} listed twice");
        }
        assert!(is_video_ext("flv", false) && is_video_ext("m2ts", false), "the old thumbnail list is kept");
        assert!(!is_video_ext("ts", true) && !is_video_ext("", true));
        assert!(!is_audio_ext("mid", true), "MIDI needs a synthesizer, not a decoder");
        let all: Vec<&str> =
            VIDEO_EXT.iter().chain(VIDEO_EXT_FFMPEG).chain(AUDIO_EXT).chain(AUDIO_EXT_FFMPEG).copied().collect();
        assert!(all.iter().all(|e| *e == e.to_ascii_lowercase() && !e.starts_with('.')), "lower-case, no dot");
    }
}
