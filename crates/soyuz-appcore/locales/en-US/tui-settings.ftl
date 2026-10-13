### Soyuz UI strings (en-US source): tui-settings
### The terminal UI: the settings editor and the key bindings page (Phase T6).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-settings.ftl.

## The editor
tui-settings-title = Settings
tui-settings-results = Results for “{ $query }”
tui-settings-saving = Saving…
tui-settings-saved = Saved
tui-settings-search-hint = Type to search all settings
tui-settings-filter-hint = Type to filter the shortcuts
tui-settings-no-results = Nothing matches.
tui-settings-hint-rows = ↑↓ move · Enter edit · ←→ change · Del reset · / search · Tab sections · Ctrl+R reset section · Ctrl+E edit file · F1 help · Esc close
tui-settings-hint-nav = ↑↓ section · Enter/Tab keys · Ctrl+E edit file · Esc close
tui-settings-hint-search = Enter/↓ go to the results · Esc clear
tui-settings-hint-edit = Enter save · Esc cancel
tui-settings-hint-keys = Enter add shortcut · ←→ choose · Del remove · Backspace preset · Alt+C conflicts · Alt+M changed · Esc close
tui-settings-choose-hint = ↑↓ choose · Enter set · Esc cancel

## Values and descriptions
tui-settings-unset = (not set)
tui-settings-empty = (empty)
tui-settings-table-summary = { $count ->
    [one] 1 item — Enter to edit
   *[other] { $count } items — Enter to edit
}
tui-settings-table-hint = Enter opens the list of entries
tui-settings-opaque-summary = { $count ->
    [one] 1 entry
   *[other] { $count } entries
}
tui-settings-opaque-hint = Edit this one in config.toml (Ctrl+E)
tui-settings-list-desc = Enter edits the list
tui-settings-range = Range: { $min } – { $max }
tui-settings-values = Values: { $values }
tui-settings-default = Default: { $value }
tui-settings-restart-note = Takes effect after a restart

## Edits
tui-settings-invalid = Invalid value: { $error }
tui-settings-not-a-number = “{ $text }” is not a number
tui-settings-restart = Takes effect after a restart
tui-settings-reset-title = Reset { $section }?
tui-settings-reset-body = Every setting on this page goes back to its default. Your own entries (profiles, rules, saved searches, pinned folders) stay.
tui-settings-section-reset = { $section } reset to the defaults

## The list editor
tui-settings-list-item = Item
tui-settings-list-add = Add
tui-settings-list-remove = Remove
tui-settings-list-empty = (empty list)
tui-settings-list-hint = ↑↓ choose · Enter put the field into the item · Insert new item · Alt+↑↓ move · Alt+S save · Esc cancel
tui-settings-list-map-entry = “{ $entry }” needs the form name = value

## The key bindings page and the shortcut recorder
tui-settings-keys-problems = { $count ->
    [one] 1 problem with the shortcuts:
   *[other] { $count } problems with the shortcuts:
}
tui-settings-keys-more = …and { $count } more
tui-settings-keys-conflicts-only = Only conflicts
tui-settings-keys-modified-only = Only changed shortcuts
tui-settings-keys-filter-desc = Enter or Space switches the filter.
tui-settings-keys-contexts = Where: { $contexts }
tui-settings-keys-overlay = Added by the terminal UI for terminals that cannot send the original: { $keys }
tui-settings-keys-overlay-fixed = { $keys } is added by the terminal UI and cannot be removed
tui-settings-record-title = Shortcut for { $action }
tui-settings-record-waiting = Press the keys (up to three for a sequence)…
tui-settings-record-keys = Keys: { $keys }
tui-settings-record-used = Already used by: { $actions }
tui-settings-record-hint = Enter saves · Esc cancels · every other key is recorded
tui-settings-record-clear = Clear
