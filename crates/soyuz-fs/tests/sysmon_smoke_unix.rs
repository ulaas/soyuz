//! Real readings on Linux and macOS: this process and a child it spawns. Only that child is
//! ever signalled. `cargo test -p soyuz-fs sysmon -- --nocapture` prints a sample.

#![cfg(unix)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use soyuz_fs::sysmon::{
    self, ControlError, DetailsWant, Probe, ProcKey, ProcPriority, ProcRaw, ProcSignal, ProcState, Want,
};

/// A child that sleeps until killed; reaped on drop, whatever a test did.
struct Sleeper(Child);

impl Sleeper {
    fn spawn() -> Self {
        Sleeper(Command::new("sleep").arg("300").stdin(Stdio::null()).spawn().expect("spawn sleep"))
    }
    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn find(probe: &mut Probe, pid: u32) -> Option<ProcRaw> {
    probe.sample(Want { hardware: false, ..Want::default() }).processes.into_iter().find(|p| p.key.pid == pid)
}

/// Waits for a state; a signalled process needs a moment. Zombie also ends the wait for kills.
fn wait_state(probe: &mut Probe, pid: u32, wanted: ProcState) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if find(probe, pid).is_some_and(|p| p.state == wanted) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn sysmon_sample_is_sane() {
    let mut probe = Probe::new();
    let a = probe.sample(Want::default());
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    let b = probe.sample(Want::default());
    let full = started.elapsed();

    assert!(b.at_ms > 1_500_000_000_000);
    assert!(b.mono_ms >= a.mono_ms + 200);
    assert!(b.uptime_secs > 0);
    assert!(!b.cores.is_empty());
    assert!(b.memory.total > 0);
    assert!(b.memory.available <= b.memory.total);
    assert!(b.processes.len() > 1);

    let me = b.processes.iter().find(|p| p.key.pid == std::process::id()).expect("this process is listed");
    assert!(me.key.start_ms > 1_500_000_000_000);
    assert!(me.key.start_raw > 0);
    assert!(me.rss > 0);
    assert!(me.threads >= 1);
    assert!(!me.name.is_empty());
    assert!(me.exe.is_some(), "own path readable");
    assert!(me.cmdline.is_some(), "own command line readable");
    assert!(me.user.is_some(), "own user known");
    assert!(!me.kernel);
    // The key is stable across samples.
    let earlier = a.processes.iter().find(|p| p.key.pid == std::process::id()).unwrap();
    assert_eq!(earlier.key, me.key);

    println!("--- full sample: {full:?}, {} processes ---", b.processes.len());
    println!("uptime {} s, load {:?}, threads {:?}", b.uptime_secs, b.load, b.total_threads);
    println!("cpu_total {:?}", b.cpu_total);
    println!("cores {}, core0 {:?}", b.cores.len(), b.cores[0]);
    println!("ctx {:?} intr {:?} temp {:?}", b.ctx_switches, b.interrupts, b.cpu_temp_c);
    println!("memory {:?}", b.memory);
    println!("disks {:#?}", b.disks);
    println!("nets {:#?}", b.nets);
    println!("gpus {:#?}", b.gpus);
    println!("sensors {:?}", b.sensors);
    println!("batteries {:?}", b.batteries);
    println!("me {me:#?}");
}

#[test]
fn sysmon_sample_cost() {
    let mut probe = Probe::new();
    let _ = probe.sample(Want::default());
    for (label, want) in [
        ("full", Want::default()),
        ("no gpu", Want { process_gpu: false, ..Want::default() }),
        ("no details", Want { process_details: false, process_gpu: false, hardware: true, processes: true }),
        ("totals", Want { processes: false, process_details: false, process_gpu: false, hardware: false }),
    ] {
        let started = Instant::now();
        let mut n = 0;
        for _ in 0..5 {
            n = probe.sample(want).processes.len();
        }
        println!("cost {label}: {:?} per sample, {n} processes", started.elapsed() / 5);
    }
}

#[test]
fn sysmon_lists_do_not_fail() {
    let info = sysmon::system_info();
    println!("{info:#?}");
    assert!(info.logical_cores > 0);
    assert!(info.memory_total > 0);
    assert!(!info.os.is_empty());
    assert!(!info.kernel.is_empty());
    assert!(info.boot_unix > 1_000_000_000);

    let c = sysmon::connections();
    println!("connections: {}, first {:?}", c.len(), c.first());
    let s = sysmon::services();
    println!("services: {}, first {:?}", s.len(), s.first());
    let st = sysmon::startup_items();
    println!("startup: {}, first {:?}", st.len(), st.first());
}

#[test]
fn sysmon_finds_own_listening_socket() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let c = sysmon::connections();
    let mine = c.iter().find(|c| c.local_port == port).expect("own socket listed");
    assert_eq!(mine.local_addr, "127.0.0.1");
    assert_eq!(mine.state, "LISTEN");
    assert_eq!(mine.pid, Some(std::process::id()));
}

#[test]
fn sysmon_details_of_self() {
    let mut probe = Probe::new();
    let me = find(&mut probe, std::process::id()).unwrap();
    let all = DetailsWant { threads: true, modules: true, open_files: true, environment: true };
    let d = sysmon::process_details(me.key, all).unwrap();
    assert!(d.exe.is_some());
    assert!(d.cwd.is_some());
    assert!(d.cmdline.is_some());
    assert!(!d.threads.is_empty());
    assert!(!d.environment.is_empty());
    assert_eq!(me.exe.as_deref(), d.exe.as_deref());
    // Only what is asked for gets read.
    let bare = sysmon::process_details(me.key, DetailsWant::default()).unwrap();
    assert!(bare.exe.is_some() && bare.cmdline.is_some());
    assert!(bare.threads.is_empty() && bare.modules.is_empty() && bare.open_files.is_empty());
    assert!(bare.environment.is_empty());
    println!("details: {} threads, {} modules, {} files", d.threads.len(), d.modules.len(), d.open_files.len());
}

#[test]
fn sysmon_control_of_own_child() {
    let child = Sleeper::spawn();
    let pid = child.pid();
    let mut probe = Probe::new();
    // The child may still be between fork and exec on the first look.
    let end = Instant::now() + Duration::from_secs(5);
    let key = loop {
        if let Some(p) = find(&mut probe, pid).filter(|p| p.name.contains("sleep")) {
            break p.key;
        }
        assert!(Instant::now() < end, "child not listed");
        std::thread::sleep(Duration::from_millis(10));
    };

    // A wrong start time must refuse, and leave the child alone.
    let stale = ProcKey { start_raw: key.start_raw - 1, ..key };
    assert_eq!(sysmon::signal(stale, ProcSignal::Kill), Err(ControlError::Gone));
    assert!(find(&mut probe, pid).is_some());

    sysmon::signal(key, ProcSignal::Suspend).unwrap();
    assert!(wait_state(&mut probe, pid, ProcState::Stopped), "stopped");
    sysmon::signal(key, ProcSignal::Resume).unwrap();
    assert!(wait_state(&mut probe, pid, ProcState::Sleeping), "resumed");

    // Raising the nice value needs no privilege.
    sysmon::set_priority(key, ProcPriority::BelowNormal).unwrap();
    let p = find(&mut probe, pid).unwrap();
    assert_eq!(p.nice, Some(10));
    assert_eq!(p.priority, Some(ProcPriority::BelowNormal));

    #[cfg(target_os = "linux")]
    {
        let cpus = sysmon::affinity(key).unwrap();
        assert!(!cpus.is_empty());
        sysmon::set_affinity(key, &cpus[..1]).unwrap();
        assert_eq!(sysmon::affinity(key).unwrap(), cpus[..1]);
        assert!(sysmon::set_affinity(key, &[]).is_err());
    }
    #[cfg(target_os = "macos")]
    assert_eq!(sysmon::affinity(key), Err(ControlError::Unsupported));

    sysmon::signal(key, ProcSignal::Kill).unwrap();
    assert!(wait_state(&mut probe, pid, ProcState::Zombie), "killed, unreaped");
}

#[test]
fn sysmon_control_refuses_dangerous_pids() {
    // 0 and values that turn negative would address process groups or everything.
    for pid in [0, u32::MAX, i32::MAX as u32 + 1, u32::MAX - 1] {
        let key = ProcKey { pid, ..ProcKey::default() };
        assert_eq!(sysmon::signal(key, ProcSignal::Hangup), Err(ControlError::Gone), "pid {pid}");
    }
    let gone = ProcKey { pid: i32::MAX as u32, start_ms: 1, start_raw: 1 };
    assert_eq!(sysmon::signal(gone, ProcSignal::Kill), Err(ControlError::Gone));
    assert!(sysmon::process_details(gone, DetailsWant::default()).is_err());
}

#[test]
fn sysmon_probe_can_move_to_a_worker_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<Probe>();
}

/// The budget of the plan: 400 processes in under 30 ms in a release build. Spawns its own
/// sleepers, so it is opt-in: `cargo test --release -p soyuz-fs --test sysmon_smoke_unix -- --ignored`.
#[test]
#[ignore = "spawns hundreds of child processes"]
fn sysmon_cost_with_400_processes() {
    let mut probe = Probe::new();
    let have = probe.sample(Want { hardware: false, ..Want::default() }).processes.len();
    let children: Vec<Sleeper> = (have..400).map(|_| Sleeper::spawn()).collect();
    let _ = probe.sample(Want::default());
    for (label, want) in [
        ("full", Want::default()),
        ("no hardware", Want { hardware: false, ..Want::default() }),
        ("no gpu, no hardware", Want { hardware: false, process_gpu: false, ..Want::default() }),
        ("no details", Want { process_details: false, process_gpu: false, hardware: false, processes: true }),
    ] {
        let started = Instant::now();
        let mut n = 0;
        for _ in 0..10 {
            n = probe.sample(want).processes.len();
        }
        println!("cost {label}: {:?} per sample, {n} processes", started.elapsed() / 10);
    }
    drop(children);
}

#[test]
fn sysmon_text_is_shared_between_samples() {
    let mut probe = Probe::new();
    let a = find(&mut probe, std::process::id()).unwrap();
    let b = find(&mut probe, std::process::id()).unwrap();
    // Reference counts, not copies: the point of the per-process cache.
    assert!(std::sync::Arc::ptr_eq(&a.name, &b.name));
    assert!(std::sync::Arc::ptr_eq(a.cmdline.as_ref().unwrap(), b.cmdline.as_ref().unwrap()));
    assert!(std::sync::Arc::ptr_eq(a.exe.as_ref().unwrap(), b.exe.as_ref().unwrap()));
    assert!(std::sync::Arc::ptr_eq(a.user.as_ref().unwrap(), b.user.as_ref().unwrap()));
}

#[test]
fn sysmon_signal_after_exit_is_gone() {
    let mut child = Sleeper::spawn();
    let pid = child.pid();
    let mut probe = Probe::new();
    let end = Instant::now() + Duration::from_secs(5);
    let key = loop {
        if let Some(p) = find(&mut probe, pid).filter(|p| p.name.contains("sleep")) {
            break p.key;
        }
        assert!(Instant::now() < end, "child not listed");
        std::thread::sleep(Duration::from_millis(10));
    };
    sysmon::signal(key, ProcSignal::Kill).unwrap();
    child.0.wait().unwrap();
    // Reaped: the PID is free, and whatever may reuse it must not get the signal.
    assert_eq!(sysmon::signal(key, ProcSignal::Kill), Err(ControlError::Gone));
    assert_eq!(sysmon::set_priority(key, ProcPriority::Normal), Err(ControlError::Gone));
}
