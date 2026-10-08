//! An in-memory [`Provider`] for conformance tests and the `SOYUZ_FAKE_REMOTE` debug
//! hook (browse and copy without a server). It supports resumable writes and range
//! reads, and can inject a failure after N transferred bytes (a dropped connection)
//! and a per-chunk delay (to test pause / cancel).

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use soyuz_core::{FsErrorKind, FsResult};

use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream, err};
use crate::uri::{RemoteUri, Scheme, join_path, name_of, normalize_path, parent_path};

#[derive(Clone, Debug)]
enum Node {
    Dir { modified: SystemTime },
    File { data: Vec<u8>, modified: SystemTime },
}

#[derive(Default)]
struct Faults {
    /// Fail the next read / write once this many bytes moved (then clear).
    fail_after: Option<u64>,
    moved: u64,
    delay: Option<Duration>,
}

pub struct FakeProvider {
    name: String,
    nodes: Arc<Mutex<BTreeMap<String, Node>>>,
    faults: Arc<Mutex<Faults>>,
    caps: Mutex<Capabilities>,
    /// Bytes written through `create_write` (resume tests check what was re-sent).
    pub bytes_written: Arc<AtomicU64>,
}

impl FakeProvider {
    pub fn new(name: &str) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert("/".to_string(), Node::Dir { modified: SystemTime::now() });
        FakeProvider {
            name: name.to_string(),
            nodes: Arc::new(Mutex::new(nodes)),
            faults: Arc::new(Mutex::new(Faults::default())),
            caps: Mutex::new(Capabilities {
                range_read: true,
                resume_write: true,
                atomic_write: false,
                rename: true,
                rename_replaces: false,
                set_times: true,
                watch: false,
            }),
            bytes_written: Arc::new(AtomicU64::new(0)),
        }
    }

    /// `fake://<name>/` for this provider.
    pub fn uri(&self) -> RemoteUri {
        RemoteUri {
            scheme: Scheme::Fake,
            user: None,
            host: self.name.clone(),
            port: None,
            path: "/".into(),
            query: Vec::new(),
        }
    }

    /// Register it with the registry under `fake://<name>`.
    pub fn install(self) -> Arc<FakeProvider> {
        let me = Arc::new(self);
        crate::registry::register(&me.uri(), me.clone());
        me
    }

    pub fn set_capabilities(&self, c: Capabilities) {
        *self.caps.lock() = c;
    }

    /// Fail the next transfer once `n` more bytes moved.
    pub fn fail_after(&self, n: u64) {
        let mut f = self.faults.lock();
        f.fail_after = Some(n);
        f.moved = 0;
    }

    pub fn set_delay(&self, d: Option<Duration>) {
        self.faults.lock().delay = d;
    }

    /// Add a file (parents are created).
    pub fn put(&self, path: &str, data: &[u8]) {
        let path = normalize_path(path);
        self.mkdirs(&parent_path(&path).unwrap_or_else(|| "/".into()));
        self.nodes.lock().insert(path, Node::File { data: data.to_vec(), modified: SystemTime::now() });
    }

    pub fn mkdirs(&self, path: &str) {
        let path = normalize_path(path);
        let mut cur = String::from("/");
        let mut nodes = self.nodes.lock();
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            cur = join_path(&cur, seg);
            nodes.entry(cur.clone()).or_insert(Node::Dir { modified: SystemTime::now() });
        }
    }

    /// Contents of a file.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        match self.nodes.lock().get(&normalize_path(path)) {
            Some(Node::File { data, .. }) => Some(data.clone()),
            _ => None,
        }
    }

    pub fn exists(&self, path: &str) -> bool {
        self.nodes.lock().contains_key(&normalize_path(path))
    }

    /// All paths (sorted), for assertions.
    pub fn paths(&self) -> Vec<String> {
        self.nodes.lock().keys().cloned().collect()
    }

    fn entry(path: &str, n: &Node) -> RemoteEntry {
        let name = name_of(path).to_string();
        match n {
            Node::Dir { modified } => RemoteEntry::dir(name, Some(*modified)),
            Node::File { data, modified } => RemoteEntry::file(name, data.len() as u64, Some(*modified)),
        }
    }

    fn parent_is_dir(&self, path: &str) -> bool {
        let parent = parent_path(path).unwrap_or_else(|| "/".into());
        matches!(self.nodes.lock().get(&parent), Some(Node::Dir { .. }))
    }
}

/// Advance the fault counter; `Err` when the injected failure triggers.
fn tick(faults: &Mutex<Faults>, n: u64) -> io::Result<()> {
    let delay = {
        let mut f = faults.lock();
        if let Some(limit) = f.fail_after {
            if f.moved + n > limit {
                f.fail_after = None;
                f.moved = 0;
                return Err(io::Error::new(io::ErrorKind::ConnectionReset, "connection reset (injected)"));
            }
            f.moved += n;
        }
        f.delay
    };
    if let Some(d) = delay {
        std::thread::sleep(d);
    }
    Ok(())
}

impl Provider for FakeProvider {
    fn describe(&self) -> String {
        format!("Test server {}", self.name)
    }

    fn capabilities(&self) -> Capabilities {
        *self.caps.lock()
    }

    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
        let path = normalize_path(path);
        let nodes = self.nodes.lock();
        match nodes.get(&path) {
            Some(Node::Dir { .. }) => {}
            Some(_) => return Err(err(FsErrorKind::NotDirectory, "list", &path, "not a folder")),
            None => return Err(err(FsErrorKind::NotFound, "list", &path, "no such folder")),
        }
        Ok(nodes
            .iter()
            .filter(|(p, _)| p.as_str() != "/" && parent_path(p).as_deref() == Some(path.as_str()))
            .map(|(p, n)| Self::entry(p, n))
            .collect())
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let path = normalize_path(path);
        match self.nodes.lock().get(&path) {
            Some(n) => Ok(Self::entry(&path, n)),
            None => Err(err(FsErrorKind::NotFound, "stat", &path, "not found")),
        }
    }

    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        let path = normalize_path(path);
        let data = match self.nodes.lock().get(&path) {
            Some(Node::File { data, .. }) => data.clone(),
            Some(_) => return Err(err(FsErrorKind::IsDirectory, "open", &path, "is a folder")),
            None => return Err(err(FsErrorKind::NotFound, "open", &path, "not found")),
        };
        let start = (offset as usize).min(data.len());
        Ok(Box::new(FakeRead { data, pos: start, faults: self.faults.clone() }))
    }

    fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        let path = normalize_path(path);
        if !self.parent_is_dir(&path) {
            return Err(err(FsErrorKind::NotFound, "create", &path, "parent folder missing"));
        }
        let resume = opts.resume && self.caps.lock().resume_write;
        let existing = match self.nodes.lock().get(&path) {
            Some(Node::Dir { .. }) => return Err(err(FsErrorKind::IsDirectory, "create", &path, "is a folder")),
            Some(Node::File { data, .. }) if resume => data.clone(),
            _ => Vec::new(),
        };
        let resumed = existing.len() as u64;
        // The file exists (possibly empty / partial) from the start, like a real server.
        self.nodes.lock().insert(path.clone(), Node::File { data: existing.clone(), modified: SystemTime::now() });
        Ok(Box::new(FakeWrite {
            nodes: self.nodes.clone(),
            path,
            buf: existing,
            resumed,
            faults: self.faults.clone(),
            counter: self.bytes_written.clone(),
        }))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        let path = normalize_path(path);
        if !self.parent_is_dir(&path) {
            return Err(err(FsErrorKind::NotFound, "mkdir", &path, "parent folder missing"));
        }
        let mut nodes = self.nodes.lock();
        if nodes.contains_key(&path) {
            return Err(err(FsErrorKind::AlreadyExists, "mkdir", &path, "already exists"));
        }
        nodes.insert(path, Node::Dir { modified: SystemTime::now() });
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        let (from, to) = (normalize_path(from), normalize_path(to));
        let mut nodes = self.nodes.lock();
        if !nodes.contains_key(&from) {
            return Err(err(FsErrorKind::NotFound, "rename", &from, "not found"));
        }
        if nodes.contains_key(&to) {
            return Err(err(FsErrorKind::AlreadyExists, "rename", &to, "already exists"));
        }
        let prefix = format!("{from}/");
        let moved: Vec<String> = nodes.keys().filter(|k| **k == from || k.starts_with(&prefix)).cloned().collect();
        for k in moved {
            if let Some(n) = nodes.remove(&k) {
                let nk = format!("{to}{}", &k[from.len()..]);
                nodes.insert(nk, n);
            }
        }
        Ok(())
    }

    fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
        let path = normalize_path(path);
        let mut nodes = self.nodes.lock();
        match nodes.get(&path) {
            None => return Err(err(FsErrorKind::NotFound, "delete", &path, "not found")),
            Some(Node::Dir { .. }) if !is_dir => {
                return Err(err(FsErrorKind::IsDirectory, "delete", &path, "is a folder"));
            }
            Some(Node::File { .. }) if is_dir => {
                return Err(err(FsErrorKind::NotDirectory, "delete", &path, "is a file"));
            }
            _ => {}
        }
        let prefix = format!("{path}/");
        if is_dir && nodes.keys().any(|k| k.starts_with(&prefix)) {
            return Err(err(FsErrorKind::NotEmpty, "delete", &path, "folder not empty"));
        }
        nodes.remove(&path);
        Ok(())
    }

    fn set_times(&self, path: &str, modified: SystemTime) -> FsResult<()> {
        let path = normalize_path(path);
        match self.nodes.lock().get_mut(&path) {
            Some(Node::File { modified: m, .. }) | Some(Node::Dir { modified: m }) => {
                *m = modified;
                Ok(())
            }
            None => Err(err(FsErrorKind::NotFound, "set_times", &path, "not found")),
        }
    }

    fn uri_of(&self, path: &str) -> Option<RemoteUri> {
        Some(self.uri().with_path(path))
    }
}

struct FakeRead {
    data: Vec<u8>,
    pos: usize,
    faults: Arc<Mutex<Faults>>,
}

impl Read for FakeRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len() - self.pos).min(64 * 1024);
        if n == 0 {
            return Ok(0);
        }
        tick(&self.faults, n as u64)?;
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct FakeWrite {
    nodes: Arc<Mutex<BTreeMap<String, Node>>>,
    path: String,
    buf: Vec<u8>,
    resumed: u64,
    faults: Arc<Mutex<Faults>>,
    counter: Arc<AtomicU64>,
}

impl FakeWrite {
    fn flush_to_store(&self) {
        self.nodes.lock().insert(self.path.clone(), Node::File { data: self.buf.clone(), modified: SystemTime::now() });
    }
}

impl Write for FakeWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        tick(&self.faults, buf.len() as u64)?;
        self.buf.extend_from_slice(buf);
        self.counter.fetch_add(buf.len() as u64, Ordering::Relaxed);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl WriteStream for FakeWrite {
    fn resumed_at(&self) -> u64 {
        self.resumed
    }
    fn commit(self: Box<Self>) -> FsResult<()> {
        self.flush_to_store();
        Ok(())
    }
    fn abort(self: Box<Self>, keep_partial: bool) {
        if keep_partial {
            self.flush_to_store();
        } else {
            self.nodes.lock().remove(&self.path);
        }
    }
}
