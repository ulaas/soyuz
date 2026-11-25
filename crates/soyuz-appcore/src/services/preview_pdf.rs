//! PDF previews: the facade over the PDF engine ([`super::pdf`]).
//!
//! * **Pages** render through the engine `[preview] pdf_engine` picks: hayro (pure Rust,
//!   always built in) or pdfium (cargo feature `pdf`, on by default). Soyuz never
//!   downloads or links pdfium at build time: the library is loaded at run time from
//!   `[preview] pdfium_dir`, the folder of `soyuz.exe`, or the config dir (first hit
//!   wins); when it is missing, hayro renders instead.
//! * `lopdf` (pure Rust) is the last resort: it still reads the page count, document
//!   info and the text of files both engines reject.
//!
//! [`extract_text`] is the shared text entry point (preview "Text" view); [`search_text`]
//! reads the whole document for content search and the index (`soyuz_core::doc_text`).

use std::path::{Path, PathBuf};

/// Pages whose text is extracted for the preview.
pub(crate) const TEXT_PAGES: u32 = 8;
/// Characters of extracted text kept.
pub const TEXT_CHARS: usize = 24_000;
/// Pages read for content search and the index.
const SEARCH_PAGES: u32 = 5_000;
/// Characters of a document's text searched / indexed.
const SEARCH_CHARS: usize = 4_000_000;
/// Time budget of one document's text for content search and the index: past it the
/// engine is abandoned and lopdf tries.
pub(crate) const SEARCH_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);

pub use super::pdf::PdfSrc;
use super::pdf::{self as engine, EngineKind, PdfError};

/// The engine kind `[preview] pdf_engine` asks for.
pub fn engine_kind(e: soyuz_core::config::PdfEngine) -> EngineKind {
    match e {
        soyuz_core::config::PdfEngine::Hayro => EngineKind::Hayro,
        soyuz_core::config::PdfEngine::Pdfium => EngineKind::Pdfium,
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
    v.extend(system_lib_dirs());
    v
}

/// Linux: the usual system library folders (distribution packages and `make install`
/// put `libpdfium.so` there). The dynamic loader's own search path (`LD_LIBRARY_PATH`,
/// `ld.so.cache`) is tried as well when none of the folders has the library.
fn system_lib_dirs() -> Vec<PathBuf> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let multiarch = format!("/usr/lib/{}-linux-gnu", std::env::consts::ARCH);
    ["/usr/lib64", multiarch.as_str(), "/usr/lib", "/usr/local/lib64", "/usr/local/lib"]
        .iter()
        .map(PathBuf::from)
        .collect()
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

/// Whether pdfium can be loaded: found in `dirs`, or (Linux) reachable through the
/// dynamic loader's own search path (`LD_LIBRARY_PATH`, `ld.so.cache`), which the
/// pdfium thread tries last. Does file I/O; call off the UI thread.
pub fn pdfium_available(dirs: &[PathBuf]) -> bool {
    find_pdfium(dirs).is_some() || (cfg!(target_os = "linux") && loader_has_pdfium())
}

/// `libpdfium.so` in an `LD_LIBRARY_PATH` folder or listed in `/etc/ld.so.cache`
/// (remembered for a minute: thumbnail workers ask once per PDF).
fn loader_has_pdfium() -> bool {
    static MEMO: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);
    let mut memo = MEMO.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, found)) = *memo
        && at.elapsed() < std::time::Duration::from_secs(60)
    {
        return found;
    }
    let found = loader_scan();
    *memo = Some((std::time::Instant::now(), found));
    found
}

fn loader_scan() -> bool {
    let name = pdfium_file_name();
    let env = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
    if std::env::split_paths(&env).any(|d| !d.as_os_str().is_empty() && d.join(name).is_file()) {
        return true;
    }
    // The cache lists library names as NUL-terminated strings.
    let needle = format!("{name}\0");
    std::fs::read("/etc/ld.so.cache").map(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes())).unwrap_or(false)
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

/// Trailing spaces and runs of blank lines removed; at most `max` characters.
pub(crate) fn clean_text_max(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max.saturating_mul(4)));
    let mut chars = 0usize;
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
        chars += l.chars().count() + 1;
        if chars > max {
            break;
        }
    }
    out.chars().take(max).collect::<String>().trim().to_string()
}

/// Pure-Rust info + text (no page rendering).
pub fn info_lopdf(src: &PdfSrc, cap: u64) -> Result<PdfInfo, String> {
    lopdf_text(src, cap, TEXT_PAGES, TEXT_CHARS)
}

/// [`info_lopdf`] with the text of the first `page_limit` pages, at most `chars` characters.
fn lopdf_text(src: &PdfSrc, cap: u64, page_limit: u32, chars: usize) -> Result<PdfInfo, String> {
    let bytes = src.bytes(cap)?;
    let password = match src {
        PdfSrc::File(p) => password_for(p),
        PdfSrc::Mem(_) => None,
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<PdfInfo, String> {
        let doc = match &password {
            Some(pw) => lopdf::Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password(pw)),
            None => lopdf::Document::load_mem(&bytes),
        };
        let doc = doc.map_err(|e| match e {
            lopdf::Error::InvalidPassword => tr!("services-pdf-password-wrong").to_string(),
            e => tr!("services-pdf-cant-read", error = e.to_string()),
        })?;
        // Encrypted with a user password nobody typed: nothing can be read.
        if doc.is_encrypted() && password.is_none() && doc.authenticate_password("").is_err() {
            return Err(tr!("services-pdf-password").into());
        }
        let readable = !doc.is_encrypted() || doc.encryption_state.is_some();
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
        if readable {
            let nums: Vec<u32> = pages.keys().copied().take(page_limit as usize).collect();
            if let Ok(t) = doc.extract_text_with_limit(&nums, 32 << 20) {
                info.text = clean_text_max(&t, chars);
            }
        }
        Ok(info)
    }));
    r.unwrap_or_else(|_| Err(tr!("services-pdf-parser-failed").into()))
}

/// Passwords typed for encrypted PDFs this session (never saved), by file.
static PASSWORDS: parking_lot::Mutex<Option<std::collections::HashMap<PathBuf, String>>> =
    parking_lot::Mutex::new(None);
/// Bumped by [`set_password`]: previews of PDFs reload when it changes.
static PASSWORD_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Remember `password` for the PDF at `path` (this session only) and reload its previews.
pub fn set_password(path: &Path, password: String) {
    PASSWORDS.lock().get_or_insert_with(Default::default).insert(path.to_path_buf(), password);
    PASSWORD_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// The password typed for the PDF at `path`, if any.
pub fn password_for(path: &Path) -> Option<String> {
    PASSWORDS.lock().as_ref()?.get(path).cloned()
}

/// Changes whenever a password is typed (part of the preview's reload key).
pub fn password_generation() -> u64 {
    PASSWORD_GEN.load(std::sync::atomic::Ordering::SeqCst)
}

/// `msg` says the PDF needs a password (none typed, or a wrong one).
pub fn is_password_error(msg: &str) -> bool {
    let msg = msg.trim_start_matches("pdfium: ");
    msg == tr!("services-pdf-password") || msg == tr!("services-pdf-password-wrong")
}

/// The engine for `want` (`[preview] pdf_engine`): pdfium only when chosen and its
/// library is found, else hayro. Probes the file system: workers only.
pub fn pick_engine(want: EngineKind, pdfium_dir: &str, cap: u64, timeout: std::time::Duration) -> engine::Engine {
    let dirs = if want == EngineKind::Pdfium { pdfium_dirs(pdfium_dir) } else { Vec::new() };
    engine::pick(want, dirs, cap, timeout)
}

/// Text of the first pages through the engine `want` picks ([`pick_engine`]), lopdf
/// last. File I/O: workers only.
pub fn extract_text(src: &PdfSrc, pdfium_dir: &str, want: EngineKind, cap: u64) -> Result<String, String> {
    let eng = pick_engine(want, pdfium_dir, cap, SEARCH_BUDGET);
    text_with(src, &eng, TEXT_PAGES, TEXT_CHARS)
}

/// Text of the whole document (up to [`SEARCH_PAGES`] pages / [`SEARCH_CHARS`]
/// characters) for content search and the index: pdfium when chosen and loadable, else
/// hayro, lopdf last when the engine fails or runs past [`SEARCH_BUDGET`]. An encrypted
/// PDF opens with the password typed this session, else it is an error.
pub fn search_text(path: &Path, pdfium_dir: &str, want: EngineKind, cap: u64) -> Result<String, String> {
    let src = PdfSrc::File(path.to_path_buf());
    let eng = pick_engine(want, pdfium_dir, cap, SEARCH_BUDGET);
    text_with(&src, &eng, SEARCH_PAGES, SEARCH_CHARS)
}

/// The first `pages` pages' text (at most `chars` characters) through `eng` within its
/// timeout, then lopdf when the engine fails (not for password errors: lopdf can't open
/// those either).
pub(crate) fn text_with(src: &PdfSrc, eng: &engine::Engine, pages: u32, chars: usize) -> Result<String, String> {
    match engine::document_text(src, eng, pages, chars, eng.timeout) {
        Ok(t) => Ok(t),
        Err(e @ PdfError::Password { .. }) => Err(e.to_string()),
        Err(e) => {
            log::debug!("PDF text through {} failed ({e}); trying lopdf", eng.kind.label());
            lopdf_text(src, eng.max_bytes, pages, chars).map(|i| i.text)
        }
    }
}

/// The pdfium backend, now [`super::pdf::pdfium_backend`] (re-exported so callers keep
/// `preview_pdf::pdfium::…`).
#[cfg(feature = "pdf")]
pub use super::pdf::pdfium_backend as pdfium;

/// "PDF preview needs pdfium" guidance listing the folders searched.
pub fn missing_message(dirs: &[PathBuf]) -> String {
    let list: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    tr!("services-pdf-missing", file = pdfium_file_name(), folders = list.join("\n\n"))
}

#[cfg(test)]
pub(crate) mod tests {
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
    fn content_search_finds_pdf_text() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("doc.pdf"), sample_pdf()).unwrap();
        std::fs::write(d.path().join("notes.txt"), "nothing here").unwrap();
        let text = search_text(&d.path().join("doc.pdf"), "Z:/no/pdfium", EngineKind::Hayro, 1 << 20).unwrap();
        assert!(text.contains("Hello from page one") && text.contains("Second page"), "{text:?}");

        // Through the hook the app registers: the walker matches the PDF's text.
        let mut config = soyuz_core::config::Config::default();
        config.preview.pdfium_dir = "Z:/no/pdfium".into();
        crate::services::search::sync_document_text(&config);
        let q = soyuz_search::SearchQuery {
            roots: vec![d.path().to_path_buf()],
            content: Some(soyuz_search::ContentPatternSpec {
                regex: false,
                text: "second PAGE".into(),
                case_sensitive: false,
            }),
            ..Default::default()
        };
        let h = soyuz_search::start(q, 1, std::sync::Arc::new(|| {})).unwrap();
        let mut hits = Vec::new();
        loop {
            match h.rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap() {
                soyuz_search::SearchEvent::Hits(v) => hits.extend(v),
                soyuz_search::SearchEvent::Done(_) => break,
                _ => {}
            }
        }
        assert_eq!(hits.len(), 1);
        assert!(hits[0].0.path.ends_with("doc.pdf"));
        assert!(hits[0].1[0].text.contains("Second page"), "{:?}", hits[0].1);

        config.search.document_text = false;
        crate::services::search::sync_document_text(&config);
        assert!(!soyuz_core::doc_text::handles(Path::new("a.pdf")));
    }

    #[test]
    fn encrypted_pdf_asks_for_its_password() {
        let mut doc = lopdf::Document::load_mem(&sample_pdf()).unwrap();
        let id = || lopdf::Object::String(b"0123456789abcdef".to_vec(), lopdf::StringFormat::Hexadecimal);
        doc.trailer.set("ID", lopdf::Object::Array(vec![id(), id()]));
        let version = lopdf::EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner",
            user_password: "secret",
            key_length: 128,
            permissions: lopdf::Permissions::all(),
        };
        let state = lopdf::EncryptionState::try_from(version).unwrap();
        doc.encrypt(&state).unwrap();
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("locked.pdf");
        doc.save(&path).unwrap();
        let src = PdfSrc::File(path.clone());
        let e = info_lopdf(&src, 1 << 20).unwrap_err();
        assert!(is_password_error(&e), "{e}");
        let gen0 = password_generation();
        set_password(&path, "nope".into());
        assert!(password_generation() > gen0, "previews reload");
        let e = info_lopdf(&src, 1 << 20).unwrap_err();
        assert_eq!(e, tr!("services-pdf-password-wrong"));
        set_password(&path, "secret".into());
        let info = info_lopdf(&src, 1 << 20).unwrap();
        assert_eq!(info.pages, 2);
        assert!(info.text.contains("Hello from page one"), "{:?}", info.text);
    }

    #[test]
    fn document_text_goes_through_the_engine() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("doc.pdf");
        std::fs::write(&path, sample_pdf()).unwrap();
        // pdfium chosen but not found: hayro reads it.
        let empty = d.path().join("no-pdfium").to_string_lossy().into_owned();
        let text = search_text(&path, &empty, EngineKind::Pdfium, 1 << 20).unwrap();
        assert!(text.contains("Hello from page one") && text.contains("Second page"), "{text:?}");
        let first = extract_text(&PdfSrc::File(path.clone()), "", EngineKind::Hayro, 1 << 20).unwrap();
        assert!(first.contains("Hello from page one"), "{first:?}");
        // lopdf takes over when the engine rejects the file.
        let eng = engine::Engine::hayro(10, std::time::Duration::from_secs(5));
        assert!(text_with(&PdfSrc::File(path.clone()), &eng, 10, 1000).is_err(), "over both caps");
        // Locked: a password error, not lopdf's guess.
        let locked = d.path().join("locked.pdf");
        std::fs::write(&locked, engine::fixtures::encrypted("pw")).unwrap();
        let e = search_text(&locked, "", EngineKind::Hayro, 1 << 20).unwrap_err();
        assert!(is_password_error(&e), "{e}");
        set_password(&locked, "pw".into());
        assert!(search_text(&locked, "", EngineKind::Hayro, 1 << 20).unwrap().contains("Hello from page one"));
    }

    /// hayro's text device vs lopdf on a generated 300-page document (run with
    /// `--nocapture` to see the times).
    #[test]
    fn document_text_of_300_pages() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("book.pdf");
        std::fs::write(&path, engine::fixtures::many_pages(300)).unwrap();
        let t = std::time::Instant::now();
        let text = search_text(&path, "", EngineKind::Hayro, 1 << 30).unwrap();
        let hayro = t.elapsed();
        assert!(text.contains("Page 0 of the sweep") && text.contains("Page 299 of the sweep"), "{}", text.len());
        let t = std::time::Instant::now();
        let lo = lopdf_text(&PdfSrc::File(path), 1 << 30, SEARCH_PAGES, SEARCH_CHARS).unwrap();
        let lopdf = t.elapsed();
        assert!(lo.text.contains("Page 299 of the sweep"));
        println!("300 pages: hayro {hayro:?}, lopdf {lopdf:?}");
        assert!(hayro < SEARCH_BUDGET, "within the search budget: {hayro:?}");
    }

    #[test]
    fn helpers() {
        assert_eq!(decode_pdf_string(&[0xFE, 0xFF, 0, b'H', 0, b'i']), "Hi");
        assert_eq!(decode_pdf_string(b"caf\xe9"), "café");
        assert_eq!(pdf_date("D:2023"), "D:2023");
        assert!(missing_message(&[PathBuf::from("C:/x")]).contains("pdfium"));
        assert!(missing_message(&[PathBuf::from("C:/x")]).contains(":\n\nC:/x\n\nOr set"), "paragraphs kept");
        assert_eq!(find_pdfium(&[PathBuf::from("Z:/definitely/missing")]), None);
        assert_eq!(clean_text_max("a\n\n\n\nb  \n", TEXT_CHARS), "a\n\nb");
        assert_eq!(clean_text_max("abc\ndef\n", 5), "abc\nd");
    }

    #[test]
    fn pdfium_search_order() {
        let d = pdfium_dirs("/opt/pdfium");
        assert_eq!(d[0], PathBuf::from("/opt/pdfium"), "configured folder first");
        let sys = d.iter().any(|p| p == Path::new("/usr/lib64") || p == Path::new("/usr/lib"));
        assert_eq!(sys, cfg!(target_os = "linux"), "system folders on Linux only");
        if cfg!(target_os = "linux") {
            let cfg = d.iter().position(|p| *p == soyuz_core::config::config_dir()).unwrap();
            let usr = d.iter().position(|p| p == Path::new("/usr/lib64")).unwrap();
            assert!(cfg < usr, "user folders before system folders");
            let t = tempfile::tempdir().unwrap();
            std::fs::write(t.path().join("libpdfium.so"), b"x").unwrap();
            let dirs = [PathBuf::from("/definitely/missing"), t.path().to_path_buf()];
            assert_eq!(find_pdfium(&dirs), Some(t.path().join("libpdfium.so")));
        }
    }
}
