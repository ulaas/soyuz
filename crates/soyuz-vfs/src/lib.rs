//! `soyuz-vfs`: remote and cloud locations.
//!
//! * [`provider::Provider`]: the backend trait (list, stat, streaming read / write,
//!   mkdir, rename, delete, set_times, capabilities). Implementations: SFTP
//!   ([`sftp`]), WebDAV ([`webdav`]), FTP / FTPS ([`ftp`]), S3-compatible ([`s3`]), the
//!   local filesystem ([`local`]), SMB 2 / 3 off Windows (`smb`) and an in-memory test
//!   server ([`fake`]).
//! * [`uri::RemoteUri`]: `sftp://user@host:port/path`, `webdav(s)://`, `ftp(s)://`,
//!   `s3://bucket/prefix?endpoint=…`, `smb://[domain;]user@server/share/path`.
//! * [`registry`]: one connection per server, connected lazily; the app installs
//!   [`registry::Hooks`] for credentials and host-key confirmation.
//! * [`transfer`]: streaming copies between any two providers with progress, cancel and
//!   resume of partial transfers.
//!
//! Everything is synchronous for callers (worker threads); network providers run on a
//! small internal tokio runtime.
#![forbid(unsafe_code)]

pub mod dav_server;
pub mod fake;
pub mod ftp;
mod http;
pub mod known_hosts;
pub mod local;
pub mod provider;
pub mod registry;
mod rt;
pub mod s3;
pub mod sftp;
#[cfg(not(windows))]
pub mod smb;
pub mod transfer;
pub mod uri;
pub mod webdav;

pub use provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream};
pub use registry::{ConnState, Credentials, Hooks, HostKeyPrompt};
pub use uri::{RemoteUri, Scheme};
