### Soyuz UI strings (en-US source): tui-monitor
### The terminal UI: the system monitor pages.
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-monitor.ftl.

## The header strip

tui-monitor-tab-overview = Over
tui-monitor-tab-processes = Proc
tui-monitor-tab-cpu = CPU
tui-monitor-tab-memory = Mem
tui-monitor-tab-disks = Disk
tui-monitor-tab-network = Net
tui-monitor-tab-gpu = GPU
tui-monitor-tab-sensors = Sens
tui-monitor-tab-connections = Conn
tui-monitor-tab-services = Svc
tui-monitor-tab-startup = Boot
tui-monitor-tab-history = Hist
tui-monitor-tab-snapshots = Snap
tui-monitor-status-hint = ? keys · Tab pages · Ctrl+Q quit
tui-monitor-paused = PAUSED
tui-monitor-interval = every { $secs } s
tui-monitor-page-of = { $page }, page { $index } of { $total }

## Shared words

tui-monitor-loading = Reading…
tui-monitor-no-matches = Nothing matches the filter
tui-monitor-match-count = { $count } of { $total }
tui-monitor-row-of = row { $index } of { $total }
tui-monitor-list-count = { $title }: { $count }
tui-monitor-list-empty = This system reports nothing here
tui-monitor-total = Total
tui-monitor-system = System
tui-monitor-read = Read
tui-monitor-write = Write
tui-monitor-download = Download
tui-monitor-upload = Upload
tui-monitor-swap = Swap
tui-monitor-temperature = Temperature
tui-monitor-processes = Processes
tui-monitor-top-cpu = Top by CPU
tui-monitor-top-memory = Top by memory

## Overview

tui-monitor-cores = { $cores } CPUs
tui-monitor-cores-threads = { $cores } cores, { $threads } threads
tui-monitor-ram = { $size } RAM
tui-monitor-uptime = up { $time }
tui-monitor-load = load { $one } { $five } { $fifteen }
tui-monitor-user-system = user { $user }, system { $system }
tui-monitor-read-write = read { $read }, write { $write }
tui-monitor-down-up = down { $down }, up { $up }
tui-monitor-threads = { $count } threads

## CPU, memory and devices

tui-monitor-iowait = waiting { $value }
tui-monitor-ctx-switches = { $count } switches/s
tui-monitor-logical-cpus = Logical CPUs
tui-monitor-cpu-n = CPU { $n }
tui-monitor-memory-used = Memory: { $used } of { $total } used ({ $percent })
tui-monitor-used = Used
tui-monitor-cached = Cached
tui-monitor-buffers = Buffers
tui-monitor-free = Free
tui-monitor-available = Available
tui-monitor-shared = Shared
tui-monitor-kernel = Kernel
tui-monitor-compressed = Compressed
tui-monitor-commit = Commit
tui-monitor-paging = Paging: in { $page_in }, out { $page_out }
tui-monitor-busy = Busy
tui-monitor-queue = Queue
tui-monitor-disk-ssd = SSD
tui-monitor-disk-hdd = HDD
tui-monitor-disk-foot = { $read } reads/s, { $write } writes/s · { $read_total } read, { $write_total } written
tui-monitor-net-up = up
tui-monitor-net-down = down
tui-monitor-net-foot = { $down } received, { $up } sent · { $errors } errors
tui-monitor-load-label = Load
tui-monitor-gpu-memory = Memory
tui-monitor-fan = Fan
tui-monitor-power = Power
tui-monitor-voltage = Voltage
tui-monitor-current = Current
tui-monitor-battery-charging = charging
tui-monitor-battery-plugged = plugged in
tui-monitor-battery-discharging = on battery
tui-monitor-battery-health = health { $value }
tui-monitor-sensors-other = Other
tui-monitor-no-disks = This system reports no disks
tui-monitor-no-networks = No network interfaces to show
tui-monitor-no-gpus = This system reports no graphics processor
tui-monitor-no-sensors = This system reports no sensors

## Processes

tui-monitor-scale-machine = 100% = the machine
tui-monitor-scale-core = 100% = one CPU
tui-monitor-proc-count = { $count } processes, { $running } running
tui-monitor-marked = { $count } marked
tui-monitor-columns-hidden = { $count } columns do not fit
tui-monitor-unknown-user = Unknown user
tui-monitor-filter-regex = regex
tui-monitor-announce-proc = { $name }, process { $pid }, CPU { $cpu }, memory { $memory }, { $state }, { $at }
tui-monitor-announce-group = { $name }, group of { $count }, { $at }
tui-monitor-details-general = General
tui-monitor-details-threads = Threads
tui-monitor-details-modules = Modules
tui-monitor-details-files = Files
tui-monitor-details-environment = Environment
tui-monitor-details-sockets = Sockets
tui-monitor-details-none = Select a process to see its details
tui-monitor-details-group = This row is a group: select one of its processes
tui-monitor-details-empty = Nothing to show

## Process menu

tui-monitor-end-tree = End process tree
tui-monitor-menu-priority = Priority
tui-monitor-menu-affinity = Processors…
tui-monitor-menu-open-location = Open file location
tui-monitor-menu-copy = Copy
tui-monitor-menu-mark = Mark
tui-monitor-menu-details = Details panel
tui-monitor-menu-columns = Columns…
tui-monitor-menu-export = Export as CSV
tui-monitor-menu-kernel = Show or hide kernel threads
tui-monitor-menu-other-users = Show or hide other users
tui-monitor-menu-heat = Heat colours and bars on or off
tui-monitor-menu-scale = Switch the CPU scale

## Dialogs

tui-monitor-columns-title = Process table columns
tui-monitor-columns-hints = Space show or hide · Shift+Up/Down move · r reset · Enter done
tui-monitor-affinity-title = Processors of { $name }
tui-monitor-affinity-hints = Space tick · Enter apply · Esc cancel
tui-monitor-snapshot-new = New snapshot
tui-monitor-snapshot-rename = Rename snapshot
tui-monitor-snapshot-name = Name
tui-monitor-snapshot-hints = Enter save · Esc cancel

## Lists

tui-monitor-col-proto = Proto
tui-monitor-col-local = Local
tui-monitor-col-remote = Remote
tui-monitor-col-state = State
tui-monitor-col-process = Process
tui-monitor-col-service = Service
tui-monitor-col-start = Starts
tui-monitor-col-unit = Name
tui-monitor-col-name = Name
tui-monitor-col-enabled = Enabled
tui-monitor-col-source = From
tui-monitor-col-command = Command
tui-monitor-col-threads = Threads
tui-monitor-col-handles = Handles

## History

tui-monitor-range-minutes = { $count } min
tui-monitor-range-hours = { $count } h
tui-monitor-range-days = { $count } d
tui-monitor-history-live = live
tui-monitor-history-range = Last { $range } · { $when }

## Snapshots and the comparison

tui-monitor-compare-sides = A: { $a } · B: { $b }
tui-monitor-side-none = not chosen
tui-monitor-side-now = now
tui-monitor-no-snapshots = No snapshots yet: press n to take one
tui-monitor-col-snapshot = Snapshot
tui-monitor-col-taken = Taken
tui-monitor-auto = automatic
tui-monitor-comparing = Comparing…
tui-monitor-compare-summary = +{ $added } -{ $removed } ~{ $changed } · { $by } · { $same }
tui-monitor-match-process = by process
tui-monitor-match-program = by program
tui-monitor-same-shown = unchanged shown
tui-monitor-same-hidden = unchanged hidden
tui-monitor-change-added = added
tui-monitor-change-removed = removed
tui-monitor-change-changed = changed
tui-monitor-change-same = unchanged
tui-monitor-export-json = Export as JSON
tui-monitor-export-csv = Export processes as CSV

## Key hints on the pages' caption lines

tui-monitor-hint-processes = / filter · V view · c columns · ? keys
tui-monitor-hint-cpu = v view · ? keys
tui-monitor-hint-network = v virtual · ? keys
tui-monitor-hint-connections = / filter · < > sort · Enter process
tui-monitor-hint-services = x start · X stop · R restart · / filter
tui-monitor-hint-lists = / filter · < > sort · ? keys
tui-monitor-hint-history = + - range · arrows pan · L live · , . cursor
tui-monitor-hint-snapshots = n new · a b sides · Enter compare · ? keys
tui-monitor-hint-compare = u unchanged · m match · / filter · Esc close

## The help overlay

tui-monitor-help-title = Keys of this page
tui-monitor-help-all-title = On every page
tui-monitor-help-all =
    Tab, Shift+Tab, 1-9, 0   other pages
    P                        pause or resume the readings
    i                        next reading interval
    arrows, Page Up/Down     move or scroll
    ? or Esc                 close this help
tui-monitor-help-overview =
    Click a tile             go to its page
tui-monitor-help-processes =
    / and Esc                filter, clear the filter
    Ctrl+R                   the whole filter is a regular expression
    V                        list, tree, by program, by user
    Left, Right              fold or unfold a row
    + and -                  unfold or fold every row
    Space                    mark a process
    < > and I                sort column, turn the order around
    c                        choose columns
    d, [ and ]               details panel, its tabs
    Shift+[ and Shift+]      scroll the details
    F9 or k                  end the process (asks first)
    K                        kill the process (asks first)
    t                        end the process and all it started
    s and r                  suspend, resume
    p and a                  priority, processors
    o                        open the program's folder
    y                        copy pid, name, path or command line
    e                        export the table as CSV
    Enter                    menu of the process
tui-monitor-help-cpu =
    v                        graphs, heat grid or bars per CPU
tui-monitor-help-devices =
    Up, Down                 scroll
tui-monitor-help-network =
    v                        show or hide virtual interfaces
tui-monitor-help-connections =
    / and Esc                filter, clear the filter
    < > and I                sort column, turn the order around
    Enter                    go to the connection's process
tui-monitor-help-services =
    / and Esc                filter, clear the filter
    < > and I                sort column, turn the order around
    x, X, R                  start, stop, restart (asks first)
tui-monitor-help-lists =
    / and Esc                filter, clear the filter
    < > and I                sort column, turn the order around
tui-monitor-help-history =
    + and -                  a longer or shorter range
    Left, Right              go back or forward in time
    L                        back to live
    , and .                  move the cursor: values and top processes then
    Esc                      hide the cursor
tui-monitor-help-snapshots =
    n                        new snapshot
    F2 or r                  rename
    Del                      delete (asks first)
    e                        export
    a                        use as the left side (A)
    b                        use as the right side (B), again for now
    Shift+B                  compare with now
    Enter                    compare
    In a comparison:
    < > and I                sort column, turn the order around
    u                        show or hide unchanged processes
    m                        match by process or by program
    / and Esc                filter, close
tui-monitor-help-query-title = The filter
tui-monitor-help-query =
    word                     in the name, command line, path, user or pid
    "two words"              one term; all terms must match
    -word                    must not match
    a|b                      either one
    /expr/ and /expr/i       a regular expression
    user:root                in one column: user name cmd path state pid ppid cgroup arch
    cpu>5  mem>100M          compare: cpu mem rss private threads handles read write gpu nice
