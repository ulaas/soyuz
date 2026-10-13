### Soyuz UI strings (en-US source): tui
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui.ftl.

## Shared: the "not available" toast
tui-not-available = Not available in the terminal UI yet

## T1: the frame loop (crash reports, interrupted jobs)

## T2.1: [tui] settings

## T2.2: theme, colours and icons

## T2.3: pane views
tui-loading = Loading…
tui-empty = This folder is empty
tui-no-matches = No items match the filter
tui-virtual = This location is not available in the terminal UI yet
tui-failed = Cannot list this folder: { $error }

## T2.4: tabs, dual and quad panes

## T2.5: breadcrumbs and the address field

## T2.6: sidebar

## T2.7: command palette

## T2.8: status line, filter, toasts and borders
tui-hint = Ctrl+Q quit · Enter open · Backspace up
tui-hint-filter = Enter first match · Esc clear

## T2.9: mouse

## T8: screen-reader mode (the status sentence, the toasts in the status row)
# $path: the folders of the panes not drawn (joined with "; ").
tui-sr-other-pane = Other pane: { $path }
# A list row read out. $kind: tui-sr-folder / -file / -link; $pos: its position, $total: the items.
tui-sr-item = { $name }, { $kind }, { $pos } of { $total }
tui-sr-item-size = { $name }, { $kind }, { $size }, { $pos } of { $total }
tui-sr-folder = folder
tui-sr-file = file
tui-sr-link = link
tui-sr-selected = selected
# $column: the sort column's label; $dir: tui-sr-ascending / tui-sr-descending.
tui-sr-sorted = sorted by { $column }, { $dir }
tui-sr-ascending = ascending
tui-sr-descending = descending
# The kind of a toast, before its title ("Warning: Copy failed").
tui-sr-info = Info
tui-sr-success = Done
tui-sr-warning = Warning
tui-sr-error = Error
tui-sr-checked = checked
tui-sr-unchecked = not checked
