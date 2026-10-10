//! Two-file text diff for the preview: when exactly two text files are
//! selected, the preview shows their differences (unified or side by side) with the
//! changed words emphasized. Built on `similar` (Patience diff, time-boxed) and computed
//! on a worker thread; each side is read up to `[preview] max_diff_kib`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use similar::{Algorithm, ChangeTag, DiffOp, TextDiff};

use super::preview::Source;
use super::preview_text;

/// Context lines around each change.
pub const CONTEXT: usize = 3;
/// Characters kept per line.
const LINE_CHARS: usize = 2_000;
/// Rows of output kept.
const MAX_ROWS: usize = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Equal,
    Delete,
    Insert,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub tag: Tag,
    /// 1-based line numbers on each side.
    pub old: Option<usize>,
    pub new: Option<usize>,
    /// `(emphasized, text)` segments, no trailing newline.
    pub segs: Vec<(bool, String)>,
}

impl Line {
    pub fn text(&self) -> String {
        self.segs.iter().map(|(_, s)| s.as_str()).collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// `@@ -a,b +c,d @@`
    Hunk(String),
    Line(Line),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SideRow {
    Hunk(String),
    Pair(Option<Line>, Option<Line>),
}

#[derive(Clone, Debug, Default)]
pub struct DiffModel {
    pub unified: Vec<Row>,
    pub side: Vec<SideRow>,
    pub added: usize,
    pub removed: usize,
    pub identical: bool,
    /// One side was cut at the size limit (or the output at the row limit).
    pub truncated: bool,
    /// Not text: no line diff, just "differ / identical".
    pub binary: bool,
    pub encodings: (&'static str, &'static str),
}

fn clip(s: &str) -> String {
    let s = s.trim_end_matches(['\n', '\r']);
    if s.chars().count() > LINE_CHARS { s.chars().take(LINE_CHARS).collect::<String>() + "…" } else { s.to_string() }
}

fn hunk_header(group: &[DiffOp]) -> String {
    let (Some(first), Some(last)) = (group.first(), group.last()) else { return String::new() };
    let o = first.old_range().start..last.old_range().end;
    let n = first.new_range().start..last.new_range().end;
    format!("@@ -{},{} +{},{} @@", o.start + 1, o.len(), n.start + 1, n.len())
}

/// Diff two texts.
pub fn diff_texts(a: &str, b: &str) -> DiffModel {
    let mut m = DiffModel { identical: a == b, ..Default::default() };
    if m.identical {
        return m;
    }
    let diff = TextDiff::configure().algorithm(Algorithm::Patience).timeout(Duration::from_secs(2)).diff_lines(a, b);
    let deadline = Some(Instant::now() + Duration::from_millis(1500));
    for group in diff.grouped_ops(CONTEXT) {
        if m.unified.len() > MAX_ROWS {
            m.truncated = true;
            break;
        }
        let h = hunk_header(&group);
        m.unified.push(Row::Hunk(h.clone()));
        m.side.push(SideRow::Hunk(h));
        for op in &group {
            let mut dels: Vec<Line> = Vec::new();
            let mut ins: Vec<Line> = Vec::new();
            for ch in diff.iter_inline_changes_deadline(op, deadline) {
                let segs: Vec<(bool, String)> = {
                    let mut v: Vec<(bool, String)> = Vec::new();
                    for (emph, s) in ch.iter_strings_lossy() {
                        let s = s.trim_end_matches(['\n', '\r']).to_string();
                        if s.is_empty() {
                            continue;
                        }
                        match v.last_mut() {
                            Some((e, prev)) if *e == emph => prev.push_str(&s),
                            _ => v.push((emph, s)),
                        }
                    }
                    let total: usize = v.iter().map(|(_, s)| s.chars().count()).sum();
                    if total > LINE_CHARS {
                        vec![(false, clip(&v.iter().map(|(_, s)| s.as_str()).collect::<String>()))]
                    } else {
                        v
                    }
                };
                let tag = match ch.tag() {
                    ChangeTag::Equal => Tag::Equal,
                    ChangeTag::Delete => Tag::Delete,
                    ChangeTag::Insert => Tag::Insert,
                };
                let line = Line { tag, old: ch.old_index().map(|i| i + 1), new: ch.new_index().map(|i| i + 1), segs };
                m.unified.push(Row::Line(line.clone()));
                match tag {
                    Tag::Equal => {
                        flush_pairs(&mut m.side, &mut dels, &mut ins);
                        m.side.push(SideRow::Pair(Some(line.clone()), Some(line)));
                    }
                    Tag::Delete => {
                        m.removed += 1;
                        dels.push(line);
                    }
                    Tag::Insert => {
                        m.added += 1;
                        ins.push(line);
                    }
                }
            }
            flush_pairs(&mut m.side, &mut dels, &mut ins);
        }
    }
    m
}

fn flush_pairs(side: &mut Vec<SideRow>, dels: &mut Vec<Line>, ins: &mut Vec<Line>) {
    let n = dels.len().max(ins.len());
    let mut d = dels.drain(..);
    let mut i = ins.drain(..);
    for _ in 0..n {
        side.push(SideRow::Pair(d.next(), i.next()));
    }
}

/// Read one side: up to `cap` bytes, `(bytes, truncated)`.
fn read_side(path: &Path, cap: u64) -> Result<(Vec<u8>, bool), String> {
    match Source::from_path(path) {
        Source::File(p) => {
            use std::io::Read;
            let f = std::fs::File::open(&p).map_err(|e| e.to_string())?;
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            let mut buf = Vec::new();
            f.take(cap).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            Ok((buf, len > cap))
        }
        Source::Entry { archive, inner } => super::preview::read_archive_entry(&archive, &inner, cap),
    }
}

/// Load and diff two files.
pub fn diff_files(a: &Path, b: &Path, cap: u64) -> Result<DiffModel, String> {
    let (ab, at) = read_side(a, cap)?;
    let (bb, bt) = read_side(b, cap)?;
    let head = |v: &[u8]| v[..v.len().min(8192)].to_vec();
    if !preview_text::looks_like_text(&head(&ab)) || !preview_text::looks_like_text(&head(&bb)) {
        return Ok(DiffModel { binary: true, identical: ab == bb && !at && !bt, ..Default::default() });
    }
    let (ta, ea) = preview_text::decode_text(&ab);
    let (tb, eb) = preview_text::decode_text(&bb);
    let mut m = diff_texts(&ta, &tb);
    m.truncated |= at || bt;
    m.encodings = (ea, eb);
    Ok(m)
}

/// One finished diff.
pub struct DiffResult {
    pub id: u64,
    pub pair: (PathBuf, PathBuf),
    pub result: Result<Arc<DiffModel>, String>,
}

/// Latest-wins diff requests on a worker thread.
pub struct DiffClient {
    latest: Arc<AtomicU64>,
    tx: Sender<DiffResult>,
    rx: Receiver<DiffResult>,
}

impl Default for DiffClient {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        DiffClient { latest: Arc::new(AtomicU64::new(0)), tx, rx }
    }
}

impl DiffClient {
    pub fn request(&self, a: PathBuf, b: PathBuf, cap: u64, ctx: &egui::Context) -> u64 {
        let id = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, latest, ctx) = (self.tx.clone(), self.latest.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("soyuz-diff".into()).spawn(move || {
            if latest.load(Ordering::SeqCst) != id {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| diff_files(&a, &b, cap)))
                .unwrap_or_else(|_| Err(tr!("services-diff-failed").into()))
                .map(Arc::new);
            if latest.load(Ordering::SeqCst) == id {
                let _ = tx.send(DiffResult { id, pair: (a, b), result });
                ctx.request_repaint();
            }
        });
        id
    }

    pub fn cancel(&self) {
        self.latest.fetch_add(1, Ordering::SeqCst);
    }

    pub fn latest(&self) -> u64 {
        self.latest.load(Ordering::SeqCst)
    }

    pub fn try_recv(&self) -> Option<DiffResult> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_and_side_by_side() {
        let a = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        let b = "one\ntwo\nthree\nFOUR!\nfive\nsix\nseven\neight\nnine\nten\neleven\n";
        let m = diff_texts(a, b);
        assert!(!m.identical);
        assert_eq!((m.added, m.removed), (2, 1));
        let hunks: Vec<&String> =
            m.unified.iter().filter_map(|r| if let Row::Hunk(h) = r { Some(h) } else { None }).collect();
        assert!(!hunks.is_empty());
        assert!(hunks[0].starts_with("@@ -1,"), "{}", hunks[0]);
        let del = m
            .unified
            .iter()
            .find_map(|r| match r {
                Row::Line(l) if l.tag == Tag::Delete => Some(l.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!((del.old, del.new, del.text().as_str()), (Some(4), None, "four"));
        // Side by side pairs the replaced line with its replacement.
        let pair = m
            .side
            .iter()
            .find_map(|r| match r {
                SideRow::Pair(Some(l), Some(r)) if l.tag == Tag::Delete => Some((l.text(), r.text())),
                _ => None,
            })
            .unwrap();
        assert_eq!(pair, ("four".to_string(), "FOUR!".to_string()));
        assert!(m.side.iter().any(|r| matches!(r, SideRow::Pair(None, Some(l)) if l.text() == "eleven")));
    }

    #[test]
    fn inline_emphasis_marks_changed_words() {
        let m = diff_texts("let x = 1;\n", "let x = 2;\n");
        let ins = m
            .unified
            .iter()
            .find_map(|r| match r {
                Row::Line(l) if l.tag == Tag::Insert => Some(l.clone()),
                _ => None,
            })
            .unwrap();
        assert!(ins.segs.iter().any(|(e, s)| *e && s.contains('2')), "{:?}", ins.segs);
        assert!(ins.segs.iter().any(|(e, s)| !*e && s.contains("let")));
    }

    #[test]
    fn identical_binary_and_files() {
        assert!(diff_texts("same\n", "same\n").identical);
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a.txt"), dir.path().join("b.txt"), dir.path().join("c.bin"));
        std::fs::write(&a, "alpha\nbeta\n").unwrap();
        std::fs::write(&b, "alpha\ngamma\n").unwrap();
        std::fs::write(&c, [0u8, 1, 2, 0, 0, 5]).unwrap();
        let m = diff_files(&a, &b, 1 << 20).unwrap();
        assert_eq!((m.added, m.removed, m.truncated), (1, 1, false));
        assert!(diff_files(&a, &c, 1 << 20).unwrap().binary);
        let m = diff_files(&a, &b, 8).unwrap();
        assert!(m.truncated, "cut at the size cap");
    }
}
