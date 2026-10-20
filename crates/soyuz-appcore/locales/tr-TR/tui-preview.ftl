### Soyuz UI strings (tr-TR): tui-preview
### The terminal UI: the preview pane and images (Phase T4).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in en-US/tui-preview.ftl.

tui-preview-loading = Yükleniyor…
tui-preview-hex = onaltılık
tui-preview-image-off = Resimler kapalı ([tui] images).
tui-preview-image-missing = Resim çizilemedi.
tui-preview-image-pending = Resim çiziliyor…
tui-preview-image-no-room = Resim için yer yok.
tui-preview-pages = { $count } sayfa
tui-preview-pdf-text-only = Yalnızca metin: { $reason }
tui-preview-pdf-locked = Parola korumalı: önizlemek için “PDF parolası…” komutunu çalıştırın (öğe menüsü veya palet).
tui-preview-rich = Bu { $kind } için henüz terminal görünümü yok; ayrıntılar aşağıda.
tui-preview-hex-hint = Baytları görmek için komut paletinde “{ $action }” komutunu çalıştırın.

## Phase V3b: ortam oynatıcı (player.rs, view/preview.rs, view/quicklook.rs)

tui-player-opening = Oynatıcı açılıyor…
tui-player-playing = Oynatılıyor
tui-player-paused = Duraklatıldı
tui-player-ended = Bitti
# $error: oynatmanın neden başarısız olduğu.
tui-player-error = Oynatma başarısız: { $error }
tui-player-muted = sessiz
tui-player-loop = döngü
# $percent: 0–100.
tui-player-volume = ses { $percent } %
# Ekran okuyucu kipi; $pos, $total: d:ss.
tui-sr-player-time = { $pos } / { $total }
# Hızlı Bakış alt bilgi ipuçları, atanmış tuşlardan sonra.
tui-player-hint-play = oynat / duraklat
tui-player-hint-seek = atla
tui-player-hint-mute = sessiz
