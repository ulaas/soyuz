//! Pure-Rust zstd stream reader (ruzstd): multiple frames, skippable frames, and content
//! checksum verification.

use std::io::{self, BufRead, Read};

use ruzstd::decoding::{FrameDecoder, StreamingDecoder};

enum State<R: BufRead> {
    Between(R),
    Frame(Box<StreamingDecoder<R, FrameDecoder>>),
    Done,
    Poisoned,
}

pub(crate) struct ZstdReader<R: BufRead> {
    state: State<R>,
}

impl<R: BufRead> ZstdReader<R> {
    pub fn new(inner: R) -> Self {
        ZstdReader { state: State::Between(inner) }
    }
}

fn invalid(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("zstd: {e}"))
}

impl<R: BufRead> Read for ZstdReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            match std::mem::replace(&mut self.state, State::Poisoned) {
                State::Done => {
                    self.state = State::Done;
                    return Ok(0);
                }
                State::Poisoned => return Err(invalid("stream is unusable after an error")),
                State::Between(mut r) => {
                    let head = r.fill_buf()?;
                    if head.is_empty() {
                        self.state = State::Done;
                        return Ok(0);
                    }
                    let mut magic = [0u8; 4];
                    let mut got = 0;
                    if head.len() >= 4 {
                        magic.copy_from_slice(&head[..4]);
                        got = 4;
                    }
                    if got == 4 && (0x184D_2A50..=0x184D_2A5F).contains(&u32::from_le_bytes(magic)) {
                        // Skippable frame: magic + u32 size + payload.
                        let mut hdr = [0u8; 8];
                        r.read_exact(&mut hdr)?;
                        let size = u64::from(u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]));
                        let skipped = io::copy(&mut (&mut r).take(size), &mut io::sink())?;
                        if skipped != size {
                            return Err(invalid("truncated skippable frame"));
                        }
                        self.state = State::Between(r);
                        continue;
                    }
                    let dec = StreamingDecoder::new(r).map_err(invalid)?;
                    self.state = State::Frame(Box::new(dec));
                }
                State::Frame(mut dec) => {
                    let n = dec.read(buf).map_err(invalid)?;
                    if n > 0 {
                        self.state = State::Frame(dec);
                        return Ok(n);
                    }
                    let fd = &dec.decoder;
                    if let (Some(stored), Some(computed)) = (fd.get_checksum_from_data(), fd.get_calculated_checksum())
                        && stored != computed
                    {
                        return Err(invalid("content checksum mismatch"));
                    }
                    self.state = State::Between(dec.into_inner());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_frame_and_skippable() {
        let a = ruzstd::encoding::compress_to_vec(&b"hello "[..], ruzstd::encoding::CompressionLevel::Fastest);
        let b = ruzstd::encoding::compress_to_vec(&b"world"[..], ruzstd::encoding::CompressionLevel::Fastest);
        let mut data = a.clone();
        data.extend_from_slice(&0x184D_2A50u32.to_le_bytes());
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(b"xyz");
        data.extend_from_slice(&b);
        let mut out = String::new();
        ZstdReader::new(&data[..]).read_to_string(&mut out).unwrap();
        assert_eq!(out, "hello world");

        let mut bad = a;
        let len = bad.len();
        bad[len - 1] ^= 0xFF; // checksum
        let mut out = Vec::new();
        assert!(ZstdReader::new(&bad[..]).read_to_end(&mut out).is_err());
    }
}
