//! Incremental Server-Sent Events parser (WHATWG event-stream format): feed arbitrary
//! byte chunks, get complete events. Handles `\n`, `\r\n` and `\r` line ends (also split
//! across chunks), comments, multi-line `data:` fields and a trailing unterminated event
//! on [`SseParser::finish`].

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// `event:` field (`None` = the default "message" type).
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    has_data: bool,
    /// The previous byte was `\r` (so a following `\n` is the second half of `\r\n`).
    prev_cr: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk; returns the events completed by it.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        for &b in chunk {
            match b {
                b'\n' if self.prev_cr => self.prev_cr = false,
                b'\n' | b'\r' => {
                    self.prev_cr = b == b'\r';
                    self.line(&mut out);
                }
                _ => {
                    self.prev_cr = false;
                    self.buf.push(b);
                }
            }
        }
        out
    }

    /// End of stream: dispatch a trailing event that lacks the final blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let mut out = Vec::new();
            self.line(&mut out);
        }
        self.dispatch()
    }

    fn line(&mut self, out: &mut Vec<SseEvent>) {
        let raw = std::mem::take(&mut self.buf);
        if raw.is_empty() {
            if let Some(e) = self.dispatch() {
                out.push(e);
            }
            return;
        }
        let line = String::from_utf8_lossy(&raw);
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.find(':') {
            Some(i) => {
                let v = &line[i + 1..];
                (&line[..i], v.strip_prefix(' ').unwrap_or(v))
            }
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data.push(value.to_string());
                self.has_data = true;
            }
            _ => {} // id / retry / unknown fields are ignored.
        }
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if !self.has_data {
            self.data.clear();
            return None;
        }
        self.has_data = false;
        let data = std::mem::take(&mut self.data).join("\n");
        Some(SseEvent { event, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::new();
        let mut v: Vec<SseEvent> = chunks.iter().flat_map(|c| p.feed(c)).collect();
        v.extend(p.finish());
        v
    }

    #[test]
    fn basic_and_split_chunks() {
        let text = b"event: ping\ndata: {\"a\":1}\n\n: comment\ndata: x\ndata: y\n\n";
        let whole = all(&[text]);
        assert_eq!(whole.len(), 2);
        assert_eq!(whole[0], SseEvent { event: Some("ping".into()), data: "{\"a\":1}".into() });
        assert_eq!(whole[1].data, "x\ny");
        // Byte-by-byte gives the same result.
        let bytes: Vec<&[u8]> = text.chunks(1).collect();
        assert_eq!(all(&bytes), whole);
    }

    #[test]
    fn crlf_split_across_chunks_and_trailing_event() {
        let v = all(&[b"data: a\r", b"\n\r", b"\ndata: b"]);
        assert_eq!(v, vec![SseEvent { event: None, data: "a".into() }, SseEvent { event: None, data: "b".into() }]);
        let cr = all(&[b"data: c\r\rdata: d\r\r"]);
        assert_eq!(cr.len(), 2);
    }
}
