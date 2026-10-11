//! OSC 7 (`ESC ] 7 ; file://host/path BEL` or `… ESC \`): the working directory that
//! shells report on their own (fish, VTE's `vte.sh`, oh-my-zsh, WezTerm integration), so
//! the cwd can follow without injecting a prompt hook.
//!
//! alacritty_terminal's parser drops OSC 7, so the raw PTY output has to go through
//! [`Osc7Scanner`] before it reaches the parser. The payload is untrusted: only `file:`
//! URIs whose host is empty, `localhost` or this machine are accepted.

use std::path::PathBuf;

/// Longest OSC 7 payload kept (longer ones are dropped).
const MAX_PAYLOAD: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    /// Saw ESC.
    Esc,
    /// Saw `ESC ]`.
    OscStart,
    /// Saw `ESC ] 7`.
    Osc7,
    /// Inside the payload.
    Payload,
    /// ESC inside the payload (ST is `ESC \`).
    PayloadEsc,
    /// Some other OSC (or an overlong payload): skip to its terminator.
    OtherOsc,
    OtherOscEsc,
}

/// Incremental OSC 7 finder over raw terminal output (sequences may be split across
/// reads). Never panics; memory is bounded.
#[derive(Clone, Debug, Default)]
pub struct Osc7Scanner {
    state: State,
    payload: Vec<u8>,
    host: Option<String>,
}

impl Osc7Scanner {
    /// A scanner accepting `file://<host>/…` for `host` (besides empty / `localhost`).
    /// `None` = this machine's name ([`local_hostname`]).
    pub fn new(host: Option<String>) -> Osc7Scanner {
        Osc7Scanner { host: host.or_else(local_hostname), ..Default::default() }
    }

    /// Feed output; returns the last directory reported in it, if any.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<PathBuf> {
        let mut found = None;
        for &b in bytes {
            self.state = match (self.state, b) {
                (State::Ground, 0x1b) => State::Esc,
                (State::Ground, _) => State::Ground,
                (State::Esc, b']') => State::OscStart,
                (State::Esc, 0x1b) => State::Esc,
                (State::Esc, _) => State::Ground,
                (State::OscStart, b'7') => State::Osc7,
                (State::OscStart | State::Osc7 | State::OtherOsc, 0x07) => State::Ground,
                (State::OscStart | State::Osc7 | State::OtherOsc, 0x1b) => State::OtherOscEsc,
                (State::OscStart, _) => State::OtherOsc,
                (State::Osc7, b';') => {
                    self.payload.clear();
                    State::Payload
                }
                (State::Osc7, _) => State::OtherOsc,
                (State::Payload, 0x07) => {
                    found = parse_osc7(&self.payload, self.host.as_deref()).or(found);
                    State::Ground
                }
                (State::Payload, 0x1b) => State::PayloadEsc,
                (State::Payload, _) if self.payload.len() >= MAX_PAYLOAD => State::OtherOsc,
                (State::Payload, _) => {
                    self.payload.push(b);
                    State::Payload
                }
                (State::PayloadEsc, b'\\') => {
                    found = parse_osc7(&self.payload, self.host.as_deref()).or(found);
                    State::Ground
                }
                // ESC + something else aborts the sequence (and may start a new one).
                (State::PayloadEsc | State::OtherOscEsc, b']') => State::OscStart,
                (State::PayloadEsc | State::OtherOscEsc, 0x1b) => State::Esc,
                (State::PayloadEsc | State::OtherOscEsc, _) => State::Ground,
                (State::OtherOsc, _) => State::OtherOsc,
            };
            if self.state == State::Ground && !self.payload.is_empty() {
                self.payload.clear();
            }
        }
        found
    }
}

/// The directory of an OSC 7 payload (`file://host/percent-encoded/path`), when the
/// host is empty, `localhost` or `local_host` (case-insensitive; a short name matches
/// its fully qualified form and vice versa). Other schemes, remote hosts, relative or
/// undecodable paths and paths with control characters give `None`.
pub fn parse_osc7(payload: &[u8], local_host: Option<&str>) -> Option<PathBuf> {
    let s = std::str::from_utf8(payload).ok()?.trim();
    let rest = strip_prefix_ci(s, "file://")?;
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    if !host_is_local(host, local_host) {
        return None;
    }
    let raw = percent_decode(path)?;
    if raw.iter().any(|&b| b < 0x20 || b == 0x7f) {
        return None;
    }
    to_path(raw)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

fn host_is_local(host: &str, local: Option<&str>) -> bool {
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let Some(local) = local.map(str::trim).filter(|l| !l.is_empty()) else { return false };
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_ascii_lowercase();
    host.eq_ignore_ascii_case(local)
        || (!host.contains('.') && short(local) == host.to_ascii_lowercase())
        || (!local.contains('.') && short(host) == local.to_ascii_lowercase())
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

#[cfg(unix)]
fn to_path(raw: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(raw)))
}

#[cfg(not(unix))]
fn to_path(raw: Vec<u8>) -> Option<PathBuf> {
    windows_path(&String::from_utf8(raw).ok()?).map(PathBuf::from)
}

/// The Windows path of a decoded OSC 7 path: only drive paths `/X:/…` (→ `X:\…`) are
/// accepted. Paths from a shell that sees another filesystem layout (WSL's `/home/me`,
/// MSYS / Cygwin's `/c/Users`) are dropped instead of becoming `\home\me` on the
/// current drive. Pure (and tested) on every OS.
#[cfg_attr(unix, allow(dead_code))]
fn windows_path(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() < 3 || b[0] != b'/' || !b[1].is_ascii_alphabetic() || b[2] != b':' {
        return None;
    }
    let rest = &s[3..];
    if !rest.is_empty() && !rest.starts_with('/') {
        return None; // `/C:dir`: drive-relative, not a real location.
    }
    let rest = rest.replace('/', "\\");
    Some(format!("{}:{}", b[1].to_ascii_uppercase() as char, if rest.is_empty() { "\\" } else { &rest }))
}

/// This machine's host name (Linux: `/proc/sys/kernel/hostname`; elsewhere
/// `COMPUTERNAME` / `HOSTNAME`).
pub fn local_hostname() -> Option<String> {
    #[cfg(target_os = "linux")]
    if let Ok(h) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let h = h.trim();
        if !h.is_empty() {
            return Some(h.to_owned());
        }
    }
    ["COMPUTERNAME", "HOSTNAME"].iter().find_map(|k| std::env::var(k).ok().filter(|h| !h.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Option<PathBuf> {
        parse_osc7(s.as_bytes(), Some("fedora.lan"))
    }

    #[cfg(unix)]
    #[test]
    fn parses_local_file_uris() {
        assert_eq!(p("file:///home/me"), Some(PathBuf::from("/home/me")));
        assert_eq!(p("file://localhost/tmp/a%20b"), Some(PathBuf::from("/tmp/a b")));
        assert_eq!(p("FILE://Fedora.lan/srv/%C3%BCr%C3%BCn"), Some(PathBuf::from("/srv/ürün")));
        assert_eq!(p("file://fedora/x"), Some(PathBuf::from("/x")), "short name of this host");
        assert_eq!(parse_osc7(b"file://fedora.lan/x", Some("fedora")), Some(PathBuf::from("/x")));
        // Non-UTF-8 bytes survive on unix.
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(p("file:///a%FF").unwrap().as_os_str().as_bytes(), b"/a\xff");
    }

    #[test]
    fn windows_paths_only_from_drive_paths() {
        assert_eq!(windows_path("/C:/Users/me").as_deref(), Some("C:\\Users\\me"));
        assert_eq!(windows_path("/d:/").as_deref(), Some("D:\\"));
        assert_eq!(windows_path("/e:").as_deref(), Some("E:\\"));
        for bad in ["/home/me", "/c/Users/me", "/mnt/c/Users", "/", "", "C:/x", "/C:x", "/1:/x"] {
            assert_eq!(windows_path(bad), None, "{bad:?}");
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn parses_windows_file_uris() {
        assert_eq!(p("file:///C:/Users/me%20x"), Some(PathBuf::from("C:\\Users\\me x")));
        assert_eq!(p("file://localhost/home/me"), None, "WSL path");
        assert_eq!(p("file:///c/Users/me"), None, "MSYS path");
    }

    #[test]
    fn rejects_foreign_or_bad_payloads() {
        for bad in [
            "file://otherhost/home/me",
            "file://fedora.example.com/x",
            "http://localhost/x",
            "file://",
            "file:///bad%2",
            "file:///bad%zz",
            "file:///new%0Aline",
            "file:///esc%1B[31m",
            "/plain/path",
            "",
        ] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
        assert_eq!(parse_osc7(b"file://somehost/x", None), None);
        assert_eq!(parse_osc7(b"\xff\xfe", None), None);
    }

    #[cfg(unix)]
    #[test]
    fn scanner_handles_terminators_splits_and_noise() {
        let mut s = Osc7Scanner::new(Some("box".into()));
        assert_eq!(s.feed(b"hello \x1b]0;title\x07 \x1b]7;file://box/a\x07$ "), Some(PathBuf::from("/a")));
        assert_eq!(s.feed(b"\x1b]7;file:///b\x1b\\"), Some(PathBuf::from("/b")));
        // Split across reads, byte by byte.
        let seq = b"x\x1b]7;file://localhost/split/dir\x1b\\y";
        let mut got = None;
        for b in seq.iter() {
            got = s.feed(std::slice::from_ref(b)).or(got);
        }
        assert_eq!(got, Some(PathBuf::from("/split/dir")));
        // Last one wins; foreign hosts and other OSCs are ignored.
        assert_eq!(
            s.feed(b"\x1b]7;file:///one\x07\x1b]7;file://evil/two\x07\x1b]8;;file:///three\x07"),
            Some(PathBuf::from("/one"))
        );
        assert_eq!(s.feed(b"\x1b]77;file:///nope\x07\x1b]0;\x1b]7;file:///late\x07"), Some(PathBuf::from("/late")));
        // An aborted sequence doesn't leak into the next.
        assert_eq!(s.feed(b"\x1b]7;file:///abort\x1bX\x1b]7;file:///ok\x07"), Some(PathBuf::from("/ok")));
        // Overlong payloads are dropped, then scanning resumes.
        let mut long = b"\x1b]7;file:///".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_PAYLOAD + 10));
        long.extend(b"\x07\x1b]7;file:///after\x07");
        assert_eq!(s.feed(&long), Some(PathBuf::from("/after")));
        assert_eq!(s.feed(b"plain output"), None);
    }

    #[test]
    fn scanner_never_panics_on_garbage() {
        let mut s = Osc7Scanner::new(None);
        let mut x: u32 = 0x1234_5678;
        let mut buf = Vec::new();
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            buf.push(match x % 7 {
                0 => 0x1b,
                1 => b']',
                2 => b'7',
                3 => b';',
                4 => 0x07,
                5 => b'\\',
                _ => (x >> 8) as u8,
            });
        }
        let _ = s.feed(&buf);
    }
}
