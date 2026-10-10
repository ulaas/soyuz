### Soyuz UI strings (en-US source): automation
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/automation.ftl.

## Job origin badges (activity panel)

automation-origin-rule = Started by an automation rule
automation-origin-script = Started by a script
automation-origin-cli = Started from the soyuz command line

## Script actions

# Palette title of a script action that is not loaded; $id is "<file>/<id>".
automation-script-fallback-title = Script: { $id }
automation-script-action = Script action
automation-scripts-still-loading = Scripts are still loading
automation-no-action = No script action “{ $id }”
automation-no-action-body = Check Settings → Scripts.
automation-start-failed = Couldn't start the script
automation-script-failed = Script “{ $title }” failed
# A toast a script asked for: $title is the action, $text the script's message.
automation-script-toast = { $title }: { $text }
automation-unknown-tag = Unknown tag color “{ $tag }”
automation-not-running = Automations are not running in this window
automation-reloading = Reloading scripts
automation-not-loaded = Scripts are not loaded yet
automation-folder-failed = Couldn't create the scripts folder
automation-open-failed = Couldn't open { $name }

## Script templates

automation-template-example = Example: actions, a column and a previewer
automation-template-tidy = Rule script: sort by extension

## Rules runtime

automation-secondary-note = Rules run in the main Soyuz window, not in secondary windows.
automation-another-window = Another Soyuz window runs the rules.
# $error: the system error.
automation-cli-unavailable = not available: { $error } (another Soyuz window may serve it)

## Labels of jobs started from the command line ($count items, $name when there is one)

automation-cli-items = { $count ->
    [one] “{ $name }”
   *[other] { $count } items
}
automation-cli-copy = { $count ->
    [one] Copy “{ $name }” to “{ $dest }”
   *[other] Copy { $count } items to “{ $dest }”
}
automation-cli-move = { $count ->
    [one] Move “{ $name }” to “{ $dest }”
   *[other] Move { $count } items to “{ $dest }”
}
automation-cli-trash = { $count ->
    [one] Move “{ $name }” to trash
   *[other] Move { $count } items to trash
}
automation-cli-delete = { $count ->
    [one] Delete “{ $name }”
   *[other] Delete { $count } items
}

## Script approval card

automation-approval-title = { $count ->
    [one] “{ $action }” wants to make { $count } change
   *[other] “{ $action }” wants to make { $count } changes
}
automation-approval-intro = A script asked for these changes. Nothing has happened yet.
automation-and-more = …and { $count } more
automation-approval-note = Each change runs as a background job you can pause, cancel and undo. Turn this card off in Settings → Scripts.
automation-approval-run = Run

## Settings → Automations

automation-rules-group = Automations
automation-rules-run = Run rules
automation-rules-run-desc = Act on new items in watched folders while Soyuz is open.
automation-rules-dry-run = Dry run
automation-rules-dry-run-desc = Every rule only writes what it would do to its run log.
automation-rules-log-size = Run log size
automation-rules-log-size-desc = Entries kept per rule.
automation-rules-cli = Accept commands from the terminal
automation-rules-cli-desc = soyuz cp | mv | rm | trash | open | reveal | jobs send their jobs to this window (current user only).
automation-rules-new = New rule
# Default name of a new rule; $n is its number.
automation-rules-default-name = Rule { $n }
automation-rules-start-from = or start from
automation-rules-preset-pdf = PDFs from Downloads → Documents/PDF + blue tag
automation-rules-preset-pdf-name = File PDFs
automation-rules-preset-installers = Trash old installers
automation-rules-preset-installers-name = Old installers
automation-rules-intro = Each step runs as a job (activity panel, undo). Rules wait until an item stops changing, and never act on partial downloads.
automation-rules-name-hint = Rule name
automation-rules-status-off = Off
automation-rules-status-watching = Watching
automation-rules-status-not-running = Not running yet
automation-rules-delete = Delete rule
automation-rules-move-up = Move up
automation-rules-move-down = Move down
automation-rules-run-now = Run now
automation-rules-run-now-tip = Act on everything that matches right now
automation-rules-watch = Watch
automation-rules-subfolders = Subfolders
automation-rules-names = Names
automation-rules-glob-hint = *.pdf;*.epub  (empty = any)
automation-rules-regex-hint = regex (optional)
automation-rules-items = Items
automation-rules-size = Size (KiB)
automation-rules-no-limit = 0 = no limit
automation-rules-age = Age (minutes)
automation-rules-by-mtime = by modification time
automation-rules-wait = Wait
automation-rules-seconds-suffix = {" "}s
automation-rules-after-change = after the last change
automation-rules-existing = Also existing items
automation-rules-conflicts = Conflicts
automation-rules-then = Then
automation-rules-run-log = RUN LOG
automation-rules-clear = Clear
automation-rules-matches-now = MATCHES NOW
automation-rules-no-matches = Nothing in the folder matches right now.
automation-rules-dest-hint = destination folder
automation-rules-script-hint = script name
automation-rules-script-tip = A script in the scripts folder that defines fn rule(path, ctx)
automation-rules-remove-step = Remove step
automation-rules-add-step = Add a step
automation-step-move = Move
automation-step-copy = Copy
automation-step-rename = Rename
automation-step-tag = Tag
automation-step-trash = Trash
automation-step-script = Script

## Settings → Scripts

automation-scripts-group = Scripting
automation-scripts-run = Run scripts
automation-scripts-run-desc = Load scripts/*.rhai: their actions, columns, previewers and rule steps.
automation-scripts-approve = Approve script changes
automation-scripts-approve-desc = Show what an action wants to change before it runs. Off: changes start right away (still undoable).
automation-scripts-read = Let scripts read files
automation-scripts-read-desc = list, stat, search and read_text. Off: scripts only see the paths they are given.
automation-scripts-timeout = Time limit
automation-scripts-timeout-desc = Longest an action or rule script may run.
automation-scripts-cell-timeout = Column and preview time limit
automation-scripts-cell-timeout-desc = Per value of a script column or previewer call.
automation-scripts-ops = Operation limit
automation-scripts-ops-desc = Script operations per call before it is stopped.
automation-scripts-read-max = Largest file read
automation-scripts-read-max-desc = read_text returns at most this much.
automation-scripts-list-max = Most entries listed
automation-scripts-list-max-desc = list() and search() return at most this many.
automation-scripts-show-folder = Show scripts folder
automation-scripts-reload = Reload
automation-scripts-new-from = New from
automation-scripts-folder-note = Scripts reload when you save them. They run sandboxed (no processes, network or direct file changes): every change they ask for becomes a job you can undo. Bind actions in Keyboard as “run_script:<file>/<id>”.
automation-scripts-loading = Loading scripts…
automation-scripts-empty = No scripts yet. Start from a template above, or put .rhai files in the scripts folder.
# What the loaded scripts register.
automation-scripts-summary = { $scripts ->
        [one] { $scripts } script
       *[other] { $scripts } scripts
    } · { $actions ->
        [one] { $actions } action
       *[other] { $actions } actions
    } · { $columns ->
        [one] { $columns } column
       *[other] { $columns } columns
    } · { $previewers ->
        [one] { $previewers } previewer
       *[other] { $previewers } previewers
    }
automation-scripts-columns-capped = Only the first 4 columns are shown.
# $error: the script's runtime error.
automation-scripts-last-run = Last run: { $error }
automation-scripts-context-menu = context menu
automation-scripts-column = Column “{ $title }”
# $exts: file extensions, e.g. "csv, .tsv".
automation-scripts-previewer = Previewer “{ $title }” for .{ $exts }
automation-scripts-rule-step = Rule step: fn rule(path, ctx)
