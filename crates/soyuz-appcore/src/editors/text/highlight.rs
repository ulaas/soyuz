//! Incremental syntax highlighting for the editor: syntect parse / highlight states are
//! checkpointed every [`STEP`] lines, only the visible lines are highlighted (within a
//! per-frame time budget; the rest of the frame falls back to plain text and asks for
//! another frame), and an edit only invalidates the states after the edited line.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use syntect::highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter as SynHighlighter, Theme};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference};

use crate::services::preview_gfx::{Rgba, rgb};
use crate::services::preview_text;

/// Lines between saved parser states.
pub const STEP: usize = 128;
/// Longer lines are not parsed (shown plain; the parser state carries over).
const MAX_PARSE_LINE: usize = 20_000;
/// Further than this from the last parser state, show an approximation first.
const APPROX_GAP: usize = 4_000;
/// Lines parsed before the view for the approximation.
const APPROX_LEAD: usize = 200;
/// Cached highlighted lines before the cache is dropped.
const MAX_CACHED: usize = 20_000;

/// A colored byte range of one line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    /// Premultiplied sRGBA (opaque).
    pub color: Rgba,
    pub italic: bool,
}

pub struct Highlighter {
    syntax: Option<&'static SyntaxReference>,
    theme: Option<&'static Theme>,
    checkpoints: Vec<(ParseState, HighlightState)>,
    cache: HashMap<usize, Arc<Vec<Span>>>,
    /// Spans from a fresh parser state near the view (big jumps), until exact ones exist.
    approx: HashMap<usize, Arc<Vec<Span>>>,
}

impl Highlighter {
    /// For a file name (plain text when no syntax matches).
    pub fn new(name: &str) -> Self {
        Highlighter {
            syntax: preview_text::syntax_for(name),
            theme: None,
            checkpoints: Vec::new(),
            cache: HashMap::new(),
            approx: HashMap::new(),
        }
    }

    pub fn language(&self) -> Option<&str> {
        self.syntax.map(|s| s.name.as_str())
    }

    /// Lines from `line` on changed: drop the states / spans that depended on them.
    pub fn invalidate_from(&mut self, line: usize) {
        self.checkpoints.truncate(line / STEP + 1);
        self.cache.retain(|l, _| *l < line);
        self.approx.retain(|l, _| *l < line);
    }

    pub fn reset(&mut self) {
        self.checkpoints.clear();
        self.cache.clear();
        self.approx.clear();
    }

    pub fn get(&self, line: usize) -> Option<&Arc<Vec<Span>>> {
        self.cache.get(&line).or_else(|| self.approx.get(&line))
    }

    /// Highlight lines `first..=last` (as far as `budget` allows). Returns `true` when
    /// every requested line has exact spans (or the file has no syntax). Far from the
    /// last saved state, an approximation (parsing from shortly before `first`) is
    /// shown until the exact pass catches up over the next frames.
    pub fn prepare(&mut self, lines: &[String], first: usize, last: usize, dark: bool, budget: Duration) -> bool {
        let Some(syntax) = self.syntax else { return true };
        let theme = preview_text::active_theme(dark);
        if !self.theme.is_some_and(|t| std::ptr::eq(t, theme)) {
            self.theme = Some(theme);
            self.reset();
        }
        let last = last.min(lines.len().saturating_sub(1));
        if (first..=last).all(|l| self.cache.contains_key(&l)) {
            return true;
        }
        if self.cache.len() > MAX_CACHED {
            self.cache.clear();
        }
        if self.approx.len() > MAX_CACHED {
            self.approx.clear();
        }
        let hl = SynHighlighter::new(theme);
        if self.checkpoints.is_empty() {
            self.checkpoints.push((ParseState::new(syntax), HighlightState::new(&hl, ScopeStack::new())));
        }
        let k = (first / STEP).min(self.checkpoints.len() - 1);
        let state = self.checkpoints[k].clone();
        let done = self.run(lines, state, k * STEP, first, last, Some(budget), false, &hl);
        if !done
            && first.saturating_sub(self.checkpoints.len() * STEP) > APPROX_GAP
            && !(first..=last).all(|l| self.approx.contains_key(&l))
        {
            let from = first.saturating_sub(APPROX_LEAD);
            let fresh = (ParseState::new(syntax), HighlightState::new(&hl, ScopeStack::new()));
            self.run(lines, fresh, from, first, last, None, true, &hl);
        }
        done
    }

    /// Parse from `line` (state `state`) through `last`, building spans for `first..`.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        lines: &[String],
        state: (ParseState, HighlightState),
        mut line: usize,
        first: usize,
        last: usize,
        budget: Option<Duration>,
        approx: bool,
        hl: &SynHighlighter<'_>,
    ) -> bool {
        let ss = &preview_text::syntax_assets().0;
        let (mut ps, mut hs) = state;
        let start = Instant::now();
        let mut buf = String::new();
        while line <= last {
            if !approx && line.is_multiple_of(STEP) && line / STEP == self.checkpoints.len() {
                self.checkpoints.push((ps.clone(), hs.clone()));
            }
            if line < first && line.is_multiple_of(32) && budget.is_some_and(|b| start.elapsed() > b) {
                return false;
            }
            let text = &lines[line];
            let want = line >= first && !self.cache.contains_key(&line);
            if text.len() > MAX_PARSE_LINE {
                if want {
                    self.store(line, Vec::new(), approx);
                }
                line += 1;
                continue;
            }
            buf.clear();
            buf.push_str(text);
            buf.push('\n');
            let ops = ps.parse_line(&buf, ss).unwrap_or_default();
            if want {
                let mut spans = Vec::new();
                let mut pos = 0u32;
                for (style, piece) in HighlightIterator::new(&mut hs, &ops, &buf, hl) {
                    let end = (pos as usize + piece.len()).min(text.len()) as u32;
                    if end > pos {
                        let c = style.foreground;
                        spans.push(Span {
                            start: pos,
                            end,
                            color: rgb(c.r, c.g, c.b),
                            italic: style.font_style.contains(FontStyle::ITALIC),
                        });
                    }
                    pos += piece.len() as u32;
                }
                self.store(line, spans, approx);
            } else {
                // Keep the highlight state in step without building spans.
                for _ in HighlightIterator::new(&mut hs, &ops, &buf, hl) {}
            }
            line += 1;
        }
        true
    }

    fn store(&mut self, line: usize, spans: Vec<Span>, approx: bool) {
        if approx {
            self.approx.insert(line, Arc::new(spans));
        } else {
            self.approx.remove(&line);
            self.cache.insert(line, Arc::new(spans));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.split('\n').map(str::to_string).collect()
    }

    #[test]
    fn highlights_visible_lines_and_invalidates_after_edits() {
        let mut src: Vec<String> = (0..400).map(|i| format!("let x{i} = {i}; // c")).collect();
        src.insert(0, "fn main() {".into());
        let mut h = Highlighter::new("a.rs");
        assert_eq!(h.language(), Some("Rust"));
        assert!(h.prepare(&src, 300, 310, true, Duration::from_secs(5)));
        assert!(h.get(305).is_some_and(|s| s.len() > 1), "keywords and comments get their own colors");
        assert!(h.get(10).is_none(), "only the requested lines are cached");
        assert_eq!(h.checkpoints.len(), 3);
        h.invalidate_from(200);
        assert_eq!(h.checkpoints.len(), 2);
        assert!(h.get(305).is_none() && h.get(150).is_none());
        // A zero budget still finishes when it starts at a checkpoint.
        assert!(h.prepare(&src, 0, 5, true, Duration::ZERO));
        // Plain text files need nothing.
        let mut plain = Highlighter::new("notes.txt");
        assert!(plain.prepare(&lines("a\nb"), 0, 1, true, Duration::ZERO));
        assert!(plain.get(0).is_none());
    }

    #[test]
    fn far_jumps_get_an_approximation_first() {
        let src: Vec<String> = (0..20_000).map(|i| format!("let x{i} = {i};")).collect();
        let mut h = Highlighter::new("a.rs");
        assert!(!h.prepare(&src, 19_000, 19_010, true, Duration::ZERO));
        assert!(h.get(19_005).is_some_and(|s| s.len() > 1), "approximate colors right away");
        assert!(h.prepare(&src, 19_000, 19_010, true, Duration::from_secs(30)));
        assert!(h.approx.is_empty() || !h.approx.contains_key(&19_005));
    }

    #[test]
    fn multiline_state_carries_across_checkpoints() {
        // A block comment opened on line 0 colors line 200 like a comment.
        let mut src = vec!["/*".to_string()];
        src.extend((0..300).map(|_| "let a = 1;".to_string()));
        src.push("*/".into());
        let mut commented = Highlighter::new("a.rs");
        commented.prepare(&src, 200, 200, true, Duration::from_secs(5));
        let mut code = Highlighter::new("a.rs");
        let plain: Vec<String> = src.iter().skip(1).cloned().collect();
        code.prepare(&plain, 199, 199, true, Duration::from_secs(5));
        assert_eq!(commented.get(200).unwrap().len(), 1, "one comment-colored span");
        assert!(code.get(199).unwrap().len() > 1);
    }
}
