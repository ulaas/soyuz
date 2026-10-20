### Soyuz UI strings (en-US source): panels
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/panels.ftl.

## Bottom panel (ui/panels/mod.rs)

panels-tab-terminal = Terminal
panels-tab-activity = Activity
panels-close-panel = Close panel
panels-close-activity = Close activity

## Activity panel: toolbar, status-bar indicator, empty state

panels-tab-jobs = Jobs
panels-tab-jobs-pending = Jobs · { $count }
panels-tab-history = History
panels-clear-finished = Clear finished
panels-cancel-all = Cancel all
panels-resume-all = Resume all
panels-pause-all = Pause all
panels-needs-input = Needs input
panels-jobs-count = { $count ->
    [one] { $count } job
   *[other] { $count } jobs
}
panels-show-activity-tip = Show activity (Ctrl+Shift+J)
panels-jobs-empty = No operations
panels-jobs-empty-hint = Copies, moves and deletes show their progress here.

## Job cards

# $position: place in the queue (1 = next).
panels-state-queued-at = Queued · #{ $position } in line
panels-state-queued = Queued
panels-state-planning = Counting items…
panels-state-working = Working…
panels-state-paused = Paused
panels-state-waiting = Waiting for your answer
panels-state-finishing = Finishing…
panels-verified = Verified
panels-verifying = Verifying
panels-verified-tip = Every copied file was read back and its BLAKE3 hash compared with the source
# $done / $total: formatted sizes ("1.5 GiB").
panels-progress-of = { $done } of { $total }
panels-items-of = { $total ->
    [one] { $done } of { $total } item
   *[other] { $done } of { $total } items
}
# $speed: "12.3 MB/s"; $eta: "2m 10s".
panels-speed-eta = { $speed } · { $eta } left
panels-speed-avg = avg { $speed }
panels-hide-details = Hide details
panels-show-log = Show log and errors
panels-more = More
panels-priority = Priority
panels-priority-high = High
panels-priority-normal = Normal
panels-priority-low = Low
panels-move-up = Move up
panels-move-down = Move down
panels-start-now = Start now
panels-resume = Resume
panels-pause = Pause
panels-resolve = Resolve…
panels-resolve-conflict = Resolve conflict
panels-remove-from-list = Remove from list
panels-retry-failed = Retry failed
panels-retry-failed-items = Retry failed items
panels-log-empty = No log entries yet.

## History tab

panels-history-empty = No history yet
panels-history-empty-hint = Finished operations appear here and can be undone.
panels-history-undone = Undone
panels-history-verified-tip = Every copied file was hash-compared with its source (BLAKE3)
panels-history-not-undoable = can't be undone
panels-history-not-undoable-tip = This operation can't be undone (e.g. a permanent delete).

## Terminal panel

# $number: session number.
panels-terminal-default-name = Terminal { $number }
# $name: session name (accessibility label of the terminal).
panels-terminal-a11y = Terminal: { $name }
panels-terminal-command-not-run = Command not run
panels-terminal-command-not-run-body = The command line or folder contains control characters (e.g. a newline) and cannot be typed into a shell safely.
panels-terminal-clip-title = The terminal wants to set your clipboard:
panels-terminal-clip-empty = (empty)
panels-terminal-clip-warning = Programs can plant commands for you to paste. Allow only if you expect it.
panels-terminal-clip-deny = Deny
panels-terminal-clip-allow-once = Allow once
panels-terminal-clip-always = Always for this session
panels-terminal-clip-always-tip = Allow clipboard writes from the terminal until Soyuz restarts
panels-terminal-new = New terminal
panels-terminal-open-here = Open terminal here
panels-terminal-open-external = Open external terminal here
panels-terminal-busy = Terminal busy — didn't change folder
# $error: the shell's start error.
panels-terminal-shell-failed = Couldn't start the shell: { $error }
panels-terminal-unknown-error = unknown error
panels-terminal-link-not-found = Link target not found
panels-terminal-exe-revealed = Executable revealed, not run
panels-terminal-exe-revealed-body = Links in terminal output never start programs. Open it from the file list if you trust it.
panels-terminal-reveal-failed = Couldn't reveal the file
panels-terminal-link-failed = Couldn't open the link

## "Why is this slow?" in a job's details.
panels-io-title = Why is this slow?
panels-io-now = now { $speed }
panels-io-avg = average { $speed }
panels-io-peak = peak { $speed }
panels-io-files-per-sec = { $rate } files/s
panels-io-avg-file = { $size } per file
panels-io-time-moving = moving { $time }
panels-io-time-stalled = stalled { $time }
panels-io-time-counting = counting { $time }
panels-io-time-queued = queued { $time }
panels-io-time-paused = paused { $time }
panels-io-time-waiting = waiting for you { $time }
panels-io-queued-for = Queued for { $time }.
panels-io-blocked-device = Waiting for job { $job }, which uses the same disk (jobs on one disk run one after another; Start now runs it anyway).
panels-io-blocked-device-any = Waiting for another job on the same disk.
panels-io-blocked-folders = Waiting for job { $job }, which works in the same folders.
panels-io-blocked-folders-any = Waiting for another job in the same folders.
panels-io-blocked-parallel = { $count } jobs already run at once (Settings → Jobs).
panels-io-blocked-probing = Finding out which disks it uses…
panels-io-hint-answer = Waiting for your answer.
panels-io-hint-stalled = Nothing moved for { $stalled } of { $total }: the disk or the network is busy, or another program (an antivirus scan) holds the files.
panels-io-hint-small-files = Mostly small files ({ $size } on average): each one costs a create, its metadata and a close, so files per second matter more than the speed.
panels-io-hint-same-disk = It reads and writes the same disk, so the two compete.
panels-io-hint-verify = Each copy is read back and hashed (verify after copy).
panels-io-hint-counting = Counting the files took { $time }.
panels-io-hint-none = Nothing holds it back: this is the speed the disks give.
