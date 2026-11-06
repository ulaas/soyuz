### Soyuz UI strings (en-US source): monitor
### The system monitor, both frontends.
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/monitor.ftl.

## Pages

monitor-page-overview = Overview
monitor-page-processes = Processes
monitor-page-cpu = CPU
monitor-page-memory = Memory
monitor-page-disks = Disks
monitor-page-network = Network
monitor-page-gpu = GPU
monitor-page-sensors = Sensors
monitor-page-connections = Connections
monitor-page-services = Services
monitor-page-startup = Startup
monitor-page-history = History
monitor-page-snapshots = Snapshots

## Process table columns

monitor-col-name = Name
monitor-col-pid = PID
monitor-col-parent = Parent
monitor-col-user = User
monitor-col-state = State
monitor-col-cpu = CPU
monitor-col-cpu-time = CPU time
monitor-col-memory = Memory
monitor-col-private = Private
monitor-col-shared = Shared
monitor-col-virtual = Virtual
monitor-col-peak-memory = Peak memory
monitor-col-memory-percent = Memory %
monitor-col-threads = Threads
monitor-col-handles = Handles
monitor-col-disk-read = Read/s
monitor-col-disk-write = Write/s
monitor-col-disk-read-total = Read total
monitor-col-disk-write-total = Written total
monitor-col-gpu = GPU
monitor-col-gpu-memory = GPU memory
monitor-col-page-faults = Page faults
monitor-col-priority = Priority
monitor-col-nice = Nice
monitor-col-session = Session
monitor-col-uptime = Running for
monitor-col-started = Started
monitor-col-path = Path
monitor-col-command = Command line
monitor-col-arch = Arch
monitor-col-elevated = Elevated
monitor-col-cgroup = Control group
monitor-col-description = Description

monitor-coldesc-name = The program's name
monitor-coldesc-pid = Process id
monitor-coldesc-parent = The id of the process that started it
monitor-coldesc-user = The account it runs under
monitor-coldesc-state = Running, sleeping, suspended and so on
monitor-coldesc-cpu = Processor load, on the scale chosen for this table
monitor-coldesc-cpu-time = Processor time used since it started
monitor-coldesc-memory = Resident memory (working set)
monitor-coldesc-private = Memory only this process holds
monitor-coldesc-shared = Resident memory shared with other processes
monitor-coldesc-virtual = Address space or commit size
monitor-coldesc-peak-memory = The most resident memory it has held
monitor-coldesc-memory-percent = Resident memory as a share of the machine's
monitor-coldesc-threads = Number of threads
monitor-coldesc-handles = Open handles or file descriptors
monitor-coldesc-disk-read = Read rate. On Windows this is all of its input, including pipes and sockets
monitor-coldesc-disk-write = Write rate. On Windows this is all of its output, including pipes and sockets
monitor-coldesc-disk-read-total = Bytes read since it started
monitor-coldesc-disk-write-total = Bytes written since it started
monitor-coldesc-gpu = Graphics processor load
monitor-coldesc-gpu-memory = Graphics memory it holds
monitor-coldesc-page-faults = Page faults since it started
monitor-coldesc-priority = Scheduling priority
monitor-coldesc-nice = Nice value, or base priority number on Windows
monitor-coldesc-session = Session or terminal
monitor-coldesc-uptime = How long it has been running
monitor-coldesc-started = When it started
monitor-coldesc-path = The executable file
monitor-coldesc-command = The full command line
monitor-coldesc-arch = Processor architecture of the program
monitor-coldesc-elevated = Runs with administrator rights
monitor-coldesc-cgroup = The Linux control group (container or service)
monitor-coldesc-description = The program's description, when it has one

## Views and states

monitor-view-list = List
monitor-view-tree = Tree
monitor-view-by-name = By program
monitor-view-by-user = By user

monitor-state-running = Running
monitor-state-sleeping = Sleeping
monitor-state-disk-wait = Disk wait
monitor-state-stopped = Stopped
monitor-state-zombie = Zombie
monitor-state-idle = Idle
monitor-state-suspended = Suspended
monitor-state-unknown = Unknown

monitor-priority-idle = Idle
monitor-priority-below-normal = Below normal
monitor-priority-normal = Normal
monitor-priority-above-normal = Above normal
monitor-priority-high = High
monitor-priority-realtime = Realtime

monitor-yes = Yes
monitor-no = No

## Process actions

monitor-action-end = End process
monitor-action-kill = Kill process
monitor-action-suspend = Suspend
monitor-action-resume = Resume
monitor-action-hangup = Send hangup
monitor-action-interrupt = Send interrupt

monitor-signal-done = { $what }: { $count ->
    [one] done
   *[other] done for { $count } processes
}
monitor-signal-failed = { $what } failed
monitor-no-windows = { $count ->
    [one] This process has no window to close
   *[other] { $count } processes have no window to close
}
monitor-signal-confirm-title = { $what }: { $count ->
    [one] this process?
   *[other] these { $count } processes?
}
monitor-signal-confirm = { $names }. Unsaved work in it may be lost.
monitor-signal-confirm-tree = { $names } and every process it started. Unsaved work in them may be lost.
monitor-risk-critical = The operating system relies on this process. Stopping it can freeze or restart the computer.
monitor-risk-other-user = This process belongs to another user.

monitor-priority-set = { $name }: priority set to { $what }
monitor-priority-failed = Could not change the priority of { $name }
monitor-affinity-set = { $name }: processor set changed
monitor-affinity-failed = Could not change the processors of { $name }
monitor-affinity-empty = Choose at least one processor
monitor-no-path = The path of this program is not known

monitor-error-gone = The process has already exited.
monitor-error-denied = Access denied.
monitor-error-unsupported = This system cannot do that.

monitor-no-data = No readings yet

## Services

monitor-service-start = Start
monitor-service-stop = Stop
monitor-service-restart = Restart
monitor-service-starting = Starting
monitor-service-stopping = Stopping
monitor-service-paused = Paused
monitor-service-failed-state = Failed
monitor-service-confirm-title = { $what } { $name }?
monitor-service-confirm = Other programs may depend on this service.
monitor-service-done = { $what }: { $name }
monitor-service-failed = { $what } { $name } failed

## Snapshots and exports

monitor-snapshot-taken = Snapshot saved: { $label }
monitor-snapshot-failed = Could not change the snapshots
monitor-snapshot-delete-title = Delete this snapshot?
monitor-snapshot-delete = "{ $label }" is removed for good.
monitor-exported = Exported
monitor-export-failed = The export failed
monitor-show-folder = Show folder

## Alerts

monitor-alert-open = Open monitor
monitor-alert-cpu = CPU load is high
monitor-alert-memory = Memory is nearly full
monitor-alert-temperature = A temperature is high
monitor-alert-body = { $value } has stayed above the limit of { $limit }.
monitor-alert-snapshot = A snapshot was saved.
