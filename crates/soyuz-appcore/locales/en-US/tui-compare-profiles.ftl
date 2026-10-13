### Soyuz UI strings (en-US source): tui-compare-profiles
### The terminal UI: editing and scheduling sync profiles (Phase T5).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-compare-profiles.ftl.

tui-cmpprof-title-new = New sync profile
tui-cmpprof-title-edit = Edit sync profile “{ $name }”
tui-cmpprof-title-rename = Rename sync profile “{ $name }”
tui-cmpprof-title-delete = Delete sync profile “{ $name }”?
tui-cmpprof-delete-body = The profile leaves the list and stops running on its schedule. Its run reports stay on disk.
tui-cmpprof-every = Every (min)
tui-cmpprof-at = At (HH:MM)
tui-cmpprof-new-name = New name
tui-cmpprof-err-empty-name = Give the profile a name.
tui-cmpprof-err-duplicate = A profile named “{ $name }” already exists.
tui-cmpprof-err-not-found = There is no profile named “{ $name }” any more.
tui-cmpprof-err-missing-path = Pick both folders.
tui-cmpprof-err-same-paths = The left and right folders are the same.
tui-cmpprof-err-running = “{ $name }” is running; cancel it (X) or wait until it ends.
tui-cmpprof-err-minutes = Enter whole minutes from { $min } to { $max }.
tui-cmpprof-err-time = Enter a time of day as HH:MM (00:00 to 23:59).
tui-cmpprof-toast-created = Sync profile “{ $name }” saved
tui-cmpprof-toast-updated = Sync profile “{ $name }” updated
tui-cmpprof-toast-renamed = Sync profile renamed to “{ $name }”
tui-cmpprof-toast-deleted = Sync profile “{ $name }” deleted
tui-cmpprof-toast-dry-run = Dry run of “{ $name }” started; the report lists what would change
tui-cmpprof-toast-cancelled = Cancelling “{ $name }”…
tui-cmpprof-hints-form = Tab next field · ←/→ pick an option · ↑↓ pick a folder, Tab takes it · Enter save · Esc cancel
tui-cmpprof-hints-rename = Enter rename · Esc cancel
tui-cmpprof-hints-delete = D or Enter delete · Esc keep it
tui-cmpprof-hints-page = ↑↓ choose · Enter load and compare · R run now · D dry run · X cancel the run · N new · S save the setup · E edit · F2 rename · Del delete · Esc back
tui-cmpprof-new = New
tui-cmpprof-edit = Edit
tui-cmpprof-save-setup = Save setup
tui-cmpprof-schedule-line = { $schedule } · { $next }
tui-cmpprof-report-line = { $when }  { $status } · { $trigger } · { $summary }
