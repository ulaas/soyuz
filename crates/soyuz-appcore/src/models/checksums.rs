//! Checksums of one file, without drawing: MD5 / SHA-1 / SHA-256 / BLAKE3 / CRC-32
//! computed on worker threads with progress (`soyuz_fs::hash_file`, cancellable), and the
//! compare field's verdict for a pasted hash (or a `sha256sum` line).
//!
//! The GUI's Properties dialog draws it as its Checksums tab
//! (`ui::dialogs::properties_checksum`), the terminal UI as the checksum section of its
//! properties dialog (`soyuz-tui`, `input::dialogs::properties`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use soyuz_core::{ByteProgress, HashAlgo, OpControl};

use crate::host::Wake;

/// Display order.
pub const ALGOS: [HashAlgo; 5] = [HashAlgo::Md5, HashAlgo::Sha1, HashAlgo::Sha256, HashAlgo::Blake3, HashAlgo::Crc32];

/// Progress wake-ups at most this often.
const WAKE_EVERY: Duration = Duration::from_millis(80);

/// Where one algorithm is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HashState {
    Idle,
    Running { done: u64, total: u64 },
    Done(String),
    Failed(String),
}

impl HashState {
    /// Done, as a fraction (`0.0` while the size is unknown).
    pub fn fraction(&self) -> f32 {
        match self {
            HashState::Running { done, total } if *total > 0 => *done as f32 / *total as f32,
            _ => 0.0,
        }
    }
}

struct Row {
    algo: HashAlgo,
    state: Arc<Mutex<HashState>>,
    ctl: Option<Arc<OpControl>>,
}

/// The checksums of one file.
pub struct ChecksumSet {
    path: PathBuf,
    rows: Vec<Row>,
}

impl std::fmt::Debug for ChecksumSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChecksumSet").field("path", &self.path).finish_non_exhaustive()
    }
}

/// Lower-case hex without whitespace; a `<hash>  <file>` line keeps only the hash.
pub fn normalize_hash(s: &str) -> String {
    let first = s.split_whitespace().next().unwrap_or("");
    let first = first.trim_start_matches('\\');
    first.trim_start_matches("0x").to_ascii_lowercase()
}

/// Algorithms whose digest has this many hex digits.
pub fn candidates(hex_len: usize) -> Vec<HashAlgo> {
    match hex_len {
        8 => vec![HashAlgo::Crc32],
        32 => vec![HashAlgo::Md5],
        40 => vec![HashAlgo::Sha1],
        64 => vec![HashAlgo::Sha256, HashAlgo::Blake3],
        _ => Vec::new(),
    }
}

/// Outcome of comparing a pasted hash with the computed ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Empty,
    NotHex,
    UnknownLength,
    Pending,
    Match(HashAlgo),
    Mismatch,
}

impl Verdict {
    /// What the compare line says (`None`: nothing typed).
    pub fn message(&self) -> Option<String> {
        Some(match self {
            Verdict::Empty => return None,
            Verdict::NotHex => tr!("dialogs-hash-not-hex").into(),
            Verdict::UnknownLength => tr!("dialogs-hash-bad-length").into(),
            Verdict::Pending => tr!("dialogs-hash-computing").into(),
            Verdict::Match(a) => tr!("dialogs-hash-match", algo = a.label()),
            Verdict::Mismatch => tr!("dialogs-hash-mismatch").into(),
        })
    }
}

/// Compare pasted `input` with the `computed` digests.
pub fn verdict(input: &str, computed: &[(HashAlgo, Option<String>)]) -> Verdict {
    let h = normalize_hash(input);
    if h.is_empty() {
        return Verdict::Empty;
    }
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Verdict::NotHex;
    }
    let cands = candidates(h.len());
    if cands.is_empty() {
        return Verdict::UnknownLength;
    }
    let mut pending = false;
    for a in cands {
        match computed.iter().find(|(x, _)| *x == a).and_then(|(_, v)| v.as_ref()) {
            Some(v) if v.eq_ignore_ascii_case(&h) => return Verdict::Match(a),
            Some(_) => {}
            None => pending = true,
        }
    }
    if pending { Verdict::Pending } else { Verdict::Mismatch }
}

impl ChecksumSet {
    pub fn new(path: PathBuf) -> ChecksumSet {
        let rows =
            ALGOS.iter().map(|a| Row { algo: *a, state: Arc::new(Mutex::new(HashState::Idle)), ctl: None }).collect();
        ChecksumSet { path, rows }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Start hashing with `algo` (nothing when it runs or is done); `wake` is called on
    /// progress (at most every 80 ms) and when it ends.
    pub fn start(&mut self, algo: HashAlgo, wake: &Wake) {
        let Some(row) = self.rows.iter_mut().find(|r| r.algo == algo) else { return };
        if matches!(*row.state.lock(), HashState::Running { .. } | HashState::Done(_)) {
            return;
        }
        let ctl = OpControl::new();
        row.ctl = Some(ctl.clone());
        *row.state.lock() = HashState::Running { done: 0, total: 0 };
        let state = row.state.clone();
        let path = self.path.clone();
        let wake = wake.clone();
        let spawn = std::thread::Builder::new().name("soyuz-hash".into()).spawn(move || {
            let mut last = Instant::now();
            let progress_state = state.clone();
            let progress_wake = wake.clone();
            let mut sink = move |p: ByteProgress| {
                *progress_state.lock() = HashState::Running { done: p.done, total: p.total };
                if last.elapsed() > WAKE_EVERY {
                    last = Instant::now();
                    progress_wake();
                }
            };
            let r = soyuz_fs::hash_file(&path, algo, &ctl, &mut sink);
            *state.lock() = match r {
                Ok(h) => HashState::Done(h),
                Err(e) if e.is_cancel() => HashState::Idle,
                Err(e) => HashState::Failed(e.to_string()),
            };
            wake();
        });
        if spawn.is_err() {
            *row.state.lock() = HashState::Failed(tr!("dialogs-hash-no-thread").into());
        }
    }

    /// Start every algorithm that is idle or failed.
    pub fn start_all(&mut self, wake: &Wake) {
        for a in ALGOS {
            self.start(a, wake);
        }
    }

    /// Stop `algo` (it goes back to idle).
    pub fn cancel(&mut self, algo: HashAlgo) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.algo == algo)
            && let Some(c) = r.ctl.take()
        {
            c.cancel();
        }
    }

    pub fn cancel_all(&mut self) {
        for r in &mut self.rows {
            if let Some(c) = r.ctl.take() {
                c.cancel();
            }
        }
    }

    /// The state of `algo`.
    pub fn state(&self, algo: HashAlgo) -> HashState {
        self.rows.iter().find(|r| r.algo == algo).map_or(HashState::Idle, |r| r.state.lock().clone())
    }

    /// Some algorithm is idle or failed ("Compute all" applies).
    pub fn any_idle(&self) -> bool {
        self.rows.iter().any(|r| matches!(*r.state.lock(), HashState::Idle | HashState::Failed(_)))
    }

    /// Some algorithm is running.
    pub fn any_running(&self) -> bool {
        self.rows.iter().any(|r| matches!(*r.state.lock(), HashState::Running { .. }))
    }

    /// Every algorithm with its digest once done.
    pub fn computed(&self) -> Vec<(HashAlgo, Option<String>)> {
        self.rows
            .iter()
            .map(|r| {
                (
                    r.algo,
                    match &*r.state.lock() {
                        HashState::Done(h) => Some(h.clone()),
                        _ => None,
                    },
                )
            })
            .collect()
    }

    /// The verdict for `input`; while it is pending, the algorithms that could match are
    /// started.
    pub fn compare(&mut self, input: &str, wake: &Wake) -> Verdict {
        let v = verdict(input, &self.computed());
        if v == Verdict::Pending {
            for a in candidates(normalize_hash(input).len()) {
                self.start(a, wake);
            }
        }
        v
    }
}

impl Drop for ChecksumSet {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_verdict() {
        assert_eq!(normalize_hash("  ABCD  file.txt\n"), "abcd");
        assert_eq!(normalize_hash("0xDEADBEEF"), "deadbeef");
        assert_eq!(normalize_hash("\\abc  f"), "abc");
        let md5 = "d41d8cd98f00b204e9800998ecf8427e".to_string();
        let computed = vec![(HashAlgo::Md5, Some(md5.clone())), (HashAlgo::Sha256, None), (HashAlgo::Blake3, None)];
        assert_eq!(verdict("", &computed), Verdict::Empty);
        assert_eq!(verdict("xyz", &computed), Verdict::NotHex);
        assert_eq!(verdict("abc", &computed), Verdict::UnknownLength);
        assert_eq!(verdict(&md5.to_uppercase(), &computed), Verdict::Match(HashAlgo::Md5));
        assert_eq!(verdict("00000000000000000000000000000000", &computed), Verdict::Mismatch);
        assert_eq!(verdict(&"a".repeat(64), &computed), Verdict::Pending);
        assert_eq!(candidates(64), vec![HashAlgo::Sha256, HashAlgo::Blake3]);
        assert!(Verdict::Empty.message().is_none());
        assert!(Verdict::Mismatch.message().is_some());
    }

    #[test]
    fn computes_real_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.txt");
        std::fs::write(&f, b"abc").unwrap();
        let wake = crate::host::no_wake();
        let mut c = ChecksumSet::new(f);
        c.start(HashAlgo::Md5, &wake);
        // A pasted SHA-256 starts the matching algorithms.
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(c.compare(sha, &wake), Verdict::Pending);
        let deadline = Instant::now() + Duration::from_secs(10);
        while c.any_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let got = c.computed();
        assert_eq!(got[0].1.as_deref(), Some("900150983cd24fb0d6963f7d28e17f72"));
        assert_eq!(got[2].1.as_deref(), Some(sha));
        assert_eq!(c.compare(&sha.to_uppercase(), &wake), Verdict::Match(HashAlgo::Sha256));
        assert_eq!(c.state(HashAlgo::Sha1), HashState::Idle);
        assert!(c.any_idle());
    }
}
