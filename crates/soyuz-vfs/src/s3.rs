//! S3-compatible object storage (AWS, MinIO, R2, B2, Wasabi …) with `rusty-s3` request
//! signing and reqwest. `s3://bucket/prefix?endpoint=https://host&region=…&style=path`.
//!
//! Folders are key prefixes (`a/b/`); "New folder" writes an empty `a/b/` marker.
//! Uploads above [`MULTIPART_MIN`] use multipart; a resumed upload finds its unfinished
//! multipart upload (`ListMultipartUploads` + `ListParts`) and continues after the last
//! complete part. Rename is server-side copy + delete.

use std::io::{Read, Write};
use std::time::{Duration, SystemTime};

use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::Url;
use rusty_s3::actions::{ListObjectsV2, ListObjectsV2Response, ListParts};
use rusty_s3::{Bucket, S3Action, UrlStyle};
use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::http::{BodyReader, client, send, send_text, status_error};
use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream, err};
use crate::registry::{Credentials, net_settings};
use crate::rt::runtime;
use crate::uri::{RemoteUri, normalize_path, remote_name_of};

/// Smallest part size S3 accepts (except the last part) and the multipart threshold.
pub const MULTIPART_MIN: u64 = 8 * 1024 * 1024;
const SIGN_FOR: Duration = Duration::from_secs(3600);

/// An uploaded part: `(number, etag, size)`.
type Part = (u16, String, u64);

pub struct S3Provider {
    uri: RemoteUri,
    bucket: Bucket,
    creds: Option<rusty_s3::Credentials>,
    client: reqwest::Client,
    timeout: Duration,
}

/// Object key of a provider path (`/a/b` → `a/b`).
fn key_of(path: &str) -> String {
    normalize_path(path).trim_start_matches('/').to_string()
}

/// List prefix of a folder path (`/` → ``, `/a` → `a/`).
fn prefix_of(path: &str) -> String {
    let k = key_of(path);
    if k.is_empty() { k } else { format!("{k}/") }
}

impl S3Provider {
    pub fn connect(uri: RemoteUri, creds: Credentials) -> FsResult<Self> {
        let net = net_settings();
        let region = uri.query_value("region").filter(|r| !r.is_empty()).unwrap_or("us-east-1").to_string();
        let endpoint = uri
            .query_value("endpoint")
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://s3.{region}.amazonaws.com"));
        let endpoint = if endpoint.contains("://") { endpoint } else { format!("https://{endpoint}") };
        let style = match uri.query_value("style") {
            Some(s) if s.eq_ignore_ascii_case("virtual") => UrlStyle::VirtualHost,
            Some(_) => UrlStyle::Path,
            None if endpoint.contains("amazonaws.com") => UrlStyle::VirtualHost,
            None => UrlStyle::Path,
        };
        let url = Url::parse(&endpoint).map_err(|e| {
            FsError::with_message(FsErrorKind::InvalidName, "connect", None, format!("bad endpoint: {e}"))
        })?;
        let bucket = Bucket::new(url, style, uri.host.clone(), region)
            .map_err(|e| FsError::with_message(FsErrorKind::InvalidName, "connect", None, e.to_string()))?;
        let key = creds.user.clone().or_else(|| uri.user.clone());
        let creds = match (key, creds.secret) {
            (Some(k), Some(s)) if !k.is_empty() => Some(rusty_s3::Credentials::new(k, s)),
            _ => None,
        };
        Ok(S3Provider { uri, bucket, creds, client: client(net.connect_timeout)?, timeout: net.io_timeout })
    }

    fn list_page(&self, prefix: &str, delimiter: bool, token: Option<&str>) -> FsResult<ListObjectsV2Response> {
        let mut a: ListObjectsV2<'_> = self.bucket.list_objects_v2(self.creds.as_ref());
        if !prefix.is_empty() {
            a.with_prefix(prefix.to_string());
        }
        if delimiter {
            a.with_delimiter("/");
        }
        if let Some(t) = token {
            a.with_continuation_token(t.to_string());
        }
        let url = a.sign(SIGN_FOR);
        let text = send_text(self.client.get(url), self.timeout, "list", prefix)?;
        ListObjectsV2::parse_response(&text)
            .map_err(|e| err(FsErrorKind::Io, "list", prefix, format!("bad server reply: {e}")))
    }

    /// Every key under `prefix` (no delimiter), for folder rename / delete.
    fn all_keys(&self, prefix: &str) -> FsResult<Vec<String>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let r = self.list_page(prefix, false, token.as_deref())?;
            out.extend(r.contents.into_iter().map(|c| c.key));
            match r.next_continuation_token {
                Some(t) => token = Some(t),
                None => return Ok(out),
            }
        }
    }

    fn head(&self, key: &str) -> FsResult<Option<(u64, Option<SystemTime>)>> {
        let url = self.bucket.head_object(self.creds.as_ref(), key).sign(SIGN_FOR);
        let resp = send(self.client.head(url), self.timeout, "stat", key)?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if let Some(e) = status_error(resp.status(), "stat", key, "") {
            return Err(e);
        }
        let h = resp.headers();
        let size = h.get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
        let modified = h.get("last-modified").and_then(|v| v.to_str().ok()).and_then(crate::webdav::parse_http_date);
        Ok(Some((size, modified)))
    }

    fn delete_key(&self, key: &str) -> FsResult<()> {
        let url = self.bucket.delete_object(self.creds.as_ref(), key).sign(SIGN_FOR);
        send_text(self.client.delete(url), self.timeout, "delete", key).map(|_| ())
    }

    fn copy_key(&self, from: &str, to: &str) -> FsResult<()> {
        let source = format!("/{}/{}", self.bucket.name(), crate::uri::url_encode_path(from));
        let mut a = self.bucket.put_object(self.creds.as_ref(), to);
        a.headers_mut().insert("x-amz-copy-source", source.clone());
        let url = a.sign(SIGN_FOR);
        let body = send_text(self.client.put(url).header("x-amz-copy-source", source), self.timeout, "copy", from)?;
        // CopyObject can fail with 200 + <Error> in the body.
        if body.contains("<Error>") {
            return Err(status_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "copy", from, &body)
                .unwrap_or_else(|| err(FsErrorKind::Io, "copy", from, "copy failed")));
        }
        Ok(())
    }

    /// Unfinished multipart upload of `key` (most recent): `(upload_id, parts)`.
    fn find_upload(&self, key: &str) -> FsResult<Option<(String, Vec<Part>)>> {
        let mut a = self.bucket.list_objects_v2(self.creds.as_ref());
        let q = a.query_mut();
        q.remove("list-type");
        q.remove("encoding-type");
        q.insert("uploads", "");
        q.insert("prefix", key.to_string());
        let url = a.sign(SIGN_FOR);
        let text = send_text(self.client.get(url), self.timeout, "resume", key)?;
        let Some(id) = parse_uploads(&text, key).into_iter().last() else { return Ok(None) };
        let url = ListParts::new(&self.bucket, self.creds.as_ref(), key, &id).sign(SIGN_FOR);
        let text = send_text(self.client.get(url), self.timeout, "resume", key)?;
        let parts = ListParts::parse_response(&text)
            .map_err(|e| err(FsErrorKind::Io, "resume", key, format!("bad server reply: {e}")))?;
        let mut v: Vec<Part> = parts.parts.into_iter().map(|p| (p.number, p.etag, p.size)).collect();
        v.sort_by_key(|p| p.0);
        Ok(Some((id, v)))
    }
}

/// Upload ids for exactly `key` in a `ListMultipartUploadsResult` (oldest first).
pub fn parse_uploads(xml: &str, key: &str) -> Vec<String> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out = Vec::new();
    let (mut k, mut id, mut field) = (String::new(), String::new(), String::new());
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                field = e.local_name().as_ref().to_string();
                if field == "Upload" {
                    k.clear();
                    id.clear();
                }
            }
            Ok(Event::Text(t)) => {
                let v = t.xml10_content().into_owned();
                match field.as_str() {
                    "Key" => k.push_str(&v),
                    "UploadId" => id.push_str(&v),
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == "Upload" && k == key && !id.is_empty() {
                    out.push(id.clone());
                }
                field.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

/// Map a ListObjectsV2 page of folder `prefix` to entries (folders from common prefixes,
/// the folder's own marker object skipped).
pub fn entries_from_list(r: &ListObjectsV2Response, prefix: &str) -> Vec<RemoteEntry> {
    let mut out = Vec::new();
    for p in &r.common_prefixes {
        let name = p.prefix.strip_prefix(prefix).unwrap_or(&p.prefix).trim_end_matches('/');
        if !name.is_empty() {
            out.push(RemoteEntry::dir(name, None));
        }
    }
    for c in &r.contents {
        let name = c.key.strip_prefix(prefix).unwrap_or(&c.key);
        if name.is_empty() || name.contains('/') {
            continue;
        }
        let modified = chrono::DateTime::parse_from_rfc3339(&c.last_modified)
            .ok()
            .and_then(|d| u64::try_from(d.timestamp()).ok())
            .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s));
        out.push(RemoteEntry::file(name, c.size, modified));
    }
    out
}

impl Provider for S3Provider {
    fn describe(&self) -> String {
        format!("S3 {}", self.bucket.name())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            range_read: true,
            resume_write: true,
            atomic_write: true,
            rename: false,
            rename_replaces: true,
            set_times: false,
            watch: false,
        }
    }

    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
        let prefix = prefix_of(path);
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let r = self.list_page(&prefix, true, token.as_deref())?;
            out.extend(entries_from_list(&r, &prefix));
            match r.next_continuation_token {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(crate::provider::sanitize_listing("S3", path, out))
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let key = key_of(path);
        if key.is_empty() {
            return Ok(RemoteEntry::dir("", None));
        }
        if let Some((size, modified)) = self.head(&key)? {
            return Ok(RemoteEntry::file(remote_name_of(&key), size, modified));
        }
        let r = self.list_page(&format!("{key}/"), true, None)?;
        if !r.contents.is_empty() || !r.common_prefixes.is_empty() {
            return Ok(RemoteEntry::dir(remote_name_of(&key), None));
        }
        Err(err(FsErrorKind::NotFound, "stat", path, "not found"))
    }

    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        let key = key_of(path);
        let url = self.bucket.get_object(self.creds.as_ref(), &key).sign(SIGN_FOR);
        let mut rb = self.client.get(url);
        if offset > 0 {
            rb = rb.header("Range", format!("bytes={offset}-"));
        }
        let resp = send(rb, self.timeout, "open", path)?;
        if let Some(e) = status_error(resp.status(), "open", path, "") {
            return Err(e);
        }
        let skip = if offset > 0 && resp.status().as_u16() != 206 { offset } else { 0 };
        Ok(Box::new(BodyReader::new(resp, skip, self.timeout)))
    }

    fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        let key = key_of(path);
        let size = opts.size_hint.unwrap_or(0);
        // Parts: at least 8 MiB, at most 10 000 of them.
        let part_size = MULTIPART_MIN.max(size.div_ceil(9_000));
        let mut w = S3Write {
            p: S3Provider {
                uri: self.uri.clone(),
                bucket: self.bucket.clone(),
                creds: self.creds.clone(),
                client: self.client.clone(),
                timeout: self.timeout,
            },
            key,
            part_size,
            buf: Vec::new(),
            upload: None,
            parts: Vec::new(),
            resumed: 0,
        };
        if opts.resume
            && size >= MULTIPART_MIN
            && let Ok(Some((id, parts))) = w.p.find_upload(&w.key)
        {
            // Keep the leading run of full, consecutive parts.
            let mut keep = Vec::new();
            for (i, (n, etag, sz)) in parts.into_iter().enumerate() {
                if n as usize != i + 1 || sz != part_size {
                    break;
                }
                keep.push(etag);
            }
            w.resumed = keep.len() as u64 * part_size;
            w.parts = keep;
            w.upload = Some(id);
        }
        Ok(Box::new(w))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        let key = prefix_of(path);
        let url = self.bucket.put_object(self.creds.as_ref(), &key).sign(SIGN_FOR);
        send_text(self.client.put(url).body(Vec::new()), self.timeout, "mkdir", path).map(|_| ())
    }

    fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        let (fk, tk) = (key_of(from), key_of(to));
        if self.head(&fk)?.is_some() {
            self.copy_key(&fk, &tk)?;
            return self.delete_key(&fk);
        }
        let (fp, tp) = (format!("{fk}/"), format!("{tk}/"));
        let keys = self.all_keys(&fp)?;
        if keys.is_empty() {
            return Err(err(FsErrorKind::NotFound, "rename", from, "not found"));
        }
        for k in &keys {
            self.copy_key(k, &format!("{tp}{}", &k[fp.len()..]))?;
        }
        for k in &keys {
            self.delete_key(k)?;
        }
        Ok(())
    }

    fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
        if is_dir {
            // Remove the folder marker if any (a prefix without objects vanishes by itself).
            match self.delete_key(&prefix_of(path)) {
                Err(e) if e.kind == FsErrorKind::NotFound => Ok(()),
                r => r,
            }
        } else {
            self.delete_key(&key_of(path))
        }
    }

    fn copy_file(&self, from: &str, to: &str) -> FsResult<()> {
        self.copy_key(&key_of(from), &key_of(to))
    }

    fn uri_of(&self, path: &str) -> Option<RemoteUri> {
        Some(self.uri.with_path(path))
    }
}

struct S3Write {
    p: S3Provider,
    key: String,
    part_size: u64,
    buf: Vec<u8>,
    upload: Option<String>,
    /// ETags of uploaded parts (part n = index + 1).
    parts: Vec<String>,
    resumed: u64,
}

impl S3Write {
    fn ensure_upload(&mut self) -> FsResult<String> {
        if let Some(id) = &self.upload {
            return Ok(id.clone());
        }
        let url = self.p.bucket.create_multipart_upload(self.p.creds.as_ref(), &self.key).sign(SIGN_FOR);
        let text = send_text(self.p.client.post(url), self.p.timeout, "upload", &self.key)?;
        let r = rusty_s3::actions::CreateMultipartUpload::parse_response(&text)
            .map_err(|e| err(FsErrorKind::Io, "upload", &self.key, format!("bad server reply: {e}")))?;
        let id = r.upload_id().to_string();
        self.upload = Some(id.clone());
        Ok(id)
    }

    fn upload_part(&mut self, data: Vec<u8>) -> FsResult<()> {
        let id = self.ensure_upload()?;
        let n = (self.parts.len() + 1) as u16;
        let url = self.p.bucket.upload_part(self.p.creds.as_ref(), &self.key, n, &id).sign(SIGN_FOR);
        let resp = send(self.p.client.put(url).body(data), self.p.timeout * 5, "upload", &self.key)?;
        if let Some(e) = status_error(resp.status(), "upload", &self.key, "") {
            return Err(e);
        }
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| err(FsErrorKind::Io, "upload", &self.key, "the server sent no ETag"))?;
        self.parts.push(etag);
        Ok(())
    }

    fn io(e: FsError) -> std::io::Error {
        std::io::Error::other(e.message)
    }
}

impl Write for S3Write {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        while self.buf.len() as u64 >= self.part_size {
            let rest = self.buf.split_off(self.part_size as usize);
            let part = std::mem::replace(&mut self.buf, rest);
            self.upload_part(part).map_err(Self::io)?;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl WriteStream for S3Write {
    fn resumed_at(&self) -> u64 {
        self.resumed
    }

    fn commit(mut self: Box<Self>) -> FsResult<()> {
        if self.upload.is_none() {
            // Small object: one PUT.
            let url = self.p.bucket.put_object(self.p.creds.as_ref(), &self.key).sign(SIGN_FOR);
            let body = std::mem::take(&mut self.buf);
            return send_text(self.p.client.put(url).body(body), self.p.timeout * 5, "write", &self.key).map(|_| ());
        }
        if !self.buf.is_empty() || self.parts.is_empty() {
            let last = std::mem::take(&mut self.buf);
            self.upload_part(last)?;
        }
        let id = self.upload.clone().unwrap_or_default();
        let etags: Vec<&str> = self.parts.iter().map(String::as_str).collect();
        let action = self.p.bucket.complete_multipart_upload(self.p.creds.as_ref(), &self.key, &id, etags.into_iter());
        let url = action.sign(SIGN_FOR);
        let body = action.body();
        let text = send_text(self.p.client.post(url).body(body), self.p.timeout * 5, "write", &self.key)?;
        if text.contains("<Error>") {
            return Err(err(FsErrorKind::Io, "write", &self.key, "the server could not assemble the upload"));
        }
        Ok(())
    }

    fn abort(mut self: Box<Self>, keep_partial: bool) {
        if keep_partial {
            return; // the unfinished multipart upload is the resumable state
        }
        if let Some(id) = self.upload.take() {
            let url = self.p.bucket.abort_multipart_upload(self.p.creds.as_ref(), &self.key, &id).sign(SIGN_FOR);
            let _ = runtime().block_on(self.p.client.delete(url).send());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>photos</Name><Prefix>2024/</Prefix><KeyCount>4</KeyCount><MaxKeys>1000</MaxKeys>
  <Delimiter>/</Delimiter><IsTruncated>false</IsTruncated>
  <Contents><Key>2024/</Key><LastModified>2024-10-01T10:00:00.000Z</LastModified><ETag>"d41d8"</ETag><Size>0</Size><StorageClass>STANDARD</StorageClass></Contents>
  <Contents><Key>2024/beach.jpg</Key><LastModified>2024-10-02T08:30:15.000Z</LastModified><ETag>"abc"</ETag><Size>2048576</Size><StorageClass>STANDARD</StorageClass></Contents>
  <CommonPrefixes><Prefix>2024/raw/</Prefix></CommonPrefixes>
  <CommonPrefixes><Prefix>2024/edits/</Prefix></CommonPrefixes>
</ListBucketResult>"#;

    #[test]
    fn list_xml_to_entries() {
        let r = ListObjectsV2::parse_response(LIST).unwrap();
        let e = entries_from_list(&r, "2024/");
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e.iter().any(|x| x.name == "raw" && x.is_dir));
        assert!(e.iter().any(|x| x.name == "edits" && x.is_dir));
        let f = e.iter().find(|x| x.name == "beach.jpg").unwrap();
        assert_eq!(f.size, 2_048_576);
        let secs = f.modified.unwrap().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 1_727_857_815);
    }

    #[test]
    fn uploads_xml() {
        let xml = r#"<ListMultipartUploadsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
<Bucket>b</Bucket><Upload><Key>a/big.iso</Key><UploadId>U1</UploadId></Upload>
<Upload><Key>a/big.iso.bak</Key><UploadId>U2</UploadId></Upload>
<Upload><Key>a/big.iso</Key><UploadId>U3</UploadId></Upload></ListMultipartUploadsResult>"#;
        assert_eq!(parse_uploads(xml, "a/big.iso"), vec!["U1".to_string(), "U3".to_string()]);
    }

    #[test]
    fn keys_and_prefixes() {
        assert_eq!(key_of("/a/b"), "a/b");
        assert_eq!(prefix_of("/"), "");
        assert_eq!(prefix_of("/a"), "a/");
    }
}
