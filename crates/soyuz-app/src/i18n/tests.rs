//! Catalog integrity: both locales parse, have the same keys, every key used in code
//! exists (with the arguments it needs), and no key is unused.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;

/// `key -> message source` (value plus indented continuation lines) of one `.ftl` file.
fn messages(src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut cur: Option<(String, String)> = None;
    for line in src.lines() {
        let starts_entry = line.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        if starts_entry {
            if let Some((k, v)) = cur.take() {
                out.insert(k, v);
            }
            if let Some((k, v)) = line.split_once('=') {
                cur = Some((k.trim().to_string(), v.to_string()));
            }
        } else if line.starts_with(' ') && !line.trim().is_empty() {
            if let Some((_, v)) = cur.as_mut() {
                v.push('\n');
                v.push_str(line);
            }
        } else if let Some((k, v)) = cur.take() {
            out.insert(k, v);
        }
    }
    if let Some((k, v)) = cur {
        out.insert(k, v);
    }
    out
}

/// Every `$variable` a message references.
fn variables(msg: &str) -> BTreeSet<String> {
    let re = regex::Regex::new(r"\$([A-Za-z_][A-Za-z0-9_-]*)").unwrap();
    re.captures_iter(msg).map(|c| c[1].to_string()).collect()
}

fn all_messages(l: Locale) -> BTreeMap<String, String> {
    let mut all = BTreeMap::new();
    for (area, en, tr) in FILES {
        let src = if l == Locale::EnUs { en } else { tr };
        for (k, v) in messages(src) {
            assert!(all.insert(k.clone(), v).is_none(), "{}: duplicate key {k} ({area}.ftl)", l.tag());
        }
    }
    all
}

#[test]
fn files_parse_cleanly() {
    for (area, en, tr) in FILES {
        for (tag, src) in [("en-US", en), ("tr-TR", tr)] {
            if let Err((_, errs)) = FluentResource::try_new(src.to_string()) {
                panic!("{tag}/{area}.ftl: {errs:?}");
            }
        }
    }
}

#[test]
fn locales_have_the_same_keys_per_file() {
    for (area, en, tr) in FILES {
        let e: BTreeSet<_> = messages(en).into_keys().collect();
        let t: BTreeSet<_> = messages(tr).into_keys().collect();
        let only_en: Vec<_> = e.difference(&t).collect();
        let only_tr: Vec<_> = t.difference(&e).collect();
        assert!(
            only_en.is_empty() && only_tr.is_empty(),
            "{area}.ftl: missing in tr-TR {only_en:?}, missing in en-US {only_tr:?}"
        );
    }
    // No duplicates across files either.
    let _ = all_messages(Locale::EnUs);
    let _ = all_messages(Locale::TrTr);
}

#[test]
fn translations_use_the_same_variables() {
    let en = all_messages(Locale::EnUs);
    let tr = all_messages(Locale::TrTr);
    for (k, v) in &en {
        let Some(t) = tr.get(k) else { continue };
        assert_eq!(variables(v), variables(t), "{k}: en-US and tr-TR reference different $variables");
    }
}

/// One `tr!("key", a = …, b = …)` call site.
struct Call {
    file: String,
    key: String,
    args: BTreeSet<String>,
}

/// All `.rs` sources of this crate with comment lines blanked out.
fn sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let src = std::fs::read_to_string(&p).unwrap();
                out.push((p.display().to_string(), src));
            }
        }
    }
    let mut out = vec![];
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

fn strip_comments(src: &str) -> String {
    src.lines().map(|l| if l.trim_start().starts_with("//") { "" } else { l }).collect::<Vec<_>>().join("\n")
}

/// Parse every `tr!(…)` invocation (string-literal key, `name = expr` arguments).
fn calls() -> Vec<Call> {
    let mut out = vec![];
    for (file, src) in sources() {
        if Path::new(&file).ends_with("i18n/tests.rs") {
            continue;
        }
        let src = strip_comments(&src);
        let b = src.as_bytes();
        let mut from = 0;
        while let Some(pos) = src[from..].find("tr!(") {
            let start = from + pos + 4;
            from = start;
            // `tr!` must not be the tail of another identifier (`xtr!`).
            if from > 4 && (b[from - 5].is_ascii_alphanumeric() || b[from - 5] == b'_') {
                continue;
            }
            let rest = src[start..].trim_start();
            let Some(lit) = rest.strip_prefix('"') else { continue };
            let Some(end) = lit.find('"') else { continue };
            let key = lit[..end].to_string();
            // Find the matching ')' (strings and nested parens aware).
            let body_start = start + (src[start..].len() - rest.len()) + 1 + end + 1;
            let mut depth = 1;
            let mut i = body_start;
            let mut in_str = false;
            while i < b.len() && depth > 0 {
                let c = b[i];
                if in_str {
                    if c == b'\\' {
                        i += 1;
                    } else if c == b'"' {
                        in_str = false;
                    }
                } else {
                    match c {
                        b'"' => in_str = true,
                        b'(' | b'[' | b'{' => depth += 1,
                        b')' | b']' | b'}' => depth -= 1,
                        _ => {}
                    }
                }
                i += 1;
            }
            let body = &src[body_start..i.saturating_sub(1)];
            // Top-level comma split.
            let mut args = BTreeSet::new();
            let (mut d, mut s, mut q) = (0i32, 0usize, false);
            let bb = body.as_bytes();
            let mut parts = vec![];
            for (j, &c) in bb.iter().enumerate() {
                if q {
                    if c == b'"' && (j == 0 || bb[j - 1] != b'\\') {
                        q = false;
                    }
                    continue;
                }
                match c {
                    b'"' => q = true,
                    b'(' | b'[' | b'{' => d += 1,
                    b')' | b']' | b'}' => d -= 1,
                    b',' if d == 0 => {
                        parts.push(&body[s..j]);
                        s = j + 1;
                    }
                    _ => {}
                }
            }
            parts.push(&body[s..]);
            for p in parts {
                if let Some((name, _)) = p.split_once('=') {
                    let name = name.trim();
                    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        args.insert(name.to_string());
                    }
                }
            }
            out.push(Call { file: file.clone(), key, args });
        }
    }
    out
}

#[test]
fn every_key_used_in_code_exists_with_its_arguments() {
    let en = all_messages(Locale::EnUs);
    let tr = all_messages(Locale::TrTr);
    let mut problems = vec![];
    for c in calls() {
        for (tag, msgs) in [("en-US", &en), ("tr-TR", &tr)] {
            match msgs.get(&c.key) {
                None => problems.push(format!("{}: {} missing in {tag}", c.file, c.key)),
                Some(m) => {
                    let missing: Vec<_> = variables(m).into_iter().filter(|v| !c.args.contains(v)).collect();
                    // `-term` references are not variables; `$x` names must all be passed.
                    if !missing.is_empty() {
                        problems.push(format!("{}: {} ({tag}) needs {missing:?}, passed {:?}", c.file, c.key, c.args));
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn no_unused_keys() {
    let srcs = sources();
    let all_src: String = srcs.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join("\n");
    let re = regex::Regex::new(r"i18n-dynamic:\s*([a-z0-9-]+)").unwrap();
    let prefixes: Vec<String> = re.captures_iter(&all_src).map(|c| c[1].to_string()).collect();
    let unused: Vec<String> = all_messages(Locale::EnUs)
        .into_keys()
        .filter(|k| !all_src.contains(&format!("\"{k}\"")) && !prefixes.iter().any(|p| k.starts_with(p.as_str())))
        .collect();
    assert!(unused.is_empty(), "{} unused keys: {unused:?}", unused.len());
}

#[test]
fn plurals_and_selectors() {
    let items = |n: u64, l| {
        let mut a = FluentArgs::new();
        a.set("count", n);
        t_args_in("common-items", &a, l)
    };
    assert_eq!(items(1, Locale::EnUs), "1 item");
    assert_eq!(items(3, Locale::EnUs), "3 items");
    assert_eq!(items(1234, Locale::EnUs), "1,234 items");
    assert_eq!(items(1, Locale::TrTr), "1 öğe");
    assert_eq!(items(3, Locale::TrTr), "3 öğe");
    assert_eq!(items(1234, Locale::TrTr), "1.234 öğe");
    let kind = |k: &str, l| {
        let mut a = FluentArgs::new();
        a.set("kind", k.to_string());
        t_args_in("common-kind", &a, l)
    };
    assert_eq!(kind("folder", Locale::TrTr), "Klasör");
    assert_eq!(kind("link", Locale::TrTr), "Bağlantı");
    assert_eq!(kind("whatever", Locale::TrTr), "Öğe");
    assert_eq!(kind("file", Locale::EnUs), "File");
}

#[test]
fn numbers_in_messages() {
    let n = |v: f64, l| localize_number(&FluentNumber::from(v), l);
    assert_eq!(n(1234.5, Locale::TrTr), "1.234,5");
    assert_eq!(n(1234.5, Locale::EnUs), "1,234.5");
    assert_eq!(n(-12.0, Locale::TrTr), "-12");
    assert_eq!(n(1_000_000.0, Locale::TrTr), "1.000.000");
}

#[test]
fn fallback_and_missing() {
    assert_eq!(t_in("common-cancel", Locale::TrTr), "İptal");
    assert_eq!(t_in("common-cancel", Locale::EnUs), "Cancel");
    assert_eq!(t_in("no-such-key-for-tests", Locale::TrTr), "no-such-key-for-tests");
    assert!(has("common-ok"));
    assert!(!has("no-such-key-for-tests"));
}

#[test]
fn negotiation() {
    assert_eq!(negotiate("tr-TR"), Some(Locale::TrTr));
    assert_eq!(negotiate("tr"), Some(Locale::TrTr));
    assert_eq!(negotiate("tr_TR"), Some(Locale::TrTr));
    assert_eq!(negotiate("en-GB"), Some(Locale::EnUs));
    assert_eq!(negotiate("de-DE"), None);
    assert_eq!(resolve("tr-TR"), Locale::TrTr);
    assert_eq!(resolve("en"), Locale::EnUs);
    assert_eq!(resolve("xx"), Locale::EnUs);
}
