# soyuz-sysmon: public API

For the frontend agents: everything is at the crate root unless a module is named. Raw records (`ProcKey`, `ProcRaw`, `ProcState`, `ProcSignal`, `Want`, `DetailsWant`, `GpuRaw`, `SensorRaw`, `BatteryRaw`, `SystemInfo`, `Connection`, `ServiceInfo`, `StartupItem`, `ControlError`, ...) and config enums (`ProcColumn`, `ProcView`, `CpuScale`, `SnapshotCommands`, `Monitor`) are re-exported at the root. The OS calls not wrapped here (`service_control`, `set_priority`, `affinity`, `set_affinity`) stay in `soyuz_fs::sysmon`.

## Rules
- `None` / `CellValue::Missing` means the OS does not offer the value. Show a dash, never 0.
- `wake` callbacks run on a worker. Only post a wake-up there (`ctx.request_repaint()`, a channel send). Take no lock the UI can hold while calling the hub.
- Rows reorder every sample: selection and scroll follow `RowId`, never an index.
- Call `build` through `TableCache`, and `cell` only for rows on screen. Do not repaint continuously: repaint on wake and input.
- `snapshot::*`, `store::*` block: run them on a `Loader`. `cell`, `build`, `Filter`, `compare`, `Hub` readers do not touch the disk.
- Strings in frames and `fetch` lists are already sanitized (controls, ANSI, bidi overrides). For other OS text use `sanitize_text`.

## hub (the sampler)
| Item | |
|---|---|
| `Hub::global() -> &'static Hub` | The process-wide sampler over the real probe. |
| `Hub::new_with_source(impl Fn() -> Box<dyn Source> + Send + Sync) -> Hub` | A hub over a fake source (tests). |
| `hub.subscribe(interval_ms: u32, want: Want, wake: Wake) -> Subscription` | Interest in frames; drop ends it. Smallest interval and OR of all `Want`s win. |
| `hub.configure(HubConfig)` | Applies settings; alert limits or background history start the worker with no tab. |
| `HubConfig::from_config(&Monitor, data_dir: Option<PathBuf>)` | Settings to hub config. `data_dir` holds `history/` and `snapshots/`. |
| `hub.latest() -> Option<Arc<Frame>>` | Newest frame. `None` before the first one and after the worker ended. |
| `hub.generation() -> u64` | Changes with every frame; the `TableCache` key. |
| `hub.with_history(\|h: &History\| ...) -> R` | Reads history; keep it short. |
| `hub.proc_series(ProcKey) -> Vec<ProcSample>` | Short per-process trail. |
| `hub.pause(bool)`, `hub.is_paused()` | Freeze or resume sampling (whole process). |
| `hub.take_alerts() -> Vec<Alert>` | Alerts since the last call. |
| `hub.system_info() -> Arc<SystemInfo>` | Read once on the worker; empty until the first start. |
| `hub.data_dir() -> Option<PathBuf>` | The configured folder. |
| `hub.series_loader(Metric, from_ms, to_ms, max_points, Wake) -> Loader<Vec<Pt>>` | Memory plus files older than 48 h. Use when `from_ms` is older than `History::range`. |
| `hub.snapshot_now(label: String, Wake) -> Loader<io::Result<Option<SnapshotMeta>>>` | Saves a manual snapshot; `Ok(None)`: no frame or no folder. |
| `hub.set_linger(Duration)` | How long an idle worker keeps its probe (default 60 s). |
| `Wake = Arc<dyn Fn() + Send + Sync>` | The callback type. |
| `trait Source { sample(&mut self, Want) -> RawSample; system_info(); connections() }` | Where samples come from. |

## frame
`Frame { at_ms, uptime_secs, load, first, gap, elapsed_ms, cpu: CpuFrame, memory: MemFrame, disks: Vec<DiskFrame>, nets: Vec<NetFrame>, gpus: Vec<GpuRaw>, sensors: Vec<SensorRaw>, batteries: Vec<BatteryRaw>, procs: Arc<[ProcRow]>, counts: Counts, added: Vec<ProcKey>, removed: Vec<ProcKey> }`
- `frame.ncpu()`, `frame.disk_totals() -> (read, write)`, `frame.net_totals() -> (rx, tx)`, `frame.max_temp_c()`, `frame.proc(key) -> Option<&ProcRow>`.
- `CpuFrame { percent, user, system, iowait, steal, irq, cores: Vec<CoreFrame{percent,user,system,mhz}>, mhz_avg, temp_c, ctx_per_sec, irq_per_sec }`, percent of the machine. `percent = 1 - d(idle)/d(all)`, so iowait counts as busy.
- `MemFrame { raw: MemoryRaw, used, percent, swap_percent, page_in_rate, page_out_rate }`.
- `DiskFrame { id, model, read_rate, write_rate, read_iops, write_iops, busy_percent, queue, rotational, size, read_bytes, write_bytes }`; `disk.in_totals()` is false for `dm-*`, `md*`, `zram*`.
- `NetFrame { id, name, description, rx_rate, tx_rate, rx_packets_rate, tx_packets_rate, rx_bytes, tx_bytes, rx_errors, tx_errors, up, virtual_if, link_bps, addresses, mac }`; `net.in_totals()` is `up && !virtual_if`.
- `ProcRow { raw: ProcRaw, cpu_percent /* 100 = one logical CPU */, read_rate, write_rate, mem_percent }`.
- `Counts { processes: u32, threads: u64, handles: Option<u64>, running: u32 }`, without the idle pseudo process.
- `state_id(ProcState) -> &'static str`: `running`, `disk_wait`, ...
- `added` / `removed`: flash new and exited rows. A frame of the hub never has `first` set (it primes with a hidden sample).

## table
| Item | |
|---|---|
| `TableOpts { view, sort_by, descending, keep_parents, show_kernel, show_other_users, me, scale, ncpu, now_ms }` | `TableOpts::from_config(&Monitor, me, &Frame)`. |
| `build(&Frame, &Filter, &TableOpts, &HashSet<RowId>) -> Vec<TableRow>` | Lays out the rows. The idle process never appears. |
| `TableCache::new()`, `.rows(&Frame, generation, &Filter, &TableOpts, &HashSet<RowId>) -> &[TableRow]`, `.invalidate()`, `.builds()` | Rebuilds only on change; a moved `now_ms` alone does not. |
| `TableRow { kind: RowKind, depth: u16, has_children, collapsed, matched, last_sibling, ancestors_last: u64, totals: Option<Totals> }` | `matched` false: draw dimmed. `ancestors_last` bit d: the ancestor at depth d was last, no guide line. |
| `row.id(&Frame) -> RowId`, `row.proc(&Frame) -> Option<&ProcRow>` | |
| `RowKind::Proc(usize)`, `RowKind::Group { label: Arc<str>, count: u32 }` | Empty label in ByUser: user unknown, sorts last. |
| `RowId::Proc(ProcKey)`, `RowId::Group(String)` | Stable identity; also the collapsed set's key. |
| `position_of(&[TableRow], &Frame, &RowId) -> Option<usize>` | Follow the selection. |
| `cell(ProcColumn, &ProcRow, now_ms, ncpu, CpuScale) -> CellValue` | The Cpu column is `cpu_percent / ncpu` on Machine scale. |
| `cell_total(ProcColumn, &Totals, ncpu, CpuScale) -> CellValue` | **Deviation:** plan had `(col, totals)`; the scale is needed for Cpu. `Missing` for columns that do not sum. |
| `CellValue::{Text(String), Count(u64), Bytes(u64), Rate(f64), Percent(f32), Millis(u64), Seconds(u64), UnixMs(i64), State, Priority, Bool, Int(i64), Missing}` | Pid, Parent and Nice are `Int`. |
| `Totals { count, cpu_percent, cpu_ms, rss, private, shared, virt, mem_percent, threads, handles, read_rate, write_rate, read_total, write_total, gpu_percent, gpu_mem, page_faults }`, `.add(&ProcRow)` | |

## filter
`Filter::parse(text, regex_mode) -> Result<Filter, FilterError { message, at }>`, `Filter::regex(text)`, `Filter::default()` (matches all), `.matches(&ProcRow)` (`cpu` where 100 = one CPU), **`.matches_in(&ProcRow, ncpu, CpuScale)`** (added: `cpu>5` compares what the column shows; `build` uses this), `.is_empty()`, `.text()`, `.is_regex_mode()`.

Numeric fields: `cpu mem memory rss mem% private shared virt peak threads handles fds read write read_total write_total gpu gpu_mem pid ppid nice faults cpu_time`. Text fields: `user name cmd path state pid ppid cgroup arch`. An unfinished `user:` or `cpu>` while typing is no term, not an error.

## history
- `History::new(days)`, `.push(&Frame) -> Option<AggPoint>` (returns a finished coarse point), `.series(&Metric, from_ms, to_ms, max_points) -> Vec<Pt>` (0 = no decimation), `.tops_near(t_ms) -> Option<Tops>`, `.range()`, `.full_range()` (with disk), `.prune(now_ms)`, `.set_days`, `.set_disk_oldest`, `.load_coarse(Vec<AggPoint>)`, `.clear()`.
- **Deviation (review 9):** `Pt { t, avg, max }`, not a tuple. `pt.is_break()`: NaN, so break the line. Draw `avg` with a faint `max` band.
- `Metric::{CpuTotal, CpuUser, CpuSystem, Core(u16) /* fine tier only */, MemPercent, MemUsed /* bytes */, SwapPercent, DiskRead, DiskWrite, DiskReadOf(id), DiskWriteOf(id), DiskBusyOf(id), NetRx, NetTx, NetRxOf(id), NetTxOf(id), GpuLoad(u16), GpuMem(u16) /* bytes */, MaxTemp, ProcessCount}`.
- `Tops { by_cpu: Vec<TopProc>, by_mem: Vec<TopProc> }`, `TopProc { name: Arc<str>, pid, cpu, mem }`.
- `Point<V>`, `FinePoint = Point<f32>`, `AggPoint = Point<Agg>`, `Agg { avg, max }`; `point.value(&Metric) -> Option<V>`.
- `ProcHistory::new()`, `.push(&Frame)`, `.series(ProcKey) -> Vec<ProcSample { t_ms, cpu_percent, rss }>`, `.len()`. **Deviation (review 12):** 300 samples, rss to a KiB.

## store (`soyuz_sysmon::store`; blocking)
`Writer::new(dir)`, `.append(&AggPoint)`; `load(dir, days, now_ms)`, `load_since(dir, from_ms)`, `load_series(dir, &Metric, from_ms, to_ms, max_points)`, `oldest(dir)`, `prune(dir, days, now_ms)`. `dir` is the data folder, `history/` is below it. The hub does all of this; pages use `hub.series_loader`.

## snapshot (`soyuz_sysmon::snapshot`; blocking)
- `Snapshot { v, meta: SnapshotMeta, info, frame, connections }`, `SnapshotMeta { v, id, label, taken_ms, auto, hostname, procs, cpu_percent, mem_percent }`.
- `Snapshot::capture(&Frame, &SystemInfo, Vec<Connection>, label, auto, SnapshotCommands)`: masks command lines. **Deviation:** the plan had no constructor.
- `save(dir, &Snapshot) -> io::Result<SnapshotMeta>`, `list(dir)` (newest first), `load(dir, id)`, `rename(dir, id, label)`, `delete(dir, id)`, `prune_auto(dir, keep)` (at most `keep` in all, removes only automatic ones).
- `export_json(&Snapshot, &mut dyn Write)`; `export_procs_csv(&[ProcRow], &[ProcColumn], CsvOpts { ncpu, scale, now_ms, commands }, &mut dyn Write)`. **Deviation:** `CsvOpts` added, for review 5.
- `redact::redact_cmdline(&str) -> Cow<str>`, `redact::apply_to_row(&ProcRow, SnapshotCommands) -> ProcRow`.

## compare
- `compare(&Frame, &Frame, MatchBy, Tolerance) -> Comparison`. **Deviation (review 16):** `Tolerance { cpu_points, rel, bytes_floor }`, `Tolerance::default()` (1 point, 2 %, 1 MiB), `Tolerance::exact()`.
- `MatchBy::{Process, Program}`, `Change::{Added, Removed, Changed, Same}`.
- `Comparison { totals: TotalsDiff, procs: Vec<ProcDiff>, added, removed, changed, same }`.
- `ProcDiff { name, user, a: Option<DiffValues>, b: Option<DiffValues>, change, key_a, key_b, pid_a, pid_b, count_a, count_b }`.
- `DiffValues { cpu_percent, rss, private, threads, handles, read_bytes, write_bytes, gpu_percent, gpu_mem }`.
- `TotalsDiff { at_ms, cpu, mem_used, mem_percent, swap_used, processes, threads, handles, disk_read, disk_write, net_rx, net_tx, disks: Vec<DiskDiff>, nets: Vec<NetDiff>, gpus: Vec<GpuDiff> }`, all `(a, b)` pairs. `compare::{DiskDiff, NetDiff, GpuDiff}`.
- `sort_diffs(&mut [ProcDiff], DiffSort, descending)`, `DiffSort::{Name, User, Change, CpuDelta, RssDelta, PrivateDelta, ThreadsDelta, HandlesDelta, ReadDelta, WriteDelta, GpuDelta, GpuMemDelta, CountDelta}`.

## control (review 4)
- `signal_tree(&Frame, ProcKey, ProcSignal) -> TreeSignalReport { done, failed }`: children first, each with its own start-time check; critical descendants skipped; a process already gone is not a failure.
- `signal_row(&ProcRow, ProcSignal) -> Result<(), ControlError>`: refuses what `refuses` says, else `soyuz_fs::sysmon::signal`. Use it for single processes.
- `refuses(&ProcRaw, ProcSignal) -> bool`: Suspend on a critical process.
- `risk(&ProcRaw, me: Option<&str>) -> ProcRisk::{Normal, OtherUser, Critical}`: which warning the confirm dialog shows.
- `is_critical(&ProcRaw)`, `descendants(&Frame, ProcKey) -> Vec<ProcKey>`.

## alerts
`Alert { kind: AlertKind::{Cpu, Memory, Temperature}, value, limit, snapshot_id: Option<String> }`, `AlertLimits { cpu_percent, memory_percent, temperature_c, sustain_secs }`, `AlertLimits::from_config(&Monitor)`, `AlertWatcher::new()`, `.check(&Frame, &Monitor)`, `.check_limits(&Frame, &AlertLimits)`. The hub runs these; frontends call `hub.take_alerts()`.

## fetch (`soyuz_sysmon::fetch`)
- `Loader::spawn(name, Wake, impl FnOnce() -> T) -> Loader<T>`, `.poll() -> Option<T>` (once), `.is_pending()`.
- `connections(Wake)` (refresh every 3-5 s), `services(Wake)`, `startup_items(Wake)`, `process_details(ProcKey, DetailsWant, Wake)` (on selection change and about every 2 s). All sanitized.

## rates
`Rater::new()`, `.set_interval(ms)`, `.push(RawSample) -> Frame`, `.reset()`. The hub owns one; only tests and tools use it directly.

## format, sanitize
`format_rate(bytes_per_sec) -> String` (`1.5 MiB/s`), `format_duration(secs) -> String` (`3:04:05`, `2d 03:04:05`). Sizes: `soyuz_core::format_size`. `sanitize_text(&str) -> Cow<str>`; `sanitize::{sanitize_arc, sanitize_path, sanitize_raw, sanitize_services, sanitize_startup, sanitize_connections, sanitize_details}`.
