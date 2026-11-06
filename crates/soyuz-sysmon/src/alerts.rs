//! Threshold alerts. An alert fires once when a value stayed above its limit for the sustain
//! time, and re-arms when the value drops 5 points below the limit. A frame with a gap (suspend,
//! clock jump) restarts the timers.

use soyuz_core::config::monitor::Monitor;

use crate::frame::Frame;

const REARM_MARGIN: f32 = 5.0;

/// What went over its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Cpu,
    Memory,
    Temperature,
}

/// One alert. `snapshot_id` is set when the hub saved a snapshot for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub kind: AlertKind,
    pub value: f32,
    pub limit: f32,
    pub snapshot_id: Option<String>,
}

/// The alert part of the settings. A limit of 0 is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlertLimits {
    pub cpu_percent: u32,
    pub memory_percent: u32,
    pub temperature_c: u32,
    pub sustain_secs: u32,
}

impl AlertLimits {
    pub fn from_config(cfg: &Monitor) -> Self {
        AlertLimits {
            cpu_percent: cfg.alert_cpu_percent,
            memory_percent: cfg.alert_memory_percent,
            temperature_c: cfg.alert_temperature_c,
            sustain_secs: cfg.alert_sustain_secs,
        }
    }

    /// Any limit set: sampling must go on with no monitor tab open.
    pub fn any(&self) -> bool {
        self.cpu_percent > 0 || self.memory_percent > 0 || self.temperature_c > 0
    }
}

#[derive(Default)]
struct Watch {
    above_ms: u64,
    fired: bool,
}

impl Watch {
    fn step(
        &mut self,
        kind: AlertKind,
        value: Option<f32>,
        limit: u32,
        elapsed_ms: u64,
        sustain_ms: u64,
    ) -> Option<Alert> {
        let (Some(value), true) = (value, limit > 0) else {
            *self = Watch::default();
            return None;
        };
        let limit = limit as f32;
        if value > limit {
            self.above_ms = self.above_ms.saturating_add(elapsed_ms);
            if !self.fired && self.above_ms >= sustain_ms {
                self.fired = true;
                return Some(Alert { kind, value, limit, snapshot_id: None });
            }
        } else {
            self.above_ms = 0;
            if value < limit - REARM_MARGIN {
                self.fired = false;
            }
        }
        None
    }
}

/// Follows the frames and fires alerts.
#[derive(Default)]
pub struct AlertWatcher {
    cpu: Watch,
    memory: Watch,
    temperature: Watch,
}

impl AlertWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// With the limits of the settings.
    pub fn check(&mut self, frame: &Frame, cfg: &Monitor) -> Vec<Alert> {
        self.check_limits(frame, &AlertLimits::from_config(cfg))
    }

    /// The alerts this frame fires. The first frame of a run has no valid CPU figure and is skipped.
    pub fn check_limits(&mut self, frame: &Frame, limits: &AlertLimits) -> Vec<Alert> {
        if frame.first {
            return Vec::new();
        }
        if frame.gap {
            // Keep which alerts have fired, restart the clocks.
            self.cpu.above_ms = 0;
            self.memory.above_ms = 0;
            self.temperature.above_ms = 0;
            return Vec::new();
        }
        let sustain_ms = u64::from(limits.sustain_secs) * 1000;
        let e = frame.elapsed_ms;
        let mut out = Vec::new();
        out.extend(self.cpu.step(AlertKind::Cpu, Some(frame.cpu.percent), limits.cpu_percent, e, sustain_ms));
        out.extend(self.memory.step(
            AlertKind::Memory,
            Some(frame.memory.percent),
            limits.memory_percent,
            e,
            sustain_ms,
        ));
        out.extend(self.temperature.step(
            AlertKind::Temperature,
            frame.max_temp_c(),
            limits.temperature_c,
            e,
            sustain_ms,
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SensorKind;
    use crate::SensorRaw;

    fn frame(cpu: f32, elapsed_ms: u64) -> Frame {
        let mut f = Frame::default();
        f.cpu.percent = cpu;
        f.elapsed_ms = elapsed_ms;
        f
    }

    fn limits() -> AlertLimits {
        AlertLimits { cpu_percent: 80, memory_percent: 0, temperature_c: 0, sustain_secs: 10 }
    }

    #[test]
    fn fires_once_after_the_sustain_time() {
        let mut w = AlertWatcher::new();
        let l = limits();
        for _ in 0..9 {
            assert!(w.check_limits(&frame(90.0, 1000), &l).is_empty());
        }
        let fired = w.check_limits(&frame(95.0, 1000), &l);
        assert_eq!(fired.len(), 1);
        assert_eq!((fired[0].kind, fired[0].value, fired[0].limit), (AlertKind::Cpu, 95.0, 80.0));
        for _ in 0..50 {
            assert!(w.check_limits(&frame(90.0, 1000), &l).is_empty(), "once");
        }
    }

    #[test]
    fn dipping_below_restarts_the_timer() {
        let mut w = AlertWatcher::new();
        let l = limits();
        for _ in 0..9 {
            w.check_limits(&frame(90.0, 1000), &l);
        }
        w.check_limits(&frame(70.0, 1000), &l);
        for _ in 0..9 {
            assert!(w.check_limits(&frame(90.0, 1000), &l).is_empty());
        }
        assert_eq!(w.check_limits(&frame(90.0, 1000), &l).len(), 1);
    }

    #[test]
    fn re_arms_only_five_points_below_the_limit() {
        let mut w = AlertWatcher::new();
        let l = AlertLimits { sustain_secs: 0, ..limits() };
        assert_eq!(w.check_limits(&frame(90.0, 1000), &l).len(), 1);
        w.check_limits(&frame(78.0, 1000), &l);
        assert!(w.check_limits(&frame(90.0, 1000), &l).is_empty(), "78 is within the margin");
        w.check_limits(&frame(74.0, 1000), &l);
        assert_eq!(w.check_limits(&frame(90.0, 1000), &l).len(), 1);
    }

    #[test]
    fn a_gap_restarts_the_timers_and_the_first_frame_counts_for_nothing() {
        let mut w = AlertWatcher::new();
        let l = limits();
        let mut first = frame(100.0, 60_000);
        first.first = true;
        assert!(w.check_limits(&first, &l).is_empty());
        for _ in 0..9 {
            w.check_limits(&frame(90.0, 1000), &l);
        }
        let mut gap = frame(90.0, 3_600_000);
        gap.gap = true;
        assert!(w.check_limits(&gap, &l).is_empty());
        assert!(w.check_limits(&frame(90.0, 1000), &l).is_empty(), "not a whole sleep's worth of sustain");
    }

    #[test]
    fn off_means_off_and_turning_off_resets() {
        let mut w = AlertWatcher::new();
        let off = AlertLimits { sustain_secs: 0, ..Default::default() };
        assert!(w.check_limits(&frame(100.0, 1000), &off).is_empty());
        let on = AlertLimits { sustain_secs: 0, ..limits() };
        assert_eq!(w.check_limits(&frame(100.0, 1000), &on).len(), 1);
        w.check_limits(&frame(100.0, 1000), &off);
        assert_eq!(w.check_limits(&frame(100.0, 1000), &on).len(), 1);
    }

    #[test]
    fn memory_and_temperature() {
        let mut w = AlertWatcher::new();
        let l = AlertLimits { cpu_percent: 0, memory_percent: 90, temperature_c: 80, sustain_secs: 0 };
        let mut f = frame(100.0, 1000);
        f.memory.percent = 95.0;
        f.sensors = vec![
            SensorRaw { kind: SensorKind::Temperature, value: 85.0, ..Default::default() },
            SensorRaw { kind: SensorKind::Fan, value: 5000.0, ..Default::default() },
        ];
        let kinds: Vec<_> = w.check_limits(&f, &l).iter().map(|a| a.kind).collect();
        assert_eq!(kinds, [AlertKind::Memory, AlertKind::Temperature]);
        // No temperature known: no alert, never a fake value.
        let mut w = AlertWatcher::new();
        assert!(w.check_limits(&frame(0.0, 1000), &l).is_empty());
    }

    #[test]
    fn limits_come_from_the_settings() {
        let cfg = Monitor { alert_cpu_percent: 90, alert_sustain_secs: 3, ..Default::default() };
        let l = AlertLimits::from_config(&cfg);
        assert_eq!(l.cpu_percent, 90);
        assert!(l.any());
        assert!(!AlertLimits::from_config(&Monitor::default()).any());
        let mut w = AlertWatcher::new();
        let f = frame(95.0, 3000);
        assert_eq!(w.check(&f, &cfg).len(), 1);
    }
}
