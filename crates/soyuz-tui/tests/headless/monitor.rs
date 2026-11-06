//! Headless tests of the system monitor pages (`soyuz_tui::view::monitor`). Each runs in its
//! own sandbox; most feed made-up frames (`MonitorState::feed_test_frames`), so no number on
//! the screen is live. No test signals a process: the kill test only opens the confirmation
//! on a made-up pid and cancels it. The snapshot test uses the real sampler, whose data folder
//! is the sandbox's config directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::cmd::Cmd;
use soyuz_core::Location;
use soyuz_core::config::monitor::{MonitorPage, ProcColumn, ProcView};
use soyuz_core::config::{Action, Config, TuiColorMode, TuiIcons};
use soyuz_sysmon::{
    CoreFrame, Counts, CpuFrame, DiskFrame, Frame, GpuRaw, MemFrame, MemoryRaw, NetFrame, ProcKey, ProcRaw, ProcRow,
    ProcState, SensorKind, SensorRaw, SystemInfo,
};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::snapshot::SnapOpts;
use soyuz_tui::view::monitor::MonitorVm;

use crate::common::{self, Run, dialog_title, key, key_mods, shift};

/// The app on a test terminal, driven frame by frame.
struct Rig {
    app: App,
    term: Terminal<TestBackend>,
    io: NullIo,
}

impl Rig {
    fn new(config: Config, data: &std::path::Path, w: u16, h: u16) -> Rig {
        let app = common::new_app(config, data);
        Rig { app, term: Terminal::new(TestBackend::new(w, h)).unwrap(), io: NullIo::default() }
    }

    fn frame(&mut self, events: &[Event]) {
        self.app.frame(&mut self.term, events, &mut self.io).unwrap();
    }

    /// One key, and the frame after it that applied what it asked for.
    fn press(&mut self, e: Event) {
        self.frame(&[e]);
        self.frame(&[]);
    }

    fn typed(&mut self, text: &str) {
        for c in text.chars() {
            self.press(key(KeyCode::Char(c)));
        }
    }

    fn resize(&mut self, w: u16, h: u16) {
        self.term = Terminal::new(TestBackend::new(w, h)).unwrap();
        self.frame(&[]);
    }

    /// Open the monitor (fed with made-up frames unless `real`).
    fn open(&mut self, real: bool) {
        if !real {
            self.app.core.ui.monitor.feed_test_frames(frames(), info());
        }
        self.app.core.cmds.push(Cmd::Action(Action::OpenMonitor));
        self.frame(&[]);
        self.frame(&[]);
        assert!(matches!(self.app.core.ws.pane().location, Location::Monitor));
    }

    fn vm(&self) -> Arc<MonitorVm> {
        self.app.view().and_then(|v| v.focused_pane()).and_then(|p| p.monitor.clone()).expect("the monitor is drawn")
    }

    fn goto(&mut self, page: MonitorPage) {
        let pane = self.app.core.ws.pane().id;
        self.app.core.cmds.push(Cmd::Monitor(pane, soyuz_appcore::models::monitor::MonitorCmd::SetPage(page)));
        self.frame(&[]);
        self.frame(&[]);
        assert_eq!(self.vm().page, page);
    }

    fn screen(&self) -> String {
        common::screen(&self.term)
    }

    /// Frames until `when` holds (the workers report through wake-ups).
    fn wait(&mut self, what: &str, when: impl Fn(&Rig) -> bool) {
        let start = Instant::now();
        loop {
            self.frame(&[]);
            if when(self) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}\n{}", self.screen());
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn config() -> Config {
    let mut c = common::test_config();
    // Fixed numbers on the screen: no alert, no recording.
    c.monitor.background_history = false;
    c.monitor.history_persist = false;
    c
}

fn proc_row(pid: u32, parent: Option<u32>, name: &str, user: &str, cpu: f32, rss_mib: u64) -> ProcRow {
    ProcRow {
        raw: ProcRaw {
            key: ProcKey { pid, start_ms: 1_000 + i64::from(pid), start_raw: 1_000 + u64::from(pid) },
            parent,
            name: name.into(),
            exe: Some(PathBuf::from(format!("/opt/fake/{name}")).into()),
            cmdline: Some(format!("/opt/fake/{name} --serve").into()),
            user: Some(user.into()),
            state: if cpu > 5.0 { ProcState::Running } else { ProcState::Sleeping },
            rss: rss_mib * 1024 * 1024,
            threads: 4,
            ..ProcRaw::default()
        },
        cpu_percent: cpu,
        read_rate: Some(1024.0),
        write_rate: None,
        mem_percent: rss_mib as f32 / 163.84,
    }
}

const NOW: i64 = 1_800_000_000_000;

/// Sixty seconds of a made-up machine; its processes are not real ones.
fn frames() -> Vec<Frame> {
    let procs: Arc<[ProcRow]> = vec![
        proc_row(900_001, None, "init", "root", 0.1, 12),
        proc_row(900_002, Some(900_001), "shell", "ann", 1.5, 40),
        proc_row(900_003, Some(900_002), "builder", "ann", 88.0, 2048),
        proc_row(900_004, Some(900_002), "editor", "ann", 6.0, 512),
        proc_row(900_005, Some(900_001), "database", "svc", 22.0, 4096),
        proc_row(900_006, Some(900_005), "db-worker", "svc", 11.0, 256),
    ]
    .into();
    (0..60)
        .map(|i| {
            let wave = (i as f32 / 6.0).sin() * 30.0 + 40.0;
            Frame {
                at_ms: NOW - (59 - i) * 1000,
                uptime_secs: 90_000,
                load: Some([1.5, 1.25, 1.0]),
                elapsed_ms: 1000,
                cpu: CpuFrame {
                    percent: wave,
                    user: wave * 0.7,
                    system: wave * 0.3,
                    cores: (0..8)
                        .map(|c| CoreFrame {
                            percent: (wave + c as f32 * 5.0).min(100.0),
                            user: 0.0,
                            system: 0.0,
                            mhz: Some(3200),
                        })
                        .collect(),
                    mhz_avg: Some(3200),
                    temp_c: Some(61.0),
                    ..CpuFrame::default()
                },
                memory: MemFrame {
                    raw: MemoryRaw {
                        total: 16 << 30,
                        available: 6 << 30,
                        cached: Some(3 << 30),
                        buffers: Some(1 << 30),
                        swap_total: 4 << 30,
                        swap_used: 1 << 30,
                        ..MemoryRaw::default()
                    },
                    used: 10 << 30,
                    percent: 62.5,
                    swap_percent: 25.0,
                    page_in_rate: Some(2048.0),
                    page_out_rate: Some(0.0),
                },
                disks: vec![DiskFrame {
                    id: "nvme0".into(),
                    model: "Fake NVMe".into(),
                    read_rate: f64::from(wave) * 1e6,
                    write_rate: 2e6,
                    read_iops: 120.0,
                    write_iops: 40.0,
                    busy_percent: Some(35.0),
                    queue: Some(1.5),
                    rotational: Some(false),
                    size: Some(1 << 40),
                    read_bytes: 5 << 30,
                    write_bytes: 2 << 30,
                }],
                nets: vec![NetFrame {
                    id: "eth0".into(),
                    name: "eth0".into(),
                    rx_rate: 3e6,
                    tx_rate: f64::from(wave) * 1e4,
                    rx_bytes: 7 << 30,
                    tx_bytes: 1 << 30,
                    up: true,
                    link_bps: Some(1_000_000_000),
                    addresses: vec!["192.0.2.10".into()],
                    ..NetFrame::default()
                }],
                gpus: vec![GpuRaw {
                    id: "gpu0".into(),
                    name: "Fake GPU".into(),
                    percent: Some(wave / 2.0),
                    mem_used: Some(2 << 30),
                    mem_total: Some(8 << 30),
                    temperature_c: Some(55.0),
                    ..GpuRaw::default()
                }],
                sensors: vec![SensorRaw {
                    id: "t0".into(),
                    group: "coretemp".into(),
                    label: "Package".into(),
                    kind: SensorKind::Temperature,
                    value: 61.0,
                    critical: Some(100.0),
                    ..SensorRaw::default()
                }],
                procs: procs.clone(),
                counts: Counts { processes: 6, threads: 24, handles: None, running: 3 },
                ..Frame::default()
            }
        })
        .collect()
}

fn info() -> SystemInfo {
    SystemInfo {
        hostname: "fakehost".into(),
        os: "Test OS".into(),
        kernel: "1.0".into(),
        arch: "x86_64".into(),
        cpu_model: "Fake CPU".into(),
        physical_cores: 4,
        logical_cores: 8,
        memory_total: 16 << 30,
        ..SystemInfo::default()
    }
}

/// Run `test` in a sandbox.
fn sandboxed(name: &str, test: fn(PathBuf)) {
    match common::relaunch(module_path!(), name, |_| {}) {
        Run::Child(data) => {
            test(data);
            common::ok();
        }
        Run::Parent(_) => {}
    }
}

#[test]
fn the_shortcut_opens_one_tab_and_every_page_draws() {
    sandboxed("the_shortcut_opens_one_tab_and_every_page_draws", |data| {
        let mut r = Rig::new(config(), &data, 120, 40);
        r.app.core.ui.monitor.feed_test_frames(frames(), info());
        r.frame(&[]);
        let shortcut = key_mods(KeyCode::Char('y'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        r.press(shortcut.clone());
        assert!(matches!(r.app.core.ws.pane().location, Location::Monitor));
        let tabs = r.app.core.ws.tabs.len();
        r.press(key(KeyCode::Char('2')));
        assert_eq!(r.vm().page, MonitorPage::Processes);
        for p in MonitorPage::ALL {
            r.goto(p);
            let text = r.screen();
            assert!(!text.trim().is_empty(), "{p:?}");
        }
        // Tab walks the pages; Shift+Tab goes back; the numbers jump.
        r.goto(MonitorPage::Overview);
        r.press(key(KeyCode::Tab));
        assert_eq!(r.vm().page, MonitorPage::Processes);
        r.press(key_mods(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(r.vm().page, MonitorPage::Overview);
        r.press(key(KeyCode::Char('3')));
        assert_eq!(r.vm().page, MonitorPage::Cpu);
        // A second press of the shortcut finds the tab.
        r.press(shortcut);
        assert_eq!(r.app.core.ws.tabs.len(), tabs);
        // The core's global keys still work: the palette opens from a monitor page.
        r.press(common::ctrl('p'));
        assert!(r.app.core.palette.open);
    });
}

#[test]
fn the_process_filter_is_live_with_regex_and_errors() {
    sandboxed("the_process_filter_is_live_with_regex_and_errors", |data| {
        let mut r = Rig::new(config(), &data, 120, 40);
        r.open(false);
        r.goto(MonitorPage::Processes);
        assert_eq!(r.vm().rows, 6);
        r.press(key(KeyCode::Char('/')));
        r.typed("db");
        assert_eq!(r.vm().rows, 1, "{}", r.screen());
        assert!(r.screen().contains("db-worker"));
        assert!(r.screen().contains("1 of 6"), "the match count\n{}", r.screen());
        // Letters typed in the field are not the page's keys.
        assert!(r.app.view().unwrap().dialog.is_none());
        // Whole-query regex: `db` still matches, a broken expression says why and keeps the last good filter.
        r.press(key_mods(KeyCode::Char('r'), KeyModifiers::CONTROL));
        r.press(key(KeyCode::Char('[')));
        assert!(r.vm().filter_error.is_some(), "{}", r.screen());
        assert_eq!(r.vm().rows, 1, "the last good filter stays");
        assert!(r.screen().contains("! "), "{}", r.screen());
        // Esc clears the filter.
        r.press(key(KeyCode::Esc));
        assert_eq!(r.vm().rows, 6);
        assert!(r.vm().filter_error.is_none());
        assert!(!r.screen().contains("0 of 6"));
        // The language: a column term and a size.
        r.press(key_mods(KeyCode::Char('r'), KeyModifiers::CONTROL));
        r.press(key(KeyCode::Char('/')));
        r.typed("user:ann mem>100M");
        assert_eq!(r.vm().rows, 2, "builder and editor\n{}", r.screen());
        // Enter leaves the field with the filter on; Esc on the page clears it.
        r.press(key(KeyCode::Enter));
        assert_eq!(r.vm().rows, 2);
        r.press(key(KeyCode::Esc));
        assert_eq!(r.vm().rows, 6);
    });
}

#[test]
fn views_cycle_and_the_tree_folds() {
    sandboxed("views_cycle_and_the_tree_folds", |data| {
        let mut r = Rig::new(config(), &data, 120, 40);
        r.open(false);
        r.goto(MonitorPage::Processes);
        assert_eq!(r.app.core.config.monitor.view, ProcView::List);
        r.press(shift(KeyCode::Char('V')));
        assert_eq!(r.app.core.config.monitor.view, ProcView::Tree);
        assert!(r.screen().contains("`-"), "ASCII guides\n{}", r.screen());
        r.press(key(KeyCode::Home));
        assert_eq!(r.vm().rows, 6);
        // Left folds the root, its totals show; Right unfolds it.
        r.press(key(KeyCode::Left));
        assert_eq!(r.vm().rows, 1, "{}", r.screen());
        r.press(key(KeyCode::Right));
        assert_eq!(r.vm().rows, 6);
        r.press(key(KeyCode::Char('-')));
        assert_eq!(r.vm().rows, 1);
        r.press(shift(KeyCode::Char('+')));
        assert_eq!(r.vm().rows, 6);
        r.press(shift(KeyCode::Char('V')));
        assert_eq!(r.app.core.config.monitor.view, ProcView::ByName);
        r.press(shift(KeyCode::Char('V')));
        assert_eq!(r.app.core.config.monitor.view, ProcView::ByUser);
        assert!(r.screen().contains("ann (3)"), "{}", r.screen());
        r.press(shift(KeyCode::Char('V')));
        assert_eq!(r.app.core.config.monitor.view, ProcView::List);
        // Sort keys and marks.
        let before = r.app.core.config.monitor.sort_by;
        r.press(shift(KeyCode::Char('>')));
        assert_ne!(r.app.core.config.monitor.sort_by, before);
        r.press(key(KeyCode::Char(' ')));
        assert!(r.screen().contains("1 marked"), "{}", r.screen());
        // The help names the query language; Esc closes it.
        r.press(shift(KeyCode::Char('?')));
        assert!(r.vm().help);
        for _ in 0..3 {
            r.press(key(KeyCode::PageDown));
        }
        assert!(r.screen().contains("mem>100M"), "{}", r.screen());
        r.press(key(KeyCode::Esc));
        assert!(!r.vm().help);
    });
}

#[test]
fn a_column_toggled_in_the_chooser_is_saved() {
    match common::relaunch(module_path!(), "a_column_toggled_in_the_chooser_is_saved", |_| {}) {
        Run::Child(data) => {
            let mut r = Rig::new(config(), &data, 120, 40);
            r.open(false);
            r.goto(MonitorPage::Processes);
            r.press(key(KeyCode::Char('c')));
            assert!(dialog_title(&r.app).is_some_and(|t| t.contains("columns")), "{}", r.screen());
            let n = r.app.core.config.monitor.columns.len();
            // Name is first and always stays.
            r.press(key(KeyCode::Char(' ')));
            assert_eq!(r.app.core.config.monitor.columns.len(), n);
            // The second column goes.
            r.press(key(KeyCode::Down));
            let second = r.app.core.config.monitor.columns[1].id;
            r.press(key(KeyCode::Char(' ')));
            assert_eq!(r.app.core.config.monitor.columns.len(), n - 1);
            assert!(!r.app.core.config.monitor.columns.iter().any(|c| c.id == second));
            // Move a column down.
            let moving = r.app.core.config.monitor.columns[1].id;
            r.press(shift(KeyCode::Down));
            assert_eq!(r.app.core.config.monitor.columns[2].id, moving);
            // The last one on the list, a hidden one, shows now.
            r.press(key(KeyCode::End));
            r.press(key(KeyCode::Char(' ')));
            assert!(r.app.core.config.monitor.columns.iter().any(|c| c.id == ProcColumn::Description));
            r.press(key(KeyCode::Enter));
            assert!(dialog_title(&r.app).is_none());
            r.app.finish();
            let saved = std::fs::read_to_string(soyuz_core::config::config_dir().join("config.toml")).unwrap();
            assert!(saved.contains("description"), "{saved}");
            common::ok();
        }
        Run::Parent(p) => {
            let saved = std::fs::read_to_string(p.config_dir().join("config.toml")).unwrap();
            assert!(saved.contains("[monitor]") || saved.contains("[[monitor.columns]]"), "{saved}");
        }
    }
}

#[test]
fn ending_a_process_asks_first() {
    sandboxed("ending_a_process_asks_first", |data| {
        let mut r = Rig::new(config(), &data, 120, 40);
        r.open(false);
        r.goto(MonitorPage::Processes);
        for k in [key(KeyCode::Char('k')), key(KeyCode::F(9)), shift(KeyCode::Char('K')), key(KeyCode::Char('t'))] {
            r.press(k);
            assert!(dialog_title(&r.app).is_some(), "{}", r.screen());
            // Cancelled: no pid here is real, and none is signalled.
            r.press(key(KeyCode::Esc));
            assert!(dialog_title(&r.app).is_none());
        }
    });
}

#[test]
fn a_snapshot_is_taken_and_compared_with_now() {
    sandboxed("a_snapshot_is_taken_and_compared_with_now", |data| {
        let mut r = Rig::new(config(), &data, 120, 40);
        r.open(true);
        r.goto(MonitorPage::Snapshots);
        r.wait("the first reading", |r| r.app.core.ui.monitor.frame.is_some());
        r.press(key(KeyCode::Char('n')));
        assert!(dialog_title(&r.app).is_some(), "{}", r.screen());
        // The field starts with a date: replace it.
        r.press(key_mods(KeyCode::Char('u'), KeyModifiers::CONTROL));
        r.press(key(KeyCode::Home));
        for _ in 0..40 {
            r.frame(&[key(KeyCode::Delete)]);
        }
        r.typed("before");
        r.press(key(KeyCode::Enter));
        assert!(dialog_title(&r.app).is_none());
        r.wait("the snapshot in the list", |r| r.vm().rows == 1);
        assert!(r.screen().contains("before"), "{}", r.screen());
        assert!(soyuz_core::config::config_dir().join("monitor").join("snapshots").is_dir());
        // Rename.
        r.press(key(KeyCode::F(2)));
        for _ in 0..10 {
            r.frame(&[key(KeyCode::Backspace)]);
        }
        r.typed("first");
        r.press(key(KeyCode::Enter));
        r.wait("the new name", |r| r.screen().contains("first"));
        // Compare with now.
        r.press(key(KeyCode::Char('a')));
        r.press(key(KeyCode::Enter));
        let pane = r.app.core.ws.pane().id;
        r.wait("the comparison", move |r| {
            r.app.core.ui.monitor.peek(pane).is_some_and(|v| v.snapshots.comparison.is_some())
        });
        r.frame(&[]);
        assert!(r.screen().contains("by program"), "{}", r.screen());
        r.press(key(KeyCode::Char('m')));
        r.wait("the match change", move |r| {
            r.screen().contains("by process")
                && r.app.core.ui.monitor.peek(pane).is_some_and(|v| !v.snapshots.comparing)
        });
        r.press(key(KeyCode::Char('u')));
        assert!(r.screen().contains("unchanged shown"), "{}", r.screen());
        r.press(key(KeyCode::Esc));
        assert!(!r.screen().contains("unchanged shown"), "{}", r.screen());
        // Delete asks first.
        r.press(key(KeyCode::Delete));
        assert!(dialog_title(&r.app).is_some());
        r.press(key(KeyCode::Esc));
        assert_eq!(r.vm().rows, 1);
    });
}

/// Every page, at every small size down to nothing, in each look.
#[test]
fn every_page_survives_tiny_sizes_in_every_look() {
    sandboxed("every_page_survives_tiny_sizes_in_every_look", |data| {
        for (icons, color) in [
            (TuiIcons::Ascii, TuiColorMode::Mono),
            (TuiIcons::Unicode, TuiColorMode::Ansi16),
            (TuiIcons::Nerd, TuiColorMode::Truecolor),
        ] {
            let mut c = config();
            c.tui.icons = icons;
            c.tui.color_mode = color;
            let mut r = Rig::new(c, &data, 40, 12);
            r.open(false);
            for p in MonitorPage::ALL {
                r.goto(p);
                for w in [1u16, 2, 3, 5, 10, 13, 20, 40] {
                    for h in [1u16, 2, 3, 4, 6, 9, 14] {
                        r.resize(w, h);
                    }
                }
                r.resize(10, 1);
                // The help and the fields at the small sizes too.
                r.press(shift(KeyCode::Char('?')));
                r.resize(10, 2);
                r.press(key(KeyCode::Esc));
                r.resize(40, 12);
            }
            r.goto(MonitorPage::Processes);
            r.press(key(KeyCode::Char('/')));
            r.typed("日本語");
            r.resize(6, 3);
            r.press(key(KeyCode::Esc));
            r.resize(40, 12);
        }
    });
}

#[test]
fn ascii_screens_are_ascii_and_mono_screens_have_their_marks() {
    sandboxed("ascii_screens_are_ascii_and_mono_screens_have_their_marks", |data| {
        let mut c = config();
        c.tui.color_mode = TuiColorMode::Mono;
        c.tui.borders = soyuz_core::config::TuiBorders::Ascii;
        let mut r = Rig::new(c, &data, 160, 50);
        r.open(false);
        for p in MonitorPage::ALL {
            r.goto(p);
            let screen = r.screen();
            for line in screen.lines() {
                // The degree sign of a temperature is the one thing left.
                assert!(line.chars().all(|c| c.is_ascii() || c == '°'), "{p:?}: {line}");
            }
        }
        r.goto(MonitorPage::Cpu);
        assert!(r.screen().contains('#'), "the chart\n{}", r.screen());
        // The legend names the series.
        assert!(r.screen().contains("Total") && r.screen().contains("System"), "{}", r.screen());
    });
}

#[test]
fn a_screen_reader_gets_lines_and_no_charts() {
    sandboxed("a_screen_reader_gets_lines_and_no_charts", |data| {
        let mut c = config();
        c.tui.screen_reader = true;
        let mut r = Rig::new(c, &data, 100, 30);
        r.open(false);
        for p in MonitorPage::ALL {
            r.goto(p);
            assert!(!r.screen().chars().any(soyuz_tui::a11y::is_box_char), "{p:?}\n{}", r.screen());
            assert!(!r.vm().announce.is_empty(), "{p:?}");
        }
        r.goto(MonitorPage::Processes);
        let said = r.vm().announce.clone();
        assert!(said.contains("process") && said.contains("row 1 of 6"), "{said}");
        // The status line says it, and the caret is on the row.
        assert!(r.screen().contains(&said[..said.len().min(20)]), "{}", r.screen());
        assert!(r.app.hits().caret().is_some());
        r.press(key(KeyCode::Down));
        assert!(r.vm().announce.contains("row 2 of 6"), "{}", r.vm().announce);
        r.goto(MonitorPage::Cpu);
        assert!(r.screen().contains("Total"), "values in words\n{}", r.screen());
    });
}

#[test]
fn clicks_select_sort_and_switch_pages() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use soyuz_tui::layout::HitTarget;
    use soyuz_tui::view::monitor::Hit;
    sandboxed("clicks_select_sort_and_switch_pages", |data| {
        let mut r = Rig::new(config(), &data, 160, 40);
        r.open(false);
        r.goto(MonitorPage::Processes);
        let find = |r: &Rig, want: Hit| {
            r.app.hits().iter().find_map(|(rect, t)| match t {
                HitTarget::Monitor { hit, .. } if *hit == want => Some((rect.x, rect.y)),
                _ => None,
            })
        };
        let click = |r: &mut Rig, at: (u16, u16), kind| {
            r.press(Event::Mouse(MouseEvent { kind, column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
        };
        let down = MouseEventKind::Down(MouseButton::Left);
        // A row.
        let at = find(&r, Hit::ProcRow(2)).expect("row 2");
        click(&mut r, at, down);
        let pane = r.app.core.ws.pane().id;
        let selected = r.app.core.ui.monitor.peek(pane).and_then(|v| v.selected.clone());
        assert!(selected.is_some());
        assert!(r.vm().announce.contains("row 3 of 6"), "{}", r.vm().announce);
        // A column title sorts.
        let at = find(&r, Hit::ProcHeader(ProcColumn::Pid)).expect("the Pid column");
        click(&mut r, at, down);
        assert_eq!(r.app.core.config.monitor.sort_by, ProcColumn::Pid);
        // A right click opens the menu.
        let at = find(&r, Hit::ProcRow(0)).expect("row 0");
        click(&mut r, at, MouseEventKind::Down(MouseButton::Right));
        assert!(r.screen().contains("Kill process"), "{}", r.screen());
        r.press(key(KeyCode::Esc));
        // The wheel scrolls and a tab switches the page.
        click(&mut r, at, MouseEventKind::ScrollDown);
        let at = find(&r, Hit::Tab(MonitorPage::Memory)).expect("the Memory tab");
        click(&mut r, at, down);
        assert_eq!(r.vm().page, MonitorPage::Memory);
    });
}

#[test]
fn history_moves_through_time_and_lists_show_empty_states() {
    sandboxed("history_moves_through_time_and_lists_show_empty_states", |data| {
        let mut r = Rig::new(config(), &data, 120, 40);
        r.open(false);
        r.goto(MonitorPage::History);
        let pane = r.app.core.ws.pane().id;
        let range = |r: &Rig| r.app.core.ui.monitor.peek(pane).map_or(0, |v| v.history.range_secs);
        let before = range(&r);
        r.press(shift(KeyCode::Char('+')));
        assert!(range(&r) > before);
        r.press(key(KeyCode::Char('-')));
        assert_eq!(range(&r), before);
        r.press(key(KeyCode::Left));
        assert!(r.app.core.ui.monitor.peek(pane).is_some_and(|v| v.history.end_ms.is_some()), "panned");
        r.press(key(KeyCode::Char('l')));
        assert!(r.app.core.ui.monitor.peek(pane).is_some_and(|v| v.history.end_ms.is_none()), "live");
        r.press(key(KeyCode::Char(',')));
        assert!(r.app.core.ui.monitor.peek(pane).is_some_and(|v| v.history.cursor_ms.is_some()));
        r.press(key(KeyCode::Esc));
        assert!(r.app.core.ui.monitor.peek(pane).is_some_and(|v| v.history.cursor_ms.is_none()));
    });
}

/// The screen with made-up frames (no live value in it).
#[test]
fn screens_match_their_snapshots() {
    sandboxed("screens_match_their_snapshots", |data| {
        let cases = [
            (MonitorPage::Overview, "monitor_overview"),
            (MonitorPage::Processes, "monitor_processes"),
            (MonitorPage::Cpu, "monitor_cpu"),
            (MonitorPage::Memory, "monitor_memory"),
            (MonitorPage::Disks, "monitor_disks"),
        ];
        let mut c = config();
        c.monitor.page = MonitorPage::Overview;
        // The panel's dates depend on the time zone.
        c.monitor.details_panel = false;
        let mut r = Rig::new(c, &data, 100, 30);
        r.open(false);
        for (page, name) in cases {
            r.goto(page);
            common::assert_screen(name, &r.term, &SnapOpts::text());
        }
    });
}

/// Not a check: prints every page with real readings at two sizes, to read them.
/// `SOYUZ_MONITOR_SHOTS=1 cargo test -p soyuz-tui --test headless_tui monitor_shots -- --nocapture`
#[test]
fn monitor_shots() {
    if std::env::var_os("SOYUZ_MONITOR_SHOTS").is_none() {
        return;
    }
    match common::relaunch(module_path!(), "monitor_shots", |_| {}) {
        Run::Child(data) => {
            for (w, h, sidebar) in [(80u16, 24u16, false), (200, 60, false), (100, 30, true)] {
                let mut c = config();
                c.layout.sidebar = sidebar;
                c.tui.icons = TuiIcons::Unicode;
                let mut r = Rig::new(c, &data, w, h);
                r.open(true);
                r.wait("a reading", |r| r.app.core.ui.monitor.frame.is_some());
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(6) {
                    r.frame(&[]);
                    std::thread::sleep(Duration::from_millis(100));
                }
                for p in MonitorPage::ALL {
                    r.goto(p);
                    let settle = Instant::now();
                    while settle.elapsed() < Duration::from_millis(1500) {
                        r.frame(&[]);
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    println!("===== {p:?} {w}x{h}\n{}", r.screen());
                }
                r.goto(MonitorPage::Processes);
                r.press(shift(KeyCode::Char('V')));
                r.press(key(KeyCode::Char('/')));
                r.typed("chrome");
                r.press(key(KeyCode::Enter));
                println!("===== Tree {w}x{h}\n{}", r.screen());
                r.press(key(KeyCode::Esc));
                r.press(shift(KeyCode::Char('V')));
                r.press(key(KeyCode::Char('c')));
                println!("===== Groups and chooser {w}x{h}\n{}", r.screen());
                r.press(key(KeyCode::Esc));
                r.press(shift(KeyCode::Char('V')));
                r.press(shift(KeyCode::Char('V')));
                r.goto(MonitorPage::History);
                r.press(key(KeyCode::Char(',')));
                r.press(key(KeyCode::Char(',')));
                println!("===== History cursor {w}x{h}\n{}", r.screen());
                r.press(key(KeyCode::Esc));
                r.goto(MonitorPage::Cpu);
                r.press(key(KeyCode::Char('v')));
                println!("===== CPU heat {w}x{h}\n{}", r.screen());
                r.press(key(KeyCode::Char('v')));
                println!("===== CPU bars {w}x{h}\n{}", r.screen());
                r.press(key(KeyCode::Char('v')));
            }
            common::ok();
        }
        Run::Parent(p) => println!("{}", p.log),
    }
}
