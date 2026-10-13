### Soyuz arayüz metinleri (tr-TR): tui
### Anahtarlar en-US/tui.ftl ile birebir aynı olmalı.

## Shared: the "not available" toast
tui-not-available = Terminal arayüzünde henüz yok

## T1: the frame loop (crash reports, interrupted jobs)

## T2.1: [tui] settings

## T2.2: theme, colours and icons

## T2.3: pane views
tui-loading = Yükleniyor…
tui-empty = Bu klasör boş
tui-no-matches = Filtreyle eşleşen öğe yok
tui-virtual = Bu konum terminal arayüzünde henüz yok
tui-failed = Bu klasör listelenemiyor: { $error }

## T2.4: tabs, dual and quad panes

## T2.5: breadcrumbs and the address field

## T2.6: sidebar

## T2.7: command palette

## T2.8: status line, filter, toasts and borders
tui-hint = Ctrl+Q çık · Enter aç · Backspace yukarı
tui-hint-filter = Enter ilk eşleşme · Esc temizle

## T2.9: mouse

## T8: screen-reader mode (the status sentence, the toasts in the status row)
# $path: the folders of the panes not drawn (joined with "; ").
tui-sr-other-pane = Diğer bölme: { $path }
# A list row read out. $kind: tui-sr-folder / -file / -link; $pos: its position, $total: the items.
tui-sr-item = { $name }, { $kind }, { $total } öğeden { $pos }.
tui-sr-item-size = { $name }, { $kind }, { $size }, { $total } öğeden { $pos }.
tui-sr-folder = klasör
tui-sr-file = dosya
tui-sr-link = bağlantı
tui-sr-selected = seçili
# $column: the sort column's label; $dir: tui-sr-ascending / tui-sr-descending.
tui-sr-sorted = sıralama: { $column }, { $dir }
tui-sr-ascending = artan
tui-sr-descending = azalan
# The kind of a toast, before its title ("Warning: Copy failed").
tui-sr-info = Bilgi
tui-sr-success = Tamamlandı
tui-sr-warning = Uyarı
tui-sr-error = Hata
tui-sr-checked = işaretli
tui-sr-unchecked = işaretli değil
