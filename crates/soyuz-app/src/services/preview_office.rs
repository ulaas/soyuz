//! Office document previews: structure and text out of the zip + XML
//! containers of OOXML (docx / xlsx / pptx) and ODF (odt / ods / odp). Nothing is
//! rendered faithfully; the goal is a fast, readable outline: headings, bold / italic
//! runs and lists for text documents, the first sheet as a table for spreadsheets, and
//! slide titles plus bullet text for presentations.
//!
//! Every zip part is size-checked before it is inflated (`[preview] max_document_mib`)
//! and the XML event stream is bounded, so a zip bomb or a gigantic sheet only yields a
//! truncated preview.

use std::collections::HashMap;
use std::io::{Read, Seek};

use super::preview_xml::{self, X};

/// Upper bound of XML events read from one part.
const MAX_EVENTS: usize = 2_000_000;
/// Paragraphs kept for a text document.
const MAX_PARAS: usize = 5_000;
/// Columns kept for a spreadsheet.
const MAX_COLS: usize = 40;
/// Slides kept for a presentation.
const MAX_SLIDES: usize = 500;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Para {
    /// 0 = body text, 1–6 = heading level (1 also for "Title").
    pub level: u8,
    /// List item (bullet or numbered).
    pub list: bool,
    pub runs: Vec<Run>,
}

impl Para {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DocModel {
    pub format: &'static str,
    pub title: Option<String>,
    pub author: Option<String>,
    pub paras: Vec<Para>,
    pub words: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SheetModel {
    pub format: &'static str,
    /// All sheet names (the first one is shown).
    pub sheets: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub ncols: usize,
    /// More rows / columns exist than were read.
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slide {
    pub title: String,
    pub body: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SlidesModel {
    pub format: &'static str,
    pub title: Option<String>,
    pub slides: Vec<Slide>,
}

#[derive(Clone, Debug)]
pub enum Office {
    Doc(DocModel),
    Sheet(SheetModel),
    Slides(SlidesModel),
}

/// Which parser handles an extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficeKind {
    Docx,
    Xlsx,
    Pptx,
    Odt,
    Ods,
    Odp,
}

pub fn kind_for_ext(ext: &str) -> Option<OfficeKind> {
    Some(match ext {
        "docx" | "docm" | "dotx" => OfficeKind::Docx,
        "xlsx" | "xlsm" | "xltx" => OfficeKind::Xlsx,
        "pptx" | "pptm" | "ppsx" | "potx" => OfficeKind::Pptx,
        "odt" | "ott" => OfficeKind::Odt,
        "ods" | "ots" => OfficeKind::Ods,
        "odp" | "otp" => OfficeKind::Odp,
        _ => return None,
    })
}

/// A zip container with capped part reads.
pub struct Container<R: Read + Seek> {
    zip: zip::ZipArchive<R>,
    cap: u64,
}

impl<R: Read + Seek> Container<R> {
    pub fn new(r: R, cap: u64) -> Result<Self, String> {
        let zip = zip::ZipArchive::new(r).map_err(|e| tr!("services-office-bad-container", error = e.to_string()))?;
        Ok(Container { zip, cap })
    }

    /// Raw bytes of a part (`None` when missing, encrypted or over the cap).
    pub fn bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        let name = name.trim_start_matches('/');
        let mut f = self.zip.by_name(name).ok()?;
        if f.size() > self.cap {
            return None;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.by_ref().take(self.cap).read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    pub fn text(&mut self, name: &str) -> Option<String> {
        self.bytes(name).map(|b| preview_xml::decode(&b))
    }

    pub fn names(&self) -> Vec<String> {
        self.zip.file_names().map(str::to_string).collect()
    }
}

/// Parse an Office / ODF document.
pub fn parse<R: Read + Seek>(r: R, kind: OfficeKind, cap: u64, sheet_rows: usize) -> Result<Office, String> {
    let mut c = Container::new(r, cap)?;
    Ok(match kind {
        OfficeKind::Docx => Office::Doc(docx(&mut c)?),
        OfficeKind::Odt => Office::Doc(odt(&mut c)?),
        OfficeKind::Xlsx => Office::Sheet(xlsx(&mut c, sheet_rows)?),
        OfficeKind::Ods => Office::Sheet(ods(&mut c, sheet_rows)?),
        OfficeKind::Pptx => Office::Slides(pptx(&mut c)?),
        OfficeKind::Odp => Office::Slides(odp(&mut c)?),
    })
}

fn word_count(paras: &[Para]) -> usize {
    paras.iter().map(|p| p.runs.iter().map(|r| r.text.split_whitespace().count()).sum::<usize>()).sum()
}

/// `docProps/core.xml` (OOXML) title and creator.
fn ooxml_core<R: Read + Seek>(c: &mut Container<R>) -> (Option<String>, Option<String>) {
    let Some(xml) = c.text("docProps/core.xml") else { return (None, None) };
    let ev = preview_xml::events(&xml, 10_000);
    let mut title = None;
    let mut author = None;
    for (i, e) in ev.iter().enumerate() {
        if e.is_start("title") {
            title = Some(preview_xml::text_until_end(&ev, i).0);
        } else if e.is_start("creator") {
            author = Some(preview_xml::text_until_end(&ev, i).0);
        }
    }
    let clean = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    (clean(title), clean(author))
}

/// `w:val` toggles: absent / "true" / "1" / "on" = on.
fn toggle_on(e: &X) -> bool {
    !matches!(e.attr("val"), Some("0" | "false" | "off" | "none"))
}

fn heading_from_name(name: &str) -> Option<u8> {
    let n = name.to_ascii_lowercase().replace(' ', "");
    if n == "title" {
        return Some(1);
    }
    let rest = n.strip_prefix("heading")?;
    rest.parse::<u8>().ok().filter(|l| (1..=9).contains(l)).map(|l| l.min(6))
}

fn docx<R: Read + Seek>(c: &mut Container<R>) -> Result<DocModel, String> {
    // Style id → heading level, from the style name ("heading 1", "Title") or its
    // outline level (localized style names still carry `w:outlineLvl`).
    let mut styles: HashMap<String, u8> = HashMap::new();
    if let Some(xml) = c.text("word/styles.xml") {
        let ev = preview_xml::events(&xml, 400_000);
        let mut cur: Option<String> = None;
        for e in &ev {
            if e.is_start("style") {
                cur = e.attr("styleId").map(str::to_string);
            } else if let Some(id) = &cur {
                if e.is_start("name") {
                    if let Some(l) = e.attr("val").and_then(heading_from_name) {
                        styles.insert(id.clone(), l);
                    }
                } else if e.is_start("outlineLvl")
                    && let Some(l) = e.attr("val").and_then(|v| v.parse::<u8>().ok())
                    && l < 9
                {
                    styles.entry(id.clone()).or_insert((l + 1).min(6));
                }
            }
            if e.is_end("style") {
                cur = None;
            }
        }
    }
    let xml = c.text("word/document.xml").ok_or_else(|| tr!("services-office-no-body", part = "word/document.xml"))?;
    let ev = preview_xml::events(&xml, MAX_EVENTS);
    let mut doc = DocModel { format: tr!("services-office-word"), ..Default::default() };
    let mut para: Option<Para> = None;
    let (mut bold, mut italic, mut in_rpr, mut in_ppr, mut in_text) = (false, false, false, false, false);
    for e in &ev {
        match e {
            X::Start { name, empty, .. } => match name.as_str() {
                "p" if !empty => para = Some(Para::default()),
                "pPr" => in_ppr = !empty,
                "pStyle" if in_ppr => {
                    if let (Some(p), Some(v)) = (para.as_mut(), e.attr("val")) {
                        p.level = styles.get(v).copied().or_else(|| heading_from_name(v)).unwrap_or(0);
                    }
                }
                "outlineLvl" if in_ppr => {
                    if let (Some(p), Some(l)) = (para.as_mut(), e.attr("val").and_then(|v| v.parse::<u8>().ok()))
                        && l < 9
                        && p.level == 0
                    {
                        p.level = (l + 1).min(6);
                    }
                }
                "numPr" if in_ppr => {
                    if let Some(p) = para.as_mut() {
                        p.list = true;
                    }
                }
                "r" => {
                    bold = false;
                    italic = false;
                }
                "rPr" if !in_ppr => in_rpr = !empty,
                "b" if in_rpr => bold = toggle_on(e),
                "i" if in_rpr => italic = toggle_on(e),
                "t" => in_text = !empty,
                "tab" if !in_ppr => push_run(para.as_mut(), "\t", bold, italic),
                "br" | "cr" => push_run(para.as_mut(), "\n", bold, italic),
                _ => {}
            },
            X::End { name } => match name.as_str() {
                "p" => {
                    if let Some(p) = para.take() {
                        doc.paras.push(p);
                        if doc.paras.len() >= MAX_PARAS {
                            doc.truncated = true;
                            break;
                        }
                    }
                }
                "pPr" => in_ppr = false,
                "rPr" => in_rpr = false,
                "t" => in_text = false,
                _ => {}
            },
            X::Text(t) if in_text => push_run(para.as_mut(), t, bold, italic),
            X::Text(_) => {}
        }
    }
    if ev.len() >= MAX_EVENTS {
        doc.truncated = true;
    }
    finish_doc(&mut doc);
    (doc.title, doc.author) = ooxml_core(c);
    Ok(doc)
}

fn push_run(p: Option<&mut Para>, text: &str, bold: bool, italic: bool) {
    let Some(p) = p else { return };
    if let Some(last) = p.runs.last_mut()
        && last.bold == bold
        && last.italic == italic
    {
        last.text.push_str(text);
        return;
    }
    p.runs.push(Run { text: text.to_string(), bold, italic });
}

/// Drop empty paragraphs (but keep one blank between blocks) and count words.
fn finish_doc(doc: &mut DocModel) {
    let mut out: Vec<Para> = Vec::with_capacity(doc.paras.len());
    for p in doc.paras.drain(..) {
        let blank = p.text().trim().is_empty();
        if blank && out.last().is_none_or(|l: &Para| l.text().trim().is_empty()) {
            continue;
        }
        out.push(p);
    }
    while out.last().is_some_and(|l| l.text().trim().is_empty()) {
        out.pop();
    }
    doc.words = word_count(&out);
    doc.paras = out;
}

/// ODF automatic text styles: style name → (bold, italic). Paragraph styles map to a
/// heading level when their display name is a heading.
fn odf_styles(ev: &[X], map: &mut HashMap<String, (bool, bool)>, heads: &mut HashMap<String, u8>) {
    let mut cur: Option<String> = None;
    for e in ev {
        if e.is_start("style") {
            cur = e.attr("name").map(str::to_string);
            if let (Some(n), Some(l)) =
                (&cur, e.attr("parent-style-name").and_then(|p| heading_from_name(&p.replace('_', ""))))
            {
                heads.insert(n.clone(), l);
            }
        } else if e.is_start("text-properties")
            && let Some(n) = &cur
        {
            let bold = e.attr("font-weight").is_some_and(|w| w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600));
            let italic = e.attr("font-style").is_some_and(|s| s == "italic" || s == "oblique");
            map.insert(n.clone(), (bold, italic));
        }
        if e.is_end("style") {
            cur = None;
        }
    }
}

fn odf_meta<R: Read + Seek>(c: &mut Container<R>) -> (Option<String>, Option<String>) {
    let Some(xml) = c.text("meta.xml") else { return (None, None) };
    let ev = preview_xml::events(&xml, 10_000);
    let (mut title, mut author) = (None, None);
    for (i, e) in ev.iter().enumerate() {
        if e.is_start("title") {
            title = Some(preview_xml::text_until_end(&ev, i).0);
        } else if (e.is_start("initial-creator") && author.is_none()) || e.is_start("creator") {
            author = Some(preview_xml::text_until_end(&ev, i).0);
        }
    }
    let clean = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    (clean(title), clean(author))
}

fn odt<R: Read + Seek>(c: &mut Container<R>) -> Result<DocModel, String> {
    let mut map = HashMap::new();
    let mut heads = HashMap::new();
    if let Some(xml) = c.text("styles.xml") {
        odf_styles(&preview_xml::events(&xml, 400_000), &mut map, &mut heads);
    }
    let xml = c.text("content.xml").ok_or_else(|| tr!("services-office-no-body", part = "content.xml"))?;
    let ev = preview_xml::events(&xml, MAX_EVENTS);
    odf_styles(&ev, &mut map, &mut heads);
    let mut doc = DocModel { format: tr!("services-office-odt"), ..Default::default() };
    let mut para: Option<Para> = None;
    let mut list_depth = 0usize;
    // Span style stack: (bold, italic) of each open span.
    let mut spans: Vec<(bool, bool)> = Vec::new();
    let mut in_body = false;
    let mut para_style = (false, false);
    for e in &ev {
        match e {
            X::Start { name, empty, .. } => match name.as_str() {
                "text" if !empty => in_body = true,
                "list-item" if !empty => list_depth += 1,
                "h" | "p" if in_body && !empty => {
                    let level = if name == "h" {
                        e.attr("outline-level").and_then(|l| l.parse::<u8>().ok()).unwrap_or(1).clamp(1, 6)
                    } else {
                        e.attr("style-name").and_then(|s| heads.get(s).copied()).unwrap_or(0)
                    };
                    para_style = e.attr("style-name").and_then(|s| map.get(s).copied()).unwrap_or_default();
                    para = Some(Para { level, list: list_depth > 0, runs: Vec::new() });
                    spans.clear();
                }
                "span" if !empty => {
                    spans.push(e.attr("style-name").and_then(|s| map.get(s).copied()).unwrap_or_default());
                }
                "s" => {
                    let n = e.attr("c").and_then(|c| c.parse::<usize>().ok()).unwrap_or(1).min(64);
                    let (b, i) = style_of(&spans, para_style);
                    push_run(para.as_mut(), &" ".repeat(n), b, i);
                }
                "tab" => {
                    let (b, i) = style_of(&spans, para_style);
                    push_run(para.as_mut(), "\t", b, i);
                }
                "line-break" => {
                    let (b, i) = style_of(&spans, para_style);
                    push_run(para.as_mut(), "\n", b, i);
                }
                _ => {}
            },
            X::End { name } => match name.as_str() {
                "h" | "p" => {
                    if let Some(p) = para.take() {
                        doc.paras.push(p);
                        if doc.paras.len() >= MAX_PARAS {
                            doc.truncated = true;
                            break;
                        }
                    }
                }
                "span" => {
                    spans.pop();
                }
                "list-item" => list_depth = list_depth.saturating_sub(1),
                _ => {}
            },
            X::Text(t) => {
                if para.is_some() {
                    let (b, i) = style_of(&spans, para_style);
                    push_run(para.as_mut(), t, b, i);
                }
            }
        }
    }
    if ev.len() >= MAX_EVENTS {
        doc.truncated = true;
    }
    finish_doc(&mut doc);
    (doc.title, doc.author) = odf_meta(c);
    Ok(doc)
}

fn style_of(spans: &[(bool, bool)], base: (bool, bool)) -> (bool, bool) {
    spans.iter().fold(base, |(b, i), (sb, si)| (b || *sb, i || *si))
}

/// "B12" → zero-based column index 1.
pub fn col_index(cell_ref: &str) -> Option<usize> {
    let mut n = 0usize;
    let mut any = false;
    for ch in cell_ref.chars() {
        if ch.is_ascii_alphabetic() {
            n = n.checked_mul(26)?.checked_add((ch.to_ascii_uppercase() as u8 - b'A' + 1) as usize)?;
            any = true;
        } else {
            break;
        }
    }
    any.then(|| n - 1)
}

/// Relationship id → target of an OOXML `.rels` part (targets resolved against `base`).
fn rels<R: Read + Seek>(c: &mut Container<R>, part: &str, base: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(xml) = c.text(part) else { return out };
    for e in preview_xml::events(&xml, 100_000) {
        if e.is_start("Relationship")
            && let (Some(id), Some(t)) = (e.attr("Id"), e.attr("Target"))
        {
            out.insert(id.to_string(), resolve(base, t));
        }
    }
    out
}

/// Resolve a relative zip path (`../media/x.png`) against a base folder (`ppt/slides`).
pub fn resolve(base: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn xlsx<R: Read + Seek>(c: &mut Container<R>, max_rows: usize) -> Result<SheetModel, String> {
    let mut m = SheetModel { format: tr!("services-office-excel"), ..Default::default() };
    let wb_rels = rels(c, "xl/_rels/workbook.xml.rels", "xl");
    let mut first_target = None;
    if let Some(xml) = c.text("xl/workbook.xml") {
        for e in preview_xml::events(&xml, 100_000) {
            if e.is_start("sheet") {
                m.sheets.push(e.attr("name").unwrap_or(tr!("services-office-sheet")).to_string());
                if first_target.is_none() {
                    first_target = e.attr("id").and_then(|id| wb_rels.get(id)).cloned();
                }
            }
        }
    }
    let target = first_target.unwrap_or_else(|| "xl/worksheets/sheet1.xml".into());
    let mut shared: Vec<String> = Vec::new();
    if let Some(xml) = c.text("xl/sharedStrings.xml") {
        let ev = preview_xml::events(&xml, MAX_EVENTS);
        let mut cur: Option<String> = None;
        let mut in_t = false;
        let mut in_rph = false;
        for e in &ev {
            match e {
                X::Start { name, empty, .. } => match name.as_str() {
                    "si" if !empty => cur = Some(String::new()),
                    "si" => shared.push(String::new()),
                    "t" => in_t = !empty,
                    "rPh" => in_rph = !empty,
                    _ => {}
                },
                X::End { name } => match name.as_str() {
                    "si" => shared.push(cur.take().unwrap_or_default()),
                    "t" => in_t = false,
                    "rPh" => in_rph = false,
                    _ => {}
                },
                X::Text(t) if in_t && !in_rph => {
                    if let Some(s) = cur.as_mut() {
                        s.push_str(t);
                    }
                }
                X::Text(_) => {}
            }
        }
    }
    let xml = c.text(&target).ok_or(tr!("services-office-first-sheet"))?;
    let ev = preview_xml::events(&xml, MAX_EVENTS);
    let mut row: Option<Vec<String>> = None;
    let mut cell: Option<(usize, String, String)> = None; // (col, type, value)
    let mut in_v = false;
    let mut in_is_t = false;
    let mut next_col = 0usize;
    for e in &ev {
        match e {
            X::Start { name, empty, .. } => match name.as_str() {
                "row" => {
                    let r = e.attr("r").and_then(|r| r.parse::<usize>().ok());
                    // Gaps between numbered rows become blank rows (capped).
                    if let Some(r) = r {
                        let have = m.rows.len();
                        for _ in have..(r.saturating_sub(1)).min(have + 50).min(max_rows) {
                            m.rows.push(Vec::new());
                        }
                    }
                    next_col = 0;
                    if *empty {
                        m.rows.push(Vec::new());
                    } else {
                        row = Some(Vec::new());
                    }
                }
                "c" => {
                    let col = e.attr("r").and_then(col_index).unwrap_or(next_col);
                    next_col = col + 1;
                    if !empty {
                        cell = Some((col, e.attr("t").unwrap_or("n").to_string(), String::new()));
                    }
                }
                "v" => in_v = !empty,
                "t" => in_is_t = !empty && cell.as_ref().is_some_and(|c| c.1 == "inlineStr"),
                _ => {}
            },
            X::End { name } => match name.as_str() {
                "v" => in_v = false,
                "t" => in_is_t = false,
                "c" => {
                    if let (Some((col, ty, v)), Some(r)) = (cell.take(), row.as_mut()) {
                        if col >= MAX_COLS {
                            m.truncated = true;
                            continue;
                        }
                        let shown = match ty.as_str() {
                            "s" => {
                                v.trim().parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default()
                            }
                            "b" => {
                                if v.trim() == "1" { tr!("services-office-true") } else { tr!("services-office-false") }
                                    .to_string()
                            }
                            "e" => v,
                            _ => tidy_number(&v),
                        };
                        if r.len() <= col {
                            r.resize(col + 1, String::new());
                        }
                        r[col] = shown;
                    }
                }
                "row" => {
                    if let Some(r) = row.take() {
                        m.rows.push(r);
                    }
                    if m.rows.len() >= max_rows {
                        m.truncated = true;
                        break;
                    }
                }
                _ => {}
            },
            X::Text(t) => {
                if let Some(c) = cell.as_mut()
                    && (in_v || in_is_t)
                {
                    c.2.push_str(t);
                }
            }
        }
    }
    m.rows.truncate(max_rows);
    finish_sheet(&mut m);
    Ok(m)
}

/// "3.0000000000000004" → "3", "0.1" stays: trims float noise from stored numbers.
fn tidy_number(v: &str) -> String {
    let t = v.trim();
    match t.parse::<f64>() {
        Ok(f) if f.is_finite() && t.len() > 12 => {
            let s = format!("{:.10}", f);
            let s = s.trim_end_matches('0').trim_end_matches('.');
            s.to_string()
        }
        _ => t.to_string(),
    }
}

/// Trim trailing empty columns / rows and record the column count.
fn finish_sheet(m: &mut SheetModel) {
    for r in &mut m.rows {
        while r.last().is_some_and(|c| c.trim().is_empty()) {
            r.pop();
        }
    }
    while m.rows.last().is_some_and(|r| r.is_empty()) {
        m.rows.pop();
    }
    m.ncols = m.rows.iter().map(Vec::len).max().unwrap_or(0);
}

fn ods<R: Read + Seek>(c: &mut Container<R>, max_rows: usize) -> Result<SheetModel, String> {
    let xml = c.text("content.xml").ok_or_else(|| tr!("services-office-no-body-sheet", part = "content.xml"))?;
    let ev = preview_xml::events(&xml, MAX_EVENTS);
    let mut m = SheetModel { format: tr!("services-office-ods"), ..Default::default() };
    let mut table_depth = 0usize;
    let mut first_done = false;
    let mut row: Option<(Vec<String>, usize)> = None; // (cells, repeat)
    let mut cell: Option<(String, usize, Option<String>)> = None; // (text, repeat, value attr)
    let mut p_count = 0usize;
    for e in &ev {
        match e {
            X::Start { name, empty, .. } => match name.as_str() {
                "table" if !empty => {
                    m.sheets.push(e.attr("name").unwrap_or(tr!("services-office-sheet")).to_string());
                    table_depth += 1;
                }
                "table-row" if table_depth == 1 && !first_done => {
                    let rep = e.attr("number-rows-repeated").and_then(|r| r.parse::<usize>().ok()).unwrap_or(1);
                    if *empty {
                        push_rows(&mut m, Vec::new(), rep, max_rows);
                    } else {
                        row = Some((Vec::new(), rep));
                    }
                }
                "table-cell" | "covered-table-cell" if row.is_some() => {
                    let rep = e.attr("number-columns-repeated").and_then(|r| r.parse::<usize>().ok()).unwrap_or(1);
                    let val = e
                        .attr("value")
                        .or_else(|| e.attr("date-value"))
                        .or_else(|| e.attr("boolean-value"))
                        .map(str::to_string);
                    if *empty {
                        push_cells(row.as_mut(), val.unwrap_or_default(), rep, &mut m.truncated);
                    } else {
                        cell = Some((String::new(), rep, val));
                        p_count = 0;
                    }
                }
                "p" if cell.is_some() => {
                    if p_count > 0
                        && let Some(c) = cell.as_mut()
                    {
                        c.0.push('\n');
                    }
                    p_count += 1;
                }
                "s" if cell.is_some() => {
                    if let Some(c) = cell.as_mut() {
                        c.0.push(' ');
                    }
                }
                _ => {}
            },
            X::End { name } => match name.as_str() {
                "table" => {
                    table_depth = table_depth.saturating_sub(1);
                    if table_depth == 0 && !m.sheets.is_empty() {
                        first_done = true;
                    }
                }
                "table-cell" | "covered-table-cell" => {
                    if let Some((text, rep, val)) = cell.take() {
                        let shown = if text.is_empty() { val.unwrap_or_default() } else { text };
                        push_cells(row.as_mut(), shown, rep, &mut m.truncated);
                    }
                }
                "table-row" => {
                    if let Some((cells, rep)) = row.take() {
                        push_rows(&mut m, cells, rep, max_rows);
                    }
                }
                _ => {}
            },
            X::Text(t) => {
                if let Some(c) = cell.as_mut() {
                    c.0.push_str(t);
                }
            }
        }
    }
    finish_sheet(&mut m);
    Ok(m)
}

fn push_cells(row: Option<&mut (Vec<String>, usize)>, v: String, rep: usize, truncated: &mut bool) {
    let Some((cells, _)) = row else { return };
    // Trailing repeated blanks (often 16 000+) are cut by MAX_COLS.
    for _ in 0..rep {
        if cells.len() >= MAX_COLS {
            if !v.is_empty() {
                *truncated = true;
            }
            return;
        }
        cells.push(v.clone());
    }
}

fn push_rows(m: &mut SheetModel, cells: Vec<String>, rep: usize, max_rows: usize) {
    // A huge repeat of blank rows (the sheet's tail) adds nothing.
    let blank = cells.iter().all(|c| c.is_empty());
    let rep = if blank { rep.min(50) } else { rep };
    for _ in 0..rep {
        if m.rows.len() >= max_rows {
            if !blank {
                m.truncated = true;
            }
            return;
        }
        m.rows.push(cells.clone());
    }
}

fn natural_slide_no(path: &str) -> u32 {
    path.rsplit('/')
        .next()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(u32::MAX)
}

fn pptx<R: Read + Seek>(c: &mut Container<R>) -> Result<SlidesModel, String> {
    let mut m = SlidesModel { format: tr!("services-office-pptx"), ..Default::default() };
    let pres_rels = rels(c, "ppt/_rels/presentation.xml.rels", "ppt");
    let mut order: Vec<String> = Vec::new();
    if let Some(xml) = c.text("ppt/presentation.xml") {
        for e in preview_xml::events(&xml, 100_000) {
            if e.is_start("sldId")
                && let Some(t) = e.attr("id").and_then(|id| pres_rels.get(id))
            {
                order.push(t.clone());
            }
        }
    }
    if order.is_empty() {
        order = c.names().into_iter().filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml")).collect();
        order.sort_by_key(|n| natural_slide_no(n));
    }
    for part in order.iter().take(MAX_SLIDES) {
        let Some(xml) = c.text(part) else { continue };
        m.slides.push(pptx_slide(&preview_xml::events(&xml, 400_000)));
    }
    (m.title, _) = ooxml_core(c);
    Ok(m)
}

fn pptx_slide(ev: &[X]) -> Slide {
    let mut s = Slide::default();
    let mut in_sp = 0usize;
    let mut is_title = false;
    let mut para = String::new();
    let mut in_t = false;
    let mut texts: Vec<(bool, String)> = Vec::new();
    for e in ev {
        match e {
            X::Start { name, empty, .. } => match name.as_str() {
                "sp" if !empty => {
                    in_sp += 1;
                    is_title = false;
                }
                "ph" if in_sp > 0 => {
                    is_title |= matches!(e.attr("type"), Some("title" | "ctrTitle"));
                }
                "p" if in_sp > 0 && !empty => para.clear(),
                "t" => in_t = !empty,
                "br" => para.push(' '),
                _ => {}
            },
            X::End { name } => match name.as_str() {
                "sp" => in_sp = in_sp.saturating_sub(1),
                "t" => in_t = false,
                "p" if in_sp > 0 => {
                    let t = para.trim().to_string();
                    if !t.is_empty() {
                        texts.push((is_title, t));
                    }
                    para.clear();
                }
                _ => {}
            },
            X::Text(t) if in_t => para.push_str(t),
            X::Text(_) => {}
        }
    }
    let titles: Vec<String> = texts.iter().filter(|(t, _)| *t).map(|(_, s)| s.clone()).collect();
    s.title = titles.join(" ");
    s.body = texts.into_iter().filter(|(t, _)| !t).map(|(_, s)| s).collect();
    if s.title.is_empty() && !s.body.is_empty() {
        s.title = s.body.remove(0);
    }
    s
}

fn odp<R: Read + Seek>(c: &mut Container<R>) -> Result<SlidesModel, String> {
    let xml = c.text("content.xml").ok_or_else(|| tr!("services-office-no-body-slides", part = "content.xml"))?;
    let ev = preview_xml::events(&xml, MAX_EVENTS);
    let mut m = SlidesModel { format: tr!("services-office-odp"), ..Default::default() };
    let mut slide: Option<Slide> = None;
    let mut frame_title = false;
    let mut in_notes = false;
    let mut para: Option<String> = None;
    for e in &ev {
        match e {
            X::Start { name, empty, .. } => match name.as_str() {
                "page" if !empty => slide = Some(Slide::default()),
                "notes" if !empty => in_notes = true,
                "frame" => frame_title = e.attr("class") == Some("title"),
                "p" | "h" if slide.is_some() && !in_notes && !empty => para = Some(String::new()),
                "s" | "line-break" => {
                    if let Some(p) = para.as_mut() {
                        p.push(' ');
                    }
                }
                _ => {}
            },
            X::End { name } => match name.as_str() {
                "notes" => in_notes = false,
                "p" | "h" => {
                    if let (Some(p), Some(s)) = (para.take(), slide.as_mut()) {
                        let t = p.trim().to_string();
                        if !t.is_empty() {
                            if frame_title && s.title.is_empty() {
                                s.title = t;
                            } else {
                                s.body.push(t);
                            }
                        }
                    }
                }
                "page" => {
                    if let Some(mut s) = slide.take() {
                        if s.title.is_empty() && !s.body.is_empty() {
                            s.title = s.body.remove(0);
                        }
                        m.slides.push(s);
                        if m.slides.len() >= MAX_SLIDES {
                            break;
                        }
                    }
                }
                _ => {}
            },
            X::Text(t) => {
                if let Some(p) = para.as_mut() {
                    p.push_str(t);
                }
            }
        }
    }
    (m.title, _) = odf_meta(c);
    Ok(m)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Minimal Office / ODF files built in memory for tests.
    use std::io::{Cursor, Write};

    pub fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            w.start_file(*name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    pub fn docx() -> Vec<u8> {
        zip(&[
            (
                "word/styles.xml",
                r#"<w:styles xmlns:w="w"><w:style w:styleId="Baslik1"><w:name w:val="heading 1"/></w:style>
                   <w:style w:styleId="Custom2"><w:name w:val="Fancy"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style></w:styles>"#,
            ),
            (
                "word/document.xml",
                r#"<w:document xmlns:w="w"><w:body>
                <w:p><w:pPr><w:pStyle w:val="Baslik1"/></w:pPr><w:r><w:t>Quarterly report</w:t></w:r></w:p>
                <w:p><w:r><w:t xml:space="preserve">Revenue was </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>up 12%</w:t></w:r><w:r><w:rPr><w:i w:val="1"/></w:rPr><w:t> again</w:t></w:r></w:p>
                <w:p></w:p><w:p></w:p>
                <w:p><w:pPr><w:pStyle w:val="Custom2"/></w:pPr><w:r><w:t>Details</w:t></w:r></w:p>
                <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>First &amp; foremost</w:t></w:r></w:p>
                </w:body></w:document>"#,
            ),
            (
                "docProps/core.xml",
                r#"<cp:coreProperties xmlns:cp="c" xmlns:dc="d"><dc:title>Q3</dc:title><dc:creator>Ada</dc:creator></cp:coreProperties>"#,
            ),
        ])
    }

    pub fn xlsx() -> Vec<u8> {
        zip(&[
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="r"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/sharedStrings.xml",
                r#"<sst><si><t>Item</t></si><si><t>Cost</t></si><si><r><t>Coff</t></r><r><t>ee</t></r></si></sst>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData>
                <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
                <row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2"><v>3.5</v></c><c r="D2" t="inlineStr"><is><t>note</t></is></c></row>
                <row r="4"><c r="A4" t="b"><v>1</v></c></row>
                </sheetData></worksheet>"#,
            ),
        ])
    }

    pub fn pptx() -> Vec<u8> {
        let slide = |title: &str, body: &str| {
            format!(
                r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
                <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
                <p:sp><p:nvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{body}</a:t></a:r></a:p></p:txBody></p:sp>
                </p:spTree></p:cSld></p:sld>"#
            )
        };
        let s1 = slide("Welcome", "Agenda");
        let s2 = slide("Results", "Up and to the right");
        zip(&[
            (
                "ppt/presentation.xml",
                r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId id="256" r:id="rId3"/><p:sldId id="257" r:id="rId2"/></p:sldIdLst></p:presentation>"#,
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<Relationships><Relationship Id="rId2" Target="slides/slide2.xml"/><Relationship Id="rId3" Target="slides/slide1.xml"/></Relationships>"#,
            ),
            ("ppt/slides/slide1.xml", &s1),
            ("ppt/slides/slide2.xml", &s2),
        ])
    }

    pub fn odt() -> Vec<u8> {
        zip(&[
            ("mimetype", "application/vnd.oasis.opendocument.text"),
            (
                "content.xml",
                r#"<office:document-content xmlns:office="o" xmlns:text="t" xmlns:style="s" xmlns:fo="f">
                <office:automatic-styles><style:style style:name="T1"><style:text-properties fo:font-weight="bold"/></style:style></office:automatic-styles>
                <office:body><office:text>
                <text:h text:outline-level="2">Chapter</text:h>
                <text:p>Plain <text:span text:style-name="T1">strong</text:span><text:s text:c="2"/>end</text:p>
                <text:list><text:list-item><text:p>Item one</text:p></text:list-item></text:list>
                </office:text></office:body></office:document-content>"#,
            ),
            (
                "meta.xml",
                r#"<office:document-meta xmlns:dc="d"><office:meta><dc:title>Novel</dc:title></office:meta></office:document-meta>"#,
            ),
        ])
    }

    pub fn ods() -> Vec<u8> {
        zip(&[(
            "content.xml",
            r#"<office:document-content xmlns:table="t" xmlns:text="x" xmlns:office="o"><office:body><office:spreadsheet>
            <table:table table:name="Data">
              <table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell table:number-columns-repeated="2"><text:p>b</text:p></table:table-cell></table:table-row>
              <table:table-row><table:table-cell office:value="42"/><table:table-cell table:number-columns-repeated="16000"/></table:table-row>
              <table:table-row table:number-rows-repeated="1048000"><table:table-cell/></table:table-row>
            </table:table>
            <table:table table:name="Second"></table:table>
            </office:spreadsheet></office:body></office:document-content>"#,
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const CAP: u64 = 16 << 20;

    #[test]
    fn docx_headings_runs_lists_and_props() {
        let Office::Doc(d) = parse(Cursor::new(fixtures::docx()), OfficeKind::Docx, CAP, 200).unwrap() else {
            panic!("doc")
        };
        assert_eq!(d.title.as_deref(), Some("Q3"));
        assert_eq!(d.author.as_deref(), Some("Ada"));
        assert_eq!(d.paras[0].level, 1, "localized heading style resolved by name");
        assert_eq!(d.paras[0].text(), "Quarterly report");
        let p = &d.paras[1];
        assert_eq!(p.runs.len(), 3);
        assert!(p.runs[1].bold && !p.runs[1].italic);
        assert!(p.runs[2].italic);
        assert_eq!(p.text(), "Revenue was up 12% again");
        // Two empty paragraphs collapse into one blank.
        assert_eq!(d.paras[2].text(), "");
        assert_eq!(d.paras[3].level, 2, "outline level from the style");
        assert!(d.paras[4].list);
        assert_eq!(d.paras[4].text(), "First & foremost");
        assert_eq!(d.words, 2 + 5 + 1 + 3);
    }

    #[test]
    fn xlsx_first_sheet_with_shared_strings() {
        let Office::Sheet(s) = parse(Cursor::new(fixtures::xlsx()), OfficeKind::Xlsx, CAP, 200).unwrap() else {
            panic!("sheet")
        };
        assert_eq!(s.sheets, vec!["Budget", "Other"]);
        assert_eq!(s.rows[0], vec!["Item", "Cost"]);
        assert_eq!(s.rows[1], vec!["Coffee", "3.5", "", "note"]);
        assert!(s.rows[2].is_empty(), "row 3 is a gap");
        assert_eq!(s.rows[3], vec!["TRUE"]);
        assert_eq!(s.ncols, 4);
        let Office::Sheet(s) = parse(Cursor::new(fixtures::xlsx()), OfficeKind::Xlsx, CAP, 1).unwrap() else {
            panic!()
        };
        assert_eq!(s.rows.len(), 1);
        assert!(s.truncated);
    }

    #[test]
    fn pptx_slides_in_presentation_order() {
        let Office::Slides(s) = parse(Cursor::new(fixtures::pptx()), OfficeKind::Pptx, CAP, 200).unwrap() else {
            panic!()
        };
        assert_eq!(s.slides.len(), 2);
        assert_eq!(s.slides[0], Slide { title: "Welcome".into(), body: vec!["Agenda".into()] });
        assert_eq!(s.slides[1].title, "Results");
    }

    #[test]
    fn odt_and_ods() {
        let Office::Doc(d) = parse(Cursor::new(fixtures::odt()), OfficeKind::Odt, CAP, 200).unwrap() else { panic!() };
        assert_eq!(d.title.as_deref(), Some("Novel"));
        assert_eq!((d.paras[0].level, d.paras[0].text().as_str()), (2, "Chapter"));
        assert_eq!(d.paras[1].text(), "Plain strong  end");
        assert!(d.paras[1].runs.iter().any(|r| r.bold && r.text == "strong"));
        assert!(d.paras[2].list);

        let Office::Sheet(s) = parse(Cursor::new(fixtures::ods()), OfficeKind::Ods, CAP, 200).unwrap() else {
            panic!()
        };
        assert_eq!(s.sheets, vec!["Data", "Second"]);
        assert_eq!(s.rows[0], vec!["a", "b", "b"]);
        assert_eq!(s.rows[1], vec!["42"]);
        assert_eq!(s.rows.len(), 2, "repeated blank tail trimmed");
    }

    #[test]
    fn helpers() {
        assert_eq!(col_index("A1"), Some(0));
        assert_eq!(col_index("AB12"), Some(27));
        assert_eq!(col_index("12"), None);
        assert_eq!(resolve("ppt/slides", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(resolve("xl", "/xl/worksheets/s.xml"), "xl/worksheets/s.xml");
        assert_eq!(tidy_number("3.0000000000000004"), "3");
        assert_eq!(tidy_number("0.1"), "0.1");
        assert!(parse(Cursor::new(b"not a zip".to_vec()), OfficeKind::Docx, CAP, 10).is_err());
    }
}
