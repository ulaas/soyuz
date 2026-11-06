//! System monitor logic on top of the raw readers in `soyuz_fs::sysmon`: rates and frames,
//! history in memory and on disk, snapshots and comparisons, the process filter, the process
//! table and tree, alerts, and the process-wide sampler [`hub::Hub`]. No GUI and no OS calls of
//! its own; `README-API.md` lists the public API for the frontends.

pub mod alerts;
pub mod compare;
pub mod control;
pub mod fetch;
pub mod filter;
pub mod format;
pub mod frame;
pub mod history;
pub mod hub;
pub mod rates;
pub mod redact;
pub mod sanitize;
pub mod snapshot;
pub mod store;
pub mod table;

#[cfg(test)]
pub(crate) mod testutil;

pub use alerts::{Alert, AlertKind, AlertLimits, AlertWatcher};
pub use compare::{
    Change, Comparison, DiffSort, DiffValues, MatchBy, ProcDiff, Tolerance, TotalsDiff, compare, sort_diffs,
};
pub use control::{ProcRisk, TreeSignalReport, descendants, is_critical, refuses, risk, signal_row, signal_tree};
pub use fetch::{Loader, Wake};
pub use filter::{Filter, FilterError};
pub use format::{format_duration, format_rate};
pub use frame::{CoreFrame, Counts, CpuFrame, DiskFrame, Frame, MemFrame, NetFrame, ProcRow, state_id};
pub use history::{Agg, AggPoint, FinePoint, History, Metric, Point, ProcHistory, ProcSample, Pt, TopProc, Tops};
pub use hub::{Hub, HubConfig, Source, Subscription};
pub use rates::Rater;
pub use sanitize::sanitize_text;
pub use snapshot::{Snapshot, SnapshotMeta};
pub use table::{
    CellValue, RowId, RowKind, TableCache, TableOpts, TableRow, Totals, build, cell, cell_total, position_of,
};

// The raw records the frontends handle, so that they need only one import path.
pub use soyuz_core::config::monitor::{CpuScale, Monitor, ProcColumn, ProcView, SnapshotCommands};
pub use soyuz_fs::sysmon::{
    BatteryRaw, Connection, ControlError, DetailsWant, GpuEngine, GpuRaw, MemoryRaw, ProcDetails, ProcKey,
    ProcPriority, ProcRaw, ProcSignal, ProcState, Proto, RawSample, SensorKind, SensorRaw, ServiceInfo, ServiceOp,
    ServiceState, StartupItem, SystemInfo, Want,
};

/// Version stamped on every history line and snapshot written to disk.
pub const FORMAT_VERSION: u32 = 1;
