//! Process, socket and service records of [`super`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Identifies one process across PID reuse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProcKey {
    pub pid: u32,
    /// Start time, unix milliseconds (0 when unknown).
    pub start_ms: i64,
}

/// A process's run state, on the union of the OSes' vocabularies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcState {
    Running,
    #[default]
    Sleeping,
    /// Uninterruptible wait, usually I/O.
    DiskWait,
    Stopped,
    Zombie,
    Idle,
    /// All threads suspended (Windows).
    Suspended,
    Unknown,
}

/// Scheduling priority on a shared six-step scale. Windows maps to priority classes,
/// Unix to nice values (19, 10, 0, -5, -15, -20).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcPriority {
    Idle,
    BelowNormal,
    #[default]
    Normal,
    AboveNormal,
    High,
    Realtime,
}

/// One process. Counters are since process start. `None`: the OS does not offer the value,
/// or Soyuz may not read it for this process (another user's, a protected one).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcRaw {
    pub key: ProcKey,
    pub parent: Option<u32>,
    /// Short name (`firefox.exe`, `bash`).
    pub name: String,
    pub exe: Option<PathBuf>,
    pub cmdline: Option<String>,
    pub user: Option<String>,
    pub state: ProcState,
    /// User and kernel CPU time, milliseconds.
    pub cpu_user_ms: u64,
    pub cpu_system_ms: u64,
    /// Working set / RSS.
    pub rss: u64,
    /// Private bytes (Windows) / anonymous RSS (Linux) / phys footprint (macOS).
    pub private: Option<u64>,
    /// Shared part of the resident set.
    pub shared: Option<u64>,
    /// Address space / commit size.
    pub virt: Option<u64>,
    /// Peak working set.
    pub peak_rss: Option<u64>,
    pub threads: u32,
    /// Open handles / file descriptors.
    pub handles: Option<u32>,
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub read_ops: Option<u64>,
    pub write_ops: Option<u64>,
    /// Page faults since start.
    pub page_faults: Option<u64>,
    pub priority: Option<ProcPriority>,
    /// Raw nice value (Unix) or base priority number (Windows).
    pub nice: Option<i32>,
    /// Session / terminal (Windows session id, `pts/3`).
    pub session: Option<String>,
    /// Instantaneous GPU load across engines, percent.
    pub gpu_percent: Option<f32>,
    /// GPU memory used.
    pub gpu_mem: Option<u64>,
    /// `x64`, `x86`, `arm64`.
    pub arch: Option<String>,
    /// Running elevated / as root.
    pub elevated: Option<bool>,
    /// Container / control group (Linux).
    pub cgroup: Option<String>,
    /// Kernel thread or OS-internal pseudo process.
    pub kernel: bool,
    /// Product description from the executable when cheaply known; often empty.
    pub description: Option<String>,
}

/// TCP / UDP.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Proto {
    #[default]
    Tcp,
    Udp,
    Tcp6,
    Udp6,
}

/// One open socket.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub proto: Proto,
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    /// `ESTABLISHED`, `LISTEN`, … (empty for UDP).
    pub state: String,
    pub pid: Option<u32>,
}

/// Running state of a service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Running,
    #[default]
    Stopped,
    Starting,
    Stopping,
    Paused,
    Failed,
    Unknown,
}

/// One service / daemon.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceInfo {
    /// Key for [`super::service_control`] (`Spooler`, `sshd.service`, `com.apple.foo`).
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub state: ServiceState,
    /// `auto`, `manual`, `disabled`, `enabled`, `static`, … as the OS words it.
    pub start_mode: String,
    pub pid: Option<u32>,
}

/// [`super::service_control`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceOp {
    Start,
    Stop,
    Restart,
}

/// One entry that starts with the session or the machine.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupItem {
    pub name: String,
    pub command: String,
    /// Where it is registered (`HKCU\…\Run`, `~/.config/autostart`, `~/Library/LaunchAgents`).
    pub source: String,
    pub enabled: bool,
}

/// [`super::signal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcSignal {
    /// Ask to exit: SIGTERM; on Windows a close message to the process's windows
    /// (`Unsupported` when it has none, so the caller can offer Kill).
    Terminate,
    /// Exit now: SIGKILL, `TerminateProcess`.
    Kill,
    /// SIGSTOP, `NtSuspendProcess`.
    Suspend,
    /// SIGCONT, `NtResumeProcess`.
    Resume,
    /// SIGHUP (`Unsupported` on Windows).
    Hangup,
    /// SIGINT (`Unsupported` on Windows: a console Ctrl+C cannot be aimed at one process).
    Interrupt,
}

/// A thread of a process.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub tid: u32,
    pub name: String,
    pub state: ProcState,
    pub cpu_ms: u64,
    pub priority: Option<i32>,
}

/// A mapped module / shared library.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub path: String,
    pub size: u64,
    pub base: u64,
}

/// An open file or descriptor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenFile {
    /// fd number (Unix).
    pub fd: Option<u32>,
    pub path: String,
}

/// On-demand details ([`super::process_details`]); what the OS denies stays empty, with
/// `limited` set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcDetails {
    pub exe: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub cmdline: Option<String>,
    pub threads: Vec<ThreadInfo>,
    pub modules: Vec<ModuleInfo>,
    pub open_files: Vec<OpenFile>,
    /// `NAME=value` pairs, sorted; values are untrusted data.
    pub environment: Vec<(String, String)>,
    /// Some lists could not be read (access denied).
    pub limited: bool,
}
