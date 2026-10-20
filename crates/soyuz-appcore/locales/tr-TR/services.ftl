### Soyuz arayüz metinleri (tr-TR): services
### Anahtarlar en-US/services.ftl ile birebir aynı olmalı.


## Preview kinds (shown in the preview header)

services-kind-folder = Klasör
services-kind-image = Görüntü
services-kind-svg = SVG görüntüsü
services-kind-text = Metin
services-kind-markdown = Markdown
services-kind-archive = Arşiv
services-kind-audio = Ses
services-kind-video = Video
services-kind-pdf = PDF belgesi
services-kind-binary = İkili dosya
services-kind-document = Belge
services-kind-spreadsheet = Elektronik tablo
services-kind-presentation = Sunu
services-kind-ebook = E-kitap
services-kind-font = Yazı tipi
services-kind-model = 3B model

## Preview messages

services-preview-internal-error = iç hata
services-preview-failed = Önizleme başarısız oldu
services-preview-crashed = Önizleyici bu dosyada çöktü ({ $reason }).
services-preview-entry-not-found = arşiv girdisi bulunamadı
services-preview-cant-read-file = Bu dosya okunamıyor
services-preview-limited = Önizleme ilk 64 MiB ile sınırlı
services-preview-pdf-no-feature = Sayfaları göstermek için isteğe bağlı `pdf` derleme özelliği gerekir. Ham baytları tek tıkla görebilirsiniz.
services-preview-cant-read-image = Bu görüntü okunamıyor
services-preview-unsupported-image = Desteklenmeyen görüntü
services-preview-image-too-large = Görüntü önizlenemeyecek kadar büyük
services-preview-image-too-large-body = { $mp } MP, { $limit } MP önizleme sınırını aşıyor ([preview] max_image_mp).
services-preview-cant-decode-image = Bu görüntünün kodu çözülemiyor
services-preview-script = Betik
services-preview-script-failed = Betik önizleyici başarısız oldu
services-preview-nested-archive = İç içe arşiv
services-preview-nested-archive-body = Göz atmak için bulunduğu klasörden açın.
services-preview-password = Parola korumalı
services-preview-password-body = Açmak için Enter'a basın ve parolayı yazın.
services-preview-cant-read-archive = Bu arşiv okunamıyor

## Preview fact values (labels are translated by the preview panel)

services-fact-compression-saved = %{ $ratio } tasarruf

## Audio / video metadata values

services-media-wave = WAVE ses
services-media-ieee-float = IEEE kayan nokta
services-media-bits = { $bits } bit
services-media-mono = Mono
services-media-stereo = Stereo
services-media-flac = FLAC ses
services-media-mp3 = MP3 ses
services-media-quicktime = QuickTime filmi
services-media-mpeg4-audio = MPEG-4 ses
services-media-tracks-av = Video + ses
services-media-tracks-video = Video
services-media-tracks-audio = Ses

## Rich previews (documents, e-books, fonts, 3D models, audio, video, PDF)

services-preview-too-large = Önizlenemeyecek kadar büyük
services-preview-too-large-body = { $size } MiB, { $limit } MiB sınırını aşıyor ([preview] { $key }).
services-preview-sheet-shown = { $rows } satır × { $cols } sütun
services-preview-cant-read-document = Bu belge okunamıyor
services-preview-cant-read-ebook = Bu e-kitap okunamıyor
services-preview-cant-read-font = Bu yazı tipi okunamıyor
services-preview-cant-render-font = Bu yazı tipi görüntülenemiyor
services-preview-monospaced = Eş aralıklı
services-preview-faces = { $count } (ilki gösteriliyor)
services-preview-cant-read-model = Bu model okunamıyor
services-preview-decimated = { $count } (seyreltilmiş)
services-preview-waveform-skipped = { $size } MiB üzerinde atlandı ([preview] max_audio_mib)
services-preview-poster-disk-only = Kapak kareleri yalnızca diskteki dosyalar için oluşturulur.
services-preview-no-ffmpeg-libs = FFmpeg kitaplıkları yüklü olmadığından kapak karesi yok. Soyuz onları programın yanında dağıtır; eklemek için “FFmpeg çalışma ortamını indir” komutunu çalıştırın (Ayarlar → Önizleme → Medya ya da komut paleti), GitHub'daki “{ $tag }” sürümünü indirip { $dir } klasörüne açın ya da Ayarlar → Önizleme → Medya'da klasörlerini gösterin.
services-preview-crashed-last-time = Önizlenmedi: bu dosyanın çözülmesi geçen sefer medya kitaplığını çökertti. Yeniden denemek için Hızlı Bakış'ta açın.
services-preview-no-pdf-renderer = Bu derlemede PDF işleyici yok (cargo özelliği `pdf`).
services-preview-no-pdf-renderer-short = Bu derlemede PDF işleyici yok
services-preview-cant-read-pdf = Bu PDF okunamıyor
services-preview-text-only = yalnızca metin

## Audio and video previews

services-audio-unsupported = Desteklenmeyen ses ({ $error })
services-audio-no-track = Ses izi yok
services-audio-unsupported-codec = Desteklenmeyen kodek ({ $error })
services-audio-player-failed = Ses oynatıcı bu dosyada başarısız oldu
services-audio-no-device = Ses çıkış aygıtı yok ({ $error })
services-audio-cant-play = Bu dosya oynatılamıyor ({ $error })
services-audio-no-ffmpeg = FFmpeg kitaplıkları yüklü değil ([media] audio_backend = ffmpeg)
services-audio-open-timeout = Dosyayı oynatma için açmak çok uzun sürdü
# $major: avcodec ana sürümü (63).
services-audio-decoder-ffmpeg = FFmpeg (avcodec { $major })
services-audio-decoder-symphonia = Yerleşik (Symphonia)
services-video-no-frame = Kapak karesi yok ({ $error })
services-diff-failed = Bu dosyalarda karşılaştırma başarısız oldu

## Places, timeline, stores

services-places-onedrive-work = OneDrive (iş)
services-timeline-summary = Bugün { $today } · Bu hafta { $week } · Toplam { $total }
services-store-blocked = { $path } başlangıçta okunamadı; üzerine yazılmıyor (düzeltin veya silin ve yeniden başlatın)

## Configuration reload

services-config-reloaded-warnings = Yapılandırma uyarılarla yeniden yüklendi
services-config-reloaded = Yapılandırma yeniden yüklendi
services-config-error-at = config.toml: { $line }. satır, { $col }. sütunda hata
services-config-load-failed = config.toml yüklenemedi
services-config-error-body =
    { $error }
    Önceki ayarlar etkin kalır.
services-config-edit = Yapılandırmayı düzenle

## Office documents, e-books and PDF

services-office-bad-container = Geçerli bir belge kapsayıcısı değil ({ $error })
services-office-no-body = Bu belgenin okunabilir bir gövdesi yok ({ $part })
services-office-no-body-sheet = Bu elektronik tablonun okunabilir bir gövdesi yok ({ $part })
services-office-no-body-slides = Bu sununun okunabilir bir gövdesi yok ({ $part })
services-office-first-sheet = İlk sayfa okunamıyor
services-office-word = Word belgesi
services-office-odt = OpenDocument metni
services-office-excel = Excel çalışma kitabı
services-office-ods = OpenDocument elektronik tablosu
services-office-pptx = PowerPoint sunusu
services-office-odp = OpenDocument sunusu
services-office-sheet = Sayfa
services-office-true = DOĞRU
services-office-false = YANLIŞ
services-epub-no-container = EPUB değil (META-INF/container.xml eksik)
services-epub-no-package = EPUB bir paket belgesi belirtmiyor
services-epub-package-unreadable = EPUB paket belgesi okunamıyor
services-pdf-over-limit = { $limit } MiB belge sınırından büyük
services-pdf-cant-read = Bu PDF okunamıyor ({ $error })
services-pdf-parser-failed = PDF ayrıştırıcı bu dosyada başarısız oldu
services-pdf-renderer-stopped = PDF işleyici durdu
services-pdf-renderer-failed = PDF işleyici bu dosyada başarısız oldu
services-pdf-too-large = Bu PDF çok büyük
services-pdf-password = Bu PDF parola korumalı
services-pdf-password-wrong = Bu parola bu PDF'i açmadı
services-pdf-cant-open = Bu PDF açılamıyor ({ $error })
services-pdf-no-page = { $page }. sayfa yok ({ $error })
services-pdf-cant-render-page = Bu sayfa işlenemiyor ({ $error })
services-pdf-page-empty = Sayfa boş olarak işlendi
services-pdf-missing =
    Sayfaları göstermek için pdfium kullanılır, ancak Soyuz ile birlikte gelmez. Hazır derlenmiş bir { $file } dosyası indirin (ör. GitHub'daki bblanchon/pdfium-binaries sürümlerinden, işlemcinize uygun olanı) ve şu klasörlerden birine koyun:

    { $folders }

    Ya da [preview] pdfium_dir ayarını yapın. Yeniden başlatmak gerekmez.

## Fonts and 3D models

services-font-collection = TrueType koleksiyonu
services-font-bad-woff = Hasarlı WOFF dosyası
services-font-woff-table = WOFF tablosu çok büyük
services-font-unreadable = Okunabilir bir yazı tipi değil ({ $error })
services-font-empty-collection = Yazı tipi koleksiyonu boş
services-font-unnamed = Adsız yazı tipi
services-font-regular = Normal
services-font-glyphs-caption = Glifler ({ $count } gösteriliyor)
services-font-cant-render = Bu yazı tipi görüntülenemiyor
services-font-pangram = Pijamalı hasta yağız şoföre çabucak güvendi.
services-font-characters = Karakterler
services-model-gltf-binary = glTF ikili
services-model-unknown = Bilinmeyen model biçimi
services-model-no-triangles = Modelde üçgen yok
services-model-bad-ascii-stl = Hasarlı ASCII STL
services-model-bad-stl = Hasarlı STL dosyası
services-model-bad-gltf = Okunabilir bir glTF dosyası değil ({ $error })

## Archives and tags

services-archive-password = Bu arşiv parola korumalı
services-archive-wrong-password = Yanlış parola
services-archive-corrupt = Bozuk arşiv: { $error }
services-archive-unsupported = { $format } arşivleri desteklenmiyor
services-archive-open-failed = { $name } açılamadı
services-archive-temp-failed = geçici klasör oluşturulamıyor: { $error }
services-archive-unsafe-name = “{ $name }” güvenli bir dosya adı değil
services-tag-red = Kırmızı
services-tag-orange = Turuncu
services-tag-yellow = Sarı
services-tag-green = Yeşil
services-tag-cyan = Camgöbeği
services-tag-blue = Mavi
services-tag-purple = Mor
services-tag-gray = Gri
services-tag-unknown = Bilinmeyen etiket: “{ $name }”.

## Index search

services-index-bad-size = “{ $size }” boyutu okunamıyor
services-index-size-range = En küçük boyut en büyük boyuttan büyük
services-index-empty-query = Aranacak bir şey yazın
services-index-no-folder = Aramak için bir klasör seçin
services-index-no-roots = Dizine eklenmiş klasör yok; Ayarlar → Önizleme ve arama bölümünden ekleyin

## Crash recovery and safe mode

services-crash-safe-mode = Güvenli mod
services-crash-safe-mode-body = Varsayılan ayarlar; oturum geri yükleme, klasör izleme, küçük resimler ve git durumu yok. Güvenli moddan çıkmak için Soyuz'u normal şekilde yeniden başlatın.
services-crash-nothing-to-resume = Sürdürülecek bir şey kalmadı
services-crash-resuming = { $count } işlem sürdürülüyor
services-crash-nothing-to-roll-back = Hiçbir şey tamamlanmamıştı, bu yüzden geri alınacak bir şey yok
services-crash-rolling-back = { $count } işlem geri alınıyor
services-crash-deleted = { $count } kilitlenme raporu silindi
services-crash-restart-failed = Güvenli modda yeniden başlatılamadı

## OS clipboard (Linux / macOS)

services-clipboard-no-tool = ne wl-copy ne de xclip kullanılabilir

## Search

services-search-gone = Bu arama artık kullanılamıyor. Arama çubuğundan yeniden çalıştırın.
services-search-invalid-pattern = Geçersiz desen: { $error }
services-search-mode-contains = İçerir
services-search-mode-glob = Joker
services-search-mode-regex = Regex
services-search-mode-fuzzy = Bulanık
services-search-date-any = Herhangi bir zaman
services-search-date-day = Son 24 saat
services-search-date-week = Son 7 gün
services-search-date-month = Son 30 gün
services-search-date-year = Son 12 ay
services-search-date-older = Bir yıldan eski
services-search-kind-any = Her tür
services-search-kind-files = Dosyalar
services-search-kind-folders = Klasörler
services-search-kind-images = Görüntüler
services-search-kind-videos = Videolar
services-search-kind-audio = Ses
services-search-kind-documents = Belgeler
services-search-kind-archives = Arşivler
services-search-kind-code = Kod
services-search-title-all-indexed = Dizine eklenmiş klasörlerde her şey
services-search-title-indexed = Dizine eklenmiş klasörlerde “{ $text }”
services-search-title-all = { $folder } içinde her şey
services-search-title = { $folder } içinde “{ $text }”

## Git status

services-git-ignored = Yok sayıldı
services-git-untracked = İzlenmiyor
services-git-type-changed = Türü değişti
services-git-renamed = Yeniden adlandırıldı
services-git-added = Eklendi
services-git-deleted = Silindi
services-git-modified = Değiştirildi
services-git-conflict = Çakışma
services-git-unavailable = git kullanılamıyor: { $error }
services-git-unusual-filter = git status çalıştırılmadı: olağan dışı filtre sürücüsü adı
services-git-timeout = git status zaman aşımına uğradı
services-git-failed = git status başarısız oldu

## Update check

services-update-no-repo = depo yapılandırılmamış
services-update-github-only = yalnızca github.com depoları desteklenir
services-update-expected-owner-name = sahip/ad bekleniyordu
services-update-bad-repo = “{ $repo }” geçerli bir GitHub sahip/ad değil
services-update-unexpected-reply = beklenmeyen yanıt: { $error }
services-update-github-message = GitHub: { $message }
services-update-no-releases = deponun yayımlanmış sürümü yok
services-update-no-client = HTTP istemcisi oluşturulamıyor: { $error }
services-update-network = ağ hatası: { $error }
services-update-reply-too-large = yanıt çok büyük
services-update-not-found = sürüm bulunamadı (depo adını kontrol edin)
services-update-http-status = GitHub yanıtı: { $status } { $message }
services-update-cannot-start = başlatılamıyor: { $error }
services-update-no-asset = bu sürümde bu platform için indirme yok
services-update-bad-signature = yükleyicinin imzası geçersiz ({ $code }); indirilen dosya silindi
services-update-no-checksum = sürüm bu dosya için SHA-256 sağlama toplamı yayımlamıyor, bu yüzden doğrulanamıyor
services-update-checksums-disagree = sürümün sağlama toplamları birbiriyle uyuşmuyor; indirilmiyor
services-update-bad-asset-name = hatalı dosya adı
services-update-cannot-create = { $path } oluşturulamıyor: { $error }
services-update-cannot-write-file = { $path } yazılamıyor: { $error }
services-update-download-failed = indirme başarısız oldu: { $status }
services-update-cancelled = indirme iptal edildi
services-update-too-large = indirilen dosya beklenmedik şekilde büyük
services-update-cannot-write = indirilen dosya yazılamıyor: { $error }
services-update-checksum-mismatch = sağlama toplamı uyuşmuyor (beklenen { $expected }…, gelen { $got }…); indirilen dosya silindi
services-update-cannot-rename = indirilen dosya yeniden adlandırılamıyor: { $error }
services-update-available = Soyuz { $version } kullanıma sunuldu
services-update-available-body = Sürüm notlarını okumak ve indirmek için Ayarlar → Hakkında ve güncellemeler bölümünü açın.

## Linux (data)

services-watch-degraded = Değişiklik izleme sınırlı
# $secs = poll interval in whole seconds.
services-watch-degraded-body = Sistemin izlenen klasör sınırına ulaşıldı; bu yüzden bazı klasörler bunun yerine her { $secs } sn'de bir yeniden denetleniyor.

## Linux (platform)

services-clipboard-unavailable = pano kullanılamıyor (Wayland data-control ve X11 ekranı yok)
services-fm1-failed = "Klasörde göster" kaydedilemedi (org.freedesktop.FileManager1): { $error }
services-fm1-taken = "Klasörde göster" isteklerini zaten başka bir uygulama ({ $owner }) karşılıyor (org.freedesktop.FileManager1). O uygulamayı kapatın ya da dosya yöneticisi hizmetini kapatın, sonra bu ayarı kapatıp yeniden açın.

# $kind: "system" | "flatpak" | "appimage" | "other".
services-update-package-managed = { $kind ->
    [system] Soyuz'un bu kopyası sisteminizin paket yöneticisiyle kuruldu. Güncellemeyi oradan yapın (örneğin dnf, apt veya yazılım merkezi); Soyuz paket yöneticisine ait dosyaların üzerine asla yazmaz.
    [flatpak] Soyuz'un bu kopyası Flatpak olarak çalışıyor. “flatpak update” komutuyla veya yazılım merkezinden güncelleyin.
    [appimage] Soyuz'un bu kopyası AppImage olarak çalışıyor. Yeni AppImage dosyasını indirip eskisinin yerine koyun.
   *[other] Soyuz'un bu kopyası kendini güncelleyemez. Yeni sürümü bunu kurduğunuz yolla kurun.
}
services-update-install-not-tarball = yerinde yalnızca Linux sürüm arşivi (.tar.gz) kurulabilir
services-update-install-no-binary = arşivde bin/{ $name } yok
services-update-install-bad-archive = arşiv bozuk ya da bir Soyuz sürümü değil
services-update-install-not-elf = arşivdeki bin/soyuz bu bilgisayarın işlemcisi için bir program değil; hiçbir şey değiştirilmedi
services-update-install-lib-not-elf = arşivdeki lib/soyuz/{ $name } bu bilgisayarın işlemcisi için bir kitaplık değil; hiçbir şey değiştirilmedi
services-update-install-changed = indirilen dosya doğrulandıktan sonra değişti; yeniden indirin
# $path: folder of the running soyuz executable.
services-update-install-no-permission = { $path } klasörüne yazılamıyor ({ $error }). Soyuz sistem genelinde kurulduysa sudo ile güncelleyin, örneğin “sudo tar -C /usr/local --strip-components=1 -xf <dosya>”.
services-update-install-failed = güncelleme kurulamadı: { $error }
## FFmpeg runtime download (services::updates::ffmpeg_runtime)
services-ffmpeg-dl-flatpak = Flatpak, çalışma ortamının FFmpeg'ini kullanır; indirilecek bir şey yok.
services-ffmpeg-dl-msix = Microsoft Store / MSIX paketi FFmpeg kitaplıklarını içerir; onarmak için paketi yeniden kurun.
services-ffmpeg-dl-unsupported = Bu sistem ve işlemci için yayımlanmış bir FFmpeg çalışma ortamı yok. FFmpeg'i paket yöneticinizle kurun ve klasörünü Ayarlar → Önizleme → Medya'da seçin.
services-ffmpeg-dl-no-repo = Güncelleme deposu ayarlanmamış (Ayarlar → Hakkında ve güncellemeler), bu yüzden Soyuz internete çıkmaz.
services-ffmpeg-dl-busy = FFmpeg çalışma ortamı zaten indiriliyor.
services-ffmpeg-dl-no-release = Depoda “{ $tag }” sürümü yok.
services-ffmpeg-dl-no-asset = “{ $tag }” sürümünde bu sistem için bir indirme yok.
services-ffmpeg-dl-bad-archive = indirilen arşiv bozuk ({ $error })
services-ffmpeg-dl-incomplete = indirilen arşivde beş FFmpeg kitaplığı ve lisansları yok; hiçbir şey değiştirilmedi
services-ffmpeg-dl-in-use = { $path } değiştirilemiyor ({ $error }). Soyuz kitaplıkları oradan yüklediyse yeniden başlatıp tekrar deneyin.
services-ffmpeg-dl-started = FFmpeg çalışma ortamı indiriliyor
services-ffmpeg-dl-started-body = Geldiğinde doğrulanıp yüklenir; çalışmaya devam edebilirsiniz.
services-ffmpeg-dl-cannot = FFmpeg çalışma ortamı indirilemiyor
services-ffmpeg-dl-loaded = FFmpeg kitaplıkları zaten yüklü
services-ffmpeg-dl-done = FFmpeg çalışma ortamı kuruldu
services-ffmpeg-dl-done-body = Kitaplıklar { $dir } klasöründe ve şimdi yükleniyor; video önizlemeleri ve oynatıcı yeniden başlatmadan çalışır.
services-ffmpeg-dl-failed = FFmpeg çalışma ortamı kurulamadı
services-ffmpeg-dl-cancelled = FFmpeg çalışma ortamı indirmesi iptal edildi

## macos
services-clipboard-mac-failed = dosyalar macOS panosuna konamadı
