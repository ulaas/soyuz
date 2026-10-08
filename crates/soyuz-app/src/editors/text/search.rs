//! Find / replace engine for the text editor: literal or regex queries, case and
//! whole-word options, per-line matching (a match never spans lines), capped match
//! lists and replacement expansion (`$1`, `${name}` in regex mode).

use regex::{Regex, RegexBuilder};

use super::buffer::Pos;

/// Most matches collected for highlighting / counting.
pub const MAX_MATCHES: usize = 100_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub text: String,
    pub regex: bool,
    pub case: bool,
    pub whole_word: bool,
}

/// One match: `line`, byte range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

impl Match {
    pub fn start_pos(&self) -> Pos {
        Pos::new(self.line, self.start)
    }

    pub fn end_pos(&self) -> Pos {
        Pos::new(self.line, self.end)
    }
}

/// Compile a query (`Ok(None)` for an empty query).
pub fn compile(q: &Query) -> Result<Option<Regex>, String> {
    if q.text.is_empty() {
        return Ok(None);
    }
    let body = if q.regex { q.text.clone() } else { regex::escape(&q.text) };
    let pat = if q.whole_word { format!(r"\b(?:{body})\b") } else { body };
    RegexBuilder::new(&pat).case_insensitive(!q.case).size_limit(1 << 23).build().map(Some).map_err(|e| match e {
        regex::Error::Syntax(s) => s.lines().last().unwrap_or(tr!("editors-find-invalid-pattern")).trim().to_string(),
        other => other.to_string(),
    })
}

/// Every non-empty match, in document order (at most `limit`).
pub fn find_all<S: AsRef<str>>(lines: &[S], re: &Regex, limit: usize) -> Vec<Match> {
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        for m in re.find_iter(l.as_ref()) {
            if m.start() == m.end() {
                continue;
            }
            out.push(Match { line: i, start: m.start(), end: m.end() });
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// Index of the first match at or after `p` (wrapping to 0), `None` without matches.
pub fn next_from(matches: &[Match], p: Pos) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let i = matches.partition_point(|m| m.start_pos() < p);
    Some(if i >= matches.len() { 0 } else { i })
}

/// Index of the last match starting before `p` (wrapping to the end).
pub fn prev_from(matches: &[Match], p: Pos) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let i = matches.partition_point(|m| m.start_pos() < p);
    Some(if i == 0 { matches.len() - 1 } else { i - 1 })
}

/// The replacement text for match `m` of `line` (`$n` expansion in regex mode).
pub fn replacement(re: &Regex, line: &str, m: &Match, template: &str, regex_mode: bool) -> String {
    if !regex_mode {
        return template.to_string();
    }
    match re.captures_at(line, m.start) {
        Some(c) if c.get(0).is_some_and(|g| g.start() == m.start) => {
            let mut out = String::new();
            c.expand(template, &mut out);
            out
        }
        _ => template.to_string(),
    }
}

/// Replacements for every match, in *descending* order (ready for
/// [`super::buffer::apply_all`]).
pub fn replace_all_ranges<S: AsRef<str>>(
    lines: &[S],
    re: &Regex,
    template: &str,
    regex_mode: bool,
) -> Vec<(Pos, Pos, String)> {
    let matches = find_all(lines, re, usize::MAX);
    matches
        .iter()
        .rev()
        .map(|m| (m.start_pos(), m.end_pos(), replacement(re, lines[m.line].as_ref(), m, template, regex_mode)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str, regex: bool, case: bool, whole_word: bool) -> Query {
        Query { text: text.into(), regex, case, whole_word }
    }

    #[test]
    fn literal_case_and_whole_word() {
        let lines = ["Foo foo food", "a.b axb"];
        let re = compile(&q("foo", false, false, false)).unwrap().unwrap();
        assert_eq!(find_all(&lines, &re, 100).len(), 3);
        let re = compile(&q("foo", false, true, false)).unwrap().unwrap();
        assert_eq!(find_all(&lines, &re, 100).len(), 2);
        let re = compile(&q("foo", false, false, true)).unwrap().unwrap();
        let m = find_all(&lines, &re, 100);
        assert_eq!(m, vec![Match { line: 0, start: 0, end: 3 }, Match { line: 0, start: 4, end: 7 }]);
        // Literal mode escapes regex syntax.
        let re = compile(&q("a.b", false, true, false)).unwrap().unwrap();
        assert_eq!(find_all(&lines, &re, 100).len(), 1);
        let re = compile(&q("a.b", true, true, false)).unwrap().unwrap();
        assert_eq!(find_all(&lines, &re, 100).len(), 2);
        assert!(compile(&q("", true, true, false)).unwrap().is_none());
        assert!(compile(&q("(", true, true, false)).is_err());
        assert_eq!(find_all(&lines, &compile(&q("o", false, true, false)).unwrap().unwrap(), 2).len(), 2);
    }

    #[test]
    fn navigation_wraps() {
        let m = vec![Match { line: 1, start: 2, end: 3 }, Match { line: 3, start: 0, end: 1 }];
        assert_eq!(next_from(&m, Pos::new(0, 0)), Some(0));
        assert_eq!(next_from(&m, Pos::new(1, 2)), Some(0));
        assert_eq!(next_from(&m, Pos::new(1, 3)), Some(1));
        assert_eq!(next_from(&m, Pos::new(9, 0)), Some(0));
        assert_eq!(prev_from(&m, Pos::new(1, 2)), Some(1));
        assert_eq!(prev_from(&m, Pos::new(2, 0)), Some(0));
        assert_eq!(next_from(&[], Pos::default()), None);
    }

    #[test]
    fn replace_with_groups() {
        let lines = ["key=value", "x=1 y=2"];
        let re = compile(&q(r"(\w+)=(\w+)", true, true, false)).unwrap().unwrap();
        let r = replace_all_ranges(&lines, &re, "$2:$1", true);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], (Pos::new(1, 4), Pos::new(1, 7), "2:y".to_string()));
        assert_eq!(r[2], (Pos::new(0, 0), Pos::new(0, 9), "value:key".to_string()));
        // Literal mode inserts the template verbatim.
        let re = compile(&q("x=1", false, true, false)).unwrap().unwrap();
        let r = replace_all_ranges(&lines, &re, "$1", false);
        assert_eq!(r[0].2, "$1");
        // Applying them keeps positions valid (descending order).
        let mut b = super::super::buffer::Buffer::new("key=value\nx=1 y=2");
        let re = compile(&q(r"(\w+)=(\w+)", true, true, false)).unwrap().unwrap();
        let ranges = replace_all_ranges(b.lines(), &re, "$2", true);
        super::super::buffer::apply_all(&mut b, ranges);
        assert_eq!(b.text(), "value\n1 2");
    }
}
