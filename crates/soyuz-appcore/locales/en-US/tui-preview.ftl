### Soyuz UI strings (en-US source): tui-preview
### The terminal UI: the preview pane and images (Phase T4).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-preview.ftl.

tui-preview-loading = Loading…
tui-preview-hex = hex
tui-preview-image-off = Pictures are off ([tui] images).
tui-preview-image-missing = The picture could not be drawn.
tui-preview-image-pending = Drawing the picture…
tui-preview-image-no-room = No room for the picture.
tui-preview-pages = { $count ->
    [one] 1 page
   *[other] { $count } pages
}
tui-preview-pdf-text-only = Text only: { $reason }
tui-preview-rich = No terminal view for this { $kind } yet; the details are below.
tui-preview-hex-hint = See the bytes with “{ $action }” in the command palette.

## Phase V3b: the media player (player.rs, view/preview.rs, view/quicklook.rs)

tui-player-opening = Opening the player…
tui-player-playing = Playing
tui-player-paused = Paused
tui-player-ended = Ended
# $error: why playback failed.
tui-player-error = Playback failed: { $error }
tui-player-muted = muted
tui-player-loop = loop
# $percent: 0–100.
tui-player-volume = volume { $percent } %
# Screen-reader mode; $pos, $total: m:ss.
tui-sr-player-time = { $pos } of { $total }
# Quick Look footer hints, after the bound keys.
tui-player-hint-play = play / pause
tui-player-hint-seek = seek
tui-player-hint-mute = mute
