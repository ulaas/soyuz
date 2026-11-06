//! `[monitor]`: the system monitor (`monitor:` tabs). Everything the page lets you arrange
//! is stored here, so columns, sort order, view and graphs survive a restart.

use serde::{Deserialize, Serialize};

/// A column of the process table.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProcColumn {
    Name,
    Pid,
    /// Parent process id.
    Parent,
    User,
    /// Running, sleeping, suspended, …
    State,
    /// CPU load.
    Cpu,
    /// CPU time used since the process started.
    CpuTime,
    /// Resident memory / working set.
    Memory,
    /// Private bytes.
    Private,
    /// Shared memory.
    Shared,
    /// Address space / commit size.
    Virtual,
    /// Peak resident memory.
    PeakMemory,
    /// Memory as a share of the machine's.
    MemoryPercent,
    Threads,
    /// Open handles / file descriptors.
    Handles,
    /// Read rate (on Windows all I/O of the process, including pipes and sockets).
    DiskRead,
    /// Write rate (on Windows all I/O of the process, including pipes and sockets).
    DiskWrite,
    /// Total bytes read since start.
    DiskReadTotal,
    /// Total bytes written since start.
    DiskWriteTotal,
    /// GPU load.
    Gpu,
    /// GPU memory.
    GpuMemory,
    PageFaults,
    /// Scheduling priority.
    Priority,
    /// Nice value / base priority number.
    Nice,
    /// Session or terminal.
    Session,
    /// How long the process has been running.
    Uptime,
    /// When the process started.
    Started,
    /// Executable path.
    Path,
    /// Full command line.
    Command,
    /// Architecture.
    Arch,
    /// Elevated / root.
    Elevated,
    /// Control group (Linux).
    Cgroup,
    /// Product description.
    Description,
}

impl ProcColumn {
    pub const ALL: [ProcColumn; 33] = [
        ProcColumn::Name,
        ProcColumn::Pid,
        ProcColumn::Parent,
        ProcColumn::User,
        ProcColumn::State,
        ProcColumn::Cpu,
        ProcColumn::CpuTime,
        ProcColumn::Memory,
        ProcColumn::Private,
        ProcColumn::Shared,
        ProcColumn::Virtual,
        ProcColumn::PeakMemory,
        ProcColumn::MemoryPercent,
        ProcColumn::Threads,
        ProcColumn::Handles,
        ProcColumn::DiskRead,
        ProcColumn::DiskWrite,
        ProcColumn::DiskReadTotal,
        ProcColumn::DiskWriteTotal,
        ProcColumn::Gpu,
        ProcColumn::GpuMemory,
        ProcColumn::PageFaults,
        ProcColumn::Priority,
        ProcColumn::Nice,
        ProcColumn::Session,
        ProcColumn::Uptime,
        ProcColumn::Started,
        ProcColumn::Path,
        ProcColumn::Command,
        ProcColumn::Arch,
        ProcColumn::Elevated,
        ProcColumn::Cgroup,
        ProcColumn::Description,
    ];

    /// The stable id (`disk_read`), used in Fluent keys `monitor-col-<id with dashes>` and
    /// filter terms.
    pub fn id(self) -> &'static str {
        match self {
            ProcColumn::Name => "name",
            ProcColumn::Pid => "pid",
            ProcColumn::Parent => "parent",
            ProcColumn::User => "user",
            ProcColumn::State => "state",
            ProcColumn::Cpu => "cpu",
            ProcColumn::CpuTime => "cpu_time",
            ProcColumn::Memory => "memory",
            ProcColumn::Private => "private",
            ProcColumn::Shared => "shared",
            ProcColumn::Virtual => "virtual",
            ProcColumn::PeakMemory => "peak_memory",
            ProcColumn::MemoryPercent => "memory_percent",
            ProcColumn::Threads => "threads",
            ProcColumn::Handles => "handles",
            ProcColumn::DiskRead => "disk_read",
            ProcColumn::DiskWrite => "disk_write",
            ProcColumn::DiskReadTotal => "disk_read_total",
            ProcColumn::DiskWriteTotal => "disk_write_total",
            ProcColumn::Gpu => "gpu",
            ProcColumn::GpuMemory => "gpu_memory",
            ProcColumn::PageFaults => "page_faults",
            ProcColumn::Priority => "priority",
            ProcColumn::Nice => "nice",
            ProcColumn::Session => "session",
            ProcColumn::Uptime => "uptime",
            ProcColumn::Started => "started",
            ProcColumn::Path => "path",
            ProcColumn::Command => "command",
            ProcColumn::Arch => "arch",
            ProcColumn::Elevated => "elevated",
            ProcColumn::Cgroup => "cgroup",
            ProcColumn::Description => "description",
        }
    }

    /// Sorts largest first on the first click (numbers people look for the top of).
    pub fn descending_first(self) -> bool {
        matches!(
            self,
            ProcColumn::Cpu
                | ProcColumn::CpuTime
                | ProcColumn::Memory
                | ProcColumn::Private
                | ProcColumn::Shared
                | ProcColumn::Virtual
                | ProcColumn::PeakMemory
                | ProcColumn::MemoryPercent
                | ProcColumn::Threads
                | ProcColumn::Handles
                | ProcColumn::DiskRead
                | ProcColumn::DiskWrite
                | ProcColumn::DiskReadTotal
                | ProcColumn::DiskWriteTotal
                | ProcColumn::Gpu
                | ProcColumn::GpuMemory
                | ProcColumn::PageFaults
                | ProcColumn::Uptime
        )
    }

    /// Right-aligned numeric column.
    pub fn numeric(self) -> bool {
        self.descending_first() || matches!(self, ProcColumn::Pid | ProcColumn::Parent | ProcColumn::Nice)
    }

    /// Default width in points (GUI); terminal cells derive from it.
    pub fn default_width(self) -> f32 {
        match self {
            ProcColumn::Name => 240.0,
            ProcColumn::Path | ProcColumn::Command | ProcColumn::Description | ProcColumn::Cgroup => 320.0,
            ProcColumn::User | ProcColumn::Started => 130.0,
            ProcColumn::State | ProcColumn::Session | ProcColumn::Priority => 90.0,
            ProcColumn::Pid | ProcColumn::Parent | ProcColumn::Nice | ProcColumn::Arch | ProcColumn::Elevated => 70.0,
            _ => 88.0,
        }
    }
}

/// A process-table column and its width.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcColumnConfig {
    pub id: ProcColumn,
    /// Width in points in the GUI (the terminal UI divides by 8); 0 = the default.
    #[serde(default)]
    pub width: f32,
}

/// How the process table arranges its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcView {
    /// One flat, sorted list.
    #[default]
    List,
    /// Parents above their children.
    Tree,
    /// Grouped by program name, with totals.
    ByName,
    /// Grouped by user, with totals.
    ByUser,
}

/// How a process's CPU load is scaled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuScale {
    /// 100 % is the whole machine.
    #[default]
    Machine,
    /// 100 % is one logical CPU (a busy eight-thread program shows 800 %).
    PerCore,
}

/// How the CPU page draws its logical CPUs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreView {
    /// One small graph per logical CPU.
    #[default]
    Graphs,
    /// A compact heat grid.
    Heat,
    /// One bar per logical CPU.
    Bars,
}

/// Temperature unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TempUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

/// What of a process's command line goes into snapshots and exports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotCommands {
    /// Leave command lines out.
    None,
    /// Mask the values of flags that look like secrets (`--password x`, `user:pass@host`).
    #[default]
    Redacted,
    /// Keep them whole.
    Full,
}

/// A page of the system monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorPage {
    #[default]
    Overview,
    Processes,
    Cpu,
    Memory,
    Disks,
    Network,
    Gpu,
    /// Temperatures, fans, power, batteries.
    Sensors,
    /// Open sockets.
    Connections,
    /// Services and daemons.
    Services,
    /// Programs that start with the system.
    Startup,
    /// Recorded history with a time range.
    History,
    /// Saved snapshots and comparisons.
    Snapshots,
}

impl MonitorPage {
    pub const ALL: [MonitorPage; 13] = [
        MonitorPage::Overview,
        MonitorPage::Processes,
        MonitorPage::Cpu,
        MonitorPage::Memory,
        MonitorPage::Disks,
        MonitorPage::Network,
        MonitorPage::Gpu,
        MonitorPage::Sensors,
        MonitorPage::Connections,
        MonitorPage::Services,
        MonitorPage::Startup,
        MonitorPage::History,
        MonitorPage::Snapshots,
    ];

    /// Stable id, used in Fluent keys `monitor-page-<id>`.
    pub fn id(self) -> &'static str {
        match self {
            MonitorPage::Overview => "overview",
            MonitorPage::Processes => "processes",
            MonitorPage::Cpu => "cpu",
            MonitorPage::Memory => "memory",
            MonitorPage::Disks => "disks",
            MonitorPage::Network => "network",
            MonitorPage::Gpu => "gpu",
            MonitorPage::Sensors => "sensors",
            MonitorPage::Connections => "connections",
            MonitorPage::Services => "services",
            MonitorPage::Startup => "startup",
            MonitorPage::History => "history",
            MonitorPage::Snapshots => "snapshots",
        }
    }
}

/// `[monitor]`: the system monitor (`monitor:` tabs, "System monitor" in the palette).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Monitor {
    /// Milliseconds between readings while a monitor tab is on screen.
    pub interval_ms: u32,
    /// Page shown when a monitor tab opens.
    pub page: MonitorPage,
    /// Process table columns, left to right, with widths. The Name column is always kept.
    pub columns: Vec<ProcColumnConfig>,
    /// Column the process table sorts by.
    pub sort_by: ProcColumn,
    /// Sort largest first.
    pub sort_descending: bool,
    /// How the process table arranges its rows.
    pub view: ProcView,
    /// Keep the process filter between sessions.
    pub remember_filter: bool,
    /// The remembered process filter. Words match any text column; `/expr/` is a regular expression;
    /// `user:root`, `cpu>5`, `mem>100M` test one column.
    pub filter: String,
    /// Treat the whole filter as one regular expression.
    pub filter_regex: bool,
    /// Tree view: keep the parents of matching processes on screen, dimmed.
    pub filter_keeps_parents: bool,
    /// List kernel threads and OS-internal pseudo processes.
    pub show_kernel: bool,
    /// List other users' processes.
    pub show_other_users: bool,
    /// The scale of a process's CPU load.
    pub cpu_scale: CpuScale,
    /// How the CPU page draws its logical CPUs.
    pub core_view: CoreView,
    /// Seconds of history the live graphs show.
    pub graph_seconds: u32,
    /// Show the details panel under the process table.
    pub details_panel: bool,
    /// Height of the details panel in points (the terminal UI divides by 18).
    pub details_height: f32,
    /// Draw program icons in the process table (GUI).
    pub process_icons: bool,
    /// Read per-process GPU use (the most expensive reading; refreshed every few seconds).
    pub process_gpu: bool,
    /// Keep the selected process in view while rows reorder.
    pub follow_selected: bool,
    /// What of a process's command line goes into snapshots and exports.
    pub snapshot_commands: SnapshotCommands,
    /// Tint table cells by load, like a heat map.
    pub heat_cells: bool,
    /// Temperature unit.
    pub temperature_unit: TempUnit,
    /// Ask before ending, killing or suspending a process.
    pub confirm_kill: bool,
    /// List virtual network interfaces (loopback, tunnels, bridges) on the Network page.
    pub show_virtual_networks: bool,
    /// Keep recording history at `background_interval_secs` while no monitor tab is open.
    pub background_history: bool,
    /// Seconds between readings when recording with no monitor tab open.
    pub background_interval_secs: u32,
    /// Write history to disk, so it survives a restart.
    pub history_persist: bool,
    /// Days of history kept on disk.
    pub history_days: u32,
    /// Snapshots kept before the oldest automatic ones are removed (manual ones are never removed).
    pub snapshots_kept: u32,
    /// Raise a notification when total CPU stays above this percentage (0 = off).
    pub alert_cpu_percent: u32,
    /// Raise a notification when memory stays above this percentage (0 = off).
    pub alert_memory_percent: u32,
    /// Raise a notification when any temperature sensor stays above this many °C (0 = off).
    pub alert_temperature_c: u32,
    /// How long a value must stay above its limit before the alert fires (seconds).
    pub alert_sustain_secs: u32,
    /// Take a snapshot automatically when an alert fires.
    pub alert_snapshot: bool,
}

impl Monitor {
    pub const INTERVAL_MS_MIN: u32 = 250;
    pub const INTERVAL_MS_MAX: u32 = 10_000;
    pub const GRAPH_SECONDS_MIN: u32 = 30;
    pub const GRAPH_SECONDS_MAX: u32 = 3_600;
    pub const HISTORY_DAYS_MAX: u32 = 90;
}

/// The column set a fresh config starts with.
pub fn default_proc_columns() -> Vec<ProcColumnConfig> {
    use ProcColumn as C;
    [C::Name, C::Pid, C::User, C::State, C::Cpu, C::Memory, C::DiskRead, C::DiskWrite, C::Gpu, C::Threads, C::Command]
        .into_iter()
        .map(|id| ProcColumnConfig { id, width: 0.0 })
        .collect()
}

impl Default for Monitor {
    fn default() -> Self {
        Monitor {
            interval_ms: 1000,
            page: MonitorPage::Overview,
            columns: default_proc_columns(),
            sort_by: ProcColumn::Cpu,
            sort_descending: true,
            view: ProcView::List,
            remember_filter: false,
            filter: String::new(),
            filter_regex: false,
            filter_keeps_parents: true,
            show_kernel: false,
            show_other_users: true,
            cpu_scale: CpuScale::Machine,
            core_view: CoreView::Graphs,
            graph_seconds: 120,
            details_panel: true,
            details_height: 260.0,
            process_icons: true,
            process_gpu: true,
            follow_selected: true,
            snapshot_commands: SnapshotCommands::Redacted,
            heat_cells: true,
            temperature_unit: TempUnit::Celsius,
            confirm_kill: true,
            show_virtual_networks: false,
            background_history: false,
            background_interval_secs: 10,
            history_persist: true,
            history_days: 7,
            snapshots_kept: 50,
            alert_cpu_percent: 0,
            alert_memory_percent: 0,
            alert_temperature_c: 0,
            alert_sustain_secs: 30,
            alert_snapshot: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_columns_have_distinct_ids() {
        let mut ids: Vec<_> = ProcColumn::ALL.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ProcColumn::ALL.len());
    }

    #[test]
    fn columns_round_trip_through_toml() {
        let m = Monitor::default();
        let s = toml::to_string(&m).unwrap();
        let back: Monitor = toml::from_str(&s).unwrap();
        assert_eq!(m, back);
    }
}
