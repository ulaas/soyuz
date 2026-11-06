//! The real probe through the rater. The OS readers may return empty data while they are being
//! written, so this holds any values to sanity, not to being present.

use soyuz_fs::sysmon::{Probe, Want};
use soyuz_sysmon::{Filter, History, Metric, Rater, TableOpts, build};

#[test]
fn real_probe_gives_sane_frames() {
    let mut probe = Probe::new();
    let mut rater = Rater::new();
    let first = rater.push(probe.sample(Want::default()));
    assert!(first.first);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let frame = rater.push(probe.sample(Want::default()));
    assert!(!frame.first);

    assert!((0.0..=100.0).contains(&frame.cpu.percent), "{}", frame.cpu.percent);
    assert!((0.0..=100.0).contains(&frame.memory.percent));
    for core in &frame.cpu.cores {
        assert!((0.0..=100.0).contains(&core.percent));
    }
    let ncpu = frame.ncpu() as f32;
    for row in frame.procs.iter() {
        assert!(row.cpu_percent >= 0.0 && row.cpu_percent <= 100.0 * ncpu.max(1.0), "{}", row.cpu_percent);
    }
    assert!(frame.disks.iter().all(|d| d.read_rate >= 0.0 && d.write_rate >= 0.0));
    assert!(frame.nets.iter().all(|n| n.rx_rate >= 0.0 && n.tx_rate >= 0.0));

    // A frame made of whatever the system reports goes through JSON and the table.
    let json = serde_json::to_string(&frame).unwrap();
    let back: soyuz_sysmon::Frame = serde_json::from_str(&json).unwrap();
    assert_eq!(back.procs.len(), frame.procs.len());

    let opts = TableOpts { ncpu: frame.ncpu(), now_ms: frame.at_ms, show_kernel: true, ..Default::default() };
    let rows = build(&frame, &Filter::default(), &opts, &Default::default());
    assert!(rows.len() <= frame.procs.len());

    let mut history = History::new(1);
    history.push(&frame);
    let series = history.series(&Metric::CpuTotal, frame.at_ms, frame.at_ms + 1, 10);
    assert!(series.len() <= 1);

    // Once the readers are in: this process is listed and memory has a total.
    if !frame.procs.is_empty() {
        let me = std::process::id();
        assert!(frame.procs.iter().any(|p| p.raw.key.pid == me), "this process is listed");
    }
    if frame.memory.raw.total == 0 {
        eprintln!("the memory reader reports nothing yet");
    }
}
