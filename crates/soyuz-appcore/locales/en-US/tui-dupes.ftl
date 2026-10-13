### Soyuz UI strings (en-US source): tui-dupes
### The terminal UI: the duplicate finder (Phase T7).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-dupes.ftl.

tui-dupes-hints-list = ↑↓ move · Space mark · Enter fold / reveal · n/o/s keep in this group · N/O/S in every group · C clear · { $del }/X trash · F2 setup · F5 find · Esc close
tui-dupes-hints-setup = Tab next · ←/→ pick · Del remove the folder · Enter find · Alt+O add the other pane · F5 find · Esc close
tui-dupes-hints-add = Enter add the folder · ↑↓ pick a completion · Tab accept / next · Alt+O add the other pane · F5 find · Esc close
tui-dupes-hints-confirm = Enter move to trash · Esc cancel
tui-dupes-confirm = { $count ->
    [one] Move 1 file ({ $size }) to the trash?
   *[other] Move { $count } files ({ $size }) to the trash?
}
tui-dupes-setup-line = { $folders } · { $min } · F2 setup
tui-dupes-no-folders = No folders yet
tui-dupes-keep-keys = keep: n o s
