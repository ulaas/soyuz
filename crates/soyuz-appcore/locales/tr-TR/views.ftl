### Soyuz arayüz metinleri (tr-TR): views
### Anahtarlar en-US/views.ftl ile birebir aynı olmalı.

## Grup başlıkları (state/listing.rs)

views-group-folders = Klasörler
views-group-no-extension = Uzantısız
views-group-no-tags = Etiketsiz
views-group-size-empty = Boş
views-group-size-tiny = Çok küçük (< 16 KiB)
views-group-size-small = Küçük (< 1 MiB)
views-group-size-medium = Orta (< 128 MiB)
views-group-size-large = Büyük (< 1 GiB)
views-group-size-huge = Çok büyük (≥ 1 GiB)
views-group-date-unknown = Tarih bilinmiyor
views-group-date-future = Gelecekte
views-group-date-today = Bugün
views-group-date-yesterday = Dün
views-group-date-week = Bu hafta, daha önce
views-group-date-month = Bu ay, daha önce
views-group-date-year = Bu yıl, daha önce
views-group-date-older = Uzun zaman önce

## Tür sütunu (state/listing.rs kind_label)

views-kind-folder = Klasör
views-kind-junction = Bağlantı noktası
views-kind-folder-link = Klasör bağlantısı
views-kind-broken-link = Kırık bağlantı
views-kind-device = Aygıt
views-kind-pipe = Kanal
views-kind-socket = Soket
views-kind-file = Dosya
views-kind-ext-file = { $ext } dosyası
views-kind-png-image = PNG resmi
views-kind-jpeg-image = JPEG resmi
views-kind-gif-image = GIF resmi
views-kind-webp-image = WebP resmi
views-kind-bitmap-image = Bit eşlem resmi
views-kind-svg-image = SVG resmi
views-kind-icon = Simge
views-kind-tiff-image = TIFF resmi
views-kind-heic-image = HEIC resmi
views-kind-mp4-video = MP4 videosu
views-kind-matroska-video = Matroska videosu
views-kind-quicktime-video = QuickTime videosu
views-kind-avi-video = AVI videosu
views-kind-webm-video = WebM videosu
views-kind-mp3-audio = MP3 sesi
views-kind-flac-audio = FLAC sesi
views-kind-wav-audio = WAV sesi
views-kind-ogg-audio = Ogg sesi
views-kind-aac-audio = AAC sesi
views-kind-zip-archive = ZIP arşivi
views-kind-7-zip-archive = 7-Zip arşivi
views-kind-rar-archive = RAR arşivi
views-kind-tar-archive = Tar arşivi
views-kind-gzip-archive = Gzip arşivi
views-kind-compressed-file = Sıkıştırılmış dosya
views-kind-disc-image = Disk görüntüsü
views-kind-pdf-document = PDF belgesi
views-kind-word-document = Word belgesi
views-kind-excel-workbook = Excel çalışma kitabı
views-kind-powerpoint-deck = PowerPoint sunusu
views-kind-text-document = Metin belgesi
views-kind-text-file = Metin dosyası
views-kind-rich-text = Zengin metin
views-kind-csv-table = CSV tablosu
views-kind-html-page = HTML sayfası
views-kind-stylesheet = Stil sayfası
views-kind-rust-source = Rust kaynak kodu
views-kind-python-script = Python betiği
views-kind-c-source = C kaynak kodu
views-kind-cpp-source = C++ kaynak kodu
views-kind-csharp-source = C# kaynak kodu
views-kind-go-source = Go kaynak kodu
views-kind-java-source = Java kaynak kodu
views-kind-kotlin-source = Kotlin kaynak kodu
views-kind-swift-source = Swift kaynak kodu
views-kind-shell-script = Kabuk betiği
views-kind-powershell-script = PowerShell betiği
views-kind-batch-file = Toplu iş dosyası
views-kind-application = Uygulama
views-kind-installer = Yükleyici
views-kind-library = Kitaplık
views-kind-shortcut = Kısayol
views-kind-font = Yazı tipi
views-kind-log-file = Günlük dosyası
views-kind-settings = Ayar dosyası
views-kind-lock-file = Kilit dosyası

## Bilgisayar görünümü (ui/views/computer.rs)

views-computer-title = Aygıtlar ve sürücüler
views-computer-free-of = { $free } boş, toplam { $total }
views-computer-drive = Sürücü
views-computer-looking = Sürücüler aranıyor…
views-volume-fixed = Yerel Disk
views-volume-removable = Çıkarılabilir Disk
views-volume-network = Ağ Sürücüsü
views-volume-optical = CD Sürücüsü
views-volume-ram = RAM Diski

## Dosya listesi (ui/views/flow.rs, miller.rs)

views-a11y-files-in = { $title } içindeki dosyalar
views-a11y-row = { $kind ->
    [folder] { $name }, klasör
   *[other] { $name }, dosya
}
views-miller-unreadable = Bu klasör okunamıyor
views-miller-empty = Boş klasör

## Çöp Kutusu görünümü (ui/views/trash.rs)

views-trash-title = Çöp Kutusu
views-trash-reader-failed = Çöp Kutusu okuyucusu başlatılamadı
views-trash-reading = Çöp Kutusu okunuyor…
views-trash-empty-short = Boş
views-trash-summary = { $count } öğe · { $size }
views-trash-summary-selected = { $count } öğe · { $size } · { $selected } seçili
views-trash-empty-trash = Çöp Kutusu'nu boşalt
views-trash-empty-trash-tip = Çöp Kutusu'ndaki her şeyi kalıcı olarak sil
views-trash-delete-permanently = Kalıcı olarak sil
views-trash-restore = Geri yükle
views-trash-restore-selected = Seçilenleri geri yükle
views-trash-refresh = Yenile
views-trash-unreadable = Çöp Kutusu okunamıyor
views-trash-empty-title = Çöp Kutusu boş
views-trash-empty-subtitle = Sildiğiniz öğeler, Çöp Kutusu'nu boşaltana kadar burada kalır.
views-trash-col-original = Özgün konum
views-trash-col-deleted = Silinme tarihi
views-trash-unknown-origin = Bilinmiyor
views-trash-open-original = Özgün konumu aç
views-trash-copy-original = Özgün yolu kopyala

## Bölme içeriği durumları (ui/pane_view.rs)

views-pane-trash-pending = Çöp Kutusu'na burada henüz göz atılamıyor.
views-pane-recent = Son kullanılanlar
views-pane-recent-pending = Son kullanılan öğeler henüz izlenmiyor.
views-pane-search = Arama
views-pane-search-pending = Arama sonuçları burada görünecek.
views-pane-archive = Arşiv
views-pane-archive-pending = Arşivlere burada henüz göz atılamıyor.
views-pane-tags = Etiketler
views-pane-tags-pending = Etiketler henüz kullanılamıyor.
views-pane-access-denied = Erişim engellendi
views-pane-not-found = Klasör bulunamadı
views-pane-not-a-folder = Bu bir klasör değil
views-pane-cant-open = Bu klasör açılamıyor
views-pane-hint-go-up = Üst klasöre git
views-pane-hint-go-back = Geri git
views-pane-no-matches = Eşleşme yok
views-pane-no-matches-sub = Burada “{ $filter }” ile eşleşen bir şey yok.
views-pane-hint-clear-filter = Filtreyi temizle
views-pane-only-hidden = Yalnızca gizli öğeler
views-pane-only-hidden-sub = Bu klasörde yalnızca gizli öğeler var.
views-pane-hint-show-hidden = Gizli dosyaları göster
views-pane-timeline-empty = Bu hafta değişiklik yok
views-pane-timeline-empty-sub = Soyuz'un bildiği klasörlerde değiştirdiğiniz dosyalar en yenisi üstte olmak üzere burada görünür.
views-pane-empty = Bu klasör boş
views-pane-empty-sub = Dosyaları buraya bırakın ya da yeni bir şey oluşturun.

## Klasör ağacı (ui/tree)

views-tree-home = Giriş klasörü
views-tree-filter = Klasörleri filtrele
views-tree-clear-filter = Filtreyi temizle
views-tree-a11y = Disk ağacı
views-col-resize = { $column } sütununu yeniden boyutlandır
views-tree-no-matches = Eşleşen klasör yok
views-tree-open = Aç
views-tree-open-new-tab = Yeni sekmede aç
views-tree-open-other-pane = Diğer bölmede aç
views-tree-collapse = Daralt
views-tree-expand = Genişlet
views-tree-new-folder = Yeni klasör
views-tree-copy-path = Yolu kopyala
views-tree-add-favorite = Sık kullanılanlara ekle
views-tree-properties = Özellikler
views-tree-read-failed = Klasör okunamadı
views-tree-new-folder-name = Yeni klasör
views-tree-hidden = Disk ağacı gizlendi
views-tree-shown = Disk ağacı gösteriliyor

## Önizleme bölmesi (ui/preview)

views-preview-title = Önizleme
views-preview-diff = Fark
views-preview-show-hex = Onaltılık olarak göster
views-preview-close = Önizlemeyi kapat  (Alt+P)
views-preview-info = BİLGİ
views-preview-nothing-selected = Hiçbir şey seçili değil
views-preview-nothing-selected-sub = Önizlemek için bir dosya seçin.
views-preview-truncated = Önizleme kısaltıldı
views-preview-text-truncated = … önizleme kısaltıldı ([preview] max_text_kib)
views-preview-archive-entries = { $count } girdi
views-preview-archive-unpacked = { $format } · açılmış hâli { $size }
views-preview-browse = Göz at
views-preview-show-bytes = Baytları göster
views-preview-folder-unreadable = Bu klasör listelenemiyor
views-preview-folder-counts = { $dirs } klasör · { $files } dosya
views-preview-folder-size = { $files } dosyada { $size }
views-preview-folder-size-running = { $files } dosyada { $size } …
views-preview-folder-direct = Doğrudan içerdiği dosyalarda { $size }
views-preview-folder-hidden = { $count } gizli
views-preview-folder-newest = En yeni: { $name } · { $when }

views-preview-kind-folder = Klasör
views-preview-kind-image = Resim
views-preview-kind-svg = SVG resmi
views-preview-kind-text = Metin
views-preview-kind-archive = Arşiv
views-preview-kind-audio = Ses
views-preview-kind-video = Video
views-preview-kind-pdf = PDF belgesi
views-preview-kind-binary = İkili dosya
views-preview-kind-document = Belge
views-preview-kind-spreadsheet = Elektronik tablo
views-preview-kind-presentation = Sunu
views-preview-kind-ebook = E-kitap
views-preview-kind-font = Yazı tipi
views-preview-kind-model = 3B model

## Bilgi satırları (ui/preview/render.rs info_rows / info_label)

views-info-contains = { $files } dosya, { $dirs } klasör
views-info-size-bytes = { $size } ({ $bytes } bayt)
views-info-type = Tür
views-info-size = Boyut
views-info-contains-label = İçerik
views-info-compressed = Sıkıştırılmış
views-info-where = Konum
views-info-modified = Değiştirilme
views-info-created = Oluşturulma
views-info-accessed = Erişim
views-info-attributes = Öznitelikler
views-info-mode = İzinler
views-info-owner = Sahip
views-info-title = Başlık
views-info-artist = Sanatçı
views-info-album = Albüm
views-info-album-artist = Albüm sanatçısı
views-info-year = Yıl
views-info-genre = Tür
views-info-track = Parça
views-info-composer = Besteci
views-info-container = Kapsayıcı
views-info-format = Biçim
views-info-sample-rate = Örnekleme hızı
views-info-channels = Kanallar
views-info-bit-depth = Bit derinliği
views-info-duration = Süre
views-info-dimensions = Boyutlar
views-info-frame-rate = Kare hızı
views-info-bitrate = Bit hızı
views-info-tracks = Parçalar
views-info-color = Renk
views-info-compression = Sıkıştırma
views-info-encoding = Kodlama
views-info-encrypted = Şifreli
views-info-entries = Girdiler
views-info-language = Dil
views-info-lines = Satırlar
views-info-note = Not
views-info-pdf-version = PDF sürümü
views-info-previewer = Önizleyici
views-info-pages = Sayfalar
views-info-author = Yazar
views-info-producer = Üretici
views-info-publisher = Yayıncı
views-info-published = Yayın tarihi
views-info-version = Sürüm
views-info-epub-version = EPUB sürümü
views-info-page-size = Sayfa boyutu
views-info-paragraphs = Paragraflar
views-info-codec = Codec
views-info-copyright = Telif hakkı
views-info-designer = Tasarımcı
views-info-weight = Kalınlık
views-info-spacing = Aralık
views-info-variable-axes = Değişken eksenler
views-info-bounds = Sınırlar
views-info-faces = Yüzler
views-info-meshes = Ağlar
views-info-drawn = Çizilen
views-info-shown = Gösterilen
views-info-renderer = İşleyici
views-info-waveform = Dalga biçimi
views-info-video = Video
views-info-audio = Ses
views-info-subtitles = Altyazılar
views-info-pixel-format = Piksel biçimi
views-info-hdr = HDR
views-info-rotation = Döndürme
views-info-video-bitrate = Video bit hızı
views-info-audio-bitrate = Ses bit hızı
views-info-comment = Açıklama
views-info-encoder = Kodlayıcı
views-info-words = Sözcükler
views-info-sheets = Sayfalar
views-info-slides = Slaytlar
views-info-chapters = Bölümler
views-info-family = Aile
views-info-style = Stil
views-info-glyphs = Glifler
views-info-triangles = Üçgenler
views-info-vertices = Köşeler

## Fark (ui/preview/diff.rs)

views-diff-swap = Tarafları değiştir
views-diff-unified = Birleşik
views-diff-side-by-side = Yan yana
views-diff-failed = Bu dosyalar karşılaştırılamıyor
views-diff-identical = Dosyalar aynı
views-diff-binary-differ = İkili dosyalar farklı
views-diff-binary-sub = Taraflardan en az biri metin değil, bu yüzden satır farkı yok.
views-diff-none = Fark yok
views-diff-none-sub = İki dosyanın içeriği aynı.
views-diff-truncated = … fark kısaltıldı ([preview] max_diff_kib)

## Zengin önizlemeler (ui/preview/rich.rs)

views-rich-first-paragraphs = … önizleme ilk { $count } paragrafı gösteriyor
views-rich-doc-no-text = Bu belgede metin yok.
views-rich-first-sheet-only = Yalnızca ilk sayfa önizlenir
views-rich-sheet-empty = Bu sayfa boş
views-rich-more-rows = … daha fazla satır ([preview] sheet_rows)
views-rich-no-slides = Slayt bulunamadı.
views-rich-untitled-slide = Başlıksız slayt
views-rich-untitled = Başlıksız
views-rich-contents = İÇİNDEKİLER
views-rich-model-hint-full = Döndürmek için sürükleyin · yakınlaştırmak için kaydırın · çift tıklama sıfırlar
views-rich-model-hint = Döndürmek için sürükleyin
views-rich-play = Oynat
views-rich-audio-mono = mono
views-rich-audio-stereo = stereo
# $count: kanal sayısı (6).
views-rich-audio-channels = { $count } kanal
views-player-off = Video oynatıcı kapalı ([media] player)
# $error: dosyanın neden açılamadığı.
views-player-cant-open = Bu dosya oynatma için açılamıyor ({ $error })
views-rich-pause = Duraklat
# $name: video oynatıcının gösterdiği dosya (yüzeyi bir oynat / duraklat düğmesidir).
views-player-surface = Video: { $name }
views-player-opening = Açılıyor…
views-player-position = Konum
views-player-volume = Ses düzeyi
views-player-mute = Sessize al
views-player-unmute = Sesi aç
views-player-loop = Döngü
views-rich-poster-note = Kapak karesi (FFmpeg) · oynatma, oynatıcınızda açılır (Enter)
views-rich-prev-page = Önceki sayfa  (PgUp)
views-rich-next-page = Sonraki sayfa  (PgDn)
views-rich-page-of = Sayfa { $page } / { $pages }
views-rich-pages-text-only = { $pages } sayfa · yalnızca metin
views-rich-page = Sayfa
views-rich-text = Metin
views-rich-pdf-no-text = İlk sayfalarda ayıklanabilir metin yok.
views-rich-pdf-text-caption = METİN (İLK SAYFALAR)
views-rich-pdfium-needed = PDF sayfa önizlemesi için pdfium gerekli

## Hızlı Bakış (ui/quicklook.rs)

views-quicklook-close = Kapat  (Esc)
views-quicklook-position = { $pos } / { $count }
views-quicklook-hint-close = kapat
views-quicklook-hint-browse = gezin
views-quicklook-hint-open = aç
views-quicklook-hint-play = oynat / duraklat

## Arama çubuğu (ui/search.rs)

views-search-flat-needs-folder = Düz görünüm yalnızca klasörlerde çalışır
views-search-flat-title = { $folder } içindeki tüm dosyalar
views-search-save-failed = Aramalar kaydedilemedi
views-search-removed = Kayıtlı arama kaldırıldı: “{ $name }”
views-search-failed = Arama başarısız oldu
views-search-no-folders-to-measure = Ölçülecek klasör yok
views-search-close = Aramayı kapat  (Esc)
views-search-save-tip = Bu aramayı kaydet
views-search-stop = Durdur
views-search-run = Ara
views-search-everywhere = Her yerde
views-search-in = { $folder } içinde
views-search-everywhere-tip = Dizine eklenmiş tüm klasörlerde ara
views-search-in-tip = Şurada ara: { $path }
views-search-this-folder = Bu klasör
views-search-this-folder-named = Bu klasör ({ $folder })
views-search-all-indexed = Dizine eklenmiş tüm klasörler
views-search-all-indexed-tip = Arka plan dizininden anında sonuçlar
views-search-all-indexed-off-tip = Anında sonuçlar için arka plan dizinini Ayarlar → Önizleme ve arama bölümünden açın
views-search-gitignore-tip = .gitignore dosyalarına uy
views-search-hidden = Gizli
views-search-hidden-tip = Gizli dosyaları dahil et
views-search-kind = Tür
views-search-kind-tip = Türe göre filtrele
views-search-date = Tarih
views-search-date-tip = Değiştirilme tarihi
views-search-size = Boyut
views-search-size-tip = Dosya boyutu
views-search-contents = İçerik
views-search-contents-tip = Dosyaların içinde ara
views-search-case-tip = Büyük/küçük harf eşleştir
views-search-mode-tip = Adın nasıl eşleştirileceği
views-search-regex-tip = Normal ifade
views-search-mode-contains = İçerir
views-search-mode-glob = Joker
views-search-mode-regex = Normal ifade
views-search-mode-fuzzy = Bulanık
views-search-mode-contains-tip = Ad bu metni içerir
views-search-mode-fuzzy-tip = Gevşek eşleşme, alakaya göre sıralı
views-search-kind-any = Tüm türler
views-search-kind-files = Dosyalar
views-search-kind-folders = Klasörler
views-search-kind-images = Resimler
views-search-kind-videos = Videolar
views-search-kind-audio = Ses
views-search-kind-documents = Belgeler
views-search-kind-archives = Arşivler
views-search-kind-code = Kod
views-search-date-any = Herhangi bir zaman
views-search-date-today = Son 24 saat
views-search-date-week = Son 7 gün
views-search-date-month = Son 30 gün
views-search-date-year = Son 12 ay
views-search-date-older = Bir yıldan eski
views-search-size-caption = DOSYA BOYUTU
views-search-size-min = En az
views-search-size-min-hint = ör. 10 MB
views-search-size-max = En çok
views-search-size-max-hint = sınırsız
views-search-size-tiny = Çok küçük (< 16 KB)
views-search-size-large = Büyük (> 100 MB)
views-search-size-huge = Çok büyük (> 1 GB)
views-search-size-any = Herhangi bir boyut
views-search-hint-content = Dosya içeriklerinde ara…
views-search-hint-index = Dizinde ara…  kind:image  size:>5MB  modified:<7d  tag:red  -draft
views-search-hint-names = Bu klasörde ve alt klasörlerde adlara göre ara…  (kind:image size:>5MB …)
views-search-hint-glob = dosyalar: *.rs
views-search-save-caption = ARAMAYI KAYDET
views-search-saved = Kaydedildi: “{ $name }”
views-search-save-one-failed = Arama kaydedilemedi
views-search-head = { $title } — { $count } sonuç
views-search-from-index = dizinden { $ms } ms içinde
views-search-scanned = { $secs } sn içinde { $count } öğe tarandı
views-search-limit-reached = sonuç sınırına ulaşıldı
views-search-cancelled = iptal edildi
views-search-unreadable = { $count } okunamadı
views-search-cancel-tip = Aramayı durdur
views-search-relevance = Alaka
views-search-relevance-tip = Eşleşme kalitesine göre sırala
views-search-edit = Düzenle
views-search-edit-smart-tip = Bu akıllı klasörü düzenle
views-search-smart-folder = Akıllı klasör
views-search-smart-folder-tip = Kenar çubuğuna akıllı klasör olarak kaydet (güncel kalır)
views-search-refine = Daralt
views-search-refine-tip = Bu aramayı düzenle
views-search-sidebar = Aramalar
views-search-run-saved = Aramayı çalıştır
views-search-remove = Kaldır
views-search-defined-in-config = config.toml içinde tanımlı

## Akıllı klasörler (ui/smart.rs)

views-smart-need-name = Akıllı klasöre bir ad verin
views-smart-query-empty = Sorgu boş
views-smart-name-exists = “{ $name }” adlı bir akıllı klasör zaten var
views-smart-saved = “{ $name }” akıllı klasörü kaydedildi
views-smart-save-failed = Akıllı klasör kaydedilemedi
views-smart-deleted = “{ $name }” akıllı klasörü silindi
views-smart-deleted-body = Yalnızca kayıtlı sorgu kaldırıldı; hiçbir dosyaya dokunulmadı.
views-smart-rebuilding = Dizin yeniden oluşturuluyor
views-smart-rebuilding-body = Bu işlem bitene kadar aramalar diski tarar.
views-smart-error-title = Akıllı klasör “{ $name }”
views-smart-section = Akıllı klasörler
views-smart-open = Aç
views-smart-open-new-tab = Yeni sekmede aç
views-smart-edit = Düzenle veya yeniden adlandır…
views-smart-delete = Sil…
views-smart-new = Yeni akıllı klasör…
views-smart-new-tip = Güncel kalan kayıtlı bir sorgu, ör. kind:image modified:<7d
views-smart-help-words = ad bu sözcükleri / bu ifadeyi içerir
views-smart-help-pattern = ad deseni, uzantılar
views-smart-help-kind = image video audio document archive code file folder
views-smart-help-size = dosya boyutu
views-smart-help-age = yaş (h d w mo y) veya tarih
views-smart-help-tag = renk etiketi, yol metni, klasör
views-smart-help-content = metin dosyalarının içindeki sözcükler (içerik dizini gerekir)
views-smart-help-logic = hariç tut, ya da, grupla
views-smart-dialog-edit = Akıllı klasörü düzenle
views-smart-dialog-new = Yeni akıllı klasör
views-smart-name = Ad
views-smart-name-hint = Son fotoğraflar
views-smart-query = Sorgu
views-smart-live = Dizin tarafından yanıtlanır; dosyalar değiştikçe kendiliğinden yenilenir.
views-smart-not-covered = Dizin bunu henüz kapsamıyor; kapsayana kadar klasör diski tarar.
views-smart-index-off = Dizin oluşturma kapalı; klasör açıldığında diski tarar (Ayarlar → Önizleme ve arama).
views-smart-include-hidden = Gizli dosyaları dahil et
views-smart-type-query = Bir sorgu yazın
views-smart-name-taken = Bu ad kullanılıyor
views-smart-delete-title = Akıllı klasörü sil
views-smart-delete-confirm = “{ $name }” akıllı klasörü silinsin mi?
views-smart-delete-body = Yalnızca kayıtlı sorgu kaldırılır; gösterdiği dosyalara dokunulmaz.

## Dizin durumu (ui/smart.rs status_line / status_chip)

views-index-starting = Dizin başlatılıyor…
views-index-crawling-pct = Dizine ekleniyor %{ $pct } · { $count } öğe
views-index-crawling = Dizine ekleniyor · { $count } öğe
views-index-verifying = Değişiklikler denetleniyor · { $count } öğe
views-index-catching-up = NTFS günlüğündeki değişiklikler uygulanıyor…
views-index-contents = Dosya içerikleri okunuyor · { $count }
views-index-idle = Dizine eklendi · { $files } dosya, { $folders } klasör
views-index-failed = Dizin kapalı: { $error }
views-index-chip-pct = Dizine ekleniyor %{ $pct }
views-index-chip-indexing = Dizine ekleniyor
views-index-chip-updating = Dizin güncelleniyor
views-index-chip-contents = İçerikler dizine ekleniyor
views-index-chip-starting = Dizin
views-index-chip-idle = Dizine eklendi
views-index-chip-off = Dizin kapalı
views-index-chip-tip = Dizin ayarları için tıklayın; diğer seçenekler için sağ tıklayın
views-index-settings = Dizin ayarları…
views-index-rebuild = Dizini yeniden oluştur

## Arşivler (ui/archive.rs)

views-archive-opening = { $name } açılıyor…
views-archive-job-extract = Ayıkla: { $name }
views-archive-job-compress = Sıkıştır: { $name }
views-archive-job-delete = { $archive } arşivinden { $count } öğe sil
views-archive-job-add = { $archive } arşivine { $count } öğe ekle
views-archive-job-rename = { $archive } içinde { $name } öğesini yeniden adlandır
views-archive-select-to-extract = Ayıklamak için bir arşiv seçin
views-archive-nothing-to-extract = Ayıklanacak bir şey yok
views-archive-read-failed = Arşiv okunamadı
views-archive-compress-needs-folder = Sıkıştırma yalnızca bir klasördeki dosyalarda çalışır
views-archive-select-to-compress = Sıkıştırmak için dosya veya klasör seçin
views-archive-test-ok = { $name } sağlam
views-archive-test-ok-body = { $count } girdi sınandı
views-archive-test-damaged = { $name } içinde { $count } hasarlı girdi var
views-archive-test-failed = { $name } sınanamadı
views-archive-extract-selected = Seçilenleri ayıkla…
views-archive-extract-all = Tümünü ayıkla…
views-archive-extract-here = Buraya ayıkla
views-archive-extract = Ayıkla
views-archive-extract-to-folder = Klasöre ayıkla
views-archive-extract-to = Şuraya ayıkla…
views-archive-test = Arşivi sına
views-archive-compress-menu = Sıkıştır…
views-archive-password-title = Parola gerekli
views-archive-password-body = “{ $name }” şifreli. Göz atmak için parolasını girin.
views-archive-password-wrong = Bu parola işe yaramadı.
views-archive-unlock = Kilidi aç
views-archive-extract-what = { $name } arşivinden { $count } seçili öğe
views-archive-n-archives = { $count } arşiv
views-archive-destination = Hedef klasör
views-archive-subfolder = Arşivin adını taşıyan bir klasör oluştur
views-archive-compress = Sıkıştır
views-archive-name = Arşiv adı
views-archive-format = Biçim
views-archive-compression = Sıkıştırma
views-archive-level-store = Depola
views-archive-level-fast = Hızlı
views-archive-level-normal = Normal
views-archive-level-max = En yüksek
views-archive-password-optional = Parola (isteğe bağlı)
views-archive-names-encrypted = Dosya adları da şifrelenir.
views-archive-exists = “{ $name }” zaten var
views-archive-exists-body = Başka bir ad seçin.
views-archive-select-to-delete = Arşivden silinecek girdileri seçin
views-archive-select-one-to-rename = Yeniden adlandırmak için tek bir girdi seçin
views-archive-nothing-to-add = Eklenecek bir şey yok
views-archive-nothing-to-add-body = Önce dosyaları kopyalayın (Ctrl+C), sonra buraya yapıştırın.
views-archive-add-here = Kopyalanan dosyaları buraya ekle
views-archive-rename-menu = Arşivde yeniden adlandır…
views-archive-delete-menu = Arşivden sil
views-archive-rename-title = Arşivde yeniden adlandır
views-archive-rename-body = { $kind ->
    [folder] { $archive } içindeki “{ $name }” klasörü için yeni ad
   *[other] { $archive } içindeki “{ $name }” dosyası için yeni ad
}
views-archive-no-slashes = Ad eğik çizgi içeremez.
