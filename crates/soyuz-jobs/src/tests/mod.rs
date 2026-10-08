//! Engine tests (temp dirs, fake `FsProbe`, `throttle_bps` for deterministic timing).

mod concurrency;
mod ops;
mod prompts;
mod queue;
mod undo;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use soyuz_core::{DeviceId, FsError, FsResult};
use soyuz_fs::{FsProbe, TrashSupport};

use crate::{EngineConfig, JobEngine, JobEvent, JobId, JobKind, JobPrompt, JobSnapshot, JobSpec, JobState};

/// Probe with scripted free space, device ids (by path prefix) and trash support.
pub(crate) struct FakeProbe {
    pub space: Mutex<Option<(u64, u64)>>,
    pub devices: Mutex<Vec<(PathBuf, u64)>>,
    pub trash: Mutex<TrashSupport>,
}

impl Default for FakeProbe {
    fn default() -> Self {
        FakeProbe {
            space: Mutex::new(None),
            devices: Mutex::new(Vec::new()),
            trash: Mutex::new(TrashSupport::Unsupported("test volume has no trash".into())),
        }
    }
}

impl FsProbe for FakeProbe {
    fn space(&self, p: &Path) -> FsResult<(u64, u64)> {
        match *self.space.lock() {
            Some(s) => Ok(s),
            None => Err(FsError::unsupported("space", Some(p))),
        }
    }

    fn device_of(&self, p: &Path) -> FsResult<DeviceId> {
        let devs = self.devices.lock();
        let best = devs.iter().filter(|(root, _)| p.starts_with(root)).max_by_key(|(root, _)| root.as_os_str().len());
        Ok(DeviceId(best.map(|(_, d)| *d).unwrap_or(1)))
    }

    fn trash_support(&self, _p: &Path) -> TrashSupport {
        self.trash.lock().clone()
    }
}

pub(crate) struct H {
    pub engine: Arc<JobEngine>,
    pub probe: Arc<FakeProbe>,
    pub dir: tempfile::TempDir,
}

impl H {
    pub fn new() -> H {
        H::with(|_| {})
    }

    pub fn with(f: impl FnOnce(&mut EngineConfig)) -> H {
        let mut cfg = EngineConfig { sample_interval: Duration::from_millis(50), ..EngineConfig::default() };
        f(&mut cfg);
        let probe = Arc::new(FakeProbe::default());
        let engine = JobEngine::new(cfg, probe.clone(), Arc::new(|| {}));
        H { engine, probe, dir: tempfile::tempdir().unwrap() }
    }

    pub fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    pub fn submit(&self, kind: JobKind) -> JobId {
        self.engine.submit(JobSpec::new(kind))
    }

    pub fn snap(&self, id: JobId) -> JobSnapshot {
        self.engine.snapshot(id).expect("job exists")
    }

    /// Wait for a terminal state.
    pub fn done(&self, id: JobId) -> JobState {
        wait_for(Duration::from_secs(20), || {
            let s = self.engine.snapshot(id)?;
            s.state.is_terminal().then_some(s.state)
        })
    }

    /// Wait for `Done` and return the summary.
    pub fn ok(&self, id: JobId) -> crate::JobSummary {
        match self.done(id) {
            JobState::Done(s) => s,
            other => panic!("job {id:?} ended as {other:?}; log: {:#?}", self.engine.log(id, 0)),
        }
    }

    pub fn prompt(&self, id: JobId) -> JobPrompt {
        wait_for(Duration::from_secs(20), || {
            let s = self.engine.snapshot(id)?;
            assert!(!s.state.is_terminal(), "job ended while waiting for a prompt: {:?}", s.state);
            if s.state == JobState::WaitingForUser { s.pending_prompt } else { None }
        })
    }

    pub fn wait_state(&self, id: JobId, pred: impl Fn(&JobState) -> bool) -> JobSnapshot {
        wait_for(Duration::from_secs(20), || self.engine.snapshot(id).filter(|s| pred(&s.state)))
    }

    /// Drain events so far.
    pub fn events(&self) -> Vec<JobEvent> {
        self.engine.events().try_iter().collect()
    }
}

impl Drop for H {
    fn drop(&mut self) {
        self.engine.shutdown(Duration::from_secs(5));
    }
}

/// Poll `f` every millisecond until it yields a value; panics after `timeout`.
pub(crate) fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(start.elapsed() < timeout, "timed out after {timeout:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(crate) fn write(p: &Path, bytes: &[u8]) {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, bytes).unwrap();
}

pub(crate) fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// A spec copying with a throughput cap (deterministic duration).
pub(crate) fn throttled(kind: JobKind, bps: u64) -> JobSpec {
    let mut spec = JobSpec::new(kind);
    spec.options.copy.throttle_bps = Some(bps);
    spec
}

pub(crate) fn copy_kind(sources: &[&Path], dest: &Path) -> JobKind {
    JobKind::Copy {
        sources: sources.iter().map(|p| p.to_path_buf()).collect(),
        dest_dir: dest.to_path_buf(),
        rename_to: None,
    }
}
