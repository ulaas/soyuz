//! EPUB previews (roadmap F3): metadata from the OPF package (title, authors, language,
//! publisher, date), the cover image, and the table of contents from the EPUB 3 nav
//! document or the EPUB 2 NCX. Reading goes through the size-capped office
//! [`Container`], so hostile archives stay bounded.

use std::io::{Read, Seek};

use super::preview_office::{Container, resolve};
use super::preview_xml::{self, X};

/// Table of contents entries kept.
const MAX_TOC: usize = 2_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TocEntry {
    pub depth: u8,
    pub title: String,
}

#[derive(Clone, Debug, Default)]
pub struct EpubModel {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    /// Encoded cover image bytes (decoded by the caller).
    pub cover: Option<Vec<u8>>,
    pub toc: Vec<TocEntry>,
    /// Spine length (reading-order documents).
    pub chapters: usize,
}

struct Item {
    id: String,
    href: String,
    media: String,
    props: String,
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

pub fn parse<R: Read + Seek>(r: R, cap: u64) -> Result<EpubModel, String> {
    let mut c = Container::new(r, cap)?;
    let container = c.text("META-INF/container.xml").ok_or(tr!("services-epub-no-container"))?;
    let opf_path = preview_xml::events(&container, 10_000)
        .iter()
        .find(|e| e.is_start("rootfile"))
        .and_then(|e| e.attr("full-path").map(str::to_string))
        .ok_or(tr!("services-epub-no-package"))?;
    let opf = c.text(&opf_path).ok_or(tr!("services-epub-package-unreadable"))?;
    let base = dir_of(&opf_path).to_string();
    let ev = preview_xml::events(&opf, 400_000);
    let mut m = EpubModel::default();
    let mut items: Vec<Item> = Vec::new();
    let mut cover_id: Option<String> = None;
    let mut ncx_id: Option<String> = None;
    let mut in_meta = false;
    for (i, e) in ev.iter().enumerate() {
        match e {
            X::Start { name, empty, .. } => match name.as_str() {
                "package" => m.version = e.attr("version").map(str::to_string),
                "metadata" => in_meta = !empty,
                "title" if in_meta && m.title.is_none() => m.title = clean(preview_xml::text_until_end(&ev, i).0),
                "creator" if in_meta => {
                    if let Some(a) = clean(preview_xml::text_until_end(&ev, i).0) {
                        m.authors.push(a);
                    }
                }
                "language" if in_meta && m.language.is_none() => {
                    m.language = clean(preview_xml::text_until_end(&ev, i).0)
                }
                "publisher" if in_meta && m.publisher.is_none() => {
                    m.publisher = clean(preview_xml::text_until_end(&ev, i).0)
                }
                "date" if in_meta && m.date.is_none() => m.date = clean(preview_xml::text_until_end(&ev, i).0),
                "description" if in_meta && m.description.is_none() => {
                    m.description = clean(strip_tags(&preview_xml::text_until_end(&ev, i).0))
                }
                "meta" if in_meta && e.attr("name") == Some("cover") => {
                    cover_id = e.attr("content").map(str::to_string);
                }
                "item" => items.push(Item {
                    id: e.attr("id").unwrap_or("").to_string(),
                    href: e.attr("href").unwrap_or("").to_string(),
                    media: e.attr("media-type").unwrap_or("").to_string(),
                    props: e.attr("properties").unwrap_or("").to_string(),
                }),
                "spine" => ncx_id = e.attr("toc").map(str::to_string),
                "itemref" => m.chapters += 1,
                _ => {}
            },
            X::End { name } if name == "metadata" => in_meta = false,
            _ => {}
        }
    }
    let path_of = |it: &Item| resolve(&base, &percent_decode(&it.href));
    // Cover: EPUB 3 `properties="cover-image"`, EPUB 2 `<meta name="cover">`, then any
    // image item whose id or name says "cover".
    let cover = items
        .iter()
        .find(|it| it.props.split_whitespace().any(|p| p == "cover-image"))
        .or_else(|| cover_id.as_ref().and_then(|id| items.iter().find(|it| &it.id == id)))
        .or_else(|| {
            items.iter().find(|it| {
                it.media.starts_with("image/")
                    && (it.id.to_lowercase().contains("cover") || it.href.to_lowercase().contains("cover"))
            })
        })
        .filter(|it| it.media.starts_with("image/") || it.media.is_empty());
    if let Some(it) = cover {
        m.cover = c.bytes(&path_of(it));
    }
    // TOC: nav document first, then NCX.
    if let Some(nav) = items.iter().find(|it| it.props.split_whitespace().any(|p| p == "nav")) {
        let p = path_of(nav);
        if let Some(x) = c.text(&p) {
            m.toc = nav_toc(&preview_xml::events(&x, 400_000));
        }
    }
    if m.toc.is_empty() {
        let ncx = ncx_id
            .as_ref()
            .and_then(|id| items.iter().find(|it| &it.id == id))
            .or_else(|| items.iter().find(|it| it.media == "application/x-dtbncx+xml"));
        if let Some(n) = ncx
            && let Some(x) = c.text(&path_of(n))
        {
            m.toc = ncx_toc(&preview_xml::events(&x, 400_000));
        }
    }
    Ok(m)
}

fn clean(s: String) -> Option<String> {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

/// Descriptions are often escaped HTML.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// EPUB 3 `<nav epub:type="toc">` → nested `ol / li / a`.
fn nav_toc(ev: &[X]) -> Vec<TocEntry> {
    let mut out = Vec::new();
    let mut in_toc = false;
    let mut nav_depth = 0usize;
    let mut ol_depth = 0usize;
    let mut i = 0;
    while i < ev.len() && out.len() < MAX_TOC {
        let e = &ev[i];
        match e {
            X::Start { name, empty: false, .. } if name == "nav" => {
                if in_toc {
                    nav_depth += 1;
                } else if e.attr("type").is_some_and(|t| t.split_whitespace().any(|t| t == "toc")) {
                    in_toc = true;
                    nav_depth = 0;
                }
            }
            X::End { name } if name == "nav" && in_toc => {
                if nav_depth == 0 {
                    break;
                }
                nav_depth -= 1;
            }
            X::Start { name, empty: false, .. } if in_toc && name == "ol" => ol_depth += 1,
            X::End { name } if in_toc && name == "ol" => ol_depth = ol_depth.saturating_sub(1),
            X::Start { name, .. } if in_toc && (name == "a" || name == "span") && ol_depth > 0 => {
                let (t, next) = preview_xml::text_until_end(ev, i);
                if let Some(t) = clean(t) {
                    out.push(TocEntry { depth: (ol_depth - 1).min(8) as u8, title: t });
                }
                i = next;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// EPUB 2 NCX: nested `navPoint / navLabel / text`.
fn ncx_toc(ev: &[X]) -> Vec<TocEntry> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < ev.len() && out.len() < MAX_TOC {
        match &ev[i] {
            X::Start { name, empty: false, .. } if name == "navPoint" => depth += 1,
            X::End { name } if name == "navPoint" => depth = depth.saturating_sub(1),
            X::Start { name, .. } if name == "text" && depth > 0 => {
                let (t, next) = preview_xml::text_until_end(ev, i);
                if let Some(t) = clean(t) {
                    out.push(TocEntry { depth: (depth - 1).min(8) as u8, title: t });
                }
                i = next;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::preview_office::fixtures::zip;
    use std::io::Cursor;

    fn png() -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(4, 6, image::Rgba([200, 10, 10, 255]))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn epub3_metadata_cover_and_nav() {
        let cover = png();
        let mut bytes = zip(&[
            ("mimetype", "application/epub+zip"),
            (
                "META-INF/container.xml",
                r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
            ),
            (
                "OEBPS/content.opf",
                r#"<package version="3.0" xmlns:dc="dc"><metadata>
                <dc:title>The   Book</dc:title><dc:creator>Jane Doe</dc:creator><dc:creator>John Roe</dc:creator>
                <dc:language>en</dc:language><dc:description>&lt;p&gt;A tale.&lt;/p&gt;</dc:description></metadata>
                <manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
                <item id="c" href="images/cover%20art.png" media-type="image/png" properties="cover-image"/>
                <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
                <spine><itemref idref="ch1"/><itemref idref="ch1"/></spine></package>"#,
            ),
            (
                "OEBPS/nav.xhtml",
                r#"<html><body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">Chapter 1</a>
                <ol><li><a href="ch1.xhtml#s">Section 1.1</a></li></ol></li><li><a>Chapter 2</a></li></ol></nav>
                <nav epub:type="landmarks"><ol><li><a>Cover</a></li></ol></nav></body></html>"#,
            ),
        ]);
        // Add the binary cover with a second writer pass.
        {
            use std::io::Write;
            let mut w = zip::ZipWriter::new_append(Cursor::new(&mut bytes)).unwrap();
            w.start_file("OEBPS/images/cover art.png", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&cover).unwrap();
            w.finish().unwrap();
        }
        let m = parse(Cursor::new(bytes), 16 << 20).unwrap();
        assert_eq!(m.title.as_deref(), Some("The Book"));
        assert_eq!(m.authors, vec!["Jane Doe", "John Roe"]);
        assert_eq!(m.language.as_deref(), Some("en"));
        assert_eq!(m.description.as_deref(), Some("A tale."));
        assert_eq!(m.chapters, 2);
        assert_eq!(m.cover.as_deref(), Some(&cover[..]));
        assert_eq!(
            m.toc,
            vec![
                TocEntry { depth: 0, title: "Chapter 1".into() },
                TocEntry { depth: 1, title: "Section 1.1".into() },
                TocEntry { depth: 0, title: "Chapter 2".into() },
            ]
        );
    }

    #[test]
    fn epub2_ncx_and_meta_cover() {
        let bytes = zip(&[
            (
                "META-INF/container.xml",
                r#"<container><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#,
            ),
            (
                "content.opf",
                r#"<package version="2.0"><metadata><title>Old</title><meta name="cover" content="img"/></metadata>
                <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="img" href="c.jpg" media-type="image/jpeg"/></manifest>
                <spine toc="ncx"/></package>"#,
            ),
            (
                "toc.ncx",
                r#"<ncx><navMap><navPoint><navLabel><text>One</text></navLabel><navPoint><navLabel><text>One.A</text></navLabel></navPoint></navPoint></navMap></ncx>"#,
            ),
            ("c.jpg", "jpegbytes"),
        ]);
        let m = parse(Cursor::new(bytes), 16 << 20).unwrap();
        assert_eq!(m.title.as_deref(), Some("Old"));
        assert_eq!(m.cover.as_deref(), Some(&b"jpegbytes"[..]));
        assert_eq!(m.toc.len(), 2);
        assert_eq!(m.toc[1], TocEntry { depth: 1, title: "One.A".into() });
        assert!(parse(Cursor::new(zip(&[("x", "y")])), 1 << 20).is_err());
    }
}
