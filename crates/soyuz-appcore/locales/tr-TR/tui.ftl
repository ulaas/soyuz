### Soyuz arayüz metinleri (tr-TR): tui
### Anahtarlar en-US/tui.ftl ile birebir aynı olmalı.

## Shared: the "not available" toast and the features of later phases
tui-not-available = Terminal arayüzünde henüz yok
tui-feature-settings = Ayarlar
tui-feature-workspaces = Çalışma alanları
tui-feature-history = Sekme geçmişi
tui-feature-windows = Pencereler
tui-feature-properties = Özellikler
tui-feature-devices = Aygıtlar
tui-feature-smart = Akıllı klasörler
tui-feature-remote = Uzak konumlar
tui-feature-archive = Arşivler
tui-feature-about = Hakkında
tui-feature-confirm = Onay penceresi
tui-feature-select-pattern = Desene göre seç
tui-feature-columns = Sütunlar
tui-feature-open-with = Birlikte aç
tui-feature-batch-rename = Toplu yeniden adlandırma
tui-feature-attributes = Öznitelikler
tui-feature-links = Yeni bağlantı
tui-feature-quicklook = Hızlı Bakış
tui-feature-terminal = Terminal paneli
tui-feature-editor = Düzenleyici
tui-feature-jobs = İşler paneli
tui-feature-preview = Önizleme

## T1: the frame loop (crash reports, interrupted jobs)
tui-recovered-jobs = { $count ->
    [one] Yarıda kalan 1 işlem var — kurtarmak için grafik arayüzü açın
   *[other] Yarıda kalan { $count } işlem var — kurtarmak için grafik arayüzü açın
}
tui-crashed = Soyuz geçen sefer çöktü — rapor şuraya kaydedildi: { $path }

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

## T3: dialogs (the fallback until then)
tui-hint-close = Esc kapat
tui-hint-delete = Enter/Y onayla · N/Esc vazgeç
tui-hint-quit = K açık tut · W bitince çık · Q işleri iptal edip çık
tui-hint-prompt = Enter varsayılan yanıt · Esc sonra karar ver
tui-hint-reject = Esc reddet
tui-hint-cancel = Esc vazgeç
tui-dialog-unsupported = Bu pencere terminal arayüzünde henüz yok.
