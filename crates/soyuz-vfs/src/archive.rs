//! Archives as a [`Provider`]: the inside of a zip / tar / 7z / RAR / ISO / CAB / deb / rpm
//! (anything `soyuz_archive` reads) as a read-only location, so transfers copy out of an
//! archive like out of a server (`registry::resolve` maps `a.zip!/x/y` here).
//!
//! An archive that is not a plain local file (inside another archive, `a.zip!/b.tar`, or
//! on a server, `sftp://h/c.7z`) is first fetched to a private temp copy ([`local_file`],
//! at most [`configure`]'s size limit), so nested and remote archives browse, preview and
//! extract like local ones. Passwords typed for an archive this session come from
//! [`set_password`].

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use parking_lot::Mutex;
use soyuz_archive::{ArchiveEntry, ArchiveError, ArchiveReader, DirIndex};
use soyuz_core::location::{normalize_inner, split_vpath};
use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream};
use crate::uri::RemoteUri;

/// How large a fetched archive may be, and the folder it goes to.
#[derive(Clone, Copy, Debug)]
struct Conf {
    max_bytes: u64,
    dir: Option<fn() -> Result<PathBuf, String>>,
}

fn conf() -> &'static Mutex<Conf> {
    static C: OnceLock<Mutex<Conf>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Conf { max_bytes: 2 << 30, dir: None }))
}

/// Fetch nested / remote archives at most `max_bytes` each (0: never), into the folder
/// `dir` names (the app's private session folder; by default a fresh private folder
/// under the system temp folder).
pub fn configure(max_bytes: u64, dir: Option<fn() -> Result<PathBuf, String>>) {
    *conf().lock() = Conf { max_bytes, dir };
}

/// A folder of our own under the system temp folder (random name, created fresh, so
/// nothing planted in a shared temp folder is reused).
fn own_dir() -> Result<PathBuf, String> {
    static D: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    D.get_or_init(|| {
        let mut last = String::new();
        for _ in 0..8 {
            let mut r = [0u8; 8];
            getrandom::fill(&mut r).map_err(|e| e.to_string())?;
            let dir = std::env::temp_dir().join(format!("soyuz-nested-{:016x}", u64::from_le_bytes(r)));
            #[cfg(unix)]
            let made = {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new().mode(0o700).create(&dir)
            };
            #[cfg(not(unix))]
            let made = std::fs::create_dir(&dir);
            match made {
                Ok(()) => return Ok(dir),
                Err(e) => last = e.to_string(),
            }
        }
        Err(last)
    })
    .clone()
}

fn passwords() -> &'static Mutex<HashMap<PathBuf, String>> {
    static P: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// The password typed for `archive` (its vpath / URI / local path) this session.
pub fn set_password(archive: &Path, password: &str) {
    passwords().lock().insert(archive.to_path_buf(), password.to_string());
    providers().lock().retain(|k, _| k.0 != archive);
}

fn password_for(archive: &Path) -> Option<String> {
    passwords().lock().get(archive).cloned()
}

/// `archive` is a plain local file (not inside an archive, not on a server).
pub fn is_local(archive: &Path) -> bool {
    split_vpath(archive).is_none() && RemoteUri::from_path(archive).is_none()
}

fn not_found(op: &'static str, path: &str) -> FsError {
    FsError::new(FsErrorKind::NotFound, op, Some(Path::new(path)))
}

fn arch_err(op: &'static str, archive: &Path, e: ArchiveError) -> FsError {
    match e {
        ArchiveError::Io(e) => e,
        ArchiveError::Cancelled => FsError::new(FsErrorKind::Interrupted, op, Some(archive)),
        ArchiveError::PasswordRequired => FsError::new(FsErrorKind::PasswordRequired, op, Some(archive)),
        ArchiveError::WrongPassword => FsError::new(FsErrorKind::WrongPassword, op, Some(archive)),
        ArchiveError::Unsupported(_) => {
            FsError::with_message(FsErrorKind::Unsupported, op, Some(archive), e.to_string())
        }
        e => FsError::with_message(FsErrorKind::Corrupt, op, Some(archive), e.to_string()),
    }
}

/// A local file holding `archive`: the path itself when it is one, else a temp copy
/// fetched through its provider (kept for the session, fetched again when the source's
/// size or time changes).
pub fn local_file(archive: &Path) -> FsResult<PathBuf> {
    if is_local(archive) {
        return Ok(archive.to_path_buf());
    }
    let (prov, path) = crate::registry::resolve(archive)?;
    let st = prov.stat(&path)?;
    if st.is_dir {
        return Err(FsError::new(FsErrorKind::IsDirectory, "archive", Some(archive)));
    }
    let Conf { max_bytes, dir } = *conf().lock();
    if max_bytes == 0 {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "archive",
            Some(archive),
            "browsing archives inside archives or on servers is off ([archive] nested_max_mib)",
        ));
    }
    if st.size > max_bytes {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "archive",
            Some(archive),
            format!("larger than the {} MiB limit for archives inside archives or on servers", max_bytes >> 20),
        ));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    archive.hash(&mut h);
    st.size.hash(&mut h);
    st.modified.hash(&mut h);
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // The fetched copy keeps the archive's name (its extension picks the format).
    let safe: String =
        name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    let dir = dir.map_or_else(own_dir, |f| f()).map_err(|e| {
        FsError::with_message(FsErrorKind::Io, "archive", Some(archive), format!("no temp folder: {e}"))
    })?;
    let sub = dir.join(format!("{:016x}", h.finish()));
    let dest = sub.join(if safe.trim_matches('.').is_empty() { "archive".to_string() } else { safe });
    if dest.is_file() {
        return Ok(dest);
    }
    std::fs::create_dir_all(&sub).map_err(|e| FsError::from_io("archive", Some(&sub), &e))?;
    let part = sub.join(".fetch.part");
    let res = (|| -> std::io::Result<()> {
        let rd = prov.open_read(&path, 0).map_err(std::io::Error::other)?;
        let mut out = std::fs::File::create(&part)?;
        if std::io::copy(&mut rd.take(max_bytes + 1), &mut out)? > max_bytes {
            return Err(std::io::Error::other("the archive grew past the size limit while it was fetched"));
        }
        out.sync_all()?;
        drop(out);
        std::fs::rename(&part, &dest)
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&part);
        return Err(FsError::from_io("archive", Some(archive), &e));
    }
    Ok(dest)
}

/// Open `archive` (local, nested or remote) with the password typed for it, if any.
pub fn open(archive: &Path, password: Option<&str>) -> Result<Box<dyn ArchiveReader>, ArchiveError> {
    let local = local_file(archive).map_err(ArchiveError::Io)?;
    let pw = password.map(str::to_string).or_else(|| password_for(archive));
    soyuz_archive::open(&local, pw.as_deref())
}

type Key = (PathBuf, Option<SystemTime>);

fn providers() -> &'static Mutex<HashMap<Key, Arc<ArchiveProvider>>> {
    static P: OnceLock<Mutex<HashMap<Key, Arc<ArchiveProvider>>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// The provider of `archive` (cached while the archive is unchanged).
pub fn provider(archive: &Path) -> FsResult<Arc<ArchiveProvider>> {
    let local = local_file(archive)?;
    let mtime = std::fs::metadata(&local).and_then(|m| m.modified()).ok();
    let key = (archive.to_path_buf(), mtime);
    if let Some(p) = providers().lock().get(&key) {
        return Ok(p.clone());
    }
    let pw = password_for(archive);
    let mut r = soyuz_archive::open(&local, pw.as_deref()).map_err(|e| arch_err("open", archive, e))?;
    let entries = r.entries().map_err(|e| arch_err("list", archive, e))?.to_vec();
    let index = DirIndex::build(&entries);
    let p = Arc::new(ArchiveProvider { archive: archive.to_path_buf(), local, password: pw, entries, index });
    let mut map = providers().lock();
    map.retain(|k, _| k.0 != archive);
    if map.len() >= 8 {
        map.clear();
    }
    map.insert(key, p.clone());
    Ok(p)
}

/// The read-only inside of one archive.
pub struct ArchiveProvider {
    archive: PathBuf,
    local: PathBuf,
    password: Option<String>,
    entries: Vec<ArchiveEntry>,
    index: DirIndex,
}

impl ArchiveProvider {
    fn entry(&self, name: String, e: Option<&ArchiveEntry>, is_dir: bool) -> RemoteEntry {
        RemoteEntry {
            name,
            is_dir,
            is_link: e.is_some_and(|e| e.is_symlink),
            size: if is_dir { 0 } else { e.map_or(0, |e| e.size) },
            modified: e.and_then(|e| e.modified),
            mode: e.and_then(|e| e.mode),
        }
    }

    fn read_only(&self, op: &'static str, path: &str) -> FsError {
        FsError::with_message(
            FsErrorKind::Unsupported,
            op,
            Some(&soyuz_core::join_vpath(&self.archive, path)),
            "archives open read-only here",
        )
    }
}

impl Provider for ArchiveProvider {
    fn describe(&self) -> String {
        format!("Archive {}", soyuz_core::entry::display_name(&self.archive))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            range_read: false,
            resume_write: false,
            atomic_write: false,
            rename: false,
            rename_replaces: false,
            set_times: false,
            watch: false,
        }
    }

    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
        let kids = self.index.children(path).ok_or_else(|| not_found("list", path))?;
        Ok(kids
            .iter()
            .map(|c| self.entry(c.name.clone(), c.entry.and_then(|i| self.entries.get(i)), c.is_dir))
            .collect())
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let inner = normalize_inner(path);
        let name = inner.rsplit('/').next().unwrap_or("").to_string();
        if inner.is_empty() || self.index.is_dir(&inner) {
            let e = self.index.find(&inner).and_then(|i| self.entries.get(i));
            return Ok(self.entry(name, e, true));
        }
        let i = self.index.find(&inner).ok_or_else(|| not_found("stat", path))?;
        Ok(self.entry(name, self.entries.get(i), false))
    }

    /// A worker thread decompresses the entry into a bounded pipe (any size streams;
    /// a solid 7z block may decode earlier entries first).
    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        if offset != 0 {
            return Err(FsError::unsupported("open_read", Some(Path::new(path))));
        }
        let idx = self.index.find(path).filter(|_| !self.index.is_dir(path)).ok_or_else(|| not_found("open", path))?;
        let (local, pw, archive) = (self.local.clone(), self.password.clone(), self.archive.clone());
        let (tx, rx) = sync_channel::<std::io::Result<Vec<u8>>>(4);
        std::thread::Builder::new()
            .name("soyuz-archive-read".into())
            .spawn(move || pump(&local, pw.as_deref(), idx, &tx, &archive))
            .map_err(|e| FsError::from_io("open", Some(Path::new(path)), &e))?;
        Ok(Box::new(Pipe { rx, buf: Vec::new(), pos: 0 }))
    }

    fn create_write(&self, path: &str, _opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        Err(self.read_only("write", path))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        Err(self.read_only("mkdir", path))
    }

    fn rename(&self, from: &str, _to: &str) -> FsResult<()> {
        Err(self.read_only("rename", from))
    }

    fn delete(&self, path: &str, _is_dir: bool) -> FsResult<()> {
        Err(self.read_only("delete", path))
    }
}

fn pump(local: &Path, pw: Option<&str>, idx: usize, tx: &SyncSender<std::io::Result<Vec<u8>>>, archive: &Path) {
    let res = (|| -> std::io::Result<()> {
        let mut r = soyuz_archive::open(local, pw).map_err(|e| std::io::Error::other(arch_err("open", archive, e)))?;
        let mut rd = r.open_entry(idx).map_err(|e| std::io::Error::other(arch_err("open", archive, e)))?;
        let mut chunk = vec![0u8; 256 * 1024];
        loop {
            let n = match rd.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            // The reader went away (cancelled transfer): stop decompressing.
            if tx.send(Ok(chunk[..n].to_vec())).is_err() {
                return Ok(());
            }
        }
    })();
    if let Err(e) = res {
        let _ = tx.send(Err(e));
    }
}

/// The reading end of [`pump`]'s pipe.
struct Pipe {
    rx: Receiver<std::io::Result<Vec<u8>>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for Pipe {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(Ok(b)) => {
                    self.buf = b;
                    self.pos = 0;
                }
                Ok(Err(e)) => return Err(e),
                // The worker finished.
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use soyuz_archive::{ArchiveFormat, CreateOptions};
    use soyuz_core::OpControl;

    use super::*;

    fn pack(format: ArchiveFormat, dest: &Path, base: &Path, inputs: &[PathBuf]) {
        let opts = CreateOptions {
            format,
            level: 6,
            password: None,
            base_dir: base.to_path_buf(),
            store_symlinks: true,
            follow_symlinks: false,
        };
        soyuz_archive::create(dest, inputs, &opts, &OpControl::new(), &mut ()).unwrap();
    }

    fn read_all(path: &Path) -> Vec<u8> {
        let (prov, p) = crate::registry::resolve(path).unwrap();
        let mut out = Vec::new();
        prov.open_read(&p, 0).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn lists_reads_and_opens_nested_archives() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // inner.tar holds deep/x.txt; outer.zip holds a.txt, sub/big.bin and inner.tar.
        std::fs::create_dir_all(d.join("t/deep")).unwrap();
        std::fs::write(d.join("t/deep/x.txt"), b"nested!").unwrap();
        std::fs::create_dir_all(d.join("z/sub")).unwrap();
        pack(ArchiveFormat::Tar, &d.join("z/inner.tar"), &d.join("t"), &[d.join("t/deep")]);
        std::fs::write(d.join("z/a.txt"), b"alpha").unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(d.join("z/sub/big.bin"), &big).unwrap();
        let outer = d.join("outer.zip");
        pack(ArchiveFormat::Zip, &outer, &d.join("z"), &[d.join("z/a.txt"), d.join("z/sub"), d.join("z/inner.tar")]);
        configure(1 << 30, None);

        let p = provider(&outer).unwrap();
        let mut names: Vec<(String, bool)> = p.list("").unwrap().into_iter().map(|e| (e.name, e.is_dir)).collect();
        names.sort();
        assert_eq!(names, [("a.txt".into(), false), ("inner.tar".into(), false), ("sub".into(), true)]);
        assert!(p.stat("sub").unwrap().is_dir);
        assert_eq!(p.stat("sub/big.bin").unwrap().size, big.len() as u64);
        assert!(p.stat("nope").is_err());
        assert!(p.create_write("new.txt", &WriteOpts::default()).is_err(), "read-only");

        // Entries stream out (larger than the pipe's buffers), through the registry too.
        assert_eq!(read_all(&soyuz_core::join_vpath(&outer, "sub/big.bin")), big);
        assert_eq!(read_all(&soyuz_core::join_vpath(&outer, "a.txt")), b"alpha");

        // An archive inside the archive: fetched to a temp copy, then read like any other.
        let inner = soyuz_core::join_vpath(&outer, "inner.tar");
        assert!(!is_local(&inner));
        let local = local_file(&inner).unwrap();
        assert!(local.is_file() && local.file_name().unwrap() == "inner.tar");
        assert_eq!(local_file(&inner).unwrap(), local, "fetched once");
        let deep = soyuz_core::join_vpath(&inner, "deep/x.txt");
        assert_eq!(soyuz_core::location::split_vpath(&deep), Some((inner.clone(), "deep/x.txt".into())));
        assert_eq!(read_all(&deep), b"nested!");
        let mut r = open(&inner, None).unwrap();
        assert!(r.entries().unwrap().iter().any(|e| e.path == "deep/x.txt"));

        // Fetching can be switched off.
        configure(0, None);
        providers().lock().clear();
        let other = soyuz_core::join_vpath(&outer, "sub/big.bin");
        assert_eq!(local_file(&other).unwrap_err().kind, FsErrorKind::Unsupported);
        configure(1 << 30, None);
    }
}
