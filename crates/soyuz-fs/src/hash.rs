//! File checksums (streamed, pause/cancel aware, progress per MiB).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use soyuz_core::{ByteProgress, FsResult, HashAlgo, OpControl, ProgressSink};

use crate::err;

enum Hasher {
    Blake3(Box<blake3::Hasher>),
    Sha256(sha2::Sha256),
    Sha1(sha1::Sha1),
    Md5(md5::Md5),
    Crc32(crc32fast::Hasher),
}

impl Hasher {
    fn new(algo: HashAlgo) -> Self {
        use sha1::Digest as _;
        match algo {
            HashAlgo::Blake3 => Hasher::Blake3(Box::new(blake3::Hasher::new())),
            HashAlgo::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            HashAlgo::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            HashAlgo::Md5 => Hasher::Md5(<md5::Md5 as md5::Digest>::new()),
            HashAlgo::Crc32 => Hasher::Crc32(crc32fast::Hasher::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        use sha1::Digest as _;
        match self {
            Hasher::Blake3(x) => {
                x.update(data);
            }
            Hasher::Sha256(x) => x.update(data),
            Hasher::Sha1(x) => x.update(data),
            // md-5 is on digest 0.11, sha1 / sha2 on 0.10: name its own trait.
            Hasher::Md5(x) => md5::Digest::update(x, data),
            Hasher::Crc32(x) => x.update(data),
        }
    }

    fn finish(self) -> String {
        use sha1::Digest as _;
        match self {
            Hasher::Blake3(x) => x.finalize().to_hex().to_string(),
            Hasher::Sha256(x) => hex(&x.finalize()),
            Hasher::Sha1(x) => hex(&x.finalize()),
            Hasher::Md5(x) => hex(&md5::Digest::finalize(x)),
            Hasher::Crc32(x) => format!("{:08x}", x.finalize()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

fn open(path: &Path) -> std::io::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(crate::sys::windows::fs::SHARE_ALL)
            .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        let f = File::open(path)?;
        #[cfg(target_os = "linux")]
        let _ = rustix::fs::fadvise(&f, 0, None, rustix::fs::Advice::Sequential);
        Ok(f)
    }
}

/// Lower-case hex digest of the file contents (CRC-32 as 8 hex digits).
pub fn hash_file(path: &Path, algo: HashAlgo, ctl: &OpControl, sink: &mut dyn ProgressSink) -> FsResult<String> {
    let mut f = open(path).map_err(err::map("hash", path))?;
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut h = Hasher::new(algo);
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        ctl.checkpoint()?;
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(err::io("hash", path, e)),
        };
        h.update(&buf[..n]);
        done += n as u64;
        sink.on_progress(ByteProgress { done, total, delta: n as u64 });
    }
    Ok(h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_and_cancel() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("empty");
        std::fs::write(&f, b"").unwrap();
        let ctl = OpControl::new();
        assert_eq!(
            hash_file(&f, HashAlgo::Sha256, &ctl, &mut ()).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(hash_file(&f, HashAlgo::Crc32, &ctl, &mut ()).unwrap(), "00000000");
        ctl.cancel();
        assert!(hash_file(&f, HashAlgo::Md5, &ctl, &mut ()).unwrap_err().is_cancel());
        assert_eq!(hex(&[0x0f, 0xa0]), "0fa0");
    }
}
