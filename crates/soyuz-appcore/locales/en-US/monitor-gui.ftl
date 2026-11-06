### Soyuz UI strings (en-US source): monitor-gui
### The system monitor's graphical pages and its settings page.
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/monitor-gui.ftl.

## Rail and top bar

monitor-gui-group-live = Live
monitor-gui-group-resources = Resources
monitor-gui-group-system = System
monitor-gui-group-records = Records
monitor-gui-status = Every { $interval } · { $processes } processes · { $threads } threads
monitor-gui-paused = Paused
monitor-gui-pause = Pause readings
monitor-gui-resume = Resume readings
monitor-gui-take-snapshot = Take a snapshot
monitor-gui-interval = Reading interval
monitor-gui-every = every
monitor-gui-waiting = The first readings are on their way.
monitor-gui-now = now

## Charts

monitor-gui-avg-max = { $avg } (max { $max })
monitor-gui-resize-column = Resize the { $column } column

## Overview

monitor-gui-cpu-sub = { $cores } logical CPUs at { $clock }
monitor-gui-cpu-sub-load = { $cores } logical CPUs · load { $load }
monitor-gui-cpu-sub-plain = { $cores } logical CPUs
monitor-gui-mem-sub = { $percent } of { $total }
monitor-gui-disk-sub = { $read } read · { $write } write
monitor-gui-net-sub = { $rx } in · { $tx } out
monitor-gui-gpu-sub = { $used } of { $total } memory
monitor-gui-battery = Battery
monitor-gui-battery-charging = Charging
monitor-gui-battery-ac = Plugged in
monitor-gui-battery-discharging = On battery
monitor-gui-hottest = Hottest sensor
monitor-gui-read = Read
monitor-gui-write = Write
monitor-gui-received = Received
monitor-gui-sent = Sent
monitor-gui-host = Computer
monitor-gui-os = Operating system
monitor-gui-kernel-version = Kernel
monitor-gui-processor = Processor
monitor-gui-cores = Cores
monitor-gui-cores-value = { $physical } physical · { $logical } logical
monitor-gui-ram = Memory
monitor-gui-uptime = Up for
monitor-gui-load = Load average
monitor-gui-virtualization = Virtualization
monitor-gui-top-cpu = Top processes by CPU
monitor-gui-top-memory = Top processes by memory
monitor-gui-volumes = Volumes

## Processes

monitor-gui-filter-hint = Filter: name, user:root, cpu>5, mem>100M, /regex/
monitor-gui-filter-regex-hint = Regular expression over name, command, path and user
monitor-gui-filter-help =
    Words match the name, command line, path, user and PID.
    "quoted words" is one term; -word excludes; a|b means either.
    /expr/ or /expr/i is a regular expression.
    user:root, name:, cmd:, path:, state:, pid:, ppid:, cgroup:, arch: test one column.
    cpu>5, mem>100M, threads>=10, read>1M, gpu>0 compare numbers (K, M, G, T suffixes).
monitor-gui-filter-error = Filter: { $message }
monitor-gui-regex = Whole filter is a regular expression
monitor-gui-kernel = Show kernel and system processes
monitor-gui-other-users = Show other users' processes
monitor-gui-scale-machine = CPU: 100% is the whole machine
monitor-gui-scale-per-core = CPU: 100% is one core
monitor-gui-heat = Tint cells by load
monitor-gui-details = Details panel
monitor-gui-export-csv = Export CSV
monitor-gui-export-json = Export JSON
monitor-gui-choose-columns = Choose columns…
monitor-gui-reset-columns = Reset columns
monitor-gui-columns = Columns
monitor-gui-search-columns = Search columns
monitor-gui-unknown-user = Unknown user
monitor-gui-marked-count = { $count } marked processes
monitor-gui-end-tree = End process tree
monitor-gui-kill-tree = Kill process tree
monitor-gui-priority = Priority
monitor-gui-affinity = Processors…
monitor-gui-affinity-title = Processors
monitor-gui-affinity-for = Which logical CPUs { $name } (PID { $pid }) may run on.
monitor-gui-affinity-count = { $on } of { $total } selected
monitor-gui-apply = Apply
monitor-gui-open-location = Open file location
monitor-gui-copy-pid = PID
monitor-gui-copy-name = Name
monitor-gui-copy-path = Path
monitor-gui-copy-command = Command line
monitor-gui-toggle-group = Collapse or expand group
monitor-gui-expand-all = Expand all
monitor-gui-collapse-all = Collapse all
monitor-gui-resize-details = Resize the details panel
monitor-gui-select-process = Select a process to see its details.

## Details panel

monitor-gui-tab-general = General
monitor-gui-tab-threads = Threads
monitor-gui-tab-modules = Modules
monitor-gui-tab-files = Files
monitor-gui-tab-environment = Environment
monitor-gui-tab-sockets = Sockets
monitor-gui-no-sockets = No open sockets.
monitor-gui-nothing-here = Nothing to show.
monitor-gui-limited = Some of this could not be read (access denied).
monitor-gui-col-proto = Protocol
monitor-gui-col-local = Local
monitor-gui-col-remote = Remote
monitor-gui-col-state = State
monitor-gui-col-process = Process
monitor-gui-col-tid = TID
monitor-gui-col-thread-name = Thread
monitor-gui-col-module = Module
monitor-gui-col-size = Size
monitor-gui-col-base = Base address
monitor-gui-col-fd = FD
monitor-gui-col-path = Path
monitor-gui-col-variable = Variable
monitor-gui-col-value = Value

## CPU

monitor-gui-total = Total
monitor-gui-user = User
monitor-gui-system = System
monitor-gui-iowait = I/O wait
monitor-gui-irq = Interrupts
monitor-gui-sockets = Sockets
monitor-gui-clock = Clock
monitor-gui-max-clock = Rated clock
monitor-gui-ctx-switches = Context switches
monitor-gui-interrupts = Interrupts
monitor-gui-per-sec = { $value } /s
monitor-gui-temperature = Temperature
monitor-gui-cache-l1 = L1 cache
monitor-gui-cache-l2 = L2 cache
monitor-gui-cache-l3 = L3 cache
monitor-gui-processes = Processes
monitor-gui-threads = Threads
monitor-gui-handles = Handles
monitor-gui-running = Running
monitor-gui-logical-cpus = { $count } logical CPUs
monitor-gui-core-graphs = Graphs
monitor-gui-core-heat = Heat
monitor-gui-core-bars = Bars

## Memory

monitor-gui-mem-in-use = In use
monitor-gui-mem-composition = Composition
monitor-gui-mem-cached = Cached
monitor-gui-mem-buffers = Buffers
monitor-gui-mem-kernel = Kernel
monitor-gui-mem-compressed = Compressed
monitor-gui-mem-free = Free
monitor-gui-mem-shared = Shared
monitor-gui-swap = Swap
monitor-gui-no-swap = No swap space.
monitor-gui-commit = Commit charge
monitor-gui-commit-used = Committed
monitor-gui-commit-limit = Commit limit
monitor-gui-available = Available
monitor-gui-page-in = Paged in
monitor-gui-page-out = Paged out

## Disks

monitor-gui-no-disks = No physical disks were reported.
monitor-gui-busy = Busy
monitor-gui-busy-value = { $value } busy
monitor-gui-queue = Queue
monitor-gui-iops = IOPS r/w
monitor-gui-read-total = Read total
monitor-gui-write-total = Written total
monitor-gui-disk-read = Disk read
monitor-gui-disk-write = Disk write

## Network

monitor-gui-net-total = All interfaces
monitor-gui-net-received = Network in
monitor-gui-net-sent = Network out
monitor-gui-interfaces = { $count } interfaces
monitor-gui-show-virtual = Virtual interfaces
monitor-gui-show-virtual-n = { $count } virtual hidden
monitor-gui-up = Up
monitor-gui-down = Down
monitor-gui-virtual = Virtual
monitor-gui-rx-total = Received total
monitor-gui-tx-total = Sent total
monitor-gui-packets = Packets/s in / out
monitor-gui-errors = Errors
monitor-gui-no-interfaces = No network interfaces to show.

## GPU

monitor-gui-no-gpu = No graphics adapter
monitor-gui-no-gpu-sub = This system reports no GPU, or its driver gives no readings.
monitor-gui-load-label = Load
monitor-gui-engines = Engines
monitor-gui-gpu-memory = GPU memory
monitor-gui-gpu-shared = Shared system memory
monitor-gui-not-reported = Not reported by the driver.
monitor-gui-power = Power
monitor-gui-fan = Fan
monitor-gui-mem-clock = Memory clock

## Sensors

monitor-gui-no-sensors = No sensors
monitor-gui-no-sensors-sub = This system offers no temperature, fan or power readings.
monitor-gui-time-left = Time left
monitor-gui-health = Health
monitor-gui-cycles = Charge cycles

## Lists

monitor-gui-list-filter-hint = Filter, or /regex/
monitor-gui-count-of = { $shown } of { $total }
monitor-gui-no-connections = No open connections.
monitor-gui-no-services = No services were found.
monitor-gui-no-startup = Nothing starts with the system.
monitor-gui-no-matches = Nothing matches the filter.
monitor-gui-go-to-process = Go to process
monitor-gui-copy-remote = Copy remote address
monitor-gui-copy-local = Copy local address
monitor-gui-copy-source = Copy source
monitor-gui-col-service = Service
monitor-gui-col-service-name = Name
monitor-gui-col-start-mode = Start
monitor-gui-col-description = Description
monitor-gui-col-enabled = Enabled
monitor-gui-col-source = Source
monitor-gui-enabled = Enabled
monitor-gui-disabled = Disabled

## History

monitor-gui-live = Live
monitor-gui-recorded-since = Recorded since { $time }
monitor-gui-recording-starts = Recording starts with the first readings.
monitor-gui-nothing-recorded = Nothing recorded for this span.
monitor-gui-at-cursor = At the cursor
monitor-gui-latest = Latest
monitor-gui-no-tops = No process list was kept for this moment.

## Snapshots

monitor-gui-snapshot-label-hint = Label for the next snapshot
monitor-gui-take = Take
monitor-gui-no-snapshots = No snapshots yet
monitor-gui-no-snapshots-sub = A snapshot keeps every reading and process of a moment, to compare later.
monitor-gui-auto = auto
monitor-gui-procs-short = processes
monitor-gui-cpu-short = CPU
monitor-gui-compare-from = Compare from here (A)
monitor-gui-compare-with = Compare with this (B)
monitor-gui-rename = Rename
monitor-gui-compare-a = Snapshot A
monitor-gui-compare-b = Snapshot B
monitor-gui-pick-snapshot = Pick a snapshot
monitor-gui-by-program = By program
monitor-gui-by-process = By process
monitor-gui-compare = Compare
monitor-gui-compare-title = Compare two moments
monitor-gui-compare-sub = Pick snapshot A and B (or now), then press Compare.
monitor-gui-diff-summary = { $added } added · { $removed } removed · { $changed } changed
monitor-gui-show-unchanged = Show unchanged
monitor-gui-col-change = Change
monitor-gui-col-count = Count
monitor-gui-added = Added
monitor-gui-removed = Removed
monitor-gui-changed = Changed
monitor-gui-same = Same

## Settings page

settings-monitor-readings = Readings
settings-monitor-interval = Reading interval
settings-monitor-interval-desc = Milliseconds between readings while a monitor tab is on screen.
settings-monitor-page = Start page
settings-monitor-page-desc = The page a monitor tab opens on.
settings-monitor-graph-seconds = Live graph span
settings-monitor-graph-seconds-desc = Seconds of history the live graphs show.
settings-monitor-cpu-scale = CPU scale
settings-monitor-cpu-scale-desc = Whether a process's CPU load is a share of the whole machine or of one core.
settings-monitor-core-view = Logical CPUs
settings-monitor-core-view-desc = How the CPU page draws its logical CPUs.
settings-monitor-temperature-unit = Temperature unit
settings-monitor-virtual-networks = Virtual network interfaces
settings-monitor-virtual-networks-desc = List loopback, tunnels and bridges on the Network page.
settings-monitor-table = Process table
settings-monitor-view = Arrangement
settings-monitor-view-desc = A flat list, a tree of parents and children, or groups by program or user.
settings-monitor-sort = Sort by
settings-monitor-sort-desc = The column the process table sorts by.
settings-monitor-sort-descending = Largest first
settings-monitor-sort-descending-desc = Sort the chosen column from the largest value down.
settings-monitor-columns = Columns
settings-monitor-columns-desc = The columns of the process table. Drag headers in the table to reorder them; drag their edges to resize.
settings-monitor-columns-note = The Name column is always shown.
settings-monitor-heat = Tint cells by load
settings-monitor-heat-desc = Load columns get a stronger tint the busier they are, like a heat map.
settings-monitor-icons = Program icons
settings-monitor-icons-desc = Show each program's own icon next to its name.
settings-monitor-process-gpu = Per-process GPU use
settings-monitor-process-gpu-desc = Read the GPU use of every process (the most expensive reading; refreshed every few seconds).
settings-monitor-kernel = Kernel and system processes
settings-monitor-kernel-desc = List kernel threads and OS-internal pseudo processes.
settings-monitor-other-users = Other users' processes
settings-monitor-other-users-desc = List processes that run under other accounts.
settings-monitor-follow = Follow the selection
settings-monitor-follow-desc = Keep the selected process in view while rows reorder.
settings-monitor-details = Details panel
settings-monitor-details-desc = Show the details of the selected process under the table.
settings-monitor-details-height = Details panel height
settings-monitor-confirm-kill = Ask before ending a process
settings-monitor-confirm-kill-desc = Confirm before ending, killing or suspending a process. Critical processes always ask.
settings-monitor-remember-filter = Remember the filter
settings-monitor-remember-filter-desc = Keep the process filter between sessions.
settings-monitor-filter = Filter
settings-monitor-filter-desc = The remembered filter. Words match any text column; /expr/ is a regular expression; user:root, cpu>5, mem>100M test one column.
settings-monitor-filter-regex = Filter is a regular expression
settings-monitor-filter-regex-desc = Treat the whole filter as one regular expression.
settings-monitor-keep-parents = Keep parents when filtering the tree
settings-monitor-keep-parents-desc = The parents of matching processes stay on screen, dimmed.
settings-monitor-history = History
settings-monitor-background = Record in the background
settings-monitor-background-desc = Keep recording history while no monitor tab is open.
settings-monitor-background-interval = Background interval
settings-monitor-background-interval-desc = Seconds between readings when recording with no monitor tab open.
settings-monitor-persist = Keep history on disk
settings-monitor-persist-desc = Write the history to disk, so it survives a restart.
settings-monitor-history-days = Days kept
settings-monitor-history-days-desc = Days of history kept on disk.
settings-monitor-snapshots = Snapshots
settings-monitor-snapshots-kept = Snapshots kept
settings-monitor-snapshots-kept-desc = Before the oldest automatic snapshots are removed. Snapshots you took yourself are never removed.
settings-monitor-commands = Command lines in snapshots and exports
settings-monitor-commands-desc = Command lines can hold passwords and tokens. Redacted masks the values of flags that look like secrets.
settings-monitor-commands-none = Leave out
settings-monitor-commands-redacted = Redacted
settings-monitor-commands-full = Whole
settings-monitor-alerts = Alerts
settings-monitor-alert-cpu = CPU above
settings-monitor-alert-memory = Memory above
settings-monitor-alert-temperature = Temperature above
settings-monitor-alert-off-hint = Raise a notification when it stays above this. 0 turns the alert off.
settings-monitor-alert-sustain = For at least
settings-monitor-alert-sustain-desc = How long a value must stay above its limit before the alert fires.
settings-monitor-alert-snapshot = Snapshot on alert
settings-monitor-alert-snapshot-desc = Take a snapshot automatically when an alert fires.

monitor-gui-status = { $page } · { $processes } processes, { $running } running
