### Soyuz UI strings (en-US source): tui-versions
### The terminal UI: previous versions (Phase T7).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-versions.ftl.

# Dialog title; $name: the file or folder name.
tui-versions-title = Previous versions: { $name }
tui-versions-hints = Enter open · C compare · T copy to · R restore · F5 look again · Esc close
tui-versions-hints-folder = Enter open · T copy to · R restore · F5 look again · Esc close
# Windows, copies denied: a message only (the terminal UI never restarts itself elevated).
# $kind: "file" | "folder".
tui-versions-admin-hint = This volume has { $count ->
    [one] { $count } shadow copy
   *[other] { $count } shadow copies
    } that only administrators can read. To see earlier versions of this { $kind ->
    [folder] folder
   *[other] file
    }, close Soyuz, open a terminal with “Run as administrator” and start soyuz --tui there.
# The Properties dialog's button.
tui-versions-button = Previous versions…
