//! The crate's tokio runtime (network providers are async inside, sync outside) and
//! adapters from async streams to `std::io` streams.

use std::io::{self, Read, Write};
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use soyuz_core::{FsError, FsErrorKind, FsResult};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("soyuz-vfs-rt")
            .enable_all()
            .build()
            .expect("tokio runtime for soyuz-vfs")
    })
}

/// Run `f` to completion from a (non-runtime) worker thread, with a timeout.
pub(crate) fn block_on<T>(timeout: Duration, f: impl Future<Output = FsResult<T>>) -> FsResult<T> {
    runtime().block_on(async move {
        match tokio::time::timeout(timeout, f).await {
            Ok(r) => r,
            Err(_) => Err(FsError::with_message(FsErrorKind::Io, "network", None, "the server did not answer in time")),
        }
    })
}

/// Install rustls' ring crypto provider (HTTPS via reqwest, FTPS via suppaftp; both
/// verify certificates with the OS verifier).
pub(crate) fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// `std::io::Read` over an async reader (one `block_on` per call).
pub(crate) struct BlockingRead<R> {
    pub inner: Pin<Box<R>>,
    pub timeout: Duration,
}

impl<R: AsyncRead + Send> Read for BlockingRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let t = self.timeout;
        let inner = &mut self.inner;
        runtime().block_on(async move {
            match tokio::time::timeout(t, inner.read(buf)).await {
                Ok(r) => r,
                Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "the server stopped sending data")),
            }
        })
    }
}

/// `std::io::Write` over an async writer.
pub(crate) struct BlockingWrite<W> {
    pub inner: Pin<Box<W>>,
    pub timeout: Duration,
}

impl<W: AsyncWrite + Send> Write for BlockingWrite<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let t = self.timeout;
        let inner = &mut self.inner;
        runtime().block_on(async move {
            match tokio::time::timeout(t, inner.write(buf)).await {
                Ok(r) => r,
                Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "the server stopped accepting data")),
            }
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        let inner = &mut self.inner;
        runtime().block_on(async move { inner.flush().await })
    }
}

impl<W: AsyncWrite + Send> BlockingWrite<W> {
    pub fn shutdown(&mut self) -> io::Result<()> {
        let inner = &mut self.inner;
        runtime().block_on(async move { inner.shutdown().await })
    }
}
