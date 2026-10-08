//! URL detection in terminal text for clickable links (Ctrl+click).

/// A detected link: `[start, end)` in *char* indices of the scanned text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UrlMatch {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

const SCHEMES: &[&str] = &["https://", "http://", "file://", "ftp://", "ssh://", "git://", "mailto:", "www."];

fn is_url_char(c: char) -> bool {
    !(c.is_whitespace()
        || c.is_control()
        || matches!(c, '<' | '>' | '"' | '`' | '{' | '}' | '|' | '\\' | '^')
        || ('\u{2500}'..='\u{259F}').contains(&c)) // box drawing / block elements
}

fn starts_with_ci(chars: &[char], at: usize, pat: &str) -> bool {
    let mut i = at;
    for p in pat.chars() {
        match chars.get(i) {
            Some(c) if c.eq_ignore_ascii_case(&p) => i += 1,
            _ => return false,
        }
    }
    true
}

/// Find URLs in `text`.
pub(crate) fn find_urls(text: &str) -> Vec<UrlMatch> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let boundary = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '.');
        let scheme = if boundary { SCHEMES.iter().find(|s| starts_with_ci(&chars, i, s)) } else { None };
        let Some(scheme) = scheme else {
            i += 1;
            continue;
        };
        let mut end = i;
        while end < chars.len() && is_url_char(chars[end]) {
            end += 1;
        }
        // Trim trailing punctuation and unbalanced closing brackets.
        while let Some(&last) = chars[i..end].last() {
            let count = |c: char| chars[i..end].iter().filter(|&&x| x == c).count();
            let unbalanced = match last {
                ')' => count(')') > count('('),
                ']' => count(']') > count('['),
                _ => false,
            };
            if matches!(last, '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '*') || unbalanced {
                end -= 1;
            } else {
                break;
            }
        }
        let scheme_len = scheme.chars().count();
        if end > i + scheme_len {
            let raw: String = chars[i..end].iter().collect();
            let url = if scheme.eq_ignore_ascii_case("www.") { format!("https://{raw}") } else { raw };
            out.push(UrlMatch { start: i, end, url });
            i = end;
        } else {
            i += scheme_len;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(s: &str) -> Vec<String> {
        find_urls(s).into_iter().map(|m| m.url).collect()
    }

    #[test]
    fn detects_common_urls() {
        assert_eq!(urls("see https://example.com/a?b=c#d for more"), ["https://example.com/a?b=c#d"]);
        assert_eq!(urls("(http://x.org/wiki/Foo_(bar))."), ["http://x.org/wiki/Foo_(bar)"]);
        assert_eq!(urls("go to www.rust-lang.org, now"), ["https://www.rust-lang.org"]);
        assert_eq!(urls("file:///C:/tmp/a.txt and mailto:me@x.io"), ["file:///C:/tmp/a.txt", "mailto:me@x.io"]);
        assert_eq!(urls("<https://a.b/c>"), ["https://a.b/c"]);
        assert_eq!(urls("HTTPS://UPPER.CASE"), ["HTTPS://UPPER.CASE"]);
        assert!(urls("no links here, https:// alone").is_empty());
        assert!(urls("xhttps://glued.example").is_empty());
    }

    #[test]
    fn char_offsets() {
        let m = find_urls("é https://a.io");
        assert_eq!((m[0].start, m[0].end), (2, 14));
    }
}
