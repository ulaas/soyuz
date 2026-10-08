//! PDF previews (roadmap F3).
//!
//! * **Pages** render through pdfium (cargo feature `pdf`, on by default). Soyuz never
//!   downloads or links pdfium at build time: the library is loaded at run time from
//!   `[preview] pdfium_dir`, the folder of `soyuz.exe`, or the config dir (first hit
//!   wins). Pdfium is not thread safe, so one dedicated thread owns it and runs jobs.
//! * **Without pdfium** (or with the feature off) a pure-Rust fallback (`lopdf`) still
//!   reads the page count, document info and the text of the first pages, and the UI
//!   explains how to get page rendering.
//!
//! [`extract_text`] is the shared text entry point (preview "Text" view, and a hook for
//! search snippets).

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Pages whose text is extracted for the preview.
const TEXT_PAGES: u32 = 8;
/// Characters of extracted text kept.
pub const TEXT_CHARS: usize = 24_000;

/// Where the PDF bytes come from.
#[derive(Clone, Debug)]
pub enum PdfSrc {
    File(PathBuf),
    Mem(Arc<[u8]>),
}

impl PdfSrc {
    fn bytes(&self, cap: u64) -> Result<Vec<u8>, String> {
        match self {
            PdfSrc::File(p) => {
                let len = std::fs::metadata(p).map_err(|e| e.to_string())?.len();
                if len > cap {
                    return Err(format!("Larger than the {} MiB document limit", cap >> 20));
                }
                std::fs::read(p).map_err(|e| e.to_string())
            }
            PdfSrc::Mem(b) => Ok(b.to_vec()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PdfInfo {
    pub pages: u32,
    pub version: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub producer: Option<String>,
    pub created: Option<String>,
    pub encrypted: bool,
    /// Size of the first page in points.
    pub page_size: Option<(f32, f32)>,
    /// Text of the first pages (capped).
    pub text: String,
}

/// One rendered page (RGBA, unmultiplied).
#[derive(Clone, Debug)]
pub struct PageImage {
    pub page: u32,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Folders searched for the pdfium library, in order.
pub fn pdfium_dirs(configured: &str) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let c = configured.trim();
    if !c.is_empty() {
        v.push(PathBuf::from(c));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        v.push(d);
    }
    v.push(soyuz_core::config::config_dir());
    v
}

/// Platform file name of the pdfium library.
pub fn pdfium_file_name() -> &'static str {
    if cfg!(windows) {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    }
}

/// The first `dirs` entry holding the pdfium library.
pub fn find_pdfium(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(pdfium_file_name())).find(|p| p.is_file())
}

/// Decode a PDF text string (UTF-16BE with BOM, UTF-8 with BOM, else PDFDocEncoding ≈ Latin-1).
pub fn decode_pdf_string(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| c as char).collect()
}

/// `D:20240131120000+01'00'` → `2024-01-31 12:00`.
pub fn pdf_date(s: &str) -> String {
    let d = s.trim().trim_start_matches("D:");
    let digits: String = d.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 8 {
        let mut out = format!("{}-{}-{}", &digits[0..4], &digits[4..6], &digits[6..8]);
        if digits.len() >= 12 {
            out.push_str(&format!(" {}:{}", &digits[8..10], &digits[10..12]));
        }
        out
    } else {
        s.trim().to_string()
    }
}

fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(TEXT_CHARS * 4));
    let mut blank = 0usize;
    for line in s.lines() {
        let l = line.trim_end();
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
        if out.chars().count() > TEXT_CHARS {
            break;
        }
    }
    out.chars().take(TEXT_CHARS).collect::<String>().trim().to_string()
}

/// Pure-Rust info + text (no page rendering).
pub fn info_lopdf(src: &PdfSrc, cap: u64) -> Result<PdfInfo, String> {
    let bytes = src.bytes(cap)?;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<PdfInfo, String> {
        let doc = lopdf::Document::load_mem(&bytes).map_err(|e| format!("Can't read this PDF ({e})"))?;
        let mut info =
            PdfInfo { version: Some(doc.version.clone()), encrypted: doc.is_encrypted(), ..Default::default() };
        let pages = doc.get_pages();
        info.pages = pages.len() as u32;
        let dict = doc.trailer.get(b"Info").ok().and_then(|o| match o.as_reference() {
            Ok(id) => doc.get_dictionary(id).ok(),
            Err(_) => o.as_dict().ok(),
        });
        if let Some(d) = dict {
            let get = |k: &[u8]| {
                d.get(k)
                    .ok()
                    .and_then(|o| o.as_str().ok())
                    .map(decode_pdf_string)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            };
            info.title = get(b"Title");
            info.author = get(b"Author");
            info.producer = get(b"Producer").or_else(|| get(b"Creator"));
            info.created = get(b"CreationDate").map(|s| pdf_date(&s));
        }
        if let Some((_, &first)) = pages.iter().next()
            && let Ok(page) = doc.get_dictionary(first)
            && let Ok(mb) = page.get(b"MediaBox").and_then(|o| o.as_array())
            && mb.len() == 4
        {
            let n = |i: usize| mb[i].as_float().ok().or_else(|| mb[i].as_i64().ok().map(|v| v as f32));
            if let (Some(a), Some(b), Some(c), Some(d)) = (n(0), n(1), n(2), n(3)) {
                info.page_size = Some(((c - a).abs(), (d - b).abs()));
            }
        }
        if !info.encrypted {
            let nums: Vec<u32> = pages.keys().copied().take(TEXT_PAGES as usize).collect();
            if let Ok(t) = doc.extract_text_with_limit(&nums, 32 << 20) {
                info.text = clean_text(&t);
            }
        }
        Ok(info)
    }));
    r.unwrap_or_else(|_| Err("The PDF parser failed on this file".into()))
}

/// Text of the first pages through whichever backend is available (pdfium first).
pub fn extract_text(src: &PdfSrc, pdfium_dir: &str, cap: u64) -> Result<String, String> {
    #[cfg(feature = "pdf")]
    if let Ok(info) = pdfium::info(src, &pdfium_dirs(pdfium_dir), cap, &|| false) {
        return Ok(info.text);
    }
    let _ = pdfium_dir;
    info_lopdf(src, cap).map(|i| i.text)
}

#[cfg(feature = "pdf")]
pub mod pdfium {
    //! The pdfium thread.
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::time::Duration;

    use crossbeam_channel::{Sender, bounded, unbounded};
    use pdfium_render::prelude::*;

    use super::{PageImage, PdfInfo, PdfSrc, TEXT_PAGES, clean_text, find_pdfium, pdf_date};

    type Job = Box<dyn FnOnce(Result<&Pdfium, &str>) + Send>;

    struct Worker {
        tx: Sender<(Vec<PathBuf>, Job)>,
    }

    fn worker() -> &'static Worker {
        static W: OnceLock<Worker> = OnceLock::new();
        W.get_or_init(|| {
            let (tx, rx) = unbounded::<(Vec<PathBuf>, Job)>();
            let _ = std::thread::Builder::new().name("soyuz-pdf".into()).spawn(move || {
                let mut pdfium: Option<Pdfium> = None;
                let mut last_err = String::new();
                // What the last bind attempt saw: the search folders and the library
                // found there (with its modification time).
                type Attempt = (Vec<PathBuf>, Option<(PathBuf, Option<std::time::SystemTime>)>);
                let mut tried: Option<Attempt> = None;
                for (dirs, job) in rx.iter() {
                    // Bind once. A failed bind is retried only when the search folders
                    // change or a (different) library file shows up, so a bad DLL is not
                    // reloaded for every preview.
                    if pdfium.is_none() {
                        let lib = find_pdfium(&dirs);
                        let stamp =
                            lib.as_ref().map(|l| (l.clone(), std::fs::metadata(l).and_then(|m| m.modified()).ok()));
                        let attempt = (dirs.clone(), stamp);
                        if tried.as_ref() != Some(&attempt) {
                            tried = Some(attempt);
                            match lib {
                                Some(lib) => match Pdfium::bind_to_library(&lib) {
                                    Ok(b) => pdfium = Some(Pdfium::new(b)),
                                    Err(e) => {
                                        log::warn!("pdfium bind failed for {}: {e:?}", lib.display());
                                        last_err = format!(
                                            "{} could not be loaded. It may be too old, damaged or built for another \
                                             CPU; use a current build from bblanchon/pdfium-binaries.",
                                            lib.display()
                                        );
                                    }
                                },
                                None => last_err = super::missing_message(&dirs),
                            }
                        }
                    }
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &pdfium {
                        Some(p) => job(Ok(p)),
                        None => job(Err(&last_err)),
                    }));
                    if r.is_err() {
                        log::warn!("pdfium job panicked");
                    }
                }
            });
            Worker { tx }
        })
    }

    /// Run `f` on the pdfium thread and wait for it (giving up when `stale`).
    fn run<T: Send + 'static>(
        dirs: &[PathBuf],
        stale: &dyn Fn() -> bool,
        f: impl FnOnce(&Pdfium) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = bounded(1);
        let job: Job = Box::new(move |p| {
            let r = match p {
                Ok(p) => f(p),
                Err(e) => Err(format!("pdfium: {e}")),
            };
            let _ = tx.send(r);
        });
        worker().tx.send((dirs.to_vec(), job)).map_err(|_| "The PDF renderer stopped".to_string())?;
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(r) => return r,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) if stale() => return Err("stale".into()),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("The PDF renderer failed on this file".into()),
            }
        }
    }

    fn open<'a>(p: &'a Pdfium, src: &PdfSrc, buf: &'a mut Vec<u8>, cap: u64) -> Result<PdfDocument<'a>, String> {
        let r = match src {
            PdfSrc::File(path) => {
                let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                if len > cap.max(1 << 30) {
                    return Err("This PDF is too large".into());
                }
                p.load_pdf_from_file(path, None)
            }
            PdfSrc::Mem(b) => {
                *buf = b.to_vec();
                p.load_pdf_from_byte_slice(buf, None)
            }
        };
        r.map_err(|e| match e {
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
                "This PDF is password protected".to_string()
            }
            e => format!("Can't open this PDF ({e:?})"),
        })
    }

    /// Page count, document info and the text of the first pages.
    pub fn info(src: &PdfSrc, dirs: &[PathBuf], cap: u64, stale: &dyn Fn() -> bool) -> Result<PdfInfo, String> {
        let src = src.clone();
        run(dirs, stale, move |p| {
            let mut buf = Vec::new();
            let doc = open(p, &src, &mut buf, cap)?;
            let pages = doc.pages();
            let meta = doc.metadata();
            let get = |t: PdfDocumentMetadataTagType| {
                meta.get(t).map(|v| v.value().trim().to_string()).filter(|s| !s.is_empty())
            };
            let mut info = PdfInfo {
                pages: pages.len() as u32,
                version: Some(format!("{:?}", doc.version()).replace("Pdf", "").replace('_', ".")),
                title: get(PdfDocumentMetadataTagType::Title),
                author: get(PdfDocumentMetadataTagType::Author),
                producer: get(PdfDocumentMetadataTagType::Producer)
                    .or_else(|| get(PdfDocumentMetadataTagType::Creator)),
                created: get(PdfDocumentMetadataTagType::CreationDate).map(|d| pdf_date(&d)),
                ..Default::default()
            };
            if let Ok(r) = pages.page_size(0) {
                info.page_size = Some((r.width().value, r.height().value));
            }
            let mut text = String::new();
            for i in 0..(pages.len() as u32).min(TEXT_PAGES) {
                if let Ok(page) = pages.get(i as PdfPageIndex)
                    && let Ok(t) = page.text()
                {
                    text.push_str(&t.all());
                    text.push_str("\n\n");
                }
                if text.len() > super::TEXT_CHARS * 2 {
                    break;
                }
            }
            info.text = clean_text(&text);
            Ok(info)
        })
    }

    /// Render one page `width` pixels wide (height capped at `max_h`).
    pub fn render(
        src: &PdfSrc,
        dirs: &[PathBuf],
        cap: u64,
        page: u32,
        width: u32,
        max_h: u32,
        stale: &dyn Fn() -> bool,
    ) -> Result<PageImage, String> {
        let src = src.clone();
        run(dirs, stale, move |p| {
            let mut buf = Vec::new();
            let doc = open(p, &src, &mut buf, cap)?;
            let pg = doc.pages().get(page as PdfPageIndex).map_err(|e| format!("No page {} ({e:?})", page + 1))?;
            let cfg = PdfRenderConfig::new()
                .set_target_width(width.clamp(64, 6000) as Pixels)
                .set_maximum_height(max_h.clamp(64, 8000) as Pixels)
                .render_form_data(true);
            let bmp = pg.render_with_config(&cfg).map_err(|e| format!("Can't render this page ({e:?})"))?;
            let (w, h) = (bmp.width().max(0) as usize, bmp.height().max(0) as usize);
            let rgba = bmp.as_rgba_bytes();
            if rgba.len() < w * h * 4 {
                return Err("The page rendered empty".into());
            }
            Ok(PageImage { page, width: w, height: h, rgba })
        })
    }
}

/// "PDF preview needs pdfium" guidance listing the folders searched.
pub fn missing_message(dirs: &[PathBuf]) -> String {
    let list: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    format!(
        "Page rendering uses pdfium, which isn't bundled. Download a prebuilt {} (e.g. from the \
         bblanchon/pdfium-binaries releases on GitHub, matching your CPU) and put it in one of these folders:\n\n{}\n\n\
         Or set [preview] pdfium_dir. No restart is needed.",
        pdfium_file_name(),
        list.join("\n\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny two-page PDF built with lopdf.
    pub fn sample_pdf() -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let mut kids = Vec::new();
        for text in ["Hello from page one", "Second page text"] {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![72.into(), 700.into()]),
                    Operation::new("Tj", vec![Object::string_literal(text)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            });
            kids.push(page_id.into());
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => kids, "Count" => 2, "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let info_id = doc.add_object(dictionary! {
            "Title" => Object::string_literal("Sample doc"),
            "CreationDate" => Object::string_literal("D:20240131120000Z"),
        });
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.trailer.set("Info", info_id);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn lopdf_fallback_reads_info_and_text() {
        let src = PdfSrc::Mem(sample_pdf().into());
        let info = info_lopdf(&src, 1 << 20).unwrap();
        assert_eq!(info.pages, 2);
        assert_eq!(info.title.as_deref(), Some("Sample doc"));
        assert_eq!(info.created.as_deref(), Some("2024-01-31 12:00"));
        assert!(info.text.contains("Hello from page one"), "{:?}", info.text);
        assert!(info.text.contains("Second page"));
        assert!(info_lopdf(&PdfSrc::Mem(b"%PDF-1.4 garbage".to_vec().into()), 1 << 20).is_err());
    }

    #[test]
    fn helpers() {
        assert_eq!(decode_pdf_string(&[0xFE, 0xFF, 0, b'H', 0, b'i']), "Hi");
        assert_eq!(decode_pdf_string(b"caf\xe9"), "café");
        assert_eq!(pdf_date("D:2023"), "D:2023");
        assert!(missing_message(&[PathBuf::from("C:/x")]).contains("pdfium"));
        assert_eq!(find_pdfium(&[PathBuf::from("Z:/definitely/missing")]), None);
        assert_eq!(clean_text("a\n\n\n\nb  \n"), "a\n\nb");
    }
}
