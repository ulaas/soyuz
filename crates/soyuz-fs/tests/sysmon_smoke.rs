//! The Windows system monitor readers against the real machine. Read-only, except for one
//! child process this test starts itself. Run with `--nocapture` to see the values.
#![cfg(windows)]

use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use soyuz_fs::sysmon::{
    self, ControlError, DetailsWant, Probe, ProcKey, ProcPriority, ProcSignal, ProcState, RawSample, Want,
};

const MIB: f64 = 1024.0 * 1024.0;
/// No console window for the child.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn sample_pair() -> (RawSample, RawSample) {
    let mut probe = Probe::new();
    let a = probe.sample(Want::default());
    std::thread::sleep(Duration::from_millis(1100));
    let b = probe.sample(Want::default());
    (a, b)
}

#[test]
fn real_sample_is_sane_and_printed() {
    let (a, b) = sample_pair();
    let me = std::process::id();
    let mine = b.processes.iter().find(|p| p.key.pid == me).expect("this process is listed");
    println!("this process: {mine:#?}");
    assert!(mine.name.to_lowercase().starts_with("sysmon_smoke"), "{}", mine.name);
    assert!(mine.rss > 0);
    assert!(mine.exe.as_ref().is_some_and(|e| e.is_absolute()));
    assert!(mine.cmdline.as_deref().is_some_and(|c| c.contains("sysmon_smoke")));
    assert!(mine.user.as_deref().is_some_and(|u| !u.is_empty()));
    assert_eq!(mine.arch.as_deref(), Some(if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" }));
    assert!(mine.threads >= 1);
    assert!(mine.handles.is_some_and(|h| h > 0));
    let exe = std::env::current_exe().unwrap();
    assert_eq!(mine.exe.as_deref(), Some(exe.as_path()));

    // Idle and System rows.
    let idle = b.processes.iter().find(|p| p.key.pid == 0).expect("idle");
    assert!(idle.kernel);
    assert!(idle.idle);
    assert_eq!(idle.state, ProcState::Idle);
    assert_eq!((idle.cpu_user_ms, idle.cpu_system_ms), (0, 0), "idle time is not work");
    assert_eq!(b.processes.iter().filter(|p| p.idle).count(), 1);
    let system = b.processes.iter().find(|p| p.key.pid == 4).expect("System");
    assert!(system.kernel);
    assert_eq!(&*system.name, "System");
    assert!(system.cpu_system_ms > 0);
    assert!(!mine.idle && !system.idle);
    let threads: u64 = b.processes.iter().filter(|p| !p.idle).map(|p| u64::from(p.threads)).sum();
    assert_eq!(b.total_threads, Some(threads), "no idle threads in the total");

    println!("{} processes, {:?} threads, {:?} handles", b.processes.len(), b.total_threads, b.total_handles);
    println!("uptime {} s, mono delta {} ms", b.uptime_secs, b.mono_ms - a.mono_ms);
    assert!((b.mono_ms - a.mono_ms) >= 1000, "mono clock");

    // CPU: time advances by about (logical CPUs * elapsed).
    let n = b.cores.len();
    assert!(n > 0);
    let total = |t: &sysmon::CpuTimes| t.user + t.system + t.idle + t.irq + t.softirq;
    let elapsed = b.mono_ms - a.mono_ms;
    let advanced = total(&b.cpu_total) - total(&a.cpu_total);
    println!("cpu: {n} cores, {advanced} ms advanced in {elapsed} ms wall, first {:?}", b.cores[0]);
    println!(
        "cpu busy {:.1}%, ctx {:?}, interrupts {:?}",
        100.0 * (1.0 - (b.cpu_total.idle - a.cpu_total.idle) as f64 / advanced as f64),
        b.ctx_switches.zip(a.ctx_switches).map(|(x, y)| x - y),
        b.interrupts.zip(a.interrupts).map(|(x, y)| x - y),
    );
    let expect = n as u64 * elapsed;
    assert!(advanced > expect / 2 && advanced < expect * 2, "{advanced} vs {expect}");
    assert!(b.ctx_switches.is_some_and(|c| c > 0));
    assert!(b.cores.iter().all(|c| c.mhz.is_some()));

    // Memory.
    let m = &b.memory;
    println!(
        "memory: total {:.0} MiB, available {:.0}, cached {:?}, kernel {:?}, commit {:?}/{:?}, swap {:.0}/{:.0}, compressed {:?}, page in/out {:?}/{:?}",
        m.total as f64 / MIB,
        m.available as f64 / MIB,
        m.cached.map(|v| (v as f64 / MIB) as u64),
        m.kernel.map(|v| (v as f64 / MIB) as u64),
        m.commit_used.map(|v| (v as f64 / MIB) as u64),
        m.commit_limit.map(|v| (v as f64 / MIB) as u64),
        m.swap_used as f64 / MIB,
        m.swap_total as f64 / MIB,
        m.compressed.map(|v| (v as f64 / MIB) as u64),
        m.page_in,
        m.page_out,
    );
    assert!(m.total > 0 && m.available > 0 && m.available < m.total);
    assert!(m.swap_used <= m.swap_total);
    assert!(m.commit_used.is_some() && m.commit_limit.is_some());

    // Disks.
    for (x, y) in a.disks.iter().zip(&b.disks) {
        println!(
            "disk {} '{}': size {:?} rotational {:?}, read {} write {} ops {}/{}, in the last second read {} write {} busy {:?} ms, queue {:?}",
            y.id,
            y.model,
            y.size,
            y.rotational,
            y.read_bytes,
            y.write_bytes,
            y.read_ops,
            y.write_ops,
            y.read_bytes.saturating_sub(x.read_bytes),
            y.write_bytes.saturating_sub(x.write_bytes),
            y.busy_ms.zip(x.busy_ms).map(|(p, q)| p.saturating_sub(q)),
            y.queue,
        );
        assert!(y.read_bytes >= x.read_bytes);
    }
    assert!(!b.disks.is_empty(), "this machine has a disk");

    // Network.
    for net in &b.nets {
        println!(
            "net {} '{}' ({}): up {} virtual {} link {:?} rx {} tx {} pkts {}/{} errs {}/{} mac {:?} {:?}",
            net.id,
            net.name,
            net.description,
            net.up,
            net.virtual_if,
            net.link_bps,
            net.rx_bytes,
            net.tx_bytes,
            net.rx_packets,
            net.tx_packets,
            net.rx_errors,
            net.tx_errors,
            net.mac,
            net.addresses,
        );
    }
    assert!(!b.nets.is_empty());

    // GPU, sensors, batteries (whatever this machine has).
    for g in &b.gpus {
        println!("gpu: {g:#?}");
    }
    for s in &b.sensors {
        println!("sensor: {s:?}");
    }
    println!("batteries: {:?}", b.batteries);
    let gpu_rows = b.processes.iter().filter(|p| p.gpu_percent.unwrap_or(0.0) > 0.0 || p.gpu_mem.unwrap_or(0) > 0);
    for p in gpu_rows.take(8) {
        println!("gpu use: {} pid {} load {:?} mem {:?}", p.name, p.key.pid, p.gpu_percent, p.gpu_mem);
    }

    // A sample of odd rows: what reads as `None` for other accounts' and protected processes.
    let none_exe = b.processes.iter().filter(|p| !p.kernel && p.exe.is_none()).count();
    let none_user = b.processes.iter().filter(|p| !p.kernel && p.user.is_none()).count();
    let none_cmd = b.processes.iter().filter(|p| !p.kernel && p.cmdline.is_none()).count();
    let described = b.processes.iter().filter(|p| p.description.is_some()).count();
    println!("no path: {none_exe}, no user: {none_user}, no command line: {none_cmd}, with description: {described}");
    for p in b.processes.iter().filter(|p| !p.kernel && p.user.is_none()).take(5) {
        println!("  limited: {} pid {} exe {:?}", p.name, p.key.pid, p.exe);
    }
}

#[test]
fn hardware_off_skips_the_slow_parts() {
    let mut probe = Probe::new();
    let s = probe.sample(Want { processes: false, process_details: false, process_gpu: false, hardware: false });
    assert!(s.processes.is_empty());
    assert!(s.gpus.is_empty());
    assert!(s.sensors.is_empty());
    assert!(s.total_threads.is_some());
    assert!(!s.cores.is_empty());
    assert!(s.memory.total > 0);
}

#[test]
fn system_info_is_filled_in() {
    let i = sysmon::system_info();
    println!("{i:#?}");
    assert!(!i.hostname.is_empty());
    assert!(i.os.starts_with("Windows"), "{}", i.os);
    assert!(!i.kernel.is_empty());
    assert!(!i.arch.is_empty());
    assert!(!i.cpu_model.is_empty());
    assert!(i.physical_cores > 0);
    assert!(i.logical_cores >= i.physical_cores);
    assert!(i.sockets > 0);
    assert!(i.memory_total > 0);
    assert!(i.boot_unix > 1_000_000_000);
}

#[test]
fn lists_are_readable() {
    let conns = sysmon::connections();
    println!("{} connections, first three: {:#?}", conns.len(), conns.iter().take(3).collect::<Vec<_>>());
    assert!(!conns.is_empty());
    let mut hist = std::collections::BTreeMap::new();
    for c in &conns {
        *hist.entry((format!("{:?}", c.proto), c.state.clone())).or_insert(0u32) += 1;
    }
    println!("sockets by state: {hist:?}");
    assert!(conns.iter().any(|c| c.state == "LISTEN"));
    assert!(conns.iter().any(|c| c.local_addr.contains(':')) || conns.iter().all(|c| !c.local_addr.is_empty()));

    let services = sysmon::services();
    assert!(conns.iter().filter(|c| matches!(c.proto, sysmon::Proto::Tcp | sysmon::Proto::Tcp6)).all(|c| {
        // A blank state only for an unknown code on a connected socket.
        !c.state.is_empty() || c.remote_port != 0
    }));
    println!("{} services, first three: {:#?}", services.len(), services.iter().take(3).collect::<Vec<_>>());
    assert!(services.len() > 10);
    assert!(services.iter().any(|s| !s.start_mode.is_empty()));
    assert!(services.iter().any(|s| s.pid.is_some()));

    let items = sysmon::startup_items();
    println!("{} startup items: {:#?}", items.len(), items.iter().take(5).collect::<Vec<_>>());
}

/// A child of this test only: `ping` waits between its probes and has no window.
fn spawn_child() -> Child {
    Command::new("ping")
        .args(["-n", "120", "127.0.0.1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .expect("spawn ping")
}

fn wait_for_state(probe: &mut Probe, pid: u32, f: impl Fn(ProcState) -> bool) -> bool {
    let want = Want { processes: true, process_details: false, process_gpu: false, hardware: false };
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(5) {
        let s = probe.sample(want);
        if s.processes.iter().any(|p| p.key.pid == pid && f(p.state)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn control_calls_act_on_our_own_child_only() {
    let mut child = spawn_child();
    let pid = child.id();
    let mut probe = Probe::new();
    let s = probe.sample(Want::default());
    let row = s.processes.iter().find(|p| p.key.pid == pid).expect("child listed").clone();
    let key = row.key;
    assert!(key.start_ms > 0);
    assert_eq!(row.parent, Some(std::process::id()));
    // Whatever it inherited (CI runners start tests below normal).
    assert!(row.priority.is_some());

    // A wrong start time must be refused before anything happens.
    assert!(key.start_raw > 0);
    let wrong = ProcKey { start_raw: key.start_raw - 1, ..key };
    assert_eq!(sysmon::signal(wrong, ProcSignal::Kill), Err(ControlError::Gone));
    assert_eq!(sysmon::set_priority(wrong, ProcPriority::Idle), Err(ControlError::Gone));
    assert_eq!(sysmon::affinity(wrong), Err(ControlError::Gone));
    assert!(matches!(sysmon::process_details(wrong, DetailsWant::default()), Err(ControlError::Gone)));

    assert_eq!(sysmon::signal(key, ProcSignal::Hangup), Err(ControlError::Unsupported));
    assert_eq!(sysmon::signal(key, ProcSignal::Interrupt), Err(ControlError::Unsupported));
    // No window to close.
    assert_eq!(sysmon::signal(key, ProcSignal::Terminate), Err(ControlError::Unsupported));

    sysmon::signal(key, ProcSignal::Suspend).expect("suspend");
    assert!(wait_for_state(&mut probe, pid, |s| s == ProcState::Suspended), "suspended state");
    sysmon::signal(key, ProcSignal::Resume).expect("resume");
    assert!(wait_for_state(&mut probe, pid, |s| s != ProcState::Suspended), "resumed state");

    sysmon::set_priority(key, ProcPriority::BelowNormal).expect("priority");
    let s = probe.sample(Want::default());
    let row = s.processes.iter().find(|p| p.key.pid == pid).unwrap();
    assert_eq!(row.priority, Some(ProcPriority::BelowNormal));

    let cpus = sysmon::affinity(key).expect("affinity");
    assert!(!cpus.is_empty());
    sysmon::set_affinity(key, &cpus[..1]).expect("set affinity");
    assert_eq!(sysmon::affinity(key).unwrap(), cpus[..1]);
    assert!(sysmon::set_affinity(key, &[]).is_err());

    // Only what is asked for.
    let d = sysmon::process_details(key, DetailsWant::default()).expect("no lists");
    assert!(d.exe.is_some() && d.threads.is_empty() && d.modules.is_empty() && !d.limited);
    let only_threads = DetailsWant { threads: true, ..DetailsWant::default() };
    let d = sysmon::process_details(key, only_threads).expect("threads");
    assert!(!d.threads.is_empty() && d.modules.is_empty());
    let d = sysmon::process_details(key, DetailsWant { threads: true, modules: true, ..DetailsWant::default() })
        .expect("details");
    println!(
        "details: exe {:?}, cmd {:?}, {} threads, {} modules, limited {}",
        d.exe,
        d.cmdline,
        d.threads.len(),
        d.modules.len(),
        d.limited
    );
    assert!(d.exe.is_some());
    assert!(!d.threads.is_empty());
    assert!(!d.modules.is_empty());
    assert!(!d.limited);
    assert!(d.modules.iter().any(|m| m.path.to_lowercase().ends_with("ping.exe")));

    sysmon::signal(key, ProcSignal::Kill).expect("kill");
    child.wait().expect("reaped");
    // Our `Child` still held a handle until the wait; now the process is gone.
    assert_eq!(sysmon::signal(key, ProcSignal::Kill), Err(ControlError::Gone));
    drop(child);
    assert!(matches!(sysmon::process_details(key, DetailsWant::default()), Err(ControlError::Gone)));
}

/// Per-process GPU use has its own slower cadence and carries over between readings.
#[test]
fn process_gpu_keeps_its_own_cadence() {
    let mut probe = Probe::new();
    let want = Want::default();
    let use_of = |s: &RawSample| -> Vec<(u32, Option<u64>)> {
        let mut v: Vec<_> = s.processes.iter().map(|p| (p.key.pid, p.gpu_mem)).filter(|x| x.1 > Some(0)).collect();
        v.sort_unstable();
        v
    };
    // The first reading fills memory; rate counters need the second.
    let _ = probe.sample(want);
    std::thread::sleep(Duration::from_millis(200));
    let reading = probe.sample(want);
    let first = use_of(&reading);
    assert!(reading.processes.iter().any(|p| p.gpu_percent.is_some()) || reading.gpus.is_empty());
    let mut same = 0;
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(100));
        let s = probe.sample(want);
        if use_of(&s).iter().filter(|x| first.contains(x)).count() == first.len() {
            same += 1;
        }
    }
    // Inside 2.5 s nothing is read again, so the values of live processes hold exactly.
    println!("{} processes with GPU memory, unchanged in {same} of 4 quick samples", first.len());
    assert_eq!(same, 4);
    // Off means off.
    let s = probe.sample(Want { process_gpu: false, ..want });
    assert!(s.processes.iter().all(|p| p.gpu_percent.is_none() && p.gpu_mem.is_none()));
}

#[test]
fn details_of_a_process_we_may_not_open_are_limited() {
    let mut probe = Probe::new();
    let want = Want { processes: true, process_details: false, process_gpu: false, hardware: false };
    let s = probe.sample(want);
    let idle = s.processes.iter().find(|p| p.key.pid == 0).expect("idle");
    let lists = DetailsWant { threads: true, modules: true, ..DetailsWant::default() };
    let d = sysmon::process_details(idle.key, lists).expect("idle details");
    assert!(d.limited);
    assert!(!d.threads.is_empty());
    let system = s.processes.iter().find(|p| p.key.pid == 4).expect("System");
    let d = sysmon::process_details(system.key, lists).expect("System details");
    assert!(d.limited);
    assert!(!d.threads.is_empty());
    println!("System: {} threads, first {:?}", d.threads.len(), d.threads[0]);
}

/// The same machine seen through the performance counters, over the same seconds:
/// `--ignored --nocapture compare_with_counters`. Starts one PowerShell child, reads only.
#[test]
#[ignore = "prints a comparison of the real machine"]
fn compare_with_counters() {
    let script = r#"
$c = @('\Processor(_Total)\% Processor Time','\System\Context Switches/sec','\Processor(_Total)\Interrupts/sec',
 '\Memory\Available Bytes','\Memory\Committed Bytes','\Memory\Cache Bytes','\Memory\Pages Input/sec',
 '\Memory\Pages Output/sec','\Paging File(_Total)\% Usage','\PhysicalDisk(_Total)\Disk Read Bytes/sec',
 '\PhysicalDisk(_Total)\Disk Write Bytes/sec','\PhysicalDisk(*)\% Idle Time','\Network Interface(*)\Bytes Total/sec','\System\Processes','\System\Threads')
(Get-Counter -Counter $c -SampleInterval 3 -MaxSamples 1).CounterSamples | ForEach-Object { '{0} = {1:N1}' -f $_.Path, $_.CookedValue }
$e = (Get-Counter '\GPU Engine(*)\Utilization Percentage' -SampleInterval 2 -MaxSamples 1).CounterSamples
$e | Where-Object { $_.InstanceName -like '*engtype_3D' } | Measure-Object CookedValue -Sum | ForEach-Object { 'GPU 3D sum = {0:N1}' -f $_.Sum }
$m = (Get-Counter '\GPU Adapter Memory(*)\Dedicated Usage').CounterSamples | Measure-Object CookedValue -Sum
'GPU dedicated bytes = {0:N0}' -f $m.Sum
(Get-Counter '\Thermal Zone Information(*)\Temperature').CounterSamples | ForEach-Object { 'thermal {0} = {1} K' -f $_.InstanceName, $_.CookedValue }
Get-Process -Id PID_HERE | ForEach-Object { 'this process: WS {0} private {1} handles {2} threads {3} start {4}' -f $_.WorkingSet64, $_.PrivateMemorySize64, $_.HandleCount, $_.Threads.Count, ([DateTimeOffset]$_.StartTime).ToUnixTimeMilliseconds() }
Get-Process -Id 4 | ForEach-Object { 'System: WS {0} handles {1} threads {2}' -f $_.WorkingSet64, $_.HandleCount, $_.Threads.Count }
"#
    .replace("PID_HERE", &std::process::id().to_string());
    let mut probe = Probe::new();
    let a = probe.sample(Want::default());
    let t = Instant::now();
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .expect("powershell");
    let b = probe.sample(Want::default());
    let secs = (b.mono_ms - a.mono_ms) as f64 / 1000.0;
    println!("--- counters, inside these {:.1} s\n{}", t.elapsed().as_secs_f64(), String::from_utf8_lossy(&out.stdout));
    println!("--- Soyuz readers, average over {secs:.1} s");
    let total = |t: &sysmon::CpuTimes| t.user + t.system + t.idle + t.irq + t.softirq;
    let adv = (total(&b.cpu_total) - total(&a.cpu_total)) as f64;
    println!("cpu busy {:.1}%", 100.0 * (1.0 - (b.cpu_total.idle - a.cpu_total.idle) as f64 / adv));
    let rate = |x: Option<u64>, y: Option<u64>| y.zip(x).map(|(p, q)| p.saturating_sub(q) as f64 / secs);
    println!("context switches/s {:?}", rate(a.ctx_switches, b.ctx_switches));
    println!("interrupts/s {:?}", rate(a.interrupts, b.interrupts));
    let m = &b.memory;
    println!(
        "available {} commit {:?} cached {:?} swap {:.2}% of {}",
        m.available,
        m.commit_used,
        m.cached,
        100.0 * m.swap_used as f64 / m.swap_total.max(1) as f64,
        m.swap_total
    );
    println!("pages in/s {:?}", rate(a.memory.page_in, b.memory.page_in).map(|v| v / 4096.0));
    println!("pages out/s {:?}", rate(a.memory.page_out, b.memory.page_out).map(|v| v / 4096.0));
    let sum = |s: &RawSample, f: fn(&sysmon::DiskRaw) -> u64| s.disks.iter().map(f).sum::<u64>();
    println!("disk read B/s {:.0}", (sum(&b, |d| d.read_bytes) - sum(&a, |d| d.read_bytes)) as f64 / secs);
    println!("disk write B/s {:.0}", (sum(&b, |d| d.write_bytes) - sum(&a, |d| d.write_bytes)) as f64 / secs);
    for (x, y) in a.disks.iter().zip(&b.disks) {
        let busy = y.busy_ms.zip(x.busy_ms).map_or(0.0, |(p, q)| p.saturating_sub(q) as f64 / (secs * 10.0));
        println!("  {} busy {busy:.1}%", y.id);
    }
    let net = |s: &RawSample| s.nets.iter().map(|n| n.rx_bytes + n.tx_bytes).sum::<u64>();
    println!("network B/s {:.0}", (net(&b) - net(&a)) as f64 / secs);
    println!("processes {} threads {:?}", b.processes.len(), b.total_threads);
    for g in &b.gpus {
        println!("gpu {} load {:?} dedicated {:?}", g.name, g.percent, g.mem_used);
    }
    for s in b.sensors.iter().filter(|s| s.id.starts_with("thermal")) {
        println!("sensor {} {} C", s.label, s.value);
    }
    let me = b.processes.iter().find(|p| p.key.pid == std::process::id()).unwrap();
    println!(
        "this process: WS {} private {:?} handles {:?} threads {} start_ms {}",
        me.rss, me.private, me.handles, me.threads, me.key.start_ms
    );
    let sys = b.processes.iter().find(|p| p.key.pid == 4).unwrap();
    println!("System: WS {} handles {:?} threads {}", sys.rss, sys.handles, sys.threads);
    let by_user = |u: &str| b.processes.iter().filter(|p| p.user.as_deref() == Some(u)).count();
    println!(
        "rows by user: SYSTEM {} LOCAL SERVICE {} NETWORK SERVICE {} none {}",
        by_user("SYSTEM"),
        by_user("LOCAL SERVICE"),
        by_user("NETWORK SERVICE"),
        b.processes.iter().filter(|p| !p.kernel && p.user.is_none()).count()
    );
    for n in &b.nets {
        println!("net '{}' up {} virtual {} addrs {:?}", n.name, n.up, n.virtual_if, n.addresses);
    }
}

/// Timings of a whole sample: run with `--release --nocapture`.
#[test]
fn sample_timings() {
    let mut probe = Probe::new();
    let t = Instant::now();
    let s = probe.sample(Want::default());
    println!("first full sample: {:?} ({} processes)", t.elapsed(), s.processes.len());
    let mut measure = |label: &str, want: Want| {
        let (mut best, mut worst) = (Duration::MAX, Duration::ZERO);
        let mut sum = Duration::ZERO;
        let rounds = 10;
        for _ in 0..rounds {
            std::thread::sleep(Duration::from_millis(100));
            let t = Instant::now();
            let _ = probe.sample(want);
            let e = t.elapsed();
            best = best.min(e);
            worst = worst.max(e);
            sum += e;
            print!("{:.0} ", e.as_secs_f64() * 1000.0);
        }
        println!(
            "
{label}: average {:?}, best {best:?}, worst {worst:?}",
            sum / rounds
        );
    };
    measure("full", Want::default());
    measure("no process GPU", Want { process_gpu: false, ..Want::default() });
    measure("no hardware", Want { hardware: false, process_gpu: false, ..Want::default() });
    measure(
        "processes, no details",
        Want { hardware: false, process_gpu: false, process_details: false, processes: true },
    );
    measure("totals only", Want { processes: false, process_details: false, process_gpu: false, hardware: false });
}

/// A window of a child of this test, off screen and never activated: `Terminate` closes it, and a
/// process without a window answers `Unsupported`.
#[test]
fn terminate_closes_the_window_of_our_child() {
    let script = r#"
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'
public class SoyuzTestForm : System.Windows.Forms.Form {
    protected override bool ShowWithoutActivation { get { return true; } }
    public SoyuzTestForm() {
        StartPosition = System.Windows.Forms.FormStartPosition.Manual;
        Location = new System.Drawing.Point(-4000, -4000);
        ShowInTaskbar = false;
    }
}
'@
[System.Windows.Forms.Application]::Run((New-Object SoyuzTestForm))
"#;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("form.ps1");
    std::fs::write(&file, script).unwrap();
    let log = std::fs::File::create(dir.path().join("err.txt")).unwrap();
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .expect("spawn powershell");
    let pid = child.id();
    let mut probe = Probe::new();
    let key = probe.sample(Want::default()).processes.iter().find(|p| p.key.pid == pid).expect("listed").key;

    // The window takes a moment to appear; until then the answer is `Unsupported`.
    let deadline = Instant::now() + Duration::from_secs(60);
    let closed = loop {
        match sysmon::signal(key, ProcSignal::Terminate) {
            Ok(()) => break true,
            Err(ControlError::Unsupported) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => {
                eprintln!("terminate: {e}");
                break false;
            }
        }
    };
    let mut exited = false;
    if closed {
        let until = Instant::now() + Duration::from_secs(20);
        while Instant::now() < until {
            if child.try_wait().expect("try_wait").is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let err = std::fs::read_to_string(dir.path().join("err.txt")).unwrap_or_default();
    assert!(closed, "the close request reached the window; child said: {err}");
    assert!(exited, "the program ended after the close request");

    // The windowless child: Terminate says so, and Kill is what is left.
    let mut quiet = spawn_child();
    let qpid = quiet.id();
    let qkey = probe.sample(Want::default()).processes.iter().find(|p| p.key.pid == qpid).expect("listed").key;
    assert_eq!(sysmon::signal(qkey, ProcSignal::Terminate), Err(ControlError::Unsupported));
    sysmon::signal(qkey, ProcSignal::Kill).expect("kill");
    let _ = quiet.wait();
}

/// The service calls report failure on a name that does not exist (no real service is touched).
#[test]
fn service_control_reports_an_unknown_service() {
    let result = sysmon::service_control("soyuz-test-no-such-service", sysmon::ServiceOp::Start);
    assert!(result.is_err(), "{result:?}");
}
