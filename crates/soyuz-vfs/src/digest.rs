//! HTTP Digest access authentication (RFC 7616) for WebDAV: parse a server's
//! `WWW-Authenticate: Digest …` challenge and answer it (MD5 or SHA-256, `qop=auth` or
//! the RFC 2069 form without `qop`). The `-sess` algorithms and `auth-int` are not
//! supported (such a challenge is ignored, so Basic is tried).
//!
//! [`verify`] checks an `Authorization` header against a challenge; the test WebDAV
//! server uses it.

use md5::Digest as _;

/// The hash a challenge asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Md5,
    Sha256,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Algorithm::Md5 => "MD5",
            Algorithm::Sha256 => "SHA-256",
        }
    }

    fn hash(self, s: &str) -> String {
        let bytes: Vec<u8> = match self {
            Algorithm::Md5 => md5::Md5::digest(s.as_bytes()).to_vec(),
            Algorithm::Sha256 => sha2::Sha256::digest(s.as_bytes()).to_vec(),
        };
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// A Digest challenge (`WWW-Authenticate: Digest realm=…, nonce=…`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: Algorithm,
    /// The server offers `qop=auth` (answered with `nc` / `cnonce`).
    pub qop_auth: bool,
    /// The last request failed only because the nonce was old.
    pub stale: bool,
}

/// `key=value` / `key="quoted, value"` pairs of one header value, in order.
fn params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().trim_start_matches(',').trim().to_ascii_lowercase();
        rest = rest[eq + 1..].trim_start();
        let value;
        if let Some(q) = rest.strip_prefix('"') {
            let mut v = String::new();
            let mut chars = q.char_indices();
            let mut end = q.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            v.push(n);
                        }
                    }
                    '"' => {
                        end = i + 1;
                        break;
                    }
                    c => v.push(c),
                }
            }
            value = v;
            rest = q[end.min(q.len())..].trim_start();
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            value = rest[..end].trim().to_string();
            rest = &rest[end..];
        }
        rest = rest.trim_start().trim_start_matches(',').trim_start();
        out.push((key, value));
    }
    out
}

/// The best Digest challenge among `WWW-Authenticate` header values (SHA-256 before MD5);
/// `None` when there is none we can answer.
pub fn parse_challenges<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<Challenge> {
    let mut best: Option<Challenge> = None;
    for v in values {
        // One header may hold several challenges ("Digest …, Basic …"): split at schemes.
        let lower = v.to_ascii_lowercase();
        let mut cut = Vec::new();
        for (i, _) in lower.match_indices("digest ") {
            if i == 0 || lower[..i].trim_end().ends_with(',') {
                cut.push(i);
            }
        }
        for (n, &i) in cut.iter().enumerate() {
            let end = cut.get(n + 1).copied().unwrap_or(v.len());
            let body = &v[i + "digest ".len()..end];
            // Stop at a following non-Digest scheme ("…, Basic realm=…").
            let body = match body.to_ascii_lowercase().find(", basic ") {
                Some(b) => &body[..b],
                None => body,
            };
            if let Some(c) = challenge(body)
                && best.as_ref().is_none_or(|b| b.algorithm == Algorithm::Md5 && c.algorithm == Algorithm::Sha256)
            {
                best = Some(c);
            }
        }
    }
    best
}

fn challenge(body: &str) -> Option<Challenge> {
    let p = params(body);
    let get = |k: &str| p.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let algorithm = match get("algorithm").map(|a| a.to_ascii_uppercase()).as_deref() {
        None | Some("MD5") => Algorithm::Md5,
        Some("SHA-256") => Algorithm::Sha256,
        _ => return None,
    };
    let qop = get("qop");
    let qop_auth = qop.as_deref().is_some_and(|q| q.split(',').any(|x| x.trim().eq_ignore_ascii_case("auth")));
    if qop.is_some() && !qop_auth {
        return None; // auth-int only
    }
    Some(Challenge {
        realm: get("realm").unwrap_or_default(),
        nonce: get("nonce")?,
        opaque: get("opaque"),
        algorithm,
        qop_auth,
        stale: get("stale").is_some_and(|s| s.eq_ignore_ascii_case("true")),
    })
}

/// The response hash for `method` `uri` with nonce count `nc` and client nonce `cnonce`.
fn response(c: &Challenge, user: &str, password: &str, method: &str, uri: &str, nc: u32, cnonce: &str) -> String {
    let h = |s: String| c.algorithm.hash(&s);
    let ha1 = h(format!("{user}:{}:{password}", c.realm));
    let ha2 = h(format!("{method}:{uri}"));
    if c.qop_auth {
        h(format!("{ha1}:{}:{nc:08x}:{cnonce}:auth:{ha2}", c.nonce))
    } else {
        h(format!("{ha1}:{}:{ha2}", c.nonce))
    }
}

fn quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The `Authorization` header value answering `c` for `method` `uri` (the request target:
/// path and query, as sent).
pub fn authorization(
    c: &Challenge,
    user: &str,
    password: &str,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let r = response(c, user, password, method, uri, nc, cnonce);
    let mut v = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", algorithm={}, response=\"{r}\"",
        quote(user),
        quote(&c.realm),
        quote(&c.nonce),
        quote(uri),
        c.algorithm.name()
    );
    if c.qop_auth {
        v.push_str(&format!(", qop=auth, nc={nc:08x}, cnonce=\"{}\"", quote(cnonce)));
    }
    if let Some(o) = &c.opaque {
        v.push_str(&format!(", opaque=\"{}\"", quote(o)));
    }
    v
}

/// A fresh client nonce (not secret; it only has to differ between requests).
pub fn cnonce(nc: u32) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    Algorithm::Sha256.hash(&format!("{t}:{nc}:{}", std::process::id()))[..32].to_string()
}

/// Whether the `Authorization` header value `header` answers `c` for `user` / `password`.
pub fn verify(c: &Challenge, header: &str, user: &str, password: &str, method: &str) -> bool {
    let Some(body) = header.trim().strip_prefix("Digest ") else { return false };
    let p = params(body);
    let get = |k: &str| p.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let (Some(u), Some(uri), Some(resp), Some(nonce)) = (get("username"), get("uri"), get("response"), get("nonce"))
    else {
        return false;
    };
    if u != user || nonce != c.nonce {
        return false;
    }
    let nc = get("nc").and_then(|n| u32::from_str_radix(n, 16).ok()).unwrap_or(0);
    let cnonce = get("cnonce").unwrap_or_default();
    response(c, user, password, method, uri, nc, cnonce) == resp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7616 §3.9.1: the same request answered with SHA-256 and with MD5.
    #[test]
    fn rfc7616_example() {
        let mut c = Challenge {
            realm: "http-auth@example.org".into(),
            nonce: "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v".into(),
            opaque: Some("FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS".into()),
            algorithm: Algorithm::Sha256,
            qop_auth: true,
            stale: false,
        };
        let cn = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        assert_eq!(
            response(&c, "Mufasa", "Circle of Life", "GET", "/dir/index.html", 1, cn),
            "753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1"
        );
        c.algorithm = Algorithm::Md5;
        assert_eq!(
            response(&c, "Mufasa", "Circle of Life", "GET", "/dir/index.html", 1, cn),
            "8ca523f5e9506fed4657c9700eebdbec"
        );
        let h = authorization(&c, "Mufasa", "Circle of Life", "GET", "/dir/index.html", 1, cn);
        assert!(h.contains("nc=00000001") && h.contains("opaque=\"FQhe"), "{h}");
        assert!(verify(&c, &h, "Mufasa", "Circle of Life", "GET"));
        assert!(!verify(&c, &h, "Mufasa", "wrong", "GET"));
        assert!(!verify(&c, &h, "Mufasa", "Circle of Life", "PUT"));
    }

    #[test]
    fn picks_the_strongest_challenge() {
        let one = r#"Digest realm="r", qop="auth", algorithm=MD5, nonce="n1", opaque="o""#;
        let two = r#"Digest realm="r", qop="auth, auth-int", algorithm=SHA-256, nonce="n2""#;
        let c = parse_challenges([one, two]).unwrap();
        assert_eq!((c.algorithm, c.nonce.as_str(), c.qop_auth), (Algorithm::Sha256, "n2", true));
        // Several schemes in one header; quoted commas.
        let mixed = r#"Basic realm="x", Digest realm="a, b", nonce="n", stale=TRUE"#;
        let c = parse_challenges([mixed]).unwrap();
        assert_eq!((c.realm.as_str(), c.stale, c.qop_auth), ("a, b", true, false));
        assert!(parse_challenges([r#"Digest realm="r", nonce="n", algorithm=MD5-sess"#]).is_none());
        assert!(parse_challenges([r#"Digest realm="r", nonce="n", qop="auth-int""#]).is_none());
        assert!(parse_challenges([r#"Basic realm="r""#]).is_none());
    }
}
