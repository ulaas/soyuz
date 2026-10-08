//! Text-side preview helpers: text/binary sniffing and decoding, syntect highlighting
//! into egui `LayoutJob`s (run on the preview workers), and a small Markdown block
//! parser whose output the preview pane renders "nicely but plainly" (PLAN M9).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Longest line we lay out (longer ones are cut with an ellipsis).
pub const MAX_LINE_CHARS: usize = 2_000;

/// Extension of a file *name* (no dot), `None` for dot-files / names without a dot.
pub fn name_ext(name: &str) -> Option<&str> {
    let idx = name.rfind('.')?;
    if idx == 0 || idx + 1 == name.len() {
        return None;
    }
    Some(&name[idx + 1..])
}

/// Heuristic: no NUL bytes and (almost) valid UTF-8 / UTF-16 with BOM.
pub fn looks_like_text(head: &[u8]) -> bool {
    if head.is_empty() {
        return true;
    }
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // A multi-byte char cut at the end of the sample is fine.
        Err(e) if e.error_len().is_none() => true,
        Err(_) => {
            // Latin-1-ish text: few control characters.
            let ctrl = head.iter().filter(|&&b| b < 0x09 || (0x0E..0x20).contains(&b)).count();
            ctrl * 100 < head.len()
        }
    }
}

/// Decode bytes as UTF-8 (lossy), UTF-16 LE/BE with BOM, stripping a UTF-8 BOM.
/// Returns the text and an encoding label.
pub fn decode_text(bytes: &[u8]) -> (String, &'static str) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(rest).into_owned(), "UTF-8 (BOM)");
    }
    for (bom, be, label) in [([0xFF, 0xFE], false, "UTF-16 LE"), ([0xFE, 0xFF], true, "UTF-16 BE")] {
        if let Some(rest) = bytes.strip_prefix(&bom) {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
                .collect();
            return (String::from_utf16_lossy(&units), label);
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), "UTF-8"),
        Err(e) if e.error_len().is_none() => (String::from_utf8_lossy(bytes).into_owned(), "UTF-8"),
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), "Latin-1"),
    }
}

/// `[preview] syntax_theme`: a syntect theme name, or `auto` (follow the app theme).
static SYNTAX_THEME: parking_lot::RwLock<String> = parking_lot::RwLock::new(String::new());

/// Apply `[preview] syntax_theme`; returns a warning for an unknown theme name.
pub fn set_syntax_theme(name: &str) -> Option<String> {
    let name = name.trim();
    let auto = name.is_empty() || name.eq_ignore_ascii_case("auto");
    *SYNTAX_THEME.write() = if auto { String::new() } else { name.to_string() };
    if auto || syntax().1.themes.contains_key(name) {
        return None;
    }
    let known: Vec<&str> = syntax().1.themes.keys().map(String::as_str).collect();
    Some(format!("[preview] syntax_theme = \"{name}\" is unknown (auto, {}); using auto", known.join(", ")))
}

fn syntax() -> &'static (SyntaxSet, ThemeSet) {
    static S: OnceLock<(SyntaxSet, ThemeSet)> = OnceLock::new();
    S.get_or_init(|| (SyntaxSet::load_defaults_newlines(), ThemeSet::load_defaults()))
}

/// Extension aliases for languages the default syntax set names differently / lacks.
fn syntax_ext(ext: &str) -> &str {
    match ext {
        "ts" | "tsx" | "mjs" | "cjs" | "jsx" => "js",
        "hpp" | "hh" | "hxx" | "cc" | "cxx" | "ino" => "cpp",
        "h" => "c",
        "yml" => "yaml",
        "ps1" | "psm1" | "psd1" | "bash" | "zsh" | "fish" | "envrc" => "sh",
        "htm" | "xhtml" | "vue" | "svelte" => "html",
        "kts" | "gradle" => "java",
        "csproj" | "props" | "targets" | "xaml" | "svg" | "plist" | "resx" | "config" => "xml",
        "jsonc" | "json5" | "ipynb" => "json",
        "toml" | "ini" | "cfg" | "conf" | "editorconfig" => "ini",
        other => other,
    }
}

/// Human-readable language name of a file for the preview header (`None` = plain text).
pub fn language_name(name: &str) -> Option<String> {
    let (ss, _) = syntax();
    let ext = name_ext(name).map(str::to_ascii_lowercase).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let token = match lower.as_str() {
        "makefile" | "gnumakefile" => "makefile",
        "dockerfile" => "dockerfile",
        "cmakelists.txt" => "cmake",
        _ => syntax_ext(&ext),
    };
    ss.find_syntax_by_extension(token)
        .or_else(|| ss.find_syntax_by_token(token))
        .filter(|s| s.name != "Plain Text")
        .map(|s| s.name.clone())
}

/// Highlight `text` into one `LayoutJob` per line. Lines past `budget` (or after
/// `cancelled` returns true) fall back to plain text in `fg`.
pub fn highlight(
    text: &str,
    name: &str,
    dark: bool,
    font: FontId,
    fg: Color32,
    budget: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Vec<LayoutJob> {
    let (ss, ts) = syntax();
    let lang = language_name(name);
    let syntax = lang.as_deref().and_then(|l| ss.find_syntax_by_name(l));
    let configured = SYNTAX_THEME.read().clone();
    let auto = if dark { "base16-ocean.dark" } else { "InspiredGitHub" };
    let theme = ts.themes.get(configured.trim()).unwrap_or(&ts.themes[auto]);
    let mut hl = syntax.map(|s| HighlightLines::new(s, theme));
    let start = Instant::now();
    let mut jobs = Vec::new();
    for (i, line) in LinesWithEndings::from(text).enumerate() {
        if hl.is_some() && i % 64 == 0 && (start.elapsed() > budget || cancelled()) {
            hl = None;
        }
        let mut job = LayoutJob::default();
        let clean = clean_line(line);
        match hl.as_mut().and_then(|h| h.highlight_line(line, ss).ok()) {
            Some(ranges) => {
                let mut used = 0usize;
                for (style, piece) in ranges {
                    let piece = clean_line(piece);
                    if piece.is_empty() || used >= MAX_LINE_CHARS {
                        continue;
                    }
                    let take: String = piece.chars().take(MAX_LINE_CHARS - used).collect();
                    used += take.chars().count();
                    let c = style.foreground;
                    let mut color = Color32::from_rgb(c.r, c.g, c.b);
                    if !dark && color == Color32::from_rgb(0x32, 0x32, 0x32) {
                        color = fg;
                    }
                    let fmt = TextFormat {
                        font_id: font.clone(),
                        color,
                        italics: style.font_style.contains(FontStyle::ITALIC),
                        ..Default::default()
                    };
                    job.append(&take, 0.0, fmt);
                }
            }
            None => {
                let take: String = clean.chars().take(MAX_LINE_CHARS).collect();
                job.append(&take, 0.0, TextFormat { font_id: font.clone(), color: fg, ..Default::default() });
            }
        }
        job.wrap.max_rows = 1;
        jobs.push(job);
    }
    if jobs.is_empty() {
        jobs.push(LayoutJob::default());
    }
    jobs
}

/// Strip line endings and expand tabs (4 columns).
pub fn clean_line(s: &str) -> String {
    let s = s.trim_end_matches(['\n', '\r']);
    if s.contains('\t') { s.replace('\t', "    ") } else { s.to_string() }
}

// ---- Markdown -----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Span>),
    Para(Vec<Span>),
    /// Bullet item with nesting depth (0-based).
    Bullet(u8, Vec<Span>),
    /// Numbered item: depth, label ("3.").
    Numbered(u8, String, Vec<Span>),
    /// Task list item: depth, checked.
    Task(u8, bool, Vec<Span>),
    Quote(Vec<Span>),
    Code {
        lang: String,
        lines: Vec<String>,
    },
    /// Table rows kept verbatim (rendered monospace).
    Table(Vec<String>),
    Rule,
}

/// Parse Markdown into display blocks (CommonMark subset: ATX headings, paragraphs,
/// lists, task lists, block quotes, fenced code, tables, rules, inline code / bold /
/// italic / links).
pub fn parse_markdown(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut lines = src.lines().peekable();
    let flush = |para: &mut Vec<String>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(inline(&para.join(" "))));
            para.clear();
        }
    };
    while let Some(raw) = lines.next() {
        let line = raw.trim_end();
        let t = line.trim_start();
        let indent = (line.len() - t.len()) as u8;
        if t.is_empty() {
            flush(&mut para, &mut out);
            continue;
        }
        if let Some(fence) = ["```", "~~~"].iter().find(|f| t.starts_with(**f)) {
            flush(&mut para, &mut out);
            let lang = t.trim_start_matches(*fence).trim().to_string();
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with(*fence) {
                    break;
                }
                code.push(clean_line(l));
            }
            out.push(Block::Code { lang, lines: code });
            continue;
        }
        if t.starts_with('#') {
            let level = t.chars().take_while(|&c| c == '#').count();
            let rest = &t[level..];
            if level <= 6 && (rest.is_empty() || rest.starts_with(' ')) {
                flush(&mut para, &mut out);
                out.push(Block::Heading(level as u8, inline(rest.trim().trim_end_matches('#').trim())));
                continue;
            }
        }
        if is_rule(t) {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
            continue;
        }
        // Setext headings: "Title\n====".
        if !para.is_empty() && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-')) && t.len() >= 2 {
            let level = if t.starts_with('=') { 1 } else { 2 };
            let text = para.join(" ");
            para.clear();
            out.push(Block::Heading(level, inline(&text)));
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            flush(&mut para, &mut out);
            out.push(Block::Quote(inline(rest.trim())));
            continue;
        }
        if t.starts_with('|') {
            flush(&mut para, &mut out);
            let mut rows = vec![t.to_string()];
            while let Some(next) = lines.peek() {
                let n = next.trim();
                if !n.starts_with('|') {
                    break;
                }
                rows.push(n.to_string());
                lines.next();
            }
            out.push(Block::Table(rows));
            continue;
        }
        let depth = indent / 2;
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
            flush(&mut para, &mut out);
            let task = rest
                .strip_prefix("[ ] ")
                .map(|r| (false, r))
                .or_else(|| rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")).map(|r| (true, r)));
            out.push(match task {
                Some((checked, r)) => Block::Task(depth, checked, inline(r)),
                None => Block::Bullet(depth, inline(rest)),
            });
            continue;
        }
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && digits < 10 {
            let after = &t[digits..];
            if let Some(rest) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
                flush(&mut para, &mut out);
                out.push(Block::Numbered(depth, format!("{}.", &t[..digits]), inline(rest)));
                continue;
            }
        }
        para.push(t.to_string());
    }
    flush(&mut para, &mut out);
    out
}

fn is_rule(t: &str) -> bool {
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-') || compact.chars().all(|c| c == '*') || compact.chars().all(|c| c == '_'))
}

/// Inline formatting: `code`, **bold**, __bold__, *italic*, _italic_, [text](url),
/// ![alt](img) (shown as alt text).
pub fn inline(s: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut cur = Span::default();
    let (mut bold, mut italic) = (false, false);
    let mut i = 0;
    let flush = |cur: &mut Span, out: &mut Vec<Span>, bold: bool, italic: bool| {
        if !cur.text.is_empty() {
            out.push(std::mem::take(cur));
        }
        cur.bold = bold;
        cur.italic = italic;
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            cur.text.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '`'
            && let Some(end) = chars[i + 1..].iter().position(|&x| x == '`')
        {
            flush(&mut cur, &mut out, bold, italic);
            let code: String = chars[i + 1..i + 1 + end].iter().collect();
            out.push(Span { text: code, code: true, ..Default::default() });
            i += end + 2;
            continue;
        }
        if (c == '*' || c == '_') && chars.get(i + 1) == Some(&c) {
            flush(&mut cur, &mut out, bold, italic);
            bold = !bold;
            cur.bold = bold;
            i += 2;
            continue;
        }
        if c == '*' || (c == '_' && (i == 0 || !chars[i - 1].is_alphanumeric())) || (c == '_' && italic) {
            flush(&mut cur, &mut out, bold, italic);
            italic = !italic;
            cur.italic = italic;
            i += 1;
            continue;
        }
        let image = c == '!' && chars.get(i + 1) == Some(&'[');
        if c == '[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = chars[open..].iter().position(|&x| x == ']').map(|p| p + open)
                && chars.get(close + 1) == Some(&'(')
                && let Some(end) = chars[close..].iter().position(|&x| x == ')').map(|p| p + close)
            {
                flush(&mut cur, &mut out, bold, italic);
                let text: String = chars[open + 1..close].iter().collect();
                out.push(Span { text, link: !image, italic: image, bold, ..Default::default() });
                i = end + 1;
                continue;
            }
        }
        cur.text.push(c);
        i += 1;
    }
    if !cur.text.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_sniffing() {
        assert!(looks_like_text(b"hello\nworld"));
        assert!(looks_like_text("ünïcødé".as_bytes()));
        assert!(!looks_like_text(&[0x7F, b'E', b'L', b'F', 0, 0, 1]));
        assert!(looks_like_text(&[0xFF, 0xFE, b'a', 0]));
        // A UTF-8 sequence cut at the end of the sample still counts as text.
        let s = "abc€".as_bytes();
        assert!(looks_like_text(&s[..s.len() - 1]));
    }

    #[test]
    fn decoding() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhi"), ("hi".to_string(), "UTF-8 (BOM)"));
        assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0]).0, "hi");
        assert_eq!(decode_text(&[0xFE, 0xFF, 0, b'h', 0, b'i']).0, "hi");
        assert_eq!(decode_text(&[b'a', 0xE9, b'b']).1, "Latin-1");
    }

    #[test]
    fn languages() {
        assert_eq!(language_name("main.rs").as_deref(), Some("Rust"));
        assert_eq!(language_name("app.tsx").as_deref(), Some("JavaScript"));
        assert_eq!(language_name("notes.txt"), None);
        assert_eq!(language_name("Makefile").as_deref(), Some("Makefile"));
    }

    #[test]
    fn highlighting_produces_one_job_per_line() {
        let src = "fn main() {\n\tprintln!(\"hi\");\n}\n";
        let jobs =
            highlight(src, "a.rs", true, FontId::monospace(12.0), Color32::WHITE, Duration::from_secs(5), &|| false);
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[1].text, "    println!(\"hi\");");
        assert!(jobs[0].sections.len() > 1, "keywords get their own colors");
        // Zero budget → plain text but still one job per line.
        let plain = highlight(src, "a.rs", true, FontId::monospace(12.0), Color32::WHITE, Duration::ZERO, &|| true);
        assert_eq!(plain.len(), 3);
        assert_eq!(plain[0].sections.len(), 1);
    }

    #[test]
    fn markdown_blocks() {
        let md = "# Title\n\nSome *em* and **strong** text\ncontinued `code`.\n\n- one\n  - nested\n- [x] done\n\n\
                  1. first\n2) second\n\n> quote\n\n```rust\nfn x() {}\n```\n---\n| a | b |\n|---|---|\n| 1 | 2 |\n\nSub\n===\n";
        let b = parse_markdown(md);
        assert!(matches!(&b[0], Block::Heading(1, s) if s[0].text == "Title"));
        match &b[1] {
            Block::Para(spans) => {
                assert!(spans.iter().any(|s| s.italic && s.text == "em"));
                assert!(spans.iter().any(|s| s.bold && s.text == "strong"));
                assert!(spans.iter().any(|s| s.code && s.text == "code"));
                assert!(spans.iter().any(|s| s.text.contains("continued")));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&b[2], Block::Bullet(0, _)));
        assert!(matches!(&b[3], Block::Bullet(1, _)));
        assert!(matches!(&b[4], Block::Task(0, true, _)));
        assert!(matches!(&b[5], Block::Numbered(0, n, _) if n == "1."));
        assert!(matches!(&b[6], Block::Numbered(0, n, _) if n == "2."));
        assert!(matches!(&b[7], Block::Quote(_)));
        assert!(matches!(&b[8], Block::Code { lang, lines } if lang == "rust" && lines.len() == 1));
        assert!(matches!(&b[9], Block::Rule));
        assert!(matches!(&b[10], Block::Table(rows) if rows.len() == 3));
        assert!(matches!(&b[11], Block::Heading(1, s) if s[0].text == "Sub"));
    }

    #[test]
    fn inline_links_and_escapes() {
        let s = inline("see [docs](http://x) and ![logo](a.png) \\*not em\\* snake_case");
        assert!(s.iter().any(|s| s.link && s.text == "docs"));
        assert!(s.iter().any(|s| !s.link && s.text == "logo"));
        let all: String = s.iter().map(|s| s.text.as_str()).collect();
        assert!(all.contains("*not em*"));
        assert!(all.contains("snake_case"), "intra-word underscores are literal: {all}");
    }
}
