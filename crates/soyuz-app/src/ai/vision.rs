//! "Suggest names" inputs: which files may show their contents to the
//! model, and downscaled pictures for vision models.
//!
//! The model never reads the disk: the harness reads a size-capped text excerpt through
//! the tool host (scope, sensitive paths, privacy) and decodes images here on the worker
//! thread, only for folders whose AI access is "contents". Everything is framed as
//! untrusted data.

use std::io::Cursor;
use std::path::Path;

use base64::Engine as _;

/// Largest image file decoded for the model.
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// What "Suggest names" may attach for a file (by extension; no I/O).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachKind {
    /// Text or document: a text excerpt.
    Text,
    /// A picture: a downscaled JPEG for vision models.
    Image,
}

const TEXT_EXT: &[&str] = &[
    "txt", "md", "markdown", "rst", "org", "adoc", "csv", "tsv", "json", "jsonl", "xml", "yaml", "yml", "toml", "ini",
    "cfg", "conf", "log", "html", "htm", "rtf", "tex", "srt", "vtt", "rs", "py", "js", "ts", "tsx", "jsx", "c", "h",
    "cpp", "hpp", "cs", "java", "kt", "go", "rb", "php", "sh", "ps1", "bat", "sql", "lua", "swift", "css", "scss",
    "vue", "svelte", "ipynb", "eml", "vcf", "ics",
];
const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico"];

/// The attachment kind of a file name.
pub fn kind_of(path: &Path) -> Option<AttachKind> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if IMAGE_EXT.contains(&ext.as_str()) {
        Some(AttachKind::Image)
    } else if TEXT_EXT.contains(&ext.as_str()) {
        Some(AttachKind::Text)
    } else {
        None
    }
}

/// Decode `path`, fit it into `max_px` × `max_px` and encode it as JPEG. Returns
/// `(media type, base64 data)`. Runs on a worker thread.
pub fn load_image(path: &Path, max_px: u32) -> Result<(String, String), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err("image too large".into());
    }
    let img = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let max = max_px.clamp(64, 2048);
    let small = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgb = image::DynamicImage::ImageRgb8(small.to_rgb8());
    let mut out = Cursor::new(Vec::new());
    rgb.write_to(&mut out, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
    Ok(("image/jpeg".into(), base64::engine::general_purpose::STANDARD.encode(out.into_inner())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_downscaling() {
        assert_eq!(kind_of(Path::new("a/Notes.MD")), Some(AttachKind::Text));
        assert_eq!(kind_of(Path::new("IMG_1.jpeg")), Some(AttachKind::Image));
        assert_eq!(kind_of(Path::new("archive.zip")), None);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.png");
        image::RgbImage::from_pixel(1600, 900, image::Rgb([200, 30, 30])).save(&p).unwrap();
        let (mt, data) = load_image(&p, 256).unwrap();
        assert_eq!(mt, "image/jpeg");
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
        let back = image::load_from_memory(&bytes).unwrap();
        assert!(back.width() <= 256 && back.height() <= 256, "{}x{}", back.width(), back.height());
    }
}
