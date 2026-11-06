//! Synthetic data for the unit tests.

use std::sync::Arc;

use soyuz_fs::sysmon::{CoreRaw, CpuTimes, ProcKey, ProcRaw, RawSample};

use crate::frame::{Frame, ProcRow};

pub fn key(pid: u32, start: i64) -> ProcKey {
    ProcKey { pid, start_ms: start, start_raw: start as u64 }
}

pub fn sample(at_ms: i64, mono_ms: u64) -> RawSample {
    RawSample { at_ms, mono_ms, ..Default::default() }
}

pub fn times(user: u64, system: u64, idle: u64) -> CpuTimes {
    CpuTimes { user, system, idle, ..Default::default() }
}

pub fn cores(n: usize) -> Vec<CoreRaw> {
    vec![CoreRaw::default(); n]
}

pub fn proc_raw(pid: u32, start: i64, name: &str, cpu_ms: u64) -> ProcRaw {
    ProcRaw { key: key(pid, start), name: Arc::from(name), cpu_user_ms: cpu_ms, threads: 1, ..Default::default() }
}

/// A row with the fields tables and filters look at most.
pub fn row(pid: u32, parent: Option<u32>, name: &str, cpu: f32, rss: u64) -> ProcRow {
    let mut raw = proc_raw(pid, 100, name, 0);
    raw.parent = parent;
    raw.rss = rss;
    ProcRow { raw, cpu_percent: cpu, ..Default::default() }
}

pub fn frame_of(rows: Vec<ProcRow>) -> Frame {
    Frame { procs: Arc::from(rows), ..Default::default() }
}
