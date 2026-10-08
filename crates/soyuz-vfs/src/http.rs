//! Shared HTTP plumbing for WebDAV and S3: the reqwest client, streaming response
//! bodies as `std::io::Read`, streaming request bodies as a [`WriteStream`], and
//! status → error mapping.

use std::io::{self, Read, Write};
use std::time::Duration;

use reqwest::{Response, StatusCode};
use soyuz_core::{FsError, FsErrorKind, FsResult};
use tokio::sync::mpsc;

use crate::rt::runtime;

pub(crate) fn client(connect_timeout: Duration) -> FsResult<reqwest::Client> {
    crate::rt::ensure_crypto_provider();
    reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .user_agent(concat!("Soyuz/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| FsError::with_message(FsErrorKind::Io, "connect", None, e.to_string()))
}

/// Map an HTTP status to an error kind (`None` = success).
pub(crate) fn status_error(status: StatusCode, op: &'static str, path: &str, body: &str) -> Option<FsError> {
    if status.is_success() {
        return None;
    }
    let (kind, msg) = match status.as_u16() {
        401 => (FsErrorKind::PermissionDenied, "sign-in failed (check the user name and password)".to_string()),
        403 => (FsErrorKind::PermissionDenied, "the server refused access".to_string()),
        404 | 410 => (FsErrorKind::NotFound, "not found".to_string()),
        405 if op == "mkdir" => (FsErrorKind::AlreadyExists, "already exists".to_string()),
        409 => (FsErrorKind::NotFound, "the parent folder does not exist".to_string()),
        412 => (FsErrorKind::AlreadyExists, "already exists".to_string()),
        413 | 507 => (FsErrorKind::NoSpace, "not enough space on the server".to_string()),
        423 => (FsErrorKind::InUse, "locked on the server".to_string()),
        _ => (FsErrorKind::Io, format!("server error {status}")),
    };
    let msg = match short_reason(body) {
        Some(r) if kind == FsErrorKind::Io => format!("{msg}: {r}"),
        _ => msg,
    };
    Some(FsError::with_message(kind, op, Some(std::path::Path::new(path)), msg))
}

/// `<Message>…</Message>` (S3) or the first line of a short text body.
fn short_reason(body: &str) -> Option<String> {
    if let (Some(a), Some(b)) = (body.find("<Message>"), body.find("</Message>"))
        && a < b
    {
        return Some(body[a + 9..b].to_string());
    }
    let t = body.trim();
    (!t.is_empty() && t.len() < 160 && !t.starts_with('<')).then(|| t.lines().next().unwrap_or("").to_string())
}

pub(crate) fn net_error(op: &'static str, path: &str, e: &reqwest::Error) -> FsError {
    let msg = if e.is_timeout() {
        "the server did not answer in time".to_string()
    } else if e.is_connect() {
        format!("cannot connect: {}", root_cause(e))
    } else {
        root_cause(e)
    };
    FsError::with_message(FsErrorKind::Io, op, Some(std::path::Path::new(path)), msg)
}

fn root_cause(e: &dyn std::error::Error) -> String {
    let mut cur = e;
    while let Some(s) = cur.source() {
        cur = s;
    }
    cur.to_string()
}

/// Send a request on the runtime and wait (with timeout) for the response headers.
pub(crate) fn send(rb: reqwest::RequestBuilder, timeout: Duration, op: &'static str, path: &str) -> FsResult<Response> {
    runtime().block_on(async move {
        match tokio::time::timeout(timeout, rb.send()).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(net_error(op, path, &e)),
            Err(_) => Err(FsError::with_message(
                FsErrorKind::Io,
                op,
                Some(std::path::Path::new(path)),
                "the server did not answer in time",
            )),
        }
    })
}

/// Send and require a success status; returns the body text.
pub(crate) fn send_text(
    rb: reqwest::RequestBuilder,
    timeout: Duration,
    op: &'static str,
    path: &str,
) -> FsResult<String> {
    let resp = send(rb, timeout, op, path)?;
    let status = resp.status();
    let text = runtime().block_on(async move { tokio::time::timeout(timeout, resp.text()).await });
    let text = match text {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(net_error(op, path, &e)),
        Err(_) => return Err(FsError::with_message(FsErrorKind::Io, op, None, "the server stopped sending data")),
    };
    match status_error(status, op, path, &text) {
        Some(e) => Err(e),
        None => Ok(text),
    }
}

/// A response body as `std::io::Read`; skips `skip` leading bytes (a server that
/// ignored a `Range` request).
pub(crate) struct BodyReader {
    resp: Response,
    buf: bytes_shim::Chunk,
    skip: u64,
    timeout: Duration,
}

/// Tiny owned-chunk cursor (avoids depending on the `bytes` crate directly).
mod bytes_shim {
    #[derive(Default)]
    pub struct Chunk {
        pub data: Vec<u8>,
        pub pos: usize,
    }
}

impl BodyReader {
    pub fn new(resp: Response, skip: u64, timeout: Duration) -> Self {
        BodyReader { resp, buf: Default::default(), skip, timeout }
    }
}

impl Read for BodyReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.buf.pos < self.buf.data.len() {
                let avail = &self.buf.data[self.buf.pos..];
                if self.skip > 0 {
                    let s = (self.skip as usize).min(avail.len());
                    self.skip -= s as u64;
                    self.buf.pos += s;
                    continue;
                }
                let n = avail.len().min(out.len());
                out[..n].copy_from_slice(&avail[..n]);
                self.buf.pos += n;
                return Ok(n);
            }
            let t = self.timeout;
            let resp = &mut self.resp;
            let next = runtime().block_on(async move { tokio::time::timeout(t, resp.chunk()).await });
            match next {
                Ok(Ok(Some(c))) => self.buf = bytes_shim::Chunk { data: c.to_vec(), pos: 0 },
                Ok(Ok(None)) => return Ok(0),
                Ok(Err(e)) => return Err(io::Error::other(root_cause(&e))),
                Err(_) => return Err(io::Error::new(io::ErrorKind::TimedOut, "the server stopped sending data")),
            }
        }
    }
}

/// A streaming request body fed from `Write` calls on a worker thread. The request
/// itself runs on the runtime; `finish` waits for the response.
pub(crate) struct BodyWriter {
    tx: Option<mpsc::Sender<Result<Vec<u8>, io::Error>>>,
    task: Option<tokio::task::JoinHandle<Result<Response, reqwest::Error>>>,
    timeout: Duration,
}

impl BodyWriter {
    /// Start `rb` with a streaming body.
    pub fn start(rb: reqwest::RequestBuilder, timeout: Duration) -> Self {
        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, io::Error>>(8);
        let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });
        let body = reqwest::Body::wrap_stream(stream);
        let task = runtime().spawn(rb.body(body).send());
        BodyWriter { tx: Some(tx), task: Some(task), timeout }
    }

    /// Close the body and wait for the response.
    pub fn finish(mut self) -> io::Result<Response> {
        drop(self.tx.take());
        self.wait()
    }

    fn wait(&mut self) -> io::Result<Response> {
        let Some(task) = self.task.take() else { return Err(io::Error::other("request already finished")) };
        let t = self.timeout;
        match runtime().block_on(async move { tokio::time::timeout(t, task).await }) {
            Ok(Ok(Ok(r))) => Ok(r),
            Ok(Ok(Err(e))) => Err(io::Error::other(root_cause(&e))),
            Ok(Err(e)) => Err(io::Error::other(e.to_string())),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "the server did not answer in time")),
        }
    }

    /// Fail the body (the server sees a broken upload) and drop the request.
    pub fn abort(mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.blocking_send(Err(io::Error::other("upload cancelled")));
        }
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

impl Write for BodyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(tx) = self.tx.as_ref() else { return Err(io::Error::other("closed")) };
        if tx.blocking_send(Ok(buf.to_vec())).is_err() {
            // The request ended early (e.g. 401 before reading the body): report why.
            drop(self.tx.take());
            return match self.wait() {
                Ok(r) => Err(io::Error::other(format!("the server stopped the upload ({})", r.status()))),
                Err(e) => Err(e),
            };
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
