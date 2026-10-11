//! Remote location URIs: `sftp://user@host:port/path`, `webdav(s)://`, `ftp(s)://`,
//! `s3://bucket/prefix?endpoint=…&region=…`, plus `fake://` (in-process test provider).
//!
//! The path is always absolute, `/`-separated and normalized (no empty, `.` or `..`
//! segments, no trailing `/` except the root). User names and path segments are
//! percent-decoded; [`RemoteUri::display`] re-encodes only what would not round-trip
//! (`%`, `?`, `#`), so the address bar stays readable.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scheme {
    Sftp,
    Ftp,
    Ftps,
    WebDav,
    WebDavs,
    S3,
    /// SMB 2 / 3 (`smb://[domain;]user@server/share/path`; off Windows, see [`crate::smb`]).
    Smb,
    /// In-memory provider (tests and the `SOYUZ_FAKE_REMOTE` debug hook).
    Fake,
}

impl Scheme {
    pub const ALL: [Scheme; 7] =
        [Scheme::Sftp, Scheme::WebDavs, Scheme::WebDav, Scheme::Ftps, Scheme::Ftp, Scheme::S3, Scheme::Smb];

    pub fn parse(s: &str) -> Option<Scheme> {
        Some(match s.to_ascii_lowercase().as_str() {
            "sftp" | "ssh" | "scp" => Scheme::Sftp,
            "ftp" => Scheme::Ftp,
            "ftps" | "ftpes" => Scheme::Ftps,
            "webdav" | "dav" => Scheme::WebDav,
            "webdavs" | "davs" => Scheme::WebDavs,
            "s3" => Scheme::S3,
            "smb" | "cifs" => Scheme::Smb,
            "fake" => Scheme::Fake,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Sftp => "sftp",
            Scheme::Ftp => "ftp",
            Scheme::Ftps => "ftps",
            Scheme::WebDav => "webdav",
            Scheme::WebDavs => "webdavs",
            Scheme::S3 => "s3",
            Scheme::Smb => "smb",
            Scheme::Fake => "fake",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scheme::Sftp => "SFTP",
            Scheme::Ftp => "FTP",
            Scheme::Ftps => "FTPS",
            Scheme::WebDav => "WebDAV",
            Scheme::WebDavs => "WebDAV (HTTPS)",
            Scheme::S3 => "S3",
            Scheme::Smb => "SMB",
            Scheme::Fake => "Test",
        }
    }

    /// Well-known port (`0` when the scheme has none, e.g. S3 where the endpoint decides).
    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Sftp => 22,
            Scheme::Ftp | Scheme::Ftps => 21,
            Scheme::WebDav => 80,
            Scheme::WebDavs => 443,
            Scheme::Smb => 445,
            Scheme::S3 | Scheme::Fake => 0,
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A parsed remote location.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct RemoteUri {
    pub scheme: Scheme,
    pub user: Option<String>,
    /// Lower-cased host name, IP address (IPv6 without brackets) or S3 bucket.
    pub host: String,
    /// `None` = the scheme's default port.
    pub port: Option<u16>,
    /// Absolute, normalized, `/`-separated (`"/"` = root).
    pub path: String,
    /// Connection parameters (S3: `endpoint`, `region`, `style`), in input order.
    pub query: Vec<(String, String)>,
}

/// Why a string is not a remote URI.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UriError {
    NotRemote,
    UnknownScheme(String),
    MissingHost,
    BadPort(String),
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UriError::NotRemote => f.write_str("not a remote address"),
            UriError::UnknownScheme(s) => write!(f, "unknown protocol “{s}”"),
            UriError::MissingHost => f.write_str("the address has no server name"),
            UriError::BadPort(p) => write!(f, "invalid port “{p}”"),
        }
    }
}

impl std::error::Error for UriError {}

/// Whether `s` looks like `scheme://…` (any scheme).
pub fn looks_remote(s: &str) -> bool {
    match s.find("://") {
        Some(i) if i >= 2 => s[..i].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        _ => false,
    }
}

impl RemoteUri {
    /// Parse; a password in the user-info part is dropped (see [`RemoteUri::parse_with_password`]).
    pub fn parse(s: &str) -> Result<RemoteUri, UriError> {
        Self::parse_with_password(s).map(|(u, _)| u)
    }

    /// Parse, returning the `user:password@` password separately (never stored in the URI).
    pub fn parse_with_password(s: &str) -> Result<(RemoteUri, Option<String>), UriError> {
        let s = s.trim();
        if !looks_remote(s) {
            return Err(UriError::NotRemote);
        }
        let i = s.find("://").ok_or(UriError::NotRemote)?;
        let scheme_str = &s[..i];
        let scheme = Scheme::parse(scheme_str).ok_or_else(|| UriError::UnknownScheme(scheme_str.to_string()))?;
        let rest = &s[i + 3..];
        // Fragment: ignored. Query: after the first `?`.
        let rest = rest.split('#').next().unwrap_or("");
        let (rest, query_str) = match rest.find('?') {
            Some(q) => (&rest[..q], Some(&rest[q + 1..])),
            None => (rest, None),
        };
        let (authority, path) = match rest.find(['/', '\\']) {
            Some(p) => (&rest[..p], &rest[p..]),
            None => (rest, ""),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(a) => (Some(&authority[..a]), &authority[a + 1..]),
            None => (None, authority),
        };
        let (user, password) = match userinfo {
            Some(ui) => match ui.find(':') {
                Some(c) => (Some(pct_decode(&ui[..c])), Some(pct_decode(&ui[c + 1..]))),
                None => (Some(pct_decode(ui)), None),
            },
            None => (None, None),
        };
        let user = user.filter(|u| !u.is_empty());
        let (host, port_str) = if let Some(h) = hostport.strip_prefix('[') {
            match h.find(']') {
                Some(e) => (&h[..e], h[e + 1..].strip_prefix(':')),
                None => return Err(UriError::MissingHost),
            }
        } else {
            match hostport.rfind(':') {
                Some(c) => (&hostport[..c], Some(&hostport[c + 1..])),
                None => (hostport, None),
            }
        };
        if host.is_empty() {
            return Err(UriError::MissingHost);
        }
        let port = match port_str {
            None | Some("") => None,
            Some(p) => Some(p.parse::<u16>().map_err(|_| UriError::BadPort(p.to_string()))?),
        };
        let port = port.filter(|&p| p != scheme.default_port() || scheme.default_port() == 0);
        let query = query_str
            .map(|q| {
                q.split('&')
                    .filter(|kv| !kv.is_empty())
                    .map(|kv| match kv.find('=') {
                        Some(e) => (pct_decode(&kv[..e]), pct_decode(&kv[e + 1..])),
                        None => (pct_decode(kv), String::new()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let host = if scheme == Scheme::S3 { host.to_string() } else { host.to_ascii_lowercase() };
        let uri = RemoteUri { scheme, user, host, port, path: normalize_user_path(&pct_decode(path)), query };
        Ok((uri, password.filter(|p| !p.is_empty())))
    }

    /// From a pane / job path (the URI stored in a `PathBuf`).
    pub fn from_path(p: &Path) -> Option<RemoteUri> {
        p.to_str().filter(|s| looks_remote(s)).and_then(|s| RemoteUri::parse(s).ok())
    }

    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.display())
    }

    pub fn effective_port(&self) -> u16 {
        self.port.unwrap_or(self.scheme.default_port())
    }

    pub fn query_value(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// The same server and connection parameters with another path.
    pub fn with_path(&self, path: &str) -> RemoteUri {
        RemoteUri { path: normalize_path(path), ..self.clone() }
    }

    pub fn is_root(&self) -> bool {
        self.path == "/"
    }

    /// Child `name` (a single segment).
    pub fn join(&self, name: &str) -> RemoteUri {
        self.with_path(&join_path(&self.path, name))
    }

    pub fn parent(&self) -> Option<RemoteUri> {
        parent_path(&self.path).map(|p| self.with_path(&p))
    }

    /// Last path segment (`None` at the root).
    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() { None } else { self.path.rsplit('/').next() }
    }

    /// `user@host[:port]` (what the breadcrumb shows for the root).
    pub fn server_label(&self) -> String {
        let mut s = String::new();
        if let Some(u) = &self.user {
            s.push_str(u);
            s.push('@');
        }
        s.push_str(&self.host_for_display());
        if let Some(p) = self.port {
            s.push(':');
            s.push_str(&p.to_string());
        }
        s
    }

    fn host_for_display(&self) -> String {
        if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() }
    }

    /// Identity of the connection (everything but the path): one provider per key.
    pub fn conn_key(&self) -> String {
        let mut k = format!("{}://{}", self.scheme, self.server_label());
        let mut q: Vec<_> = self.query.iter().map(|(a, b)| format!("{}={b}", a.to_ascii_lowercase())).collect();
        q.sort();
        if !q.is_empty() {
            k.push('?');
            k.push_str(&q.join("&"));
        }
        k
    }

    /// The connection root (`path = "/"`).
    pub fn root(&self) -> RemoteUri {
        self.with_path("/")
    }

    /// String form; round-trips through [`RemoteUri::parse`].
    pub fn display(&self) -> String {
        let mut s = format!("{}://", self.scheme);
        if let Some(u) = &self.user {
            s.push_str(&pct_encode(u, true));
            s.push('@');
        }
        s.push_str(&self.host_for_display());
        if let Some(p) = self.port {
            s.push(':');
            s.push_str(&p.to_string());
        }
        s.push_str(&pct_encode(&self.path, false));
        if !self.query.is_empty() {
            s.push('?');
            let q: Vec<String> =
                self.query.iter().map(|(k, v)| format!("{}={}", pct_encode(k, true), pct_encode(v, true))).collect();
            s.push_str(&q.join("&"));
        }
        s
    }
}

impl fmt::Display for RemoteUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

/// A UNC path (`\\server\share\folder`, `//server/share`, or a bare `\\server`) as an
/// `smb://server/share/folder` URI. `None` for anything else, including the Windows
/// device namespaces `\\?\…` and `\\.\…`.
pub fn smb_from_unc(s: &str) -> Option<RemoteUri> {
    let s = s.trim();
    let rest = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("//"))?;
    let (host, path) = match rest.find(['\\', '/']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if host.is_empty() || host == "?" || host == "." || host.contains([':', '@']) {
        return None;
    }
    Some(RemoteUri {
        scheme: Scheme::Smb,
        user: None,
        host: host.to_ascii_lowercase(),
        port: None,
        path: normalize_user_path(path),
        query: Vec::new(),
    })
}

impl RemoteUri {
    /// `\\server\share\folder` for an `smb://` URI on the default port (what the Windows
    /// redirector opens); `None` for other schemes or a custom port.
    pub fn to_unc(&self) -> Option<String> {
        if self.scheme != Scheme::Smb || self.port.is_some() {
            return None;
        }
        let mut s = format!(r"\\{}", self.host);
        if !self.is_root() {
            s.push_str(&self.path.replace('/', r"\"));
        }
        Some(s)
    }
}

/// Normalize an absolute `/`-separated provider path (`..` never escapes the root).
///
/// Only `/` separates segments: a `\` is part of a name here, so a hostile remote name
/// such as `..\..\Other\x` stays one (rejected) segment instead of becoming a path.
/// User-typed URIs and UNC paths go through [`normalize_user_path`], which accepts `\`.
pub fn normalize_path(p: &str) -> String {
    normalize_split(p, &['/'])
}

/// [`normalize_path`] for what a user typed (URI, UNC path, a `PathBuf` joined on
/// Windows): `\` separates segments too.
pub fn normalize_user_path(p: &str) -> String {
    normalize_split(p, &['/', '\\'])
}

fn normalize_split(p: &str, seps: &[char]) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split(seps) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `dir` + `/` + `name` for `/`-separated provider paths.
pub fn join_path(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// Parent of a `/`-separated provider path (`None` for `/`).
pub fn parent_path(p: &str) -> Option<String> {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    match t.rfind('/') {
        Some(0) => Some("/".into()),
        Some(i) => Some(t[..i].to_string()),
        None => Some("/".into()),
    }
}

/// Last segment of a `/`-separated provider path (a `\` is part of the name).
pub fn remote_name_of(p: &str) -> &str {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or(p)
}

/// Last segment of a `/`- (or, for local paths, `\`-) separated path.
pub fn name_of(p: &str) -> &str {
    p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(p)
}

/// Percent-decode (invalid escapes are kept literally; invalid UTF-8 is replaced).
pub fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Encode `%`, `?`, `#` (and, in `strict` mode for user names / query parts, also `@ : / & =`).
fn pct_encode(s: &str, strict: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let enc = matches!(c, '%' | '?' | '#') || (strict && matches!(c, '@' | ':' | '/' | '&' | '=' | '\\'));
        if enc {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Percent-encode a path for an HTTP request line (RFC 3986 unreserved + `/` kept).
pub fn url_encode_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_paths_keep_backslashes_in_names() {
        assert_eq!(normalize_path("/share/dir/..\\..\\Other\\v.txt"), "/share/dir/..\\..\\Other\\v.txt");
        assert_eq!(normalize_path("/a/../b"), "/b");
        assert_eq!(normalize_user_path("/share\\dir\\..\\x"), "/share/x");
        assert_eq!(remote_name_of("/d/..\\x"), "..\\x");
        let u = RemoteUri::parse("sftp://h/a").unwrap();
        assert_eq!(u.join("..\\..\\etc").path, "/a/..\\..\\etc");
        // Typed URIs (and URIs a Windows `PathBuf::join` extended) still accept `\`.
        assert_eq!(RemoteUri::parse("smb://h/share\\a").unwrap().path, "/share/a");
    }

    #[test]
    fn parse_full_sftp() {
        let u = RemoteUri::parse("sftp://alice@Example.COM:2222/home/alice/docs").unwrap();
        assert_eq!(u.scheme, Scheme::Sftp);
        assert_eq!(u.user.as_deref(), Some("alice"));
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, Some(2222));
        assert_eq!(u.path, "/home/alice/docs");
        assert_eq!(u.display(), "sftp://alice@example.com:2222/home/alice/docs");
        assert_eq!(u.conn_key(), "sftp://alice@example.com:2222");
    }

    #[test]
    fn defaults_and_normalization() {
        let u = RemoteUri::parse("sftp://host:22").unwrap();
        assert_eq!(u.port, None, "default port is dropped");
        assert_eq!(u.path, "/");
        assert!(u.is_root());
        assert_eq!(u.effective_port(), 22);
        let u = RemoteUri::parse("ftp://h//a/./b/../c/").unwrap();
        assert_eq!(u.path, "/a/c");
        let u = RemoteUri::parse("webdavs://h/../../x").unwrap();
        assert_eq!(u.path, "/x");
        assert_eq!(u.effective_port(), 443);
    }

    #[test]
    fn password_is_split_off() {
        let (u, pw) = RemoteUri::parse_with_password("ftp://bob:s%40cret@ftp.example.org/pub").unwrap();
        assert_eq!(pw.as_deref(), Some("s@cret"));
        assert_eq!(u.user.as_deref(), Some("bob"));
        assert!(!u.display().contains("cret"));
    }

    #[test]
    fn ipv6_and_errors() {
        let u = RemoteUri::parse("sftp://[::1]:2200/x").unwrap();
        assert_eq!(u.host, "::1");
        assert_eq!(u.display(), "sftp://[::1]:2200/x");
        assert_eq!(RemoteUri::parse("C:\\data"), Err(UriError::NotRemote));
        assert_eq!(RemoteUri::parse("gopher://x"), Err(UriError::UnknownScheme("gopher".into())));
        assert_eq!(RemoteUri::parse("sftp:///x"), Err(UriError::MissingHost));
        assert!(matches!(RemoteUri::parse("sftp://h:99999/"), Err(UriError::BadPort(_))));
    }

    #[test]
    fn s3_query_and_round_trip() {
        let s = "s3://My-Bucket/photos/2024?endpoint=http://127.0.0.1:9000&region=us-east-1";
        let u = RemoteUri::parse(s).unwrap();
        assert_eq!(u.host, "My-Bucket", "bucket names keep their case");
        assert_eq!(u.query_value("endpoint"), Some("http://127.0.0.1:9000"));
        assert_eq!(u.query_value("REGION"), Some("us-east-1"));
        assert_eq!(u.path, "/photos/2024");
        let back = RemoteUri::parse(&u.display()).unwrap();
        assert_eq!(back, u);
        assert_eq!(u.join("a b.jpg").path, "/photos/2024/a b.jpg");
    }

    #[test]
    fn round_trip_special_chars() {
        for p in ["/a b/ç#1/50%/q?x", "/", "/日本語"] {
            let u = RemoteUri {
                scheme: Scheme::WebDav,
                user: Some("a@b".into()),
                host: "h".into(),
                port: Some(8080),
                path: p.into(),
                query: vec![],
            };
            let s = u.display();
            assert_eq!(RemoteUri::parse(&s).unwrap(), u, "{s}");
        }
    }

    #[test]
    fn parent_join_name() {
        let u = RemoteUri::parse("webdav://h/a/b").unwrap();
        assert_eq!(u.file_name(), Some("b"));
        assert_eq!(u.parent().unwrap().path, "/a");
        assert_eq!(u.parent().unwrap().parent().unwrap().path, "/");
        assert_eq!(u.root().parent(), None);
        assert_eq!(join_path("/", "x"), "/x");
        assert_eq!(name_of(r"C:\a\b.txt"), "b.txt");
        assert_eq!(name_of("/a/b/"), "b");
    }

    #[test]
    fn looks_remote_rules() {
        assert!(looks_remote("sftp://h"));
        assert!(!looks_remote("C://x"), "drive letters are not schemes");
        assert!(!looks_remote("/tmp/x"));
        assert_eq!(url_encode_path("/a b/ü"), "/a%20b/%C3%BC");
        assert_eq!(pct_decode("a%20b%zz"), "a b%zz");
    }

    #[test]
    fn from_path_round_trip() {
        let u = RemoteUri::parse("ftps://u@h/x/y.txt").unwrap();
        assert_eq!(RemoteUri::from_path(&u.to_path_buf()), Some(u));
        assert_eq!(RemoteUri::from_path(Path::new("C:/x")), None);
    }

    #[test]
    fn smb_uris_and_unc() {
        let u = RemoteUri::parse("smb://CORP;alice@NAS:445/Media/Films").unwrap();
        assert_eq!(u.scheme, Scheme::Smb);
        assert_eq!(u.user.as_deref(), Some("CORP;alice"));
        assert_eq!(u.host, "nas");
        assert_eq!(u.port, None, "445 is the default");
        assert_eq!(u.path, "/Media/Films");
        assert_eq!(RemoteUri::parse("cifs://nas/x").unwrap().scheme, Scheme::Smb);
        let u = RemoteUri::parse(r"smb://CORP%5Calice@nas:4455/").unwrap();
        assert_eq!(u.user.as_deref(), Some(r"CORP\alice"));
        assert_eq!(u.port, Some(4455));
        assert_eq!(RemoteUri::parse(&u.display()).unwrap(), u, "{}", u.display());

        let u = smb_from_unc(r"\\NAS\Media\Films\").unwrap();
        assert_eq!(u.display(), "smb://nas/Media/Films");
        assert_eq!(u.to_unc().as_deref(), Some(r"\\nas\Media\Films"));
        assert_eq!(smb_from_unc("//nas/share").unwrap().path, "/share");
        let bare = smb_from_unc(r"\\nas").unwrap();
        assert!(bare.is_root());
        assert_eq!(bare.to_unc().as_deref(), Some(r"\\nas"));
        assert_eq!(smb_from_unc(r"\\?\C:\x"), None);
        assert_eq!(smb_from_unc(r"\\.\pipe\x"), None);
        assert_eq!(smb_from_unc(r"\\"), None);
        assert_eq!(smb_from_unc("/home/x"), None);
        assert_eq!(RemoteUri::parse("smb://nas:4455/a").unwrap().to_unc(), None);
        assert_eq!(RemoteUri::parse("sftp://nas/a").unwrap().to_unc(), None);
    }
}
