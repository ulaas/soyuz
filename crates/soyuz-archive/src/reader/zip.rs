//! zip reader (random access via the central directory).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use soyuz_core::{OpControl, ProgressSink};
use zip::ZipArchive;
use zip::result::ZipError;

use super::{Listing, drain_for_test, open_file, test_failure};
use crate::extract::{EntryData, Extractor};
use crate::util::{CHUNK, EntryExtra, local_to_system, normalize_name, read_err, unix_secs_to_time};
use crate::{
    ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, ExtractOptions, ExtractSink, ExtractSummary,
    TestReport,
};

type Zip = ZipArchive<BufReader<File>>;

pub(crate) struct ZipReader {
    path: PathBuf,
    zip: Zip,
    password: Option<String>,
    listing: Option<Listing>,
}

pub(crate) fn zip_err(e: ZipError) -> ArchiveError {
    match e {
        ZipError::InvalidPassword => ArchiveError::WrongPassword,
        ZipError::UnsupportedArchive(m) if m == ZipError::PASSWORD_REQUIRED => ArchiveError::PasswordRequired,
        ZipError::UnsupportedArchive(m) => ArchiveError::Corrupt(format!("unsupported zip feature: {m}")),
        ZipError::Io(e) => read_err(e, None, false),
        ZipError::FileNotFound => ArchiveError::Corrupt("entry not found".into()),
        other => ArchiveError::Corrupt(other.to_string()),
    }
}

impl ZipReader {
    pub fn open(path: &Path, password: Option<&str>) -> Result<Self, ArchiveError> {
        let zip = ZipArchive::new(open_file(path)?).map_err(|e| match e {
            ZipError::Io(io) => crate::util::fs_err("open_archive", path, &io),
            other => zip_err(other),
        })?;
        Ok(ZipReader { path: path.to_path_buf(), zip, password: password.map(str::to_string), listing: None })
    }

    fn load(&mut self) -> Result<&mut Listing, ArchiveError> {
        if self.listing.is_none() {
            let n = self.zip.len();
            let mut entries = Vec::with_capacity(n);
            let mut extras = Vec::with_capacity(n);
            let mut symlinks = Vec::new();
            for i in 0..n {
                let f = self.zip.by_index_raw(i).map_err(zip_err)?;
                let (path, raw) = normalize_name(f.name());
                if path.is_empty() {
                    continue;
                }
                let is_dir = f.is_dir();
                let modified = f
                    .extra_data_fields()
                    .find_map(|x| match x {
                        zip::extra_fields::ExtraField::ExtendedTimestamp(ts) => ts.mod_time(),
                        _ => None,
                    })
                    .and_then(|s| unix_secs_to_time(i64::from(s)))
                    .or_else(|| {
                        f.last_modified().and_then(|d| {
                            local_to_system(
                                i32::from(d.year()),
                                u32::from(d.month()),
                                u32::from(d.day()),
                                u32::from(d.hour()),
                                u32::from(d.minute()),
                                u32::from(d.second()),
                            )
                        })
                    });
                let is_symlink = f.is_symlink();
                if is_symlink && !f.encrypted() {
                    symlinks.push(entries.len());
                }
                entries.push(ArchiveEntry {
                    index: entries.len(),
                    path,
                    is_dir,
                    size: if is_dir { 0 } else { f.size() },
                    compressed: f.compressed_size(),
                    modified,
                    crc32: (!is_dir).then(|| f.crc32()),
                    encrypted: f.encrypted(),
                    mode: f.unix_mode(),
                    is_symlink,
                    link_target: None,
                });
                extras.push(EntryExtra { raw, ordinal: i, ..Default::default() });
            }
            // Symlink targets are the (tiny) entry contents.
            for idx in symlinks {
                let ordinal = extras[idx].ordinal;
                if let Ok(f) = self.zip.by_index(ordinal) {
                    let mut s = String::new();
                    if f.take(64 * 1024).read_to_string(&mut s).is_ok() {
                        entries[idx].link_target = Some(s);
                    }
                }
            }
            self.listing = Some(Listing { entries, extras, index: None });
        }
        Ok(self.listing.as_mut().expect("listing loaded"))
    }

    fn open_ordinal<'a>(
        zip: &'a mut Zip,
        ordinal: usize,
        encrypted: bool,
        password: Option<&str>,
    ) -> Result<zip::read::ZipFile<'a, BufReader<File>>, ArchiveError> {
        match (encrypted, password) {
            (true, None) => Err(ArchiveError::PasswordRequired),
            (true, Some(p)) => zip.by_index_decrypt(ordinal, p.as_bytes()).map_err(zip_err),
            (false, _) => zip.by_index(ordinal).map_err(zip_err),
        }
    }
}

struct ZipData<'a> {
    zip: &'a mut Zip,
    ordinal: usize,
    encrypted: bool,
    password: Option<&'a str>,
}

impl EntryData for ZipData<'_> {
    fn open(&mut self) -> Result<Box<dyn Read + '_>, ArchiveError> {
        Ok(Box::new(ZipReader::open_ordinal(self.zip, self.ordinal, self.encrypted, self.password)?))
    }
    fn can_reopen(&self) -> bool {
        true
    }
}

impl ArchiveReader for ZipReader {
    fn format(&self) -> ArchiveFormat {
        ArchiveFormat::Zip
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError> {
        Ok(&self.load()?.entries)
    }

    fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
        Ok(Some(self.load()?.index()))
    }

    fn open_entry(&mut self, index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        self.load()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let e = listing.entries.get(index).ok_or_else(|| ArchiveError::Corrupt(format!("no entry #{index}")))?;
        if e.is_dir {
            return Ok(Box::new(std::io::empty()));
        }
        let (ordinal, encrypted) = (listing.extras[index].ordinal, e.encrypted);
        let f = Self::open_ordinal(&mut self.zip, ordinal, encrypted, self.password.as_deref())?;
        Ok(Box::new(f))
    }

    fn extract(
        &mut self,
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &ExtractOptions,
        ctl: &OpControl,
        sink: &mut dyn ExtractSink,
    ) -> Result<ExtractSummary, ArchiveError> {
        self.load()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let zip = &mut self.zip;
        let password = self.password.as_deref();
        let mut ex = Extractor::new(&listing.entries, &listing.extras, selection, dest, opts, ctl, sink)?;
        // Fail fast on a missing / wrong password before writing anything.
        if let Some(i) = ex.wants_encrypted() {
            let pw = password.ok_or(ArchiveError::PasswordRequired)?;
            zip.by_index_decrypt(listing.extras[i].ordinal, pw.as_bytes()).map_err(zip_err)?;
        }
        for (i, e) in listing.entries.iter().enumerate() {
            if !ex.wants(i) {
                continue;
            }
            let mut data = ZipData { zip, ordinal: listing.extras[i].ordinal, encrypted: e.encrypted, password };
            ex.handle(i, &mut data)?;
        }
        Ok(ex.finish())
    }

    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
        self.load()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let mut report = TestReport::default();
        let mut buf = vec![0u8; CHUNK];
        for (i, e) in listing.entries.iter().enumerate() {
            ctl.checkpoint()?;
            report.entries += 1;
            if e.is_dir {
                continue;
            }
            let res =
                Self::open_ordinal(&mut self.zip, listing.extras[i].ordinal, e.encrypted, self.password.as_deref())
                    .and_then(|mut f| {
                        drain_for_test(&mut f, e.size, ctl, sink, &mut buf)
                            .map_err(|err| read_err(err, Some(ctl), e.encrypted))
                    });
            match res {
                Ok(n) if n != e.size => {
                    report.bad.push((e.path.clone(), format!("size mismatch: {n} != {}", e.size)));
                }
                Ok(_) => {}
                Err(err) => test_failure(&mut report, &e.path, err)?,
            }
        }
        Ok(report)
    }
}
