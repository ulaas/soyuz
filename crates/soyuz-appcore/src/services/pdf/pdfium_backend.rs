//! The optional pdfium backend (cargo feature `pdf`): one dedicated thread owns the
//! library (pdfium is not thread safe) and runs jobs. Moved here from
//! `preview_pdf::pdfium`, which re-exports it unchanged. No region rendering: the viewer
//! is hayro-only.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use crossbeam_channel::{Sender, bounded, unbounded};
use pdfium_render::prelude::*;

use super::PdfSrc;
use crate::services::preview_pdf::{
    PageImage, PdfInfo, TEXT_CHARS, TEXT_PAGES, clean_text_max, find_pdfium, password_for, pdf_date,
};

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
                    let stamp = lib.as_ref().map(|l| (l.clone(), std::fs::metadata(l).and_then(|m| m.modified()).ok()));
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
                            // Linux: last, the loader's own search path (`dlopen("libpdfium.so")`).
                            None if cfg!(target_os = "linux") => match Pdfium::bind_to_system_library() {
                                Ok(b) => pdfium = Some(Pdfium::new(b)),
                                Err(_) => last_err = crate::services::preview_pdf::missing_message(&dirs),
                            },
                            None => last_err = crate::services::preview_pdf::missing_message(&dirs),
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
    worker().tx.send((dirs.to_vec(), job)).map_err(|_| tr!("services-pdf-renderer-stopped").to_string())?;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(r) => return r,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) if stale() => return Err("stale".into()),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err(tr!("services-pdf-renderer-failed").into()),
        }
    }
}

fn open<'a>(p: &'a Pdfium, src: &PdfSrc, buf: &'a mut Vec<u8>, cap: u64) -> Result<PdfDocument<'a>, String> {
    let password = match src {
        PdfSrc::File(path) => password_for(path),
        PdfSrc::Mem(_) => None,
    };
    let r = match src {
        PdfSrc::File(path) => {
            let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if len > cap.max(1 << 30) {
                return Err(tr!("services-pdf-too-large").into());
            }
            p.load_pdf_from_file(path, password.as_deref())
        }
        PdfSrc::Mem(b) => {
            *buf = b.to_vec();
            p.load_pdf_from_byte_slice(buf, None)
        }
    };
    r.map_err(|e| match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) if password.is_some() => {
            tr!("services-pdf-password-wrong").to_string()
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            tr!("services-pdf-password").to_string()
        }
        e => tr!("services-pdf-cant-open", error = format!("{e:?}")),
    })
}

/// Page count, document info and the text of the first pages.
pub fn info(src: &PdfSrc, dirs: &[PathBuf], cap: u64, stale: &dyn Fn() -> bool) -> Result<PdfInfo, String> {
    info_pages(src, dirs, cap, stale, TEXT_PAGES, TEXT_CHARS)
}

/// [`info`] with the text of the first `max_pages` pages, at most `chars` characters.
pub fn info_pages(
    src: &PdfSrc,
    dirs: &[PathBuf],
    cap: u64,
    stale: &dyn Fn() -> bool,
    max_pages: u32,
    chars: usize,
) -> Result<PdfInfo, String> {
    let src = src.clone();
    run(dirs, stale, move |p| {
        let mut buf = Vec::new();
        let doc = open(p, &src, &mut buf, cap)?;
        let pages = doc.pages();
        let meta = doc.metadata();
        let get =
            |t: PdfDocumentMetadataTagType| meta.get(t).map(|v| v.value().trim().to_string()).filter(|s| !s.is_empty());
        let mut info = PdfInfo {
            pages: pages.len() as u32,
            version: Some(format!("{:?}", doc.version()).replace("Pdf", "").replace('_', ".")),
            title: get(PdfDocumentMetadataTagType::Title),
            author: get(PdfDocumentMetadataTagType::Author),
            producer: get(PdfDocumentMetadataTagType::Producer).or_else(|| get(PdfDocumentMetadataTagType::Creator)),
            created: get(PdfDocumentMetadataTagType::CreationDate).map(|d| pdf_date(&d)),
            ..Default::default()
        };
        if let Ok(r) = pages.page_size(0) {
            info.page_size = Some((r.width().value, r.height().value));
        }
        let mut text = String::new();
        for i in 0..(pages.len() as u32).min(max_pages) {
            if let Ok(page) = pages.get(i as PdfPageIndex)
                && let Ok(t) = page.text()
            {
                text.push_str(&t.all());
                text.push_str("\n\n");
            }
            if text.len() > chars.saturating_mul(2) {
                break;
            }
        }
        info.text = clean_text_max(&text, chars);
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
        let pg = doc
            .pages()
            .get(page as PdfPageIndex)
            .map_err(|e| tr!("services-pdf-no-page", page = (page + 1).to_string(), error = format!("{e:?}")))?;
        let cfg = PdfRenderConfig::new()
            .set_target_width(width.clamp(64, 6000) as Pixels)
            .set_maximum_height(max_h.clamp(64, 8000) as Pixels)
            .render_form_data(true);
        let bmp =
            pg.render_with_config(&cfg).map_err(|e| tr!("services-pdf-cant-render-page", error = format!("{e:?}")))?;
        let (w, h) = (bmp.width().max(0) as usize, bmp.height().max(0) as usize);
        let rgba = bmp.as_rgba_bytes();
        if rgba.len() < w * h * 4 {
            return Err(tr!("services-pdf-page-empty").into());
        }
        Ok(PageImage { page, width: w, height: h, rgba })
    })
}
