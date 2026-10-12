//! The window side of the CLI companion: serves `\\.\pipe\soyuz-cli-<user>` (when
//! `[cli] server_enabled`), so `soyuz cp|mv|rm|trash|open|reveal|jobs` from a terminal
//! submit jobs here. Connection threads ask the UI thread to submit (so the job gets
//! the window's copy settings and a CLI badge in the activity panel), then follow the
//! job on the shared engine and stream its progress back.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_jobs::{JobEngine, JobId, JobSpec};
use soyuz_mcp::pipe::PipeServer;
use soyuz_script::cli::proto::{self, Host};

use crate::services::mailbox::Wake;

/// A request for the UI thread.
pub enum UiReq {
    Submit(Box<JobSpec>, Sender<Result<JobId, String>>),
    Open { path: PathBuf, reveal: bool, reply: Sender<Result<(), String>> },
}

struct PipeHost {
    engine: Arc<JobEngine>,
    tx: Sender<UiReq>,
    wake: Wake,
}

impl PipeHost {
    fn ask<T>(&self, make: impl FnOnce(Sender<Result<T, String>>) -> UiReq) -> Result<T, String> {
        let (reply, rx) = crossbeam_channel::bounded(1);
        self.tx.send(make(reply)).map_err(|_| "Soyuz is closing".to_string())?;
        (self.wake)();
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "the Soyuz window did not answer".to_string())?
    }
}

impl Host for PipeHost {
    fn engine(&self) -> Arc<JobEngine> {
        self.engine.clone()
    }

    fn submit(&self, spec: JobSpec) -> Result<JobId, String> {
        self.ask(|reply| UiReq::Submit(Box::new(spec), reply))
    }

    fn open(&self, path: &Path, reveal: bool) -> Result<(), String> {
        self.ask(|reply| UiReq::Open { path: path.to_path_buf(), reveal, reply })
    }
}

/// The running server (dropping it stops listening).
pub struct CliServer {
    _pipe: PipeServer,
    pub rx: Receiver<UiReq>,
    pub name: String,
}

impl CliServer {
    pub fn start(engine: Arc<JobEngine>, wake: Wake) -> std::io::Result<CliServer> {
        let (tx, rx) = unbounded();
        let host = Arc::new(PipeHost { engine, tx, wake });
        let name = proto::pipe_name();
        let pipe = PipeServer::serve(&name, move |stream| proto::serve_connection(host.as_ref(), stream))?;
        log::info!("CLI: serving {name}");
        Ok(CliServer { _pipe: pipe, rx, name })
    }
}
