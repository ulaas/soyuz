//! The local filesystem as a [`Provider`] (native paths), so transfers treat both ends
//! alike. Bulk local↔local copies keep using the soyuz-fs copy engine; this provider is
//! only one end of a transfer that involves a remote location.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::SystemTime;

use soyuz_core::{FsError, FsResult};

use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream};

pub struct LocalProvider;

fn io(op: &'static str, path: &str, e: std::io::Error) -> FsError {
    FsError::from_io(op, Some(Path::new(path)), &e)
}

fn entry_of(name: String, md: &fs::Metadata, is_link: bool) -> RemoteEntry {
    RemoteEntry { name, is_dir: md.is_dir(), is_link, size: md.len(), modified: md.modified().ok(), mode: None }
}

impl Provider for LocalProvider {
    fn describe(&self) -> String {
        "Local disk".into()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            range_read: true,
            resume_write: true,
            atomic_write: false,
            rename: true,
            rename_replaces: true,
            set_times: true,
            watch: true,
        }
    }

    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
        let mut out = Vec::new();
        for e in fs::read_dir(path).map_err(|e| io("list", path, e))? {
            let e = e.map_err(|e| io("list", path, e))?;
            let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
            let md = match fs::metadata(e.path()) {
                Ok(m) => m,
                Err(_) => match e.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                },
            };
            out.push(entry_of(e.file_name().to_string_lossy().into_owned(), &md, is_link));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let md = fs::metadata(path).map_err(|e| io("stat", path, e))?;
        let is_link = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
        Ok(entry_of(crate::uri::name_of(path).to_string(), &md, is_link))
    }

    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        let mut f = File::open(path).map_err(|e| io("open", path, e))?;
        if offset > 0 {
            f.seek(SeekFrom::Start(offset)).map_err(|e| io("seek", path, e))?;
        }
        Ok(Box::new(f))
    }

    fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        let existing =
            if opts.resume { fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len()) } else { None };
        let file = match existing {
            Some(_) => OpenOptions::new().append(true).open(path),
            None => File::create(path),
        }
        .map_err(|e| io("create", path, e))?;
        Ok(Box::new(LocalWrite { file: Some(file), path: path.to_string(), resumed: existing.unwrap_or(0) }))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        fs::create_dir(path).map_err(|e| io("mkdir", path, e))
    }

    fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        fs::rename(from, to).map_err(|e| io("rename", from, e))
    }

    fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
        if is_dir { fs::remove_dir(path) } else { fs::remove_file(path) }.map_err(|e| io("delete", path, e))
    }

    fn set_times(&self, path: &str, modified: SystemTime) -> FsResult<()> {
        let f = OpenOptions::new().write(true).open(path).map_err(|e| io("set_times", path, e))?;
        f.set_modified(modified).map_err(|e| io("set_times", path, e))
    }
}

struct LocalWrite {
    file: Option<File>,
    path: String,
    resumed: u64,
}

impl Write for LocalWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.file.as_mut() {
            Some(f) => f.write(buf),
            None => Err(std::io::Error::other("closed")),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl WriteStream for LocalWrite {
    fn resumed_at(&self) -> u64 {
        self.resumed
    }

    fn commit(mut self: Box<Self>) -> FsResult<()> {
        if let Some(f) = self.file.take() {
            f.sync_all().map_err(|e| io("write", &self.path, e))?;
        }
        Ok(())
    }

    fn abort(mut self: Box<Self>, keep_partial: bool) {
        drop(self.file.take());
        if !keep_partial {
            let _ = fs::remove_file(&self.path);
        }
    }
}
