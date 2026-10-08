//! Byte-level progress reporting and hash algorithm vocabulary.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// One progress report for the current item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ByteProgress {
    /// Bytes processed so far in this item.
    pub done: u64,
    /// Total bytes of this item (0 if unknown).
    pub total: u64,
    /// Bytes since the previous call.
    pub delta: u64,
}

/// Receives byte progress from `fs::copy_file`, `fs::hash_file`, archive create/extract.
pub trait ProgressSink: Send {
    fn on_progress(&mut self, p: ByteProgress) {
        let _ = p;
    }
}

/// No-op sink for tests and callers that don't care.
impl ProgressSink for () {}

impl<F: FnMut(ByteProgress) + Send> ProgressSink for F {
    fn on_progress(&mut self, p: ByteProgress) {
        self(p)
    }
}

/// Checksum algorithms offered by the app and computed by `soyuz-fs`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashAlgo {
    Blake3,
    Sha256,
    Sha1,
    Md5,
    Crc32,
}

impl HashAlgo {
    pub const ALL: [HashAlgo; 5] = [HashAlgo::Blake3, HashAlgo::Sha256, HashAlgo::Sha1, HashAlgo::Md5, HashAlgo::Crc32];

    /// Display label, e.g. `"SHA-256"`.
    pub fn label(self) -> &'static str {
        match self {
            HashAlgo::Blake3 => "BLAKE3",
            HashAlgo::Sha256 => "SHA-256",
            HashAlgo::Sha1 => "SHA-1",
            HashAlgo::Md5 => "MD5",
            HashAlgo::Crc32 => "CRC-32",
        }
    }
}

impl fmt::Display for HashAlgo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::enum_str::to_str(self))
    }
}

impl FromStr for HashAlgo {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::enum_str::from_str(s).ok_or_else(|| format!("unknown hash algorithm: {s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(sink: &mut dyn ProgressSink) {
        sink.on_progress(ByteProgress { done: 5, total: 10, delta: 5 });
    }

    #[test]
    fn sinks() {
        drive(&mut ());
        let mut seen = 0;
        drive(&mut |p: ByteProgress| seen += p.delta);
        assert_eq!(seen, 5);
    }

    #[test]
    fn algo_strings() {
        for a in HashAlgo::ALL {
            assert_eq!(a.to_string().parse::<HashAlgo>().unwrap(), a);
        }
        assert_eq!(HashAlgo::Sha256.to_string(), "sha256");
    }
}
