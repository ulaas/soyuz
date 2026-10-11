//! The shared-mime-info database (freedesktop "Shared MIME-info Database" spec 0.21),
//! read from the text / binary files `update-mime-database` writes into every
//! `<data dir>/mime/`: `globs2` (weights, `:cs`), `aliases`, `subclasses`, `magic`,
//! `icons` and `generic-icons`. `mime.cache` is not needed.
//!
//! Type detection follows the spec's recommended order: literal file names, then the
//! glob with the highest weight (longest pattern on a tie); when globs are ambiguous or
//! do not match, the file's first bytes are sniffed with the magic rules; then the
//! text / binary fallback. The parsed database is cached and reloaded when any of the
//! source files changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// One `globs2` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Glob {
    pub weight: u32,
    pub mime: String,
    pub pattern: String,
    pub case_sensitive: bool,
}

impl Glob {
    fn is_literal(&self) -> bool {
        !self.pattern.contains(['*', '?', '['])
    }
}

/// One magic match line (`[indent]>offset=value[&mask][~word][+range]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Matchlet {
    pub indent: u32,
    pub offset: u32,
    pub range: u32,
    pub value: Vec<u8>,
    pub mask: Option<Vec<u8>>,
}

/// One `[priority:mime/type]` section of `magic`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MagicRule {
    pub priority: u32,
    pub mime: String,
    pub matchlets: Vec<Matchlet>,
}

impl MagicRule {
    /// Last byte any matchlet can look at.
    fn extent(&self) -> usize {
        // Saturating: offsets and ranges come from untrusted `magic` files.
        self.matchlets
            .iter()
            .map(|m| (m.offset as usize).saturating_add(m.range as usize).saturating_add(m.value.len()))
            .max()
            .unwrap_or(0)
    }

    pub fn matches(&self, data: &[u8]) -> bool {
        let mut i = 0;
        while i < self.matchlets.len() {
            if self.matchlets[i].indent == 0 && subtree_matches(&self.matchlets, i, data) {
                return true;
            }
            i += 1;
        }
        false
    }
}

/// Matchlet `i` matches and (it has no children or one child subtree matches).
fn subtree_matches(list: &[Matchlet], i: usize, data: &[u8]) -> bool {
    if !matchlet_matches(&list[i], data) {
        return false;
    }
    let depth = list[i].indent;
    let mut has_child = false;
    let mut j = i + 1;
    while j < list.len() && list[j].indent > depth {
        if list[j].indent == depth + 1 {
            has_child = true;
            if subtree_matches(list, j, data) {
                return true;
            }
        }
        j += 1;
    }
    !has_child
}

fn matchlet_matches(m: &Matchlet, data: &[u8]) -> bool {
    let len = m.value.len();
    let end = (m.offset as usize).saturating_add(m.range.max(1) as usize);
    for start in m.offset as usize..end {
        let Some(window) = start.checked_add(len).and_then(|e| data.get(start..e)) else { return false };
        let ok = match &m.mask {
            None => window == m.value.as_slice(),
            Some(mask) => window.iter().zip(&m.value).zip(mask).all(|((d, v), k)| d & k == v & k),
        };
        if ok {
            return true;
        }
    }
    false
}

/// Parse `globs2` (`weight:type:pattern[:flags]`). `__NOGLOBS__` entries are returned
/// as globs with that pattern so the loader can drop lower-priority globs.
pub(crate) fn parse_globs2(text: &str) -> Vec<Glob> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.splitn(4, ':');
            let weight = it.next()?.trim().parse().ok()?;
            let mime = it.next()?.to_string();
            let pattern = it.next()?.to_string();
            let flags = it.next().unwrap_or("");
            (!mime.is_empty() && !pattern.is_empty()).then(|| Glob {
                weight,
                mime,
                pattern,
                case_sensitive: flags.split(',').any(|f| f == "cs"),
            })
        })
        .collect()
}

/// `a b` pairs of `aliases` / `subclasses` / `icons` style files.
pub(crate) fn parse_pairs(text: &str, sep: char) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (a, b) = l.split_once(sep)?;
            let (a, b) = (a.trim(), b.trim());
            (!a.is_empty() && !b.is_empty()).then(|| (a.to_string(), b.to_string()))
        })
        .collect()
}

/// Parse the binary `magic` file. Malformed sections are skipped (up to the next line
/// starting with `[`).
pub(crate) fn parse_magic(b: &[u8]) -> Vec<MagicRule> {
    const HEADER: &[u8] = b"MIME-Magic\0\n";
    let mut out = Vec::new();
    let Some(mut rest) = b.strip_prefix(HEADER) else { return out };
    let little = cfg!(target_endian = "little");
    while !rest.is_empty() {
        // Section header.
        let Some(r) = rest.strip_prefix(b"[") else {
            rest = skip_to_section(rest);
            continue;
        };
        let Some(end) = r.iter().position(|&c| c == b'\n') else { break };
        let head = String::from_utf8_lossy(&r[..end]);
        rest = &r[end + 1..];
        let head = head.trim_end_matches(']');
        let Some((prio, mime)) = head.split_once(':') else {
            rest = skip_to_section(rest);
            continue;
        };
        let Ok(priority) = prio.parse::<u32>() else {
            rest = skip_to_section(rest);
            continue;
        };
        let mut rule = MagicRule { priority, mime: mime.to_string(), matchlets: Vec::new() };
        let mut ok = true;
        while !rest.is_empty() && rest[0] != b'[' {
            match parse_matchlet(rest, little) {
                Some((m, n)) => {
                    rule.matchlets.push(m);
                    rest = &rest[n..];
                }
                None => {
                    ok = false;
                    rest = skip_to_section(rest);
                    break;
                }
            }
        }
        if ok && !rule.matchlets.is_empty() {
            out.push(rule);
        }
    }
    out
}

fn skip_to_section(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\n' && b.get(i + 1) == Some(&b'[') {
            return &b[i + 1..];
        }
        i += 1;
    }
    &[]
}

fn read_num(b: &[u8], i: &mut usize) -> Option<u32> {
    let start = *i;
    while *i < b.len() && b[*i].is_ascii_digit() {
        *i += 1;
    }
    std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
}

/// One matchlet line; returns it and the bytes consumed (including the `\n`).
fn parse_matchlet(b: &[u8], little_endian: bool) -> Option<(Matchlet, usize)> {
    let mut i = 0;
    let indent = if b.first()?.is_ascii_digit() { read_num(b, &mut i)? } else { 0 };
    if b.get(i) != Some(&b'>') {
        return None;
    }
    i += 1;
    let offset = read_num(b, &mut i)?;
    if b.get(i) != Some(&b'=') {
        return None;
    }
    i += 1;
    let len = u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize;
    i += 2;
    let mut value = b.get(i..i + len)?.to_vec();
    i += len;
    let mut mask = None;
    let mut word = 1u32;
    let mut range = 1u32;
    loop {
        match b.get(i)? {
            b'&' => {
                mask = Some(b.get(i + 1..i + 1 + len)?.to_vec());
                i += 1 + len;
            }
            b'~' => {
                i += 1;
                word = read_num(b, &mut i)?;
            }
            b'+' => {
                i += 1;
                range = read_num(b, &mut i)?;
            }
            b'\n' => {
                i += 1;
                break;
            }
            _ => return None,
        }
    }
    // Multi-byte words are stored big-endian: swap them on little-endian hosts.
    if little_endian && (word == 2 || word == 4) && len.is_multiple_of(word as usize) {
        for chunk in value.chunks_mut(word as usize) {
            chunk.reverse();
        }
        if let Some(m) = mask.as_mut() {
            for chunk in m.chunks_mut(word as usize) {
                chunk.reverse();
            }
        }
    }
    Some((Matchlet { indent, offset, range: range.max(1), value, mask }, i))
}

/// Shell-style glob match (`*`, `?`, `[...]` with ranges and `!` / `^` negation).
pub(crate) fn fnmatch(pat: &[char], s: &[char]) -> bool {
    let (mut p, mut i) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while i < s.len() {
        if p < pat.len() {
            match pat[p] {
                '*' => {
                    star = Some((p, i));
                    p += 1;
                    continue;
                }
                '?' => {
                    p += 1;
                    i += 1;
                    continue;
                }
                '[' => {
                    if let Some((matched, next)) = class_match(pat, p, s[i]) {
                        if matched {
                            p = next;
                            i += 1;
                            continue;
                        }
                    } else if s[i] == '[' {
                        p += 1;
                        i += 1;
                        continue;
                    }
                }
                c if c == s[i] => {
                    p += 1;
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, si)) => {
                p = sp + 1;
                i = si + 1;
                star = Some((sp, si + 1));
            }
            None => return false,
        }
    }
    pat[p..].iter().all(|&c| c == '*')
}

/// `[...]` at `pat[p]`: `(matched, index after ])`; `None` when unterminated.
fn class_match(pat: &[char], p: usize, c: char) -> Option<(bool, usize)> {
    let mut j = p + 1;
    let neg = matches!(pat.get(j), Some('!' | '^'));
    if neg {
        j += 1;
    }
    let mut matched = false;
    let mut first = true;
    while j < pat.len() && (pat[j] != ']' || first) {
        first = false;
        if pat.get(j + 1) == Some(&'-') && pat.get(j + 2).is_some_and(|&e| e != ']') {
            if pat[j] <= c && c <= pat[j + 2] {
                matched = true;
            }
            j += 3;
        } else {
            if pat[j] == c {
                matched = true;
            }
            j += 1;
        }
    }
    (j < pat.len()).then_some((matched != neg, j + 1))
}

/// The loaded database.
#[derive(Debug, Default)]
pub(crate) struct MimeDb {
    pub globs: Vec<Glob>,
    /// alias → canonical.
    pub aliases: HashMap<String, String>,
    /// type → direct parents (explicit `subclasses` only).
    pub parents: HashMap<String, Vec<String>>,
    /// Sorted by priority, highest first.
    pub magic: Vec<MagicRule>,
    pub icons: HashMap<String, String>,
    pub generic_icons: HashMap<String, String>,
    /// Bytes to read for sniffing (bounded).
    pub extent: usize,
}

/// Upper bound on the bytes read for sniffing.
const MAX_SNIFF: usize = 64 * 1024;

impl MimeDb {
    /// Load from `<dir>/mime/…` for each data dir, highest priority first.
    pub fn load(data_dirs: &[PathBuf]) -> Self {
        let mut db = MimeDb::default();
        let mut no_globs: HashSet<String> = HashSet::new();
        let mut seen_globs: HashSet<(String, String)> = HashSet::new();
        let mut magic_types: HashSet<String> = HashSet::new();
        for dir in data_dirs {
            let m = dir.join("mime");
            let read = |n: &str| std::fs::read_to_string(m.join(n)).ok();
            if let Some(t) = read("globs2") {
                let globs = parse_globs2(&t);
                let mut local_no = HashSet::new();
                for g in globs {
                    if g.pattern == "__NOGLOBS__" {
                        local_no.insert(g.mime);
                        continue;
                    }
                    if no_globs.contains(&g.mime) {
                        continue;
                    }
                    if seen_globs.insert((g.mime.clone(), g.pattern.clone())) {
                        db.globs.push(g);
                    }
                }
                no_globs.extend(local_no);
            }
            if let Some(t) = read("aliases") {
                for (a, c) in parse_pairs(&t, ' ') {
                    db.aliases.entry(a).or_insert(c);
                }
            }
            if let Some(t) = read("subclasses") {
                for (c, p) in parse_pairs(&t, ' ') {
                    let v = db.parents.entry(c).or_default();
                    if !v.contains(&p) {
                        v.push(p);
                    }
                }
            }
            if let Some(t) = read("icons") {
                for (mt, i) in parse_pairs(&t, ':') {
                    db.icons.entry(mt).or_insert(i);
                }
            }
            if let Some(t) = read("generic-icons") {
                for (mt, i) in parse_pairs(&t, ':') {
                    db.generic_icons.entry(mt).or_insert(i);
                }
            }
            if let Ok(b) = std::fs::read(m.join("magic")) {
                // A higher-priority directory's rules for a type replace lower ones.
                let rules = parse_magic(&b);
                let local: HashSet<String> = rules.iter().map(|r| r.mime.clone()).collect();
                db.magic.extend(rules.into_iter().filter(|r| !magic_types.contains(&r.mime)));
                magic_types.extend(local);
            }
        }
        db.magic.sort_by_key(|r| std::cmp::Reverse(r.priority));
        db.extent = db.magic.iter().map(MagicRule::extent).max().unwrap_or(0).min(MAX_SNIFF);
        db
    }

    pub fn unalias<'a>(&'a self, m: &'a str) -> &'a str {
        self.aliases.get(m).map_or(m, String::as_str)
    }

    /// Direct parents including the implicit ones (`text/*` → `text/plain`, streams →
    /// `application/octet-stream`, `+xml` / `+json` suffixes).
    pub fn direct_parents(&self, m: &str) -> Vec<String> {
        let m = self.unalias(m);
        let mut v: Vec<String> = self.parents.get(m).cloned().unwrap_or_default();
        if m.starts_with("text/") && m != "text/plain" {
            v.push("text/plain".into());
        }
        if m.ends_with("+xml") && m != "application/xml" {
            v.push("application/xml".into());
        }
        if m.ends_with("+json") && m != "application/json" {
            v.push("application/json".into());
        }
        if !m.starts_with("inode/") && !m.starts_with("x-scheme-handler/") && m != "application/octet-stream" {
            v.push("application/octet-stream".into());
        }
        let mut out: Vec<String> = Vec::new();
        for p in v {
            let p = self.unalias(&p).to_string();
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// `m` followed by all its ancestors, breadth first (no duplicates). The generic
    /// `application/octet-stream` always comes last.
    pub fn with_ancestors(&self, m: &str) -> Vec<String> {
        let mut out = vec![self.unalias(m).to_string()];
        let mut i = 0;
        while i < out.len() {
            for p in self.direct_parents(&out[i].clone()) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
            i += 1;
        }
        if let Some(pos) = out.iter().position(|m| m == "application/octet-stream")
            && pos + 1 != out.len()
        {
            let o = out.remove(pos);
            out.push(o);
        }
        out
    }

    /// `child` is `parent` or one of its descendants.
    pub fn is_a(&self, child: &str, parent: &str) -> bool {
        let parent = self.unalias(parent);
        self.with_ancestors(child).iter().any(|a| a == parent)
    }

    /// Best glob candidates for a file name (several on a tie of weight and pattern
    /// length; empty when nothing matches). As the spec asks, the name is first matched
    /// case-sensitively against every pattern, and only when nothing matches is it
    /// lowercased and matched against the patterns not flagged `:cs`. Within a pass,
    /// literal names beat every pattern.
    pub fn glob_candidates(&self, name: &str) -> Vec<String> {
        let exact = self.glob_pass(name, false);
        if !exact.is_empty() {
            return exact;
        }
        self.glob_pass(&name.to_lowercase(), true)
    }

    fn glob_pass(&self, name: &str, folded: bool) -> Vec<String> {
        let name_c: Vec<char> = name.chars().collect();
        let mut literal: Option<(u32, Vec<&str>)> = None;
        let mut best: Option<((u32, usize), Vec<&str>)> = None;
        for g in &self.globs {
            if folded && g.case_sensitive {
                continue;
            }
            let lowered;
            let pat = if folded {
                lowered = g.pattern.to_lowercase();
                lowered.as_str()
            } else {
                g.pattern.as_str()
            };
            let hit = if g.is_literal() {
                pat == name
            } else if let Some(suffix) = pat.strip_prefix('*').filter(|s| !s.contains(['*', '?', '['])) {
                name.ends_with(suffix)
            } else {
                fnmatch(&pat.chars().collect::<Vec<_>>(), &name_c)
            };
            if !hit {
                continue;
            }
            let mime = self.unalias(&g.mime);
            if g.is_literal() {
                match &mut literal {
                    Some((w, v)) if *w == g.weight => push_unique(v, mime),
                    Some((w, _)) if *w > g.weight => {}
                    _ => literal = Some((g.weight, vec![mime])),
                }
                continue;
            }
            let key = (g.weight, g.pattern.chars().count());
            match &mut best {
                Some((k, v)) if *k == key => push_unique(v, mime),
                Some((k, _)) if *k > key => {}
                _ => best = Some((key, vec![mime])),
            }
        }
        literal.map(|l| l.1).or(best.map(|b| b.1)).unwrap_or_default().into_iter().map(str::to_string).collect()
    }

    /// Magic matches for `data`, highest priority first.
    pub fn sniff(&self, data: &[u8]) -> Vec<(u32, String)> {
        self.magic.iter().filter(|r| r.matches(data)).map(|r| (r.priority, self.unalias(&r.mime).to_string())).collect()
    }

    /// The type of a file called `name` whose first bytes `read` returns (called only
    /// when the name is not conclusive).
    pub fn type_for(&self, name: Option<&str>, read: impl FnOnce(usize) -> Option<Vec<u8>>) -> String {
        let globs = name.map(|n| self.glob_candidates(n)).unwrap_or_default();
        if globs.len() == 1 {
            return globs.into_iter().next().unwrap_or_default();
        }
        let data = read(self.extent.max(512)).unwrap_or_default();
        let sniffed = self.sniff(&data);
        if !globs.is_empty() {
            // Ambiguous globs: the candidate that the content agrees with (equal to or a
            // subclass of a sniffed type), else the first one.
            for (_, m) in &sniffed {
                if let Some(g) = globs.iter().find(|g| self.is_a(g, m)) {
                    return g.clone();
                }
            }
            return globs.into_iter().next().unwrap_or_default();
        }
        if let Some((_, m)) = sniffed.into_iter().next() {
            return m;
        }
        if looks_like_text(&data) { "text/plain".into() } else { "application/octet-stream".into() }
    }

    /// Icon names for a type, most specific first: the `icons` entry, the type with `/`
    /// replaced by `-`, the `generic-icons` entry, then `<media>-x-generic`.
    pub fn icon_names(&self, m: &str) -> Vec<String> {
        let m = self.unalias(m);
        let mut v = Vec::new();
        if let Some(i) = self.icons.get(m) {
            v.push(i.clone());
        }
        v.push(m.replace('/', "-"));
        if let Some(i) = self.generic_icons.get(m) {
            v.push(i.clone());
        }
        if let Some((media, _)) = m.split_once('/') {
            v.push(format!("{media}-x-generic"));
        }
        let mut out: Vec<String> = Vec::new();
        for i in v {
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }
}

fn push_unique<'a>(v: &mut Vec<&'a str>, m: &'a str) {
    if !v.contains(&m) {
        v.push(m);
    }
}

/// No NUL / control bytes in the sample (text files: empty, or printable + whitespace;
/// a UTF-8 sequence cut at the end of the sample is fine).
pub(crate) fn looks_like_text(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(4096)];
    !sample.iter().any(|&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b))
}

/// Source files whose modification times identify a loaded database.
fn stamp(dirs: &[PathBuf]) -> Vec<Option<SystemTime>> {
    let mut v = Vec::new();
    for d in dirs {
        for n in ["globs2", "magic", "aliases", "subclasses", "icons", "generic-icons"] {
            v.push(std::fs::metadata(d.join("mime").join(n)).and_then(|m| m.modified()).ok());
        }
    }
    v
}

struct Cached {
    dirs: Vec<PathBuf>,
    stamp: Vec<Option<SystemTime>>,
    checked: Instant,
    db: Arc<MimeDb>,
}

static CACHE: parking_lot::Mutex<Option<Cached>> = parking_lot::Mutex::new(None);

/// How often the source files are re-checked.
const RECHECK: Duration = Duration::from_secs(5);

/// The database for the current XDG data dirs (cached; reloaded when a file changes).
pub(crate) fn db() -> Arc<MimeDb> {
    let dirs = super::data_dirs();
    let mut g = CACHE.lock();
    if let Some(c) = g.as_mut()
        && c.dirs == dirs
    {
        if c.checked.elapsed() < RECHECK {
            return c.db.clone();
        }
        let s = stamp(&dirs);
        c.checked = Instant::now();
        if s == c.stamp {
            return c.db.clone();
        }
    }
    let s = stamp(&dirs);
    let db = Arc::new(MimeDb::load(&dirs));
    *g = Some(Cached { dirs, stamp: s, checked: Instant::now(), db: db.clone() });
    db
}

/// The MIME type of `path` (follows symlinks; directories are `inode/directory`, other
/// special files `inode/*`). `None` when the file cannot be stat'ed.
pub(crate) fn mime_of(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let ft = meta.file_type();
    if ft.is_dir() {
        return Some("inode/directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let special = if ft.is_block_device() {
            Some("inode/blockdevice")
        } else if ft.is_char_device() {
            Some("inode/chardevice")
        } else if ft.is_fifo() {
            Some("inode/fifo")
        } else if ft.is_socket() {
            Some("inode/socket")
        } else {
            None
        };
        if let Some(s) = special {
            return Some(s.into());
        }
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let size = meta.len();
    let db = db();
    Some(db.type_for(name.as_deref(), |n| {
        if size == 0 {
            return Some(Vec::new());
        }
        use std::io::Read;
        let mut buf = Vec::with_capacity(n.min(size as usize));
        std::fs::File::open(path).ok()?.take(n as u64).read_to_end(&mut buf).ok()?;
        Some(buf)
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn magic_file(sections: &[(u32, &str, Vec<Vec<u8>>)]) -> Vec<u8> {
        let mut f = b"MIME-Magic\0\n".to_vec();
        for (prio, mime, lines) in sections {
            f.extend_from_slice(format!("[{prio}:{mime}]\n").as_bytes());
            for l in lines {
                f.extend_from_slice(l);
            }
        }
        f
    }

    /// `[indent]>offset=value[&mask][+range]\n`.
    pub(crate) fn ml(indent: u32, offset: u32, value: &[u8], mask: Option<&[u8]>, range: Option<u32>) -> Vec<u8> {
        let mut l = Vec::new();
        if indent > 0 {
            l.extend_from_slice(indent.to_string().as_bytes());
        }
        l.extend_from_slice(format!(">{offset}=").as_bytes());
        l.extend_from_slice(&(value.len() as u16).to_be_bytes());
        l.extend_from_slice(value);
        if let Some(m) = mask {
            l.push(b'&');
            l.extend_from_slice(m);
        }
        if let Some(r) = range {
            l.extend_from_slice(format!("+{r}").as_bytes());
        }
        l.push(b'\n');
        l
    }

    pub(crate) fn sample_db() -> MimeDb {
        let d = tempfile::tempdir().unwrap();
        let m = d.path().join("mime");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(
            m.join("globs2"),
            "# comment\n\
             50:text/plain:*.txt\n\
             50:text/x-csrc:*.c\n\
             50:text/x-c++src:*.C:cs\n\
             55:application/x-compressed-tar:*.tar.gz\n\
             50:application/gzip:*.gz\n\
             50:text/x-makefile:makefile\n\
             50:text/x-makefile:Makefile\n\
             50:application/x-core:core:cs\n\
             50:image/x-old:*.img\n\
             50:application/x-raw-disk-image:*.img\n\
             10:text/x-readme:README*\n\
             40:text/x-log:*.log.[0-9]\n\
             50:application/x-old-alias:*.oldx\n",
        )
        .unwrap();
        std::fs::write(m.join("aliases"), "application/x-old-alias application/x-new\n").unwrap();
        std::fs::write(m.join("subclasses"), "text/x-csrc text/plain\napplication/x-compressed-tar application/gzip\n")
            .unwrap();
        std::fs::write(m.join("icons"), "application/gzip:package-x-generic\n").unwrap();
        std::fs::write(m.join("generic-icons"), "text/x-csrc:text-x-script\n").unwrap();
        std::fs::write(
            m.join("magic"),
            magic_file(&[
                (80, "image/png", vec![ml(0, 0, b"\x89PNG", None, None)]),
                (50, "application/x-raw-disk-image", vec![ml(0, 510, b"\x55\xAA", None, None)]),
                // A parent with two children: only the second child matches.
                (
                    60,
                    "application/x-tree",
                    vec![
                        ml(0, 0, b"TREE", None, None),
                        ml(1, 8, b"no", None, None),
                        ml(1, 8, b"ok", None, None),
                        ml(2, 10, b"!", None, None),
                    ],
                ),
                (40, "application/x-masked", vec![ml(0, 0, b"\x10\x00", Some(b"\xF0\x00"), Some(4))]),
            ]),
        )
        .unwrap();
        MimeDb::load(&[d.path().to_path_buf()])
    }

    #[test]
    fn huge_offsets_and_ranges_do_not_overflow() {
        let m = Matchlet { indent: 0, offset: u32::MAX - 1, range: u32::MAX, value: b"ab".to_vec(), mask: None };
        assert!(!matchlet_matches(&m, b"abc"));
        let rule = MagicRule { priority: 50, mime: "x/y".into(), matchlets: vec![m] };
        assert!(rule.extent() >= u32::MAX as usize);
        assert!(!rule.matches(b"ab"));
        let parsed = parse_magic(&magic_file(&[(50, "x/y", vec![ml(0, u32::MAX, b"ab", None, Some(u32::MAX))])]));
        assert!(!parsed.is_empty() && !parsed[0].matches(b"ab"));
    }

    #[test]
    fn globs_weights_case_and_literals() {
        let db = sample_db();
        let one = |n: &str| db.glob_candidates(n);
        assert_eq!(one("NOTES.TXT"), ["text/plain"]);
        assert_eq!(one("a.tar.gz"), ["application/x-compressed-tar"], "heavier glob wins");
        assert_eq!(one("a.gz"), ["application/gzip"]);
        assert_eq!(one("Makefile"), ["text/x-makefile"]);
        assert_eq!(one("x.C"), ["text/x-c++src"], "case-sensitive glob");
        assert_eq!(one("x.c"), ["text/x-csrc"]);
        assert_eq!(one("core"), ["application/x-core"]);
        assert!(one("CORE").is_empty(), ":cs literal");
        assert_eq!(one("README.md"), ["text/x-readme"], "complex glob");
        assert_eq!(one("app.log.3"), ["text/x-log"]);
        assert!(one("app.log.x").is_empty());
        assert_eq!(one("x.oldx"), ["application/x-new"], "aliases resolve");
        let amb = one("disk.img");
        assert_eq!(amb.len(), 2, "{amb:?}");
    }

    #[test]
    fn magic_parsing_and_sniffing() {
        let db = sample_db();
        assert_eq!(db.magic.len(), 4);
        assert_eq!(db.magic[0].mime, "image/png", "sorted by priority");
        let mut disk = vec![0u8; 512];
        disk[510] = 0x55;
        disk[511] = 0xAA;
        // Ambiguous glob resolved by content.
        assert_eq!(db.type_for(Some("disk.img"), |_| Some(disk.clone())), "application/x-raw-disk-image");
        assert_eq!(db.type_for(Some("disk.img"), |_| Some(vec![1, 2, 3])), "image/x-old");
        // No glob: content decides.
        assert_eq!(db.type_for(Some("noext"), |_| Some(b"\x89PNG\r\n".to_vec())), "image/png");
        assert_eq!(db.type_for(Some("noext"), |_| Some(b"TREE\0\0\0\0ok!".to_vec())), "application/x-tree");
        assert_eq!(db.type_for(Some("noext"), |_| Some(b"TREE\0\0\0\0ok?".to_vec())), "application/octet-stream");
        assert_eq!(db.type_for(Some("noext"), |_| Some(b"TREE\0\0\0\0no!".to_vec())), "application/x-tree");
        assert_eq!(db.type_for(Some("noext"), |_| Some(b"TREE\0\0\0\0xx!".to_vec())), "application/octet-stream");
        // Mask + range: any of offsets 0..4 with high nibble 1 and a zero byte after.
        assert_eq!(db.type_for(None, |_| Some(b"\0\0\x1f\0".to_vec())), "application/x-masked");
        assert_eq!(db.type_for(None, |_| Some(b"plain words\n".to_vec())), "text/plain");
        assert_eq!(db.type_for(None, |_| Some(vec![0, 1, 2, 0xff])), "application/octet-stream");
        assert_eq!(db.type_for(Some("x.txt"), |_| panic!("an unambiguous glob needs no read")), "text/plain");
    }

    #[test]
    fn malformed_magic_is_skipped() {
        let mut f = magic_file(&[(50, "a/b", vec![b">x=garbage\n".to_vec()])]);
        f.extend_from_slice(&magic_file(&[(50, "c/d", vec![ml(0, 0, b"CD", None, None)])])[12..]);
        let rules = parse_magic(&f);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].mime, "c/d");
        assert!(parse_magic(b"not magic").is_empty());
        // Truncated value.
        assert!(parse_magic(b"MIME-Magic\0\n[50:a/b]\n>0=\x00\x09ab").is_empty());
    }

    #[test]
    fn word_size_swaps_on_little_endian() {
        let line = b">0=\x00\x02\x12\x34~2\n";
        let (m, n) = parse_matchlet(line, true).unwrap();
        assert_eq!(n, line.len());
        assert_eq!(m.value, [0x34, 0x12]);
        let (m, _) = parse_matchlet(line, false).unwrap();
        assert_eq!(m.value, [0x12, 0x34]);
    }

    #[test]
    fn subclass_walk_and_icons() {
        let db = sample_db();
        assert_eq!(db.with_ancestors("text/x-csrc"), ["text/x-csrc", "text/plain", "application/octet-stream"]);
        assert_eq!(
            db.with_ancestors("application/x-compressed-tar"),
            ["application/x-compressed-tar", "application/gzip", "application/octet-stream"]
        );
        assert_eq!(db.with_ancestors("inode/directory"), ["inode/directory"]);
        assert!(db.is_a("image/svg+xml", "application/xml"));
        assert!(db.is_a("application/x-old-alias", "application/x-new"));
        assert_eq!(
            db.icon_names("application/gzip"),
            ["package-x-generic", "application-gzip", "application-x-generic"]
        );
        assert_eq!(db.icon_names("text/x-csrc"), ["text-x-csrc", "text-x-script", "text-x-generic"]);
    }

    #[test]
    fn fnmatch_classes() {
        let m = |p: &str, s: &str| fnmatch(&p.chars().collect::<Vec<_>>(), &s.chars().collect::<Vec<_>>());
        assert!(m("*.[ch]", "a.c"));
        assert!(!m("*.[!ch]", "a.c"));
        assert!(m("*.[a-z]x", "f.qx"));
        assert!(m("a*b*c", "aXXbYYc"));
        assert!(!m("a*b*c", "aXXbYY"));
        assert!(m("[]x]", "]"));
        assert!(m("a?c", "abc"));
        assert!(m("*", ""));
    }

    /// The real database of this machine: common types resolve.
    #[cfg(target_os = "linux")]
    #[test]
    fn system_database_if_present() {
        let db = db();
        if db.globs.is_empty() {
            return; // no shared-mime-info installed
        }
        assert_eq!(db.glob_candidates("a.PNG"), ["image/png"]);
        assert_eq!(db.type_for(Some("noext"), |_| Some(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec())), "image/png");
        let d = tempfile::tempdir().unwrap();
        assert_eq!(mime_of(d.path()).as_deref(), Some("inode/directory"));
        let f = d.path().join("notes");
        std::fs::write(&f, "hello\n").unwrap();
        assert_eq!(mime_of(&f).as_deref(), Some("text/plain"));
    }
}
