//! A tiny pull-parser facade over `quick-xml` for the document previewers (Office, ODF,
//! EPUB): element names come back as *local* names (namespace prefixes stripped), text has
//! entity / character references resolved, and malformed input simply ends the stream
//! instead of erroring, so a broken part yields a partial preview rather than none.

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

/// One simplified XML event.
#[derive(Clone, Debug, PartialEq)]
pub enum X {
    /// Start tag; `empty` = self-closing (`<a/>`, no matching `End` follows).
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        empty: bool,
    },
    End {
        name: String,
    },
    Text(String),
}

impl X {
    /// Attribute value by local name.
    pub fn attr(&self, key: &str) -> Option<&str> {
        match self {
            X::Start { attrs, .. } => attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    pub fn is_start(&self, n: &str) -> bool {
        matches!(self, X::Start { name, .. } if name == n)
    }

    pub fn is_end(&self, n: &str) -> bool {
        matches!(self, X::End { name } if name == n) || matches!(self, X::Start { name, empty: true, .. } if name == n)
    }
}

fn local(raw: &str) -> String {
    match raw.rsplit_once(':') {
        Some((_, l)) => l.to_string(),
        None => raw.to_string(),
    }
}

fn start(e: &BytesStart<'_>, empty: bool) -> X {
    let name = local(e.name().as_ref());
    let mut attrs = Vec::new();
    for a in e.attributes().with_checks(false).flatten() {
        let key = local(a.key.as_ref());
        let val =
            a.normalized_value(XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_else(|_| a.value.to_string());
        attrs.push((key, val));
    }
    X::Start { name, attrs, empty }
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "copy" => "©",
        "reg" => "®",
        "rsquo" => "’",
        "lsquo" => "‘",
        "rdquo" => "”",
        "ldquo" => "“",
        _ => return None,
    })
}

/// Parse `xml` into simplified events (at most `max_events`, to bound hostile input).
/// Adjacent text and references are merged into one `Text`.
pub fn events(xml: &str, max_events: usize) -> Vec<X> {
    let mut r = quick_xml::Reader::from_str(xml);
    {
        let c = r.config_mut();
        c.check_end_names = false;
        c.trim_text(false);
    }
    let mut out: Vec<X> = Vec::new();
    let push_text = |out: &mut Vec<X>, t: &str| {
        if t.is_empty() {
            return;
        }
        if let Some(X::Text(prev)) = out.last_mut() {
            prev.push_str(t);
        } else {
            out.push(X::Text(t.to_string()));
        }
    };
    while out.len() < max_events {
        match r.read_event() {
            Ok(Event::Start(e)) => out.push(start(&e, false)),
            Ok(Event::Empty(e)) => out.push(start(&e, true)),
            Ok(Event::End(e)) => out.push(X::End { name: local(e.name().as_ref()) }),
            Ok(Event::Text(t)) => push_text(&mut out, &t.xml10_content()),
            Ok(Event::CData(t)) => push_text(&mut out, &t.xml10_content()),
            Ok(Event::GeneralRef(g)) => {
                if let Ok(Some(c)) = g.resolve_char_ref() {
                    push_text(&mut out, c.encode_utf8(&mut [0u8; 4]));
                } else if let Some(s) = named_entity(&g) {
                    push_text(&mut out, s);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

/// Bytes → text (UTF-8 with BOM handling, UTF-16 by BOM).
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |le: bool, b: &[u8]| {
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(true, rest);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(false, rest);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Concatenated text of everything between `events[from]` (a start tag) and its end.
/// Returns `(text, index after the end tag)`.
pub fn text_until_end(events: &[X], from: usize) -> (String, usize) {
    let Some(X::Start { name, empty, .. }) = events.get(from) else { return (String::new(), from + 1) };
    if *empty {
        return (String::new(), from + 1);
    }
    let mut depth = 0usize;
    let mut s = String::new();
    let mut i = from + 1;
    while i < events.len() {
        match &events[i] {
            X::Start { name: n, empty: false, .. } if n == name => depth += 1,
            X::End { name: n } if n == name => {
                if depth == 0 {
                    return (s, i + 1);
                }
                depth -= 1;
            }
            X::Text(t) => s.push_str(t),
            _ => {}
        }
        i += 1;
    }
    (s, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names_entities_and_attrs() {
        let ev = events(r#"<?xml version="1.0"?><w:p a:x="1 &amp; 2"><w:t>A &amp; B&#x41;</w:t><w:br/></w:p>"#, 100);
        assert_eq!(ev[0].attr("x"), Some("1 & 2"));
        assert!(ev[0].is_start("p"));
        assert!(ev[1].is_start("t"));
        assert_eq!(ev[2], X::Text("A & BA".into()));
        assert!(ev[4].is_end("br"));
        assert_eq!(text_until_end(&ev, 0).0, "A & BA");
    }

    #[test]
    fn broken_xml_is_partial_not_fatal() {
        let ev = events("<a><b>text</c><<<", 100);
        assert!(ev.iter().any(|e| *e == X::Text("text".into())));
        assert_eq!(decode(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
    }
}
