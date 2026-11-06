//! Words and numbers of the monitor pages, shared by both frontends.

use std::time::{Duration, UNIX_EPOCH};

use soyuz_core::config::Config;
use soyuz_core::config::monitor::{MonitorPage, ProcColumn, ProcView, TempUnit};
use soyuz_core::{SizeStyle, format_size};
use soyuz_sysmon::{
    Alert, AlertKind, CellValue, ControlError, ProcPriority, ProcSignal, ProcState, SensorKind, ServiceOp,
    ServiceState, format_duration, format_rate,
};

/// The title of a page.
pub fn page_title(p: MonitorPage) -> &'static str {
    // i18n-dynamic: monitor-page-
    crate::i18n::t_dyn(&format!("monitor-page-{}", p.id()))
}

/// The title of a column.
pub fn column_title(c: ProcColumn) -> &'static str {
    // i18n-dynamic: monitor-col-
    crate::i18n::t_dyn(&format!("monitor-col-{}", c.id().replace('_', "-")))
}

/// What a column shows, for a tooltip or the chooser.
pub fn column_desc(c: ProcColumn) -> &'static str {
    // i18n-dynamic: monitor-coldesc-
    crate::i18n::t_dyn(&format!("monitor-coldesc-{}", c.id().replace('_', "-")))
}

pub fn view_title(v: ProcView) -> &'static str {
    match v {
        ProcView::List => tr!("monitor-view-list"),
        ProcView::Tree => tr!("monitor-view-tree"),
        ProcView::ByName => tr!("monitor-view-by-name"),
        ProcView::ByUser => tr!("monitor-view-by-user"),
    }
}

pub fn state(s: ProcState) -> &'static str {
    match s {
        ProcState::Running => tr!("monitor-state-running"),
        ProcState::Sleeping => tr!("monitor-state-sleeping"),
        ProcState::DiskWait => tr!("monitor-state-disk-wait"),
        ProcState::Stopped => tr!("monitor-state-stopped"),
        ProcState::Zombie => tr!("monitor-state-zombie"),
        ProcState::Idle => tr!("monitor-state-idle"),
        ProcState::Suspended => tr!("monitor-state-suspended"),
        ProcState::Unknown => tr!("monitor-state-unknown"),
    }
}

pub fn priority(p: ProcPriority) -> &'static str {
    match p {
        ProcPriority::Idle => tr!("monitor-priority-idle"),
        ProcPriority::BelowNormal => tr!("monitor-priority-below-normal"),
        ProcPriority::Normal => tr!("monitor-priority-normal"),
        ProcPriority::AboveNormal => tr!("monitor-priority-above-normal"),
        ProcPriority::High => tr!("monitor-priority-high"),
        ProcPriority::Realtime => tr!("monitor-priority-realtime"),
    }
}

pub fn signal(s: ProcSignal) -> &'static str {
    match s {
        ProcSignal::Terminate => tr!("monitor-action-end"),
        ProcSignal::Kill => tr!("monitor-action-kill"),
        ProcSignal::Suspend => tr!("monitor-action-suspend"),
        ProcSignal::Resume => tr!("monitor-action-resume"),
        ProcSignal::Hangup => tr!("monitor-action-hangup"),
        ProcSignal::Interrupt => tr!("monitor-action-interrupt"),
    }
}

pub fn service_op(op: ServiceOp) -> &'static str {
    match op {
        ServiceOp::Start => tr!("monitor-service-start"),
        ServiceOp::Stop => tr!("monitor-service-stop"),
        ServiceOp::Restart => tr!("monitor-service-restart"),
    }
}

pub fn service_state(s: ServiceState) -> &'static str {
    match s {
        ServiceState::Running => tr!("monitor-state-running"),
        ServiceState::Stopped => tr!("monitor-state-stopped"),
        ServiceState::Starting => tr!("monitor-service-starting"),
        ServiceState::Stopping => tr!("monitor-service-stopping"),
        ServiceState::Paused => tr!("monitor-service-paused"),
        ServiceState::Failed => tr!("monitor-service-failed-state"),
        ServiceState::Unknown => tr!("monitor-state-unknown"),
    }
}

pub fn control_error(e: &ControlError) -> String {
    match e {
        ControlError::Gone => tr!("monitor-error-gone").to_string(),
        ControlError::Denied => tr!("monitor-error-denied").to_string(),
        ControlError::Unsupported => tr!("monitor-error-unsupported").to_string(),
        ControlError::Other(s) => s.clone(),
    }
}

pub fn alert_title(a: &Alert) -> String {
    match a.kind {
        AlertKind::Cpu => tr!("monitor-alert-cpu"),
        AlertKind::Memory => tr!("monitor-alert-memory"),
        AlertKind::Temperature => tr!("monitor-alert-temperature"),
    }
    .to_string()
}

pub fn alert_body(a: &Alert) -> String {
    let value = format!("{:.0}", a.value);
    let limit = format!("{:.0}", a.limit);
    let mut s = tr!("monitor-alert-body", value = value.as_str(), limit = limit.as_str());
    if a.snapshot_id.is_some() {
        s.push('\n');
        s.push_str(tr!("monitor-alert-snapshot"));
    }
    s
}

/// The label of a snapshot taken without a name.
pub fn default_snapshot_label() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `12.3%`.
pub fn percent(v: f32) -> String {
    if v >= 100.0 { format!("{v:.0}%") } else { format!("{v:.1}%") }
}

/// A temperature in the configured unit (`41 °C`).
pub fn temperature(c: f32, unit: TempUnit) -> String {
    match unit {
        TempUnit::Celsius => format!("{c:.0} °C"),
        TempUnit::Fahrenheit => format!("{:.0} °F", c * 9.0 / 5.0 + 32.0),
    }
}

/// A sensor's value and unit.
pub fn sensor_value(kind: SensorKind, value: f32, unit: TempUnit) -> String {
    match kind {
        SensorKind::Temperature => temperature(value, unit),
        SensorKind::Fan => format!("{value:.0} RPM"),
        SensorKind::Power => format!("{value:.1} W"),
        SensorKind::Voltage => format!("{value:.2} V"),
        SensorKind::Current => format!("{value:.2} A"),
    }
}

/// `3:04:05.20` from milliseconds of CPU time.
pub fn cpu_time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}.{:02}", s / 3600, (s / 60) % 60, s % 60, (ms % 1000) / 10)
}

/// A link or transfer speed in bits (`1 Gbit/s`).
pub fn bits(bps: u64) -> String {
    match bps {
        b if b >= 1_000_000_000 => format!("{:.1} Gbit/s", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{:.0} Mbit/s", b as f64 / 1e6),
        b if b >= 1_000 => format!("{:.0} kbit/s", b as f64 / 1e3),
        b => format!("{b} bit/s"),
    }
}

/// `2 d 3 h`, `4 h 5 min`, `12 s`: an uptime.
pub fn uptime(secs: u64) -> String {
    format_duration(secs)
}

/// A rate in bytes per second in the configured size style.
pub fn rate(bytes_per_sec: f64, style: SizeStyle) -> String {
    match style {
        SizeStyle::Binary => format_rate(bytes_per_sec),
        _ => format!("{}/s", format_size(bytes_per_sec.max(0.0) as u64, style)),
    }
}

/// A short count: `1,234`.
pub fn count(n: u64) -> String {
    soyuz_core::locale::group_int(n)
}

/// A unix-millisecond time in the configured date format.
pub fn time(ms: i64, config: &Config) -> String {
    let t = UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64);
    soyuz_core::format_time(t, &config.general.date_format)
}

/// A cell as text. A value the OS does not offer is a dash, never 0.
pub fn cell_text(v: &CellValue, config: &Config) -> String {
    let style = config.general.size_style;
    match v {
        CellValue::Text(s) => s.clone(),
        CellValue::Count(n) => count(*n),
        CellValue::Bytes(b) => format_size(*b, style),
        CellValue::Rate(r) => rate(*r, style),
        CellValue::Percent(p) => percent(*p),
        CellValue::Millis(ms) => cpu_time(*ms),
        CellValue::Seconds(s) => format_duration(*s),
        CellValue::UnixMs(t) => time(*t, config),
        CellValue::State(s) => state(*s).to_string(),
        CellValue::Priority(p) => priority(*p).to_string(),
        CellValue::Bool(b) => if *b { tr!("monitor-yes") } else { tr!("monitor-no") }.to_string(),
        CellValue::Int(i) => i.to_string(),
        CellValue::Missing => "–".to_string(),
    }
}

/// How hot a cell is, 0..1, for tinting: load columns only. `total_mem` scales memory.
pub fn cell_heat(col: ProcColumn, v: &CellValue, total_mem: u64) -> Option<f32> {
    let share = match (col, v) {
        (ProcColumn::Cpu | ProcColumn::Gpu, CellValue::Percent(p)) => *p / 100.0,
        (ProcColumn::MemoryPercent, CellValue::Percent(p)) => *p / 25.0,
        (ProcColumn::Memory | ProcColumn::Private, CellValue::Bytes(b)) if total_mem > 0 => {
            (*b as f64 / total_mem as f64 * 4.0) as f32
        }
        (ProcColumn::DiskRead | ProcColumn::DiskWrite, CellValue::Rate(r)) => (*r / (100.0 * 1024.0 * 1024.0)) as f32,
        _ => return None,
    };
    (share > 0.005).then_some(share.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(percent(12.34), "12.3%");
        assert_eq!(percent(250.0), "250%");
        assert_eq!(temperature(100.0, TempUnit::Fahrenheit), "212 °F");
        assert_eq!(cpu_time(3_725_500), "1:02:05.50");
        assert_eq!(bits(1_000_000_000), "1.0 Gbit/s");
    }

    #[test]
    fn missing_is_a_dash() {
        let c = Config::default();
        assert_eq!(cell_text(&CellValue::Missing, &c), "–");
    }

    #[test]
    fn heat_only_for_load_columns() {
        assert!(cell_heat(ProcColumn::Pid, &CellValue::Int(1), 100).is_none());
        assert_eq!(cell_heat(ProcColumn::Cpu, &CellValue::Percent(50.0), 100), Some(0.5));
        assert!(cell_heat(ProcColumn::Cpu, &CellValue::Percent(0.0), 100).is_none());
    }

    #[test]
    fn every_column_and_page_has_words() {
        for l in [soyuz_core::Locale::EnUs, soyuz_core::Locale::TrTr] {
            for c in ProcColumn::ALL {
                let id = c.id().replace('_', "-");
                assert!(crate::i18n::has_in(&format!("monitor-col-{id}"), l), "monitor-col-{id} ({l:?})");
                assert!(crate::i18n::has_in(&format!("monitor-coldesc-{id}"), l), "monitor-coldesc-{id} ({l:?})");
            }
            for p in MonitorPage::ALL {
                assert!(crate::i18n::has_in(&format!("monitor-page-{}", p.id()), l), "monitor-page-{} ({l:?})", p.id());
            }
        }
    }
}
