### Soyuz arayüz metinleri (tr-TR): settings
### Anahtarlar en-US/settings.ftl ile birebir aynı olmalı.

settings-language-auto = Otomatik ({ $lang })
settings-general-language = Dil
settings-general-language-desc = Menülerin, iletişim kutularının ve iletilerin dili; tarih ve sayıların yazılışı. Hemen uygulanır.

## Pencere

settings-title = Ayarlar
settings-close = Ayarları kapat
settings-close-tooltip = Kapat (Esc)
settings-search-hint = Ayarlarda ara
settings-search-results = “{ $query }” için sonuçlar
settings-search-empty = Eşleşen ayar yok
settings-status-saving = Kaydediliyor…
settings-status-saved = config.toml dosyasına kaydedildi
settings-status-live = Değişiklikler hemen uygulanır
settings-edit-config = config.toml dosyasını düzenle
settings-reset-section = Bölümü sıfırla
settings-reset-section-tooltip = Bu bölümün varsayılanlarını geri yükle
settings-reset-section-done = { $section } varsayılanlara sıfırlandı
settings-save-failed = config.toml kaydedilemedi

## Bölümler

settings-section-general = Genel
settings-section-general-desc = Başlangıç, gizli dosyalar, boyutlar ve tarihler.
settings-section-appearance = Görünüm
settings-section-appearance-desc = Tema, vurgu rengi, yoğunluk ve yazı tipleri. Değişiklikler anında önizlenir.
settings-section-fonts = Yazı tipleri
settings-section-fonts-desc = Arayüz, başlıklar, terminal, kod, onaltılık görünüm ve sayılar için yazı tipleri.
settings-section-views = Görünümler ve düzen
settings-section-views-desc = Varsayılan görünüm, sütunlar, sıralama ve pencere düzeni.
settings-section-behaviour = Davranış
settings-section-behaviour-desc = Tıklama, onaylar, sürükle/bırak ve yazma.
settings-section-jobs = Dosya işlemleri
settings-section-jobs-desc = Kopyalama / taşıma motoru, doğrulama, eşitleme zamanlamaları ve önceki sürümler.
settings-section-terminal = Terminal
settings-section-terminal-desc = Tümleşik terminal paneli.
settings-section-preview = Önizleme ve arama
settings-section-preview-desc = Önizleme sınırları, arama varsayılanları ve Git tümleştirmesi.
settings-section-editor = Düzenleyici
settings-section-editor-desc = Yerleşik metin, onaltılık ve resim düzenleyicileri (Soyuz'da düzenle, F4).
settings-section-keyboard = Klavye
settings-section-keyboard-desc = Hazır ayarlar ve tüm kısayollar. Değiştirmek için bir kısayola tıklayın.
settings-section-commands = Özel komutlar
settings-section-commands-desc = Yer tutuculu kendi komutlarınız; komut paletinde ve bağlam menüsünde.
settings-section-bookmarks = Yer işaretleri
settings-section-bookmarks-desc = Sık gittiğiniz klasörler, isteğe bağlı kısayol tuşlarıyla.
settings-section-scripts = Betikler
settings-section-scripts-desc = Rhai betikleri: eylemler, bağlam menüsü girişleri, sütunlar ve önizleyiciler.
settings-section-automations = Otomasyonlar
settings-section-automations-desc = İzlenen klasörlerdeki yeni öğeleri düzenleyen kurallar ve soyuz komut satırı.
settings-section-ai = Yapay zekâ asistanı
settings-section-ai-desc = Model sağlayıcıları, API anahtarları, gizlilik, kapsam, bütçeler ve MCP bağlantıları.
settings-section-advanced = Gelişmiş
settings-section-advanced-desc = config.toml dosyasını düzenleyin, dışa veya içe aktarın; Soyuz'u varsayılan dosya yöneticiniz yapın.
settings-section-about = Hakkında ve güncellemeler
settings-section-about-desc = Sürüm ve GitHub Releases'ta daha yeni bir Soyuz olup olmadığını denetleme (yalnızca siz istediğinizde).

## Dışa / içe aktarma

settings-export-title = Ayarları dışa aktar
settings-import-title = Ayarları içe aktar
settings-exported = Ayarlar dışa aktarıldı
settings-export-failed = Dışa aktarma başarısız oldu
settings-imported = Ayarlar içe aktarıldı
settings-imported-warnings = Ayarlar uyarılarla içe aktarıldı
settings-import-failed = İçe aktarma başarısız oldu
settings-theme-import-title = Temayı içe aktar
settings-theme-export-title = Temayı dışa aktar
settings-theme-exported = Tema dışa aktarıldı
settings-theme-export-failed = Tema dışa aktarılamadı
settings-theme-import-failed = Tema içe aktarılamadı
settings-theme-imported = “{ $name }” teması içe aktarıldı
settings-theme-imported-warnings = “{ $name }” teması uyarılarla içe aktarıldı
settings-theme-invalid = Geçerli bir tema dosyası değil

## General

settings-general-sizes-desc = 1,2 GB'lık dosya { $sample } olarak gösterilir
settings-general-dates = Tarihler
settings-general-dates-desc = Dünkü dosya “{ $preview }” olarak gösterilir

## General page

settings-general-startup = Başlangıç
settings-general-start-in = Başlangıç konumu
settings-general-start-in-desc = Soyuz kayıtlı bir oturum olmadan başladığında gösterilen klasör.
settings-general-start-home = Giriş klasörü
settings-general-start-last = Son klasör
settings-general-start-folder = Klasör
settings-general-restore-tabs = Başlangıçta sekmeleri geri yükle
settings-general-restore-tabs-desc = Son oturumun sekmelerini, bölmelerini ve klasörlerini yeniden açar.
settings-general-files = Dosyalar
settings-general-show-hidden = Gizli dosyaları göster
settings-general-show-hidden-desc = Nokta ile başlayan dosyalar ve gizli özniteliği olan öğeler (Ctrl+H ile açılıp kapanır).
settings-general-show-system = Sistem dosyalarını göster
settings-general-show-system-desc = Windows'un korumalı işletim sistemi dosyaları.
settings-general-show-ext = Dosya uzantılarını göster
settings-general-show-ext-desc = Adların sonunda “.txt”, “.rs” … gösterilir.
settings-general-strict-names = Katı Windows adları
settings-general-strict-names-desc = Windows dosya adı kurallarını (<>:"|?* ve ayrılmış adlar yasak) her işletim sisteminde uygula.
settings-general-formats = Biçimler
settings-general-sizes = Dosya boyutları
settings-general-sizes-bytes = Bayt
settings-general-date-relative = Göreli
settings-general-date-short = Kısa
settings-general-date-long = Uzun
settings-general-date-custom = Özel…

## Views & layout page

settings-view-details = Ayrıntılar
settings-view-grid = Izgara
settings-view-list = Liste
settings-view-columns = Sütunlar
settings-view-gallery = Galeri
settings-views-folders = Klasörler
settings-views-default-view = Varsayılan görünüm
settings-views-default-view-desc = Özelleştirmediğiniz klasörlerde kullanılır.
settings-views-folders-first = Önce klasörler
settings-views-folders-first-desc = Sıralarken klasörleri dosyaların üstünde tutar.
settings-views-natural-sort = Doğal sıralama
settings-views-natural-sort-desc = dosya2, dosya10'dan önce gelir.
settings-views-case-sort = Büyük/küçük harf duyarsız sıralama
settings-views-case-sort-desc = “elma” ile “Elma” birlikte sıralanır.
settings-views-remember = Görünümü klasör başına hatırla
settings-views-remember-desc = Her klasör kendi görünümünü, sıralamasını ve gruplamasını korur.
settings-views-details-columns = Ayrıntılar sütunları
settings-views-details-columns-desc = Kendi düzeni olmayan klasörlerde ayrıntılar görünümünün sütunları. Genişlikleri görünümün kendisinde sürükleyerek ayarlayın.
settings-views-hide-column = Sütunu gizle
settings-views-add-column = Ekle:
settings-views-window-layout = Pencere düzeni
settings-views-sidebar = Kenar çubuğu
settings-views-sidebar-desc = Sık kullanılanlar, aygıtlar ve klasör ağacı.
settings-views-sidebar-content = Kenar çubuğu içeriği
settings-views-sidebar-content-desc = Konum bölümleri, sürücülerinizin ve giriş klasörünüzün klasör ağacı ya da ikisi birden (Ctrl+Shift+E ağacı açar/kapatır).
settings-views-sidebar-places = Konumlar
settings-views-folder-tree = Klasör ağacı
settings-views-sidebar-both = İkisi de
settings-views-tree-follow = Ağaç geçerli klasörü izlesin
settings-views-tree-follow-desc = Klasör ağacını etkin bölmenin klasörüne kadar genişletir ve onu vurgular.
settings-views-tree-drag = Sürüklerken üzerine gelince ağacı genişlet
settings-views-tree-drag-desc = Daraltılmış bir klasörü açmak için sürüklerken üzerinde bekleyin. 0 bunu kapatır.
settings-views-status-bar = Durum çubuğu
settings-views-status-bar-desc = Öğe sayıları, seçim boyutu ve boş alan.
settings-views-preview-pane = Önizleme bölmesi
settings-views-preview-pane-desc = Başlangıçta önizleme bölmesini gösterir.
settings-views-dual-pane = Başlangıçta çift bölme
settings-views-dual-pane-desc = Yeni pencereleri iki bölmeyle açar.
settings-views-dual-orientation = Çift bölme yönü
settings-views-side-by-side = Yan yana
settings-views-stacked = Üst üste
settings-views-tab-strip = Sekme şeridi
settings-always = Her zaman
settings-views-tab-strip-multiple = 2+ sekmede
settings-views-activity-panel = Etkinlik paneli
settings-views-activity-panel-desc = Süren dosya işlemlerinin gösterildiği yer.
settings-views-activity-right = Sağda
settings-views-activity-bottom = Altta
settings-views-activity-floating = Yüzen
settings-views-terminal-height = Terminal yüksekliği
settings-views-terminal-height-desc = Alt panelin varsayılan yüksekliği.
settings-views-windows = Pencereler ve çalışma alanları
settings-views-new-window = Yeni pencere açma biçimi
settings-views-new-window-desc = Ctrl+N: bu Soyuz'un başka bir penceresi (işleri ve ayarları paylaşır) ya da ayrı bir işlem.
settings-views-new-window-window = Pencere
settings-views-new-window-process = İşlem
settings-views-tear-off = Sekmeleri ayır
settings-views-tear-off-desc = Bir sekmeyi kendi penceresinde açmak için şeritten dışarı sürükleyin.
settings-views-tear-off-distance = Ayırma mesafesi
settings-views-tear-off-distance-desc = Bir sekmenin önce şeritten ne kadar uzaklaşması gerektiği.
settings-views-close-window = Sekmeli bir pencere kapatılırken
settings-views-close-window-desc = Sekmelerini başka bir pencereye taşı, kapat ya da her seferinde sor.
settings-ask = Sor
settings-views-move-tabs = Sekmeleri taşı
settings-views-close-tabs = Sekmeleri kapat
settings-views-restore-windows = Pencereleri geri yükle
settings-views-restore-windows-desc = Diğer pencereleri oturumla birlikte bulundukları yerde yeniden açar.
settings-views-sync-relative = Göreli eşitlenmiş gezinme
settings-views-sync-relative-desc = Eşitlenmiş bölmeler yalnızca alt klasöre girişleri ve yukarı çıkışları değil, kardeş klasörlere ve birden çok düzeyli geçişleri de izler.
settings-views-history-graph = Geçmiş grafiği boyutu
settings-views-history-graph-desc = “Neredeydim?” için sekme başına hatırlanan konumlar.
settings-views-timeline-range = Zaman çizelgesi aralığı
settings-views-timeline-range-desc = Zaman çizelgesinin gösterdiği değişikliklerin gün sayısı (en az bu hafta).
settings-views-timeline-folders = Zaman çizelgesi klasörleri
settings-views-timeline-folders-desc = Değişen dosyalar için taranan, son ziyaret edilen klasörler.
settings-views-timeline-items = Zaman çizelgesi öğeleri
settings-views-timeline-items-desc = Zaman çizelgesinin listelediği en fazla dosya sayısı.
settings-views-timeline-index = Zaman çizelgesi arama dizinini kullansın
settings-views-timeline-index-desc = Değişen dosyalar için varsa arama dizinine de başvurur.
settings-views-sidebar-sections = Kenar çubuğu bölümleri
settings-views-sidebar-sections-desc = Kenar çubuğunun bölümlerini seçin ve sıralayın.
settings-views-section-favorites = Sık kullanılanlar
settings-views-section-devices = Aygıtlar
settings-views-section-cloud = Bulut klasörleri
settings-views-section-tags = Etiketler
settings-views-section-network = Ağ
settings-views-section-trash = Çöp Kutusu
settings-views-section-smart = Akıllı klasörler
settings-views-toolbar = Araç çubuğu ve bağlam menüsü
settings-views-toolbar-items = Araç çubuğu öğeleri
settings-views-toolbar-items-desc = Eylem adları (ör. back, up, new_tab) ile | separator, spacer, breadcrumb, filter, view_menu, sort_menu, activity. Her satıra bir tane.
settings-views-ctx-hide = Gizli bağlam menüsü girişleri
settings-views-ctx-hide-desc = Sağ tıklama menülerinden gizlenecek eylem adları. Her satıra bir tane.

## Behaviour page

settings-behaviour-opening = Açma
settings-behaviour-single-click = Öğeleri tek tıklamayla aç
settings-behaviour-single-click-desc = Aksi hâlde çift tıklama açar.
settings-behaviour-double-click = Çift tıklama hızı
settings-behaviour-double-click-desc = İki tıklama arasındaki en uzun süre.
settings-behaviour-open-folders-in = Klasörleri şurada aç
settings-behaviour-same-tab = Aynı sekme
settings-behaviour-new-tab = Yeni sekme
settings-behaviour-middle-click = Klasöre orta tıklama
settings-behaviour-other-pane = Diğer bölme
settings-behaviour-nothing = Hiçbir şey
settings-behaviour-confirmations = Onaylar
settings-behaviour-confirm-trash = Çöp Kutusu'na taşımayı onayla
settings-behaviour-confirm-trash-desc = Delete tuşu öğeleri Çöp Kutusu'na göndermeden önce sorar.
settings-behaviour-confirm-permanent = Kalıcı silmeyi onayla
settings-behaviour-confirm-permanent-desc = Shift+Delete öğeleri kalıcı olarak silmeden önce sorar.
settings-behaviour-paste-conflict = Yapıştırılan öğeler zaten varsa
settings-behaviour-overwrite = Üzerine yaz
settings-skip = Atla
settings-behaviour-keep-both = İkisini de tut
settings-behaviour-drag-drop = Sürükle ve bırak
settings-behaviour-drop-action = Varsayılan bırakma eylemi
settings-behaviour-drop-action-desc = Bundan bağımsız olarak Ctrl kopyalar, Shift taşır, Alt bağlantı oluşturur.
settings-auto = Otomatik
settings-behaviour-drop-copy = Kopyala
settings-behaviour-drop-move = Taşı
settings-behaviour-spring = Yaylı klasörler
settings-behaviour-spring-desc = Sürüklerken bir klasör, yol parçası ya da sekme üzerinde beklemek onu açar; böylece daha derine bırakabilirsiniz.
settings-behaviour-spring-delay = Yaylı açılma gecikmesi
settings-behaviour-spring-delay-desc = Klasör açılmadan önce üzerinde ne kadar bekleneceği.
settings-behaviour-typing = Yazma ve yeniden adlandırma
settings-behaviour-type-ahead = Listede yazınca
settings-behaviour-type-ahead-desc = İlk eşleşmeye atla ya da listeyi filtrele.
settings-behaviour-jump = Atla
settings-behaviour-filter = Filtrele
settings-behaviour-rename-selects = Yeniden adlandırırken seçilen
settings-behaviour-rename-selects-desc = Yeniden adlandırma başladığında seçilen kısım (F2 ile değiştirilir).
settings-behaviour-name-only = Yalnızca ad
settings-behaviour-everything = Tümü
settings-behaviour-refreshing = Yenileme
settings-behaviour-auto-refresh = Otomatik yenile
settings-behaviour-auto-refresh-desc = Açık klasörleri diğer programların yaptığı değişiklikler için izler.
settings-behaviour-debounce = Değişiklik bekleme süresi
settings-behaviour-debounce-desc = Art arda gelen değişikliklerin durulması için ne kadar bekleneceği.

## File operations page

settings-remote-hosts-desc = { $count } ana makine anahtarına güveniliyor. Yeniden sorulması için birini unutun.
settings-remote-shares = \\sunucu için paylaşımları listele

settings-jobs-queue = Kuyruk
settings-jobs-parallel = Paralel işler
settings-jobs-parallel-desc = Aynı anda kaç kopyalama / taşıma / silme işinin çalışacağı. Yeniden başlatınca uygulanır.
settings-jobs-queue-mode = Kuyruk modu
settings-jobs-queue-mode-desc = “Aygıt başına” modu her disk çifti için aynı anda tek iş çalıştırır; HDD'lerde en hızlısı budur. Yeniden başlatınca uygulanır.
settings-jobs-per-device = Aygıt başına
settings-jobs-parallel-mode = Paralel
settings-jobs-auto-activity = Etkinlik panelini otomatik aç
settings-jobs-auto-activity-desc = Bir iş bir saniyeden uzun sürdüğünde.
settings-jobs-reset-apply-all = İşler arasında “tümüne uygula”yı sıfırla
settings-jobs-reset-apply-all-desc = Çakışma yanıtları sonraki işe aktarılmaz. Yeniden başlatınca uygulanır.
settings-jobs-history = Geçmiş boyutu
settings-jobs-history-desc = Geri almak için saklanan tamamlanmış işlemler. Yeniden başlatınca uygulanır.
settings-jobs-copying = Kopyalama
settings-jobs-verify = Kopyaladıktan sonra doğrula
settings-jobs-verify-desc = Kaynağın ve hedefin özetini (BLAKE3) alıp karşılaştırır.
settings-jobs-reflink = Yazarken kopyalama (CoW) klonları
settings-jobs-reflink-desc = Mümkün olduğunda ReFS / Btrfs / APFS üzerinde anında kopyalar.
settings-never = Hiçbir zaman
settings-jobs-buffer = Kopyalama arabelleği
settings-jobs-unbuffered = Arabelleksiz G/Ç eşiği
settings-jobs-unbuffered-desc = Büyük dosyalar işletim sistemi önbelleğini atlar.
settings-jobs-preserve = Korunacaklar
settings-jobs-preserve-desc = Kopyalara aktarılan üst veriler.
settings-jobs-preserve-times = Zaman damgaları
settings-jobs-preserve-attrs = Öznitelikler
settings-jobs-preserve-xattr = Genişletilmiş öznitelikler
settings-jobs-preserve-acl = ACL'ler
settings-jobs-preserve-owner = Sahip
settings-jobs-preserve-sparse = Seyrek dosyalar
settings-jobs-archive = Arşiv güvenliği
settings-jobs-bomb-ratio = Genişleme sınırı
settings-jobs-bomb-ratio-desc = Çıktı arşiv boyutunun bu katını aştığında ayıklamayı duraklatır (0 = kapalı).
settings-jobs-bomb-floor = …ama bundan az değil
settings-jobs-bomb-floor-desc = Genişleme sınırı bu kadar çıktıdan önce asla duraklatmaz.
settings-jobs-max-entries = En fazla girdi
settings-jobs-max-entries-desc = Bu sayıdan fazla girdide ayıklamayı duraklatır (0 = sınırsız).
settings-jobs-max-entry = En büyük girdi
settings-jobs-max-entry-desc = Bundan büyük tek bir girdide duraklatır (0 = sınırsız).
settings-jobs-zip-level = Zip'e eklerken sıkıştırma
settings-jobs-zip-level-desc = Gezindiğiniz bir zip'e yapıştırılan dosyalar için Deflate düzeyi (0 = sıkıştırmadan sakla, 9 = en küçük).
settings-jobs-special-bits = setuid / setgid / sticky bitlerini koru
settings-jobs-special-bits-desc = Kapalıyken ayıklanan dosyalardan bu özel izin bitleri kaldırılır (Linux / macOS).
settings-jobs-trash-unsupported = Çöp Kutusu kullanılamadığında
settings-jobs-trash-unsupported-desc = ör. ağ sürücüleri ve FAT birimleri.
settings-jobs-trash-delete = Sil
settings-jobs-sync = Eşitleme ve sürümler
settings-jobs-scheduler = Zamanlanmış eşitleme profillerini çalıştır
settings-jobs-scheduler-desc = Zamanlaması olan profiller Soyuz açıkken çalışır (arka plan hizmeti yoktur).
settings-jobs-dry-run = Zamanlanmış çalıştırmalar yalnızca deneme olsun
settings-jobs-dry-run-desc = Yalnızca nelerin değişeceğine dair rapor yazar; bunu Eşitleme profillerinden kendiniz uygularsınız.
settings-jobs-change-delay = “Değişiklikte” gecikmesi
settings-jobs-change-delay-desc = Eşitlemeden önce son değişiklikten sonra bu kadar bekler.
settings-jobs-reports = Profil başına saklanan raporlar
settings-jobs-reports-desc = Çalıştırma ve deneme raporları.
settings-jobs-previous-versions = Önceki sürümler
settings-jobs-previous-versions-desc = Dosyaların önceki kopyalarını Windows gölge kopyalarından listeler (Özellikler, bağlam menüsü).
settings-jobs-shadow-probe = Yoklanacak gölge kopyalar
settings-jobs-shadow-probe-desc = Çekirdek aygıt listesi okunamadığında yedek yöntem.
settings-remote = Uzak konumlar
settings-remote-cache = Klasör listelerini yeniden kullanma süresi
settings-remote-cache-desc = Uzak klasörler bu süreden sonra yeniden listelenir (Yenile her zaman yeniden listeler).
settings-remote-poll = Açık uzak klasörleri denetleme aralığı
settings-remote-poll-desc = 0 = yalnızca Yenile'de.
settings-remote-connect-timeout = Bağlantı zaman aşımı
settings-remote-connect-timeout-desc = Bu süreden sonra bağlanmaktan vazgeçer.
settings-remote-io-timeout = Sunucu zaman aşımı
settings-remote-io-timeout-desc = Sunucu işlemin ortasında yanıt vermeyi bırakırsa vazgeçer.
settings-remote-download-cache = İndirme önbelleği
settings-remote-download-cache-desc = Açılan ya da önizlenen uzak dosyalar bu boyuta kadar geçici bir klasörde tutulur.
settings-remote-preview-max = Önizlenecek en büyük dosya
settings-remote-preview-max-desc = Daha büyük uzak dosyalar yalnızca önizlemek için indirilmez.
settings-remote-shares-desc = Yalnızca sunucu adı yazmak, sunucunun paylaşılan klasörlerini listeler (Windows).
settings-remote-trusted = Güvenilen SSH sunucuları
settings-remote-forget = Unut

## Terminal page

settings-terminal-shell-desc = Otomatik algılamak için boş bırakın. Yeni oturumlara uygulanır.
settings-terminal-shell = Kabuk
settings-terminal-args = Bağımsız değişkenler
settings-terminal-args-desc = Kabuk için ek bağımsız değişkenler.
settings-terminal-integration = Kabuk tümleştirmesi
settings-terminal-integration-desc = Terminalin bulunduğu klasörü bildirmesi için isteme bir kanca ekler.
settings-terminal-scrollback = Geri kaydırma
settings-terminal-scrollback-desc = Oturum başına saklanan satırlar.
settings-terminal-look = Görünüm
settings-terminal-font-size = Yazı tipi boyutu
settings-terminal-colors = Renkler
settings-terminal-colors-desc = Otomatik, renk paletini uygulama temasından türetir.
settings-dark = Koyu
settings-light = Açık
settings-terminal-following = İzleme
settings-terminal-follow-nav = Gezinmeyi izle
settings-terminal-follow-nav-desc = Odaktaki bölme klasör değiştirdiğinde terminalde cd çalıştırır.
settings-terminal-follow-shell = Kabuğu izle
settings-terminal-follow-shell-desc = Kabukta cd yaptığınızda bölmeyi o klasöre götürür.
settings-terminal-cd-tab = Sekme değişince cd
settings-terminal-per-tab = Sekme başına bir terminal
settings-terminal-per-tab-desc = Aksi hâlde tüm sekmeler aynı oturumları paylaşır.
settings-terminal-esc = Esc odağı geri verir
settings-terminal-esc-desc = Boş istemde Esc dosya listesine geri döner.
settings-terminal-security = Güvenlik
settings-terminal-osc52 = Panoyu ayarlayan programlar
settings-terminal-osc52-desc = OSC 52, terminal çıktısının panonuzu değiştirmesine izin verir (ör. yapıştırmanız için bir komutla). Panoyu okumak her zaman engellenir.
settings-allow = İzin ver
settings-deny = Reddet

## Preview & search page

settings-preview = Önizleme
settings-preview-text-limit = Metin önizleme sınırı
settings-preview-text-limit-desc = Daha büyük dosyaların yalnızca ilk kısmı gösterilir.
settings-preview-image-limit = Resim boyutu sınırı
settings-preview-image-limit-desc = Daha büyük resimler önizleme için çözülmez.
settings-preview-syntax = Sözdizimi teması
settings-preview-syntax-desc = auto uygulama temasını izler; ya da bir syntect tema adı girin.
settings-preview-pdf = PDF önizlemesi
settings-preview-pdf-desc = Sayfalar pdfium.dll ile işlenir (birlikte gelmez; soyuz.exe'nin yanına ya da yapılandırma klasörüne koyun). Bu dosya yoksa PDF'lerin metni gösterilir.
settings-preview-pdfium = pdfium klasörü
settings-preview-pdfium-desc = pdfium.dll dosyasının bulunduğu yer. Boş: soyuz.exe'nin yanı, sonra yapılandırma klasörü.
settings-preview-pdfium-hint = soyuz.exe'nin yanında
settings-preview-office = Office belgeleri
settings-preview-office-desc = Word, Excel, PowerPoint ve OpenDocument yapısı ve metni.
settings-preview-sheet-rows = Elektronik tablo satırları
settings-preview-sheet-rows-desc = İlk sayfadan okunan satırlar.
settings-preview-epub = E-kitaplar
settings-preview-epub-desc = EPUB başlığı, yazarı, kapağı ve içindekiler.
settings-preview-doc-limit = Belge boyutu sınırı
settings-preview-doc-limit-desc = Bundan büyük Office, EPUB ve PDF dosyaları (ve içlerindeki parçalar) ayrıştırılmaz.
settings-preview-fonts = Yazı tipleri
settings-preview-fonts-desc = TTF, OTF, TTC ve WOFF için örnek metin ve glif ızgarası.
settings-preview-models = 3B modeller
settings-preview-models-desc = glTF, GLB, STL ve OBJ için hızlı görüntü; döndürmek için sürükleyin.
settings-preview-spin = Modelleri döndür
settings-preview-spin-desc = Siz sürükleyene kadar modelleri yavaşça döndürür (azaltılmış hareketle kapalı).
settings-preview-model-limit = Model boyutu sınırı
settings-preview-model-limit-desc = Daha büyük model dosyaları yüklenmez.
settings-preview-waveform = Ses dalga biçimi
settings-preview-waveform-desc = Dalga biçimini çizmek için ses dosyalarını çözer.
settings-preview-audio-limit = Ses boyutu sınırı
settings-preview-audio-limit-desc = Daha büyük ses dosyalarının dalga biçimi çizilmez.
settings-preview-playback = Ses çalma
settings-preview-playback-desc = Önizlemenin içinde çal, duraklat ve sar.
settings-preview-video = Video kapak kareleri
settings-preview-video-desc = Kuruluysa ffmpeg kullanılır. Hiçbir şey indirilmez.
settings-preview-ffmpeg = ffmpeg programı
settings-preview-ffmpeg-desc = PATH üzerinde aranan bir ad ya da tam yol.
settings-preview-diff = İki dosyayı karşılaştır
settings-preview-diff-desc = Tam olarak iki metin dosyası seçiliyken önizleme farklarını gösterir.
settings-preview-diff-limit = Fark boyutu sınırı
settings-preview-diff-limit-desc = Her dosya bu boyuta kadar karşılaştırılır.
settings-preview-thumb-mem = Küçük resim belleği
settings-preview-thumb-mem-desc = Bellekte tutulan çözülmüş küçük resimler; en uzun süredir gösterilmeyenler önce atılır. Geçerli kullanım Hakkında bölümündedir.
settings-preview-preview-mem = Önizleme belleği
settings-preview-preview-mem-desc = Bir dosyaya geri dönmek anında olsun diye saklanan hazır önizlemeler. Geçerli kullanım Hakkında bölümündedir.
settings-search = Arama
settings-search-threads = Arama iş parçacıkları
settings-search-threads-desc = 0 tüm çekirdekleri kullanır.
settings-search-gitignore = .gitignore dosyasına uy
settings-search-gitignore-desc = Yok sayılan dosyaları varsayılan olarak atlar.
settings-search-content-limit = İçerik arama sınırı
settings-search-content-limit-desc = İçerik ararken bundan büyük dosyalar atlanır.
settings-search-saved = config.toml dosyasındaki kayıtlı aramalar
settings-git-status-column = Git durumu sütunu
settings-git-status-column-desc = Depolarda değiştirilmiş / izlenmeyen işaretlerini gösterir.
settings-git-refresh = Yenileme aralığı

## Advanced page

settings-advanced-config-desc = Tüm ayarlar tek bir TOML dosyasında durur. Bir düzenleyicide yapılan değişiklikler bir saniye içinde uygulanır; ayrıştırma hataları yerleriyle gösterilir ve önceki ayarlar korunur.
settings-advanced-edit = Yapılandırma dosyasını düzenle
settings-advanced-reload = Diskten yeniden yükle
settings-advanced-copy = TOML olarak kopyala
settings-advanced-transfer = Dışa ve içe aktarma
settings-advanced-export-desc = config.toml dosyasının bir kopyasını istediğiniz yere kaydedin.
settings-advanced-export = Dışa aktar…
settings-advanced-import-desc = Bir yapılandırma dosyası yükler (geçerli ayarların yerini alır).
settings-advanced-import = İçe aktar…
settings-advanced-danger = Tehlikeli bölge
settings-advanced-reset-all = Her şeyi sıfırla
settings-advanced-reset-all-desc = Tüm ayarları varsayılanlarına döndürür.
settings-advanced-reset-all-button = Tüm ayarları sıfırla
settings-advanced-reset-all-done = Tüm ayarlar varsayılanlara sıfırlandı

## Appearance: theme color tokens (labels of [theme.colors] keys)

settings-token-bg-window = Pencere arka planı
settings-token-bg-surface1 = Kenar çubuğu, araç çubukları
settings-token-bg-surface2 = Üzerine gelinen satırlar, giriş alanları, kartlar
settings-token-bg-surface3 = Açılır pencereler, menüler, iletişim kutuları
settings-token-bg-sunken = Terminal, onaltılık görünüm
settings-token-border-subtle = Ayırıcılar
settings-token-border-strong = Odaktaki denetimler
settings-token-text-primary = Birincil metin
settings-token-text-secondary = İkincil metin
settings-token-text-tertiary = İpuçları, yer tutucular
settings-token-accent = Vurgu
settings-token-accent-hover = Vurgu (üzerine gelince)
settings-token-accent-subtle = Seçim dolgusu
settings-token-success = Başarı
settings-token-warning = Uyarı
settings-token-danger = Tehlike
settings-token-info = Bilgi
settings-token-glyph-folder = Klasör simgeleri
settings-token-glyph-image = Resim simgeleri
settings-token-glyph-video = Video simgeleri
settings-token-glyph-audio = Ses simgeleri
settings-token-glyph-archive = Arşiv simgeleri
settings-token-glyph-code = Kod simgeleri
settings-token-glyph-doc = Belge simgeleri
settings-token-glyph-exe = Program simgeleri
settings-token-tag-red = Etiket kırmızı
settings-token-tag-orange = Etiket turuncu
settings-token-tag-yellow = Etiket sarı
settings-token-tag-green = Etiket yeşil
settings-token-tag-cyan = Etiket camgöbeği
settings-token-tag-blue = Etiket mavi
settings-token-tag-purple = Etiket mor
settings-token-tag-gray = Etiket gri

## Appearance page

settings-appearance-video-thumbs-desc = Videoların ilk karesi ffmpeg ile alınır ({ $path }).
settings-appearance-video-thumbs-missing = Videoların ilk karesi ffmpeg ile alınır — ffmpeg PATH üzerinde bulunamadı, bu yüzden videolarda süreleri gösterilir.
settings-appearance-overrides = { $count } geçersiz kılma

settings-appearance-theme = Tema
settings-appearance-color-theme = Renk teması
settings-appearance-color-theme-desc = Her temanın koyu ve açık bir çeşidi vardır. Tema dosyaları themes klasöründe durur; düzenlemeler anında uygulanır.
settings-appearance-mode = Mod
settings-appearance-mode-desc = Sistemin koyu / açık ayarını anında izleyin ya da koyu veya açık modu zorlayın.
settings-appearance-mode-system = Sistem
settings-appearance-accent = Vurgu rengi
settings-appearance-accent-desc = Seçim, odak halkaları, birincil düğmeler ve bağlantılar. Sistem, Windows vurgu rengini anında izler.
settings-appearance-reduce-motion = Hareketi azalt
settings-appearance-reduce-motion-desc = Animasyonları kapatır (paneller, iletişim kutuları, bildirimler).
settings-appearance-high-contrast = Yüksek karşıtlık
settings-appearance-high-contrast-desc = Siyah / beyaz yüzeyler; daha belirgin metin, kenarlıklar ve odak halkaları.
settings-appearance-density-text = Yoğunluk ve metin
settings-appearance-density = Yoğunluk
settings-appearance-density-desc = Sıkı, rahat, geniş ya da dokunmatik: 24, 28, 32 veya 40 px liste satırları; dokunmatik ayrıca düğmeleri ve menüleri parmakla kullanım için büyütür. Bir önizlemeye tıklayın.
settings-appearance-font-size-desc = Tüm arayüz metninin temel boyutu.
settings-appearance-scale = Arayüz ölçeği
settings-appearance-scale-desc = Tüm pencereyi yakınlaştırır (Ctrl+= / Ctrl+- / Ctrl+0; kaydırıcıyı bıraktığınızda uygulanır).
settings-appearance-fonts-desc = Arayüz, başlık, terminal, kod, onaltılık ve sayı yazı tipleri; boyut farkları ve önizlemelerle.
settings-appearance-fonts-button = Yazı tipleri…
settings-appearance-fallback = Yedek yazı tipleri
settings-appearance-fallback-desc = Çince, Japonca, Korece, Arapça, İbranice ve emoji içeren adlar için sistem yazı tipleri; arka planda yüklenir. Otomatik: bir ad gerektirdiğinde; başlangıçta: terminal çıktısını da kapsar.
settings-automatic = Otomatik
settings-appearance-fallback-startup = Başlangıçta
settings-off = Kapalı
settings-appearance-icons = Simgeler ve küçük resimler
settings-appearance-file-icons = Dosya simgeleri
settings-appearance-file-icons-desc = Türe göre renkli ya da tek renk.
settings-appearance-colored = Renkli
settings-appearance-mono = Tek renk
settings-appearance-badges = Uzantı rozetleri
settings-appearance-badges-desc = Izgara ve galeri görünümlerinde simgelerin üzerinde dosya uzantısını gösteren küçük renkli etiket.
settings-appearance-thumbs = Küçük resimler
settings-appearance-thumbs-desc = Simgeler yerine resim ve video küçük resimlerini gösterir.
settings-appearance-folder-thumbs = Klasör küçük resimleri
settings-appearance-folder-thumbs-desc = Izgara görünümleri her klasörün içindeki ilk resimlerden 2×2 bir mozaik gösterir.
settings-appearance-video-thumbs = Video küçük resimleri
settings-appearance-pdf-thumbs = PDF küçük resimleri
settings-appearance-startup-placeholder = Anında başlangıç penceresi
settings-appearance-startup-placeholder-desc = Grafik çizici başlarken (yaklaşık 0,3 sn) hemen tema arka planlı sade bir pencere gösterir, ardından tam pencere açılır. Bir sonraki başlangıçta uygulanır.
settings-appearance-os-thumbs = Windows küçük resim önbelleği
settings-appearance-os-thumbs-desc = Dosyaları çözmeden önce Gezgin'in hazırladığı küçük resimleri kullanır (thumbcache_*.db, salt okunur). Gezgin'de varsa videolar ve PDF'ler ffmpeg ya da pdfium olmadan da resimle görünür.
settings-appearance-pdf-thumbs-desc = PDF'lerin ilk sayfası pdfium ile çizilir (pdfium.dll soyuz.exe'nin yanında, yapılandırma klasöründe ya da Önizleme altında belirtilen klasörde). Yoksa PDF'ler simgeleriyle görünür.
settings-appearance-icon-pack = Simge paketi
settings-appearance-icon-pack-desc = Dosya türleri ve klasörler için resimler. Kendi paketlerinizi icons klasörüne koyun; her biri pack.toml içeren ayrı bir alt klasör olmalı (kullanım kılavuzuna bakın). Paketin kapsamadığı her şey yerleşik simgesini korur.
settings-appearance-icon-pack-default = Yerleşik (Phosphor)
settings-appearance-icon-pack-flat = Düz (yerleşik)
settings-appearance-icon-pack-folder = Simge klasörü
settings-appearance-thumb-cache = Küçük resim disk önbelleği
settings-appearance-thumb-cache-desc = Yeniden başlatmalardan sonra da kalmaları için küçük resimleri thumbs klasöründe tutar (0 = kapalı). Önce en eskiler silinir.
settings-clear = Temizle
settings-appearance-thumb-size = Varsayılan küçük resim boyutu
settings-appearance-thumb-size-desc = Izgara görünümlerinin başlangıç yakınlaştırması.
settings-appearance-sidebar-width = Kenar çubuğu genişliği
settings-appearance-titlebar-desc = Soyuz'un logolu, sekmeli ve temalı pencere denetimli kendi başlık çubuğu. Kapalı: sistem başlık çubuğu.
settings-appearance-titlebar-desc-restart = Sekmeleri pencere düğmelerinin altında olan Soyuz başlık çubuğu. Kapalı: sistem başlık çubuğu. Yeniden başlattıktan sonra etkili olur.
settings-appearance-titlebar = Özel başlık çubuğu
settings-appearance-backdrop = Arka plan malzemesi
settings-appearance-backdrop-desc = Windows 11: pencerenin arkasında sistemin Mica veya Acrylic malzemesini gösterir. Yeniden başlattıktan sonra etkili olur.
settings-none = Yok
settings-appearance-snap = Ek düzenleri
settings-appearance-snap-desc = Windows 11: Ekranı kapla düğmesinin üzerine gelmek Ek düzenleri menüsünü açar. Yeniden başlattıktan sonra etkili olur.
settings-appearance-theme-colors = Tema renkleri
settings-appearance-theme-colors-desc = Herhangi bir tasarım belirtecini geçersiz kılın. Geçersiz kılmalar hem koyu hem açık temalara uygulanır; geçersiz kılınan belirteçler bir noktayla işaretlenir.
settings-appearance-reset-colors = Renkleri sıfırla
settings-appearance-token-reset = Temanın rengini kullan
settings-appearance-preview-home = Giriş
settings-appearance-preview-projects = Projeler
settings-appearance-preview-downloads = İndirilenler

settings-appearance-preview-crumb = Giriş  ›  Projeler

## Density gallery

settings-density-compact = Sıkı
settings-density-cozy = Rahat
settings-density-comfortable = Geniş
settings-density-touch = Dokunmatik
settings-density-a11y = Yoğunluk: { $name }

## Editor page

settings-editor-panel = Panel
settings-editor-dock = Yerleşim
settings-editor-dock-desc = Düzenleyici panelinin açıldığı yer. Dosya bölmelerinin üzerine tam boyutlu olarak da açılabilir.
settings-editor-text = Metin
settings-editor-font-size-desc = Metin ve onaltılık düzenleyici yazı tipi.
settings-editor-tab-width = Sekme genişliği
settings-editor-tab-width-desc = Sekme durağı başına sütun sayısı.
settings-editor-wrap = Uzun satırları kaydır
settings-editor-wrap-desc = Yana kaydırmak yerine panelin kenarında alt satıra geçer.
settings-editor-whitespace = Boşlukları göster
settings-editor-whitespace-desc = Boşlukları nokta, sekmeleri ok olarak çizer.
settings-editor-max-text = En büyük metin dosyası
settings-editor-max-text-desc = Daha büyük dosyalar onaltılık düzenleyicide salt okunur açılır.
settings-editor-saving = Kaydetme
settings-editor-backup = Önceki sürümü Çöp Kutusu'nda tut
settings-editor-backup-desc = Kaydetme bir dosyanın üzerine yazmadan önce, geri yükleyebilmeniz için bir kopyası (<ad>.bak) Çöp Kutusu'na gönderilir.
settings-editor-jpeg = JPEG kalitesi
settings-editor-jpeg-desc = Döndürülen veya kırpılan JPEG'ler yeniden kodlanır (kayıpsız bir yol yoktur); diğer biçimler kayıpsız kaydedilir.
settings-editor-max-image = En büyük resim
settings-editor-max-image-desc = Daha büyük resimler resim düzenleyici tarafından çözülmez.

## Fonts page

settings-fonts-sample-ui = Pijamalı hasta yağız şoföre çabucak güvendi — Belgeler, Resimler, Ünïcödé
settings-fonts-sample-headings = Özellikler · İndirilenler · Ayarlar
settings-fonts-using = Kullanılan: { $font }

settings-fonts-interface = Arayüz
settings-fonts-interface-desc = Etiketler, menüler, düğmeler ve dosya adları. Varsayılan: yerleşik Inter; Segoe UI gibi yüklü yazı tipleri de kullanılabilir.
settings-fonts-headings = Başlıklar
settings-fonts-headings-desc = Başlıklar, iletişim kutusu üst bilgileri ve bölüm adları. Boş: arayüz yazı tipinin yarı kalın biçimi.
settings-fonts-headings-hint = Arayüz yazı tipinin yarı kalını
settings-fonts-terminal-desc = Tümleşik terminal. Varsayılan: yerleşik Cascadia Mono (bitişik harf yok, != ve -> yazıldığı gibi görünür).
settings-fonts-code = Kod düzenleyici
settings-fonts-code-desc = Metin düzenleyici, kod önizlemeleri ve diğer eş aralıklı metinler. Varsayılan: Cascadia Mono.
settings-fonts-hex = Onaltılık görünüm
settings-fonts-hex-desc = Onaltılık düzenleyici ve onaltılık önizleme.
settings-fonts-numbers = Sayılar
settings-fonts-numbers-desc = Dosya listesindeki ve etkinlik panelindeki boyutlar, tarihler ve ilerleme değerleri. Boş: arayüz yazı tipi. IBM Plex Sans, Noto Sans ve eş aralıklı yazı tiplerinin rakamları eşit genişliktedir, bu yüzden sütunlar hizalanır.
settings-fonts-numbers-hint = Arayüz yazı tipiyle aynı
settings-fonts-monospace = Eş aralıklı
settings-fonts-rendering = İşleme
settings-fonts-hinting = İpucu verme (hinting)
settings-fonts-hinting-desc = Harf gövdelerini piksel ızgarasına oturtur: küçük boyutlarda ve %100 ölçekte daha keskin metin.
settings-fonts-subpixel = Alt piksel konumlandırma
settings-fonts-subpixel-desc = Harfleri kesirli piksel konumlarına yerleştirir: daha düzgün aralık, biraz daha yumuşak gövdeler.
settings-fonts-hinting-strength = İpucu verme gücü
settings-fonts-hinting-strength-desc = Keskin, harf genişliklerini de piksel ızgarasına oturtur (%100 ölçekte en net görünüm); Normal egui'nin varsayılanıdır; Hafif, tasarlanan biçimleri korur.
settings-fonts-hinting-crisp = Keskin
settings-fonts-hinting-normal = Normal
settings-fonts-hinting-light = Hafif
settings-fonts-gamma-dark = Metin kalınlığı (koyu temalar)
settings-fonts-gamma-light = Metin kalınlığı (açık temalar)
settings-fonts-gamma-desc = Kenar yumuşatmanın opaklığa nasıl dönüştüğü: düşük değerler metni daha kalın çizer, 1,00 doğrusaldır. Otomatik, işleyicinin kendi eğrisini kullanır.
settings-fonts-reset-all = Tüm yazı tiplerini sıfırla
settings-fonts-reset-all-desc = Keskin ipucu verme ve boyut farkı olmadan Inter / Cascadia Mono'ya döner.
settings-fonts-embedded = Yerleşik
settings-fonts-reset-one = Bu yazı tipini sıfırla
settings-fonts-offset = Boyut farkı (punto)

## Font sources and notices (theme/fonts.rs)

settings-font-source-weight = { $family } (kalınlık { $weight })
settings-font-source-embedded = { $font } (yerleşik)
settings-font-not-installed = { $missing } yüklü değil; { $used } kullanılıyor
settings-font-no-semibold = { $family } yazı tipinin yarı kalın biçimi yok; başlıklarda normal biçim kullanılıyor

## Theme gallery and accent

settings-accent-orbit-blue = Orbit mavisi
settings-accent-violet = Menekşe
settings-accent-pink = Pembe
settings-accent-red = Kırmızı
settings-accent-amber = Kehribar
settings-accent-green = Yeşil
settings-accent-teal = Deniz mavisi
settings-accent-sky = Gök mavisi
settings-theme-a11y = Tema: { $name }
settings-theme-tooltip = { $name } — { $author }
settings-theme-high-contrast = Yüksek karşıtlık

settings-theme-hc-on = Yüksek karşıtlık açık: siz kapatana kadar renk temasının yerini alır.
settings-theme-import = Temayı içe aktar…
settings-theme-export = Geçerli temayı dışa aktar…
settings-theme-export-tooltip = Geçerli renkleri (tema, vurgu ve geçersiz kılmalar) bir tema dosyası olarak kaydet
settings-theme-folder = Temalar klasörü
settings-theme-custom-tag = özel
settings-custom = Özel

## Keyboard page

settings-keys-preset-explorer-desc = Windows Gezgini alışkanlıkları: F2 yeniden adlandırır, Del, Alt+ok tuşları, Ctrl+1…9 sekmeler.
settings-keys-preset-commander-desc = Total Commander tarzı: F3 görüntüle, F5 kopyala, F6 taşı, F7 klasör, F8 sil, Tab bölmeler arasında geçer.
settings-keys-preset-finder-desc = macOS Finder: Enter yeniden adlandırır, Cmd+Aşağı açar, Cmd+Backspace Çöp Kutusu'na taşır, Cmd+1…4 görünümler.
settings-keys-preset-vim-desc = hjkl, gg / G, yy kopyalar, dd keser, pp yapıştırır, / filtreler, m ve ' ile işaretler.
settings-keys-preset-commander = Commander
settings-keys-shortcuts = Kısayollar
settings-keys-search-hint = Eylem veya tuş ara
settings-keys-reset-changes = { $count } değişikliği sıfırla
settings-keys-record-title = “{ $action }” için kısayol

settings-keys-preset = Hazır ayar
settings-keys-key-preset = Tuş hazır ayarı
settings-keys-cmd-ctrl = Cmd tuşunu Ctrl say
settings-keys-cmd-ctrl-desc = Ctrl ile yazılmış atamalar macOS'ta Cmd tuşuna da yanıt verir.
settings-keys-marks = İşaretler
settings-keys-marks-desc = m + tuş geçerli klasörü işaretler, ' + tuş ona geri döner (Vim benzeri hazır ayarında her zaman açık).
settings-keys-conflicts-only = Yalnızca çakışmalar
settings-keys-changed-only = Yalnızca değişenler
settings-keys-empty = Eşleşen kısayol yok
settings-keys-reset-one = Hazır ayara sıfırla
settings-add = Ekle
settings-keys-record = Kısayol ata

## Custom commands page

settings-commands-template-vscode = VS Code'da aç
settings-commands-template-git = Burada git status
settings-commands-template-copy-names = Adları panoya kopyala
settings-commands-template-list = Klasörü terminalde listele
settings-commands-ph-focused = odaktaki öğe (ya da klasör)
settings-commands-ph-selected = seçili yollar
settings-commands-ph-names = seçili adlar
settings-commands-ph-dir = geçerli klasör
settings-commands-ph-other = diğer bölmenin klasörü
settings-commands-ph-percent = düz bir % işareti
settings-commands-new = Yeni komut
settings-commands-record-title = “{ $name }” için kısayol tuşu
settings-bookmarks-record-title = “{ $name }” yer işareti için kısayol tuşu
settings-bookmarks-default-name = Yer işareti
settings-bookmarks-icon-default = varsayılan

settings-commands-placeholders = Yer tutucular
settings-commands-help = Tam olarak %s veya %n olan bir bağımsız değişken, her öğe için ayrı bir bağımsız değişkene genişler. Komutlar komut paletinde (Ctrl+Shift+P) ve sağ tıklama menüsünde görünür; kısayol tuşu atanabilir.
settings-commands-or-template = ya da şununla başlayın
settings-name = Ad
settings-commands-delete = Komutu sil
settings-commands-run = Çalıştır
settings-commands-command = Komut
settings-commands-command-hint = program ya da komut satırı
settings-commands-args-hint = %s   (tırnaklar sözcükleri gruplar)
settings-commands-show-for = Şunlarda göster
settings-commands-background-ctx = Klasör arka planı
settings-commands-run-in = Çalıştırma yeri
settings-commands-run-background = Arka planda
settings-commands-run-terminal = Terminal paneli
settings-commands-hotkey = Kısayol tuşu
settings-commands-record = Kısayol ata…
settings-commands-empty = Henüz özel komut yok
settings-bookmarks-add-current = Geçerli klasörü yer işaretlerine ekle
settings-bookmarks-add-empty = Boş yer işareti
settings-bookmarks-help = 1–9 arasındaki yer işaretleri Alt+1…9 ile ve kendi kısayol tuşlarıyla da açılır.
settings-bookmarks-path-hint = Klasör yolu
settings-bookmarks-record = Kısayol tuşu ata
settings-bookmarks-empty = Henüz yer işareti yok

## About & updates page

settings-about-version = Sürüm { $version }
settings-about-build = { $os } / { $arch } için { $kind ->
    [debug] hata ayıklama
   *[release] sürüm
    } derlemesi. MIT OR Apache-2.0 lisanslıdır.
settings-updates-privacy = Soyuz GitHub'dan yalnızca herkese açık sürüm listesini ister; sizinle veya dosyalarınızla ilgili hiçbir veri gönderilmez. İndirilenler doğrulanır (SHA-256 ve yükleyicinin imzası) ve siz tıklamadan asla yüklenmez.
settings-updates-up-to-date = Güncelsiniz. En yeni sürüm { $latest }.
settings-updates-failed = Güncelleme denetimi başarısız oldu: { $error }
settings-updates-download-tooltip = { $name } dosyasını İndirilenler klasörünüze kaydet
settings-updates-downloading = { $name } indiriliyor…
settings-updates-progress = { $done } / { $total }
settings-updates-saved-to = Kaydedildiği yer: { $path }
settings-updates-sha-ok = SHA-256 doğrulandı: { $hash }
settings-updates-sig-valid = Authenticode imzası: geçerli
settings-updates-sig-unsigned = Authenticode imzası: yok (imzasız derleme)
settings-updates-sig-invalid = Authenticode imzası: GEÇERSİZ ({ $code })
settings-updates-sig-unknown = Denetlenecek yükleyici imzası yok (taşınabilir zip).
settings-updates-available = { $version } sürümü kullanılabilir (sizdeki: { $current })
settings-updates-prerelease = ön sürüm

## Default file manager (Advanced page)

settings-fm-registered = Klasörler ve sürücüler artık Soyuz ile açılıyor
settings-fm-restored = Klasör işleyicisi olarak Dosya Gezgini geri yüklendi
settings-fm-desc = Diğer uygulamalardan, masaüstünden, kısayollardan veya Win+R ile açılan klasörler ve sürücüler Dosya Gezgini yerine Soyuz'da açılır. Soyuz, HKEY_CURRENT_USER\Software\Classes altında Directory ve Drive için bir “Soyuz'da aç” komutu ekler ve bunu varsayılan yapar — yalnızca sizin hesabınız için, yönetici hakkı gerekmeden, sistem genelinde hiçbir değişiklik yapılmadan. Özel klasörler (Bu bilgisayar, Denetim Masası, Çöp Kutusu) Gezgin'de kalır. “Gezgin'i geri yükle”, Soyuz'un eklediklerini tam olarak kaldırır ve önceden orada olanları geri koyar.
settings-fm-checking = Denetleniyor…
settings-fm-status-explorer = Şu anda: klasörler Dosya Gezgini ile açılıyor.
settings-fm-status-soyuz = Şu anda: klasörler Soyuz ile açılıyor ({ $command }).
settings-fm-status-modified = Soyuz kaydolduktan sonra başka bir program klasör işleyicisini değiştirdi. “Gezgin'i geri yükle”, Soyuz'un girdilerini kaldırır ve o programın seçimini yerinde bırakır.
settings-fm-status-unsupported = Bu platformda henüz kullanılamıyor.
settings-fm-temp-copy = Not: bu kopya { $path } konumundan çalışıyor; onu taşır veya silerseniz Gezgin'i geri yükleyene kadar klasörler açılmaz.

settings-about-button = Soyuz hakkında…
settings-updates = Güncellemeler
settings-updates-repo = Sürüm deposu
settings-updates-repo-desc = Soyuz güncellemelerinin geldiği GitHub deposu (sahip/ad veya URL). Boş bırakmak güncelleme denetimini kapatır.
settings-updates-weekly = Haftada bir denetle
settings-updates-weekly-desc = Başlangıçta, en fazla 7 günde bir. Yalnızca bildirim gösterir; hiçbir şey indirilmez.
settings-updates-prereleases = Ön sürümleri dahil et
settings-updates-prereleases-desc = Sürüm adaylarını ve betaları da önerir.
settings-updates-check = Güncellemeleri denetle
settings-updates-no-repo = Denetimi etkinleştirmek için bir sürüm deposu ayarlayın.
settings-updates-status = Güncelleme durumu
settings-updates-checking = GitHub Releases denetleniyor…
settings-updates-download = İndir ve doğrula
settings-updates-no-asset = Bu sürümde bu platform için indirme yok.
settings-updates-copy-link = Sürüm bağlantısını kopyala
settings-cancel = İptal
settings-updates-detached-sig = Ayrı bir imza dosyası yayımlanmış; Soyuz bunu henüz doğrulamıyor.
settings-updates-run = Yükleyiciyi çalıştır…
settings-updates-reveal = Soyuz'da göster
settings-updates-run-note = Yükleyici kendi penceresinde açılır. Güncellemeyi bitirmek için Soyuz'u kapatmanızı isteyebilir.
settings-updates-run-now = Şimdi çalıştır
settings-updates-started = Yükleyici başlatıldı
settings-updates-start-failed = Yükleyici başlatılamıyor
settings-fm-failed = Klasör işleyicisi değiştirilemedi
settings-fm = Varsayılan dosya yöneticisi
settings-fm-open-with = Klasörleri Soyuz ile aç
settings-fm-reregister = Bu kopyayı yeniden kaydet
settings-fm-make-default = Soyuz'u varsayılan yap
settings-fm-restore = Gezgin'i geri yükle

## Index group (Preview & search page)

settings-index-usn-active = NTFS değişiklik günlüğü kullanılıyor: Soyuz kapalıyken yapılan değişiklikler başlatıldıktan sonra saniyeler içinde uygulanır.
settings-index-usn-admin = NTFS değişiklik günlüğünü okumak yönetici hakları gerektirir; bu yüzden Soyuz her başlangıçtan sonra dizinlenmiş klasörleri arka planda yeniden denetler (bu sırada aramalar dizini kullanmaya devam eder). Günlüğü kullanmak için Soyuz'u yönetici olarak çalıştırın.
settings-index-usn-unsupported = Değişiklik günlüğü kullanılamıyor ({ $reason }); klasörler her başlangıçtan sonra yeniden denetlenir.
settings-index-usn-off = Değişiklik günlüğü kullanılmıyor; klasörler her başlangıçtan sonra yeniden denetlenir.
settings-index-enabled-desc = Dizinlenen klasörlerin adlarını, boyutlarını, tarihlerini ve etiketlerini yerel bir veritabanında tutar: oradaki aramalar anında sonuç verir ve akıllı klasörler canlı güncellenir.
settings-index-content-files = İçeriği dizinlenmiş { $count } dosya
settings-index-journal-changes = Başlangıçta günlükten { $count } değişiklik uygulandı
settings-index-last-write = Son değişiklik { $seconds } sn önce yazıldı
settings-index-database = Veritabanı: { $path }
settings-index-root-hint = Dizinlenecek klasör, ör. D:\Fotoğraflar
settings-index-usn-desc = Soyuz kapalıyken yapılan değişiklikleri doğrudan birimin değişiklik günlüğünden uygular (Windows, NTFS). Yönetici hakları gerektirir; aksi hâlde klasörler her başlangıçtan sonra yeniden denetlenir.

settings-index = Dizin
settings-index-enabled = Arka plan dizini
settings-index-status = Durum
settings-index-starting = Başlatılıyor…
settings-index-off = Dizinleme kapalı. Aramalar diski tarar.
settings-index-rebuild = Dizini yeniden oluştur
settings-index-rebuild-tooltip = Her şeyi unut ve klasörleri yeniden dizinle
settings-index-roots = Dizinlenen klasörler
settings-index-roots-desc = ~ giriş klasörünüzdür. Dizinlenen başka bir klasörün içindeki klasörler zaten onun kapsamındadır.
settings-index-remove-root = Bu klasörü dizinlemeyi durdur
settings-index-add-root = Klasör ekle
settings-index-skip-desc = Her satıra bir tane: bir ad (node_modules, *.tmp) ya da bir yol sonu (AppData/Local/Temp, AppData/Local/**/Cache).
settings-index-content = Dosya içeriklerini dizinle
settings-index-content-desc = content: aramaları için küçük metin dosyalarındaki sözcükler. Daha fazla disk alanı ve zaman kullanır.
settings-index-content-limit = İçerik boyutu sınırı
settings-index-content-limit-desc = Daha büyük dosyalar yalnızca adlarıyla dizinlenir.
settings-index-content-types = İçerik dosya türleri
settings-index-content-types-desc = İçerikleri dizinlenen uzantılar.
settings-index-usn = NTFS değişiklik günlüğünü kullan
settings-index-throttle = Tarama duraklaması
settings-index-throttle-desc = Dizinlerken her 1000 dosyada bir duraklar; yüksek değer diski daha az yorar.
settings-index-smart-refresh = Akıllı klasör yenileme
settings-index-smart-refresh-desc = Dosyalar değişirken akıllı klasörler en fazla bu sıklıkta güncellenir.
settings-index-use-for-search = Aramalarda dizini kullan
settings-index-use-for-search-desc = Kapalı: arama çubuğu her zaman diski tarar (akıllı klasörler yine de dizini kullanır).

## AI assistant page

settings-ai-op-failed = { $what } başarısız oldu: { $error }
settings-ai-profiles = Sağlayıcı profilleri
settings-ai-profile-removed = “{ $name }” kaldırıldı. Kayıtlı anahtarı siz silene kadar kimlik bilgisi deposunda kalır.
settings-ai-fast-model = Hızlı model
settings-ai-fast-model-desc = Adlandırma ve sınıflandırma. Yerel bir model bunun için uygundur.
settings-ai-strong-model = Güçlü model
settings-ai-strong-model-desc = Çok adımlı dosya işlerini planlama.
settings-ai-key-stored = Kayıt yeri: { $store }
settings-ai-key-env = Ortam değişkeninden ({ $var })
settings-ai-key-env-missing = { $var } ayarlanmamış
settings-ai-key-saved = Anahtar kaydedildi: { $store }
settings-ai-key-deleted = Anahtar silindi
settings-ai-store-windows = Windows Kimlik Bilgisi Yöneticisi
settings-ai-store-other = işletim sistemi anahtar zinciri
settings-ai-loading = yükleniyor…
settings-ai-spend = { $cost } · { $tokens } token · { $requests } istek
settings-ai-spend-unpriced = model fiyat tablosunda yok
settings-ai-spend-share = sınır: %{ $percent }
settings-ai-spend-exceeded = sınıra ulaşıldı, istekler engelleniyor
settings-ai-test-models-error = (model listesi alınamadı: { $error })
settings-ai-tokens = { $count } token
settings-ai-model = Model
settings-ai-price-input = Girdi
settings-ai-price-output = Çıktı
settings-ai-price-cache-read = Önbellek okuma
settings-ai-price-cache-write = Önbellek yazma

settings-ai-empty = Henüz yapay zekâ sağlayıcısı yok
settings-ai-empty-hint = Her şeyi bu bilgisayarda tutmak için yerel bir model (Ollama, LM Studio …) ya da bir bulut sağlayıcısı ekleyin.
settings-ai-assistant = Asistan
settings-ai-enabled = Yapay zekâ özelliklerini etkinleştir
settings-ai-enabled-desc = Kapalıyken asistan gizlenir ve tüm model istekleri engellenir.
settings-ai-privacy = Asistanın görebilecekleri
settings-ai-privacy-desc = Tüm klasörler için varsayılan. Yalnızca üst veri: adlar, boyutlar ve tarihler; dosya içerikleri bu bilgisayardan asla çıkmaz.
settings-ai-metadata-only = Yalnızca üst veri
settings-ai-contents = İçerikler
settings-ai-default-profile = Varsayılan profil
settings-ai-default-profile-desc = Bir görev profil belirtmediğinde kullanılır.
settings-ai-first-profile = İlk profil
settings-ai-scope = Kapsam
settings-ai-roots = İzin verilen klasörler
settings-ai-roots-desc = Her satıra bir klasör. Yapay zekâ araçları yalnızca bunların içindeki öğeleri görebilir ve değiştirebilir (boş = giriş klasörünüz).
settings-ai-blocked = Ek engellenen desenler
settings-ai-blocked-desc = Her satıra bir glob deseni (ör. **/private/**). .ssh, .env, anahtar zincirleri, tarayıcı profilleri ve cüzdanlar her zaman engellenir.
settings-ai-sensitive = Hassas konumlara izin ver
settings-ai-sensitive-desc = Yapay zekâ araçlarının .ssh, .env dosyalarına, anahtar zincirlerine, tarayıcı profillerine ve cüzdanlara erişmesine izin verir. Önerilmez.
settings-ai-read-limit = Metin okuma sınırı
settings-ai-read-limit-desc = Asistanın bir dosyadan tek seferde okuyabileceği en büyük kısım.
settings-ai-network = Ağ ve bütçe
settings-ai-timeout = Yanıt zaman aşımı
settings-ai-timeout-desc = Bir sağlayıcının yanıt vermeye başlaması için beklenecek saniye.
settings-ai-idle-timeout = Akış boşta kalma zaman aşımı
settings-ai-idle-timeout-desc = Akan bir yanıt bu kadar süre takılırsa vazgeçer.
settings-ai-retries = Yeniden denemeler
settings-ai-retries-desc = Hız sınırlarında (429), aşırı yükte ve sunucu hatalarında; retry-after değerine uyar.
settings-ai-budget-warn = Bütçe uyarısı
settings-ai-budget-warn-desc = Bir profil aylık sınırının bu oranını kullandığında uyarır.
settings-ai-tasks = Asistan görevleri
settings-ai-approvals = Onaylar
settings-ai-approvals-desc = Her zaman sor ya da küçük, çakışmasız yazma planlarının sorulmadan çalışmasına izin ver. Asistan dosya içeriği okuduktan sonra asla; yıkıcı planlar her zaman sorar.
settings-ai-always-ask = Her zaman sor
settings-ai-auto-writes = Küçük yazmalar otomatik
settings-ai-max-risk = En yüksek araç sınıfı
settings-ai-max-risk-desc = Asistana hangi araçların sunulacağı.
settings-ai-read-only = Salt okunur
settings-ai-write = Yazma
settings-ai-destructive = Çöp Kutusu / silme
settings-ai-steps = Görev başına adım
settings-ai-steps-desc = Bir görev durmadan önceki model turu sayısı.
settings-ai-token-budget = Görev başına token bütçesi
settings-ai-token-budget-desc = Tüm turların girdi + çıktı tokenları (0 = sınırsız).
settings-ai-time-limit = Görev başına süre sınırı
settings-ai-time-limit-desc = Gerçek süre; Esc bir görevi istediğiniz an durdurur.
settings-ai-auto-reads = Otomatik okumalar
settings-ai-auto-reads-desc = Asistanın görev başına kendi başına çalıştırabileceği okuma araçları.
settings-ai-compact = Şu boyuttan sonra sıkıştır
settings-ai-compact-desc = Konuşma bu boyuta ulaşınca eski araç sonuçları özetlenir.
settings-ai-keep-recent = Son sonuçları koru
settings-ai-keep-recent-desc = Sıkıştırırken olduğu gibi tutulan en yeni araç sonuçları.
settings-ai-listing = Bağlamdaki klasör girdileri
settings-ai-listing-desc = Model için özetlenen geçerli klasör girdileri.
settings-ai-confirm-cost = Pahalı görevlerden önce sor
settings-ai-confirm-cost-desc = Tahmini maliyet bunun üzerindeyse onay ister (0 = hiç sorma).
settings-ai-redact = Bulut modelleri için gizli bilgileri maskele
settings-ai-redact-desc = Bulut sağlayıcılarına gönderilen dosya verilerindeki e-posta adreslerini, API anahtarlarını ve tokenları maskeler (yerel modeller değiştirilmemiş hâlini görür).
settings-ai-name-excerpt = Ad önerileri: dosya başına metin
settings-ai-name-excerpt-desc = Ad önerirken her metin veya belge dosyasından gönderilen karakter sayısı (yalnızca “içerik” erişimi olan klasörlerde; 0 = yalnızca adlar ve üst veriler).
settings-ai-vision-px = Ad önerileri: resim boyutu
settings-ai-vision-px-desc = Görsel modellere gönderilen küçültülmüş resmin uzun kenarı (“Resim kabul eder” açık profiller, “içerik” erişimi olan klasörler; 0 = asla resim gönderme).
settings-ai-name-files = Ad önerileri: dosyalar
settings-ai-name-files-desc = İstek başına bakılan en fazla dosya sayısı.
settings-ai-folders = Klasör izinleri
settings-ai-per-folder = Klasör başına erişim
settings-ai-per-folder-desc = Bir klasör ve altındaki her şey için “Asistanın görebilecekleri” ayarını geçersiz kılar (en derindeki kural geçerlidir). Asistan panelinden veya bir klasörün Özellikler penceresinden de ayarlanabilir.
settings-ai-folder-hint = klasör yolu
settings-ai-metadata = Üst veri
settings-ai-remove-rule = Bu kuralı kaldır
settings-ai-no-rules = Klasör kuralı yok: her klasör yukarıdaki varsayılanı kullanır.
settings-ai-add-rule = Klasör kuralı ekle
settings-ai-task-model-hint = profilin modeli
settings-ai-name-unique = benzersiz bir ad gerekli
settings-ai-delete-profile = Profili sil
settings-ai-default-tag = varsayılan
settings-ai-make-default = Varsayılan yap
settings-ai-base-url = Temel URL
settings-ai-model-hint = model kimliği
settings-ai-pick = Seç…
settings-ai-api-key = API anahtarı
settings-ai-tools = Araçlar
settings-ai-native = Yerleşik
settings-ai-json = JSON yedeği
settings-ai-max-tokens = En fazla çıktı tokenı
settings-ai-vision = Resim kabul eder
settings-ai-vision-desc = görsel model: “Ad öner”, içeriğe izin verilen yerlerde küçültülmüş resimler gönderebilir
settings-ai-budget = Aylık bütçe
settings-ai-budget-hint = 0 = sınır yok
settings-ai-this-month = Bu ay
settings-ai-key-not-needed = Gerekmiyor (yerel sunucu)
settings-ai-no-key = Anahtar yok
settings-ai-key-missing = Henüz kaydedilmedi
settings-ai-key-hint = kaydetmek için bir anahtar yapıştırın
settings-ai-save-key = Anahtarı kaydet
settings-ai-forget-key = Kayıtlı anahtarı sil
settings-ai-key-ref = Anahtar başvurusu
settings-ai-key-ref-hint = credential:<ad> veya env:<DEĞİŞKEN>
settings-ai-key-ref-desc = config.toml yalnızca bu başvuruyu saklar, anahtarın kendisini asla.
settings-ai-headers = Ek üst bilgiler
settings-ai-header-value = değer
settings-ai-add-header = Üst bilgi ekle
settings-ai-headers-desc = Üst bilgi değerleri config.toml dosyasında saklanır; gizli bilgileri bunun yerine API anahtarı alanına girin.
settings-ai-testing = Sınanıyor…
settings-ai-test = Bağlantıyı sına
settings-ai-test-tooltip = Modelleri listeler ve modelden bir test aracını çağırmasını isteyen çok küçük bir istek gönderir (birkaç token harcar)
settings-ai-model-unlisted = model sunucunun listesinde yok
settings-ai-prices = Fiyatlar
settings-ai-price-table = Fiyat tablosu
settings-ai-price-table-desc = Milyon token başına USD; maliyet göstergesi ve bütçe sınırları için kullanılır. Model bir glob desenidir; ilk eşleşme geçerlidir.
settings-ai-add-price = Fiyat ekle
settings-restore-defaults = Varsayılanları geri yükle

## AI → Connections (MCP)

settings-mcp-risk-write = Okuma + yazma
settings-mcp-risk-delete = Silme dahil
settings-mcp-installed = Soyuz, { $app } uygulamasına eklendi. Etkinleşmesi için { $app } uygulamasını yeniden başlatın.
settings-mcp-connections = Bağlantılar (MCP)
settings-mcp-intro = Claude Desktop, Claude Code veya VS Code gibi yapay zekâ uygulamalarının Soyuz'u Model Context Protocol üzerinden kullanmasına izin verin. Yalnızca izin verdiğiniz klasörlerin içinde çalışırlar; değişiklikler burada onaylayacağınız bir plan olarak görünür, ardından geri alınabilir işler olarak çalışır. Her çağrı ai-audit.jsonl dosyasına yazılır.
settings-mcp-pipe = kanal { $name }
settings-mcp-pipe-off = kanal kapalı
settings-mcp-waiting = onay bekleyen { $count } istek
settings-mcp-token-stored = Token kimlik bilgisi deposuna kaydedildi
settings-mcp-token-revoked = Token iptal edildi
settings-mcp-client-removed = “{ $name }” ve tokenı kaldırıldı
settings-mcp-starts-with = başlatma komutu: { $command }
settings-mcp-clients = MCP istemcileri
settings-mcp-add-to = Soyuz'u şuna ekle …
settings-mcp-file-changes = Dosyadaki değişiklikler
settings-mcp-new-file = Yeni dosya
settings-mcp-write-to = { $app } yapılandırmasına yaz
settings-mcp-servers = Dış MCP sunucuları
settings-mcp-servers-intro = Uygulama içi asistanın kullanabileceği diğer MCP sunucularının araçları. Açıklamaları ve sonuçları güvenilmez kabul edilir; “okuma” düzeyinin üzerindeki araçlar önce onay ister.

settings-mcp-server = Soyuz MCP sunucusu
settings-mcp-attach = `soyuz --mcp` bu pencereye bağlanabilsin
settings-mcp-attach-desc = Araç çağrıları bu pencerede görünür ve yazma araçları için onay kartı çıkar. Kapalı = `soyuz --mcp` arayüzsüz çalışır.
settings-mcp-http = HTTP aktarımı
settings-mcp-http-desc = Program başlatamayan istemciler için 127.0.0.1 üzerinde Streamable HTTP. Her istemcinin bir tokenı olmalıdır.
settings-mcp-port = HTTP bağlantı noktası
settings-mcp-port-desc = Uç nokta: http://127.0.0.1:<port>/mcp
settings-mcp-origins = İzin verilen tarayıcı kökenleri
settings-mcp-origins-desc = Her satıra bir tane. Geri döngü kökenlerine her zaman izin verilir; diğerleri reddedilir (DNS yeniden bağlama savunması).
settings-mcp-register = Bilinmeyen istemcileri kaydet
settings-mcp-register-desc = Yeni bir istemci “beklemede” olarak eklenir ve siz aşağıda onaylayana kadar yalnızca okuyabilir. Kapalı = reddedilir.
settings-mcp-approval-timeout = Onay zaman aşımı
settings-mcp-approval-timeout-desc = Onay kartını kimse yanıtlamazsa değişikliği reddeder.
settings-mcp-approval-ui = Onay kartları
settings-mcp-approval-ui-desc = Bir istemcinin plan kartının göründüğü yer: asistan panelinde (bir istek geldiğinde açılır) ya da pencerenin üzerinde kalıcı bir kart olarak.
settings-mcp-approval-panel = Asistan paneli
settings-mcp-approval-modal = Kalıcı kart
settings-mcp-pending = beklemede · salt okunur
settings-mcp-approve = Onayla
settings-mcp-approved = onaylandı
settings-mcp-remove-client = İstemciyi kaldır
settings-mcp-may = İzinler
settings-mcp-ask = Değişikliklerden önce sor
settings-mcp-no-ask = değişiklikler sorulmadan çalışır
settings-mcp-folders-hint = boş = asistanın izinli klasörleri
settings-mcp-token = HTTP tokenı
settings-mcp-rotate = Tokenı yenile
settings-mcp-generate = Token oluştur
settings-mcp-revoke = İptal et
settings-mcp-token-once = Bu tokenı şimdi kopyalayın; Soyuz onu bir daha göstermeyecek.
settings-mcp-copy-token = Tokenı kopyala
settings-mcp-copy-config = Yapılandırmayı kopyala
settings-mcp-client-hint = istemci adı, ör. my-agent
settings-mcp-add-client = İstemci ekle
settings-mcp-no-clients = Henüz istemci yok. Aşağıdaki “Soyuz'u şuna ekle …” bölümünü kullanın ya da bir istemci bağlayın: burada beklemede olarak görünür.
settings-mcp-snippet = Kod parçası
settings-mcp-already = Zaten ayarlı: dosya bu girdiyi içeriyor.
settings-mcp-copy-snippet = Kod parçasını kopyala
settings-close-button = Kapat
settings-mcp-backup = Eski dosya yanında .soyuz-backup olarak saklanır.
settings-mcp-remove-server = Sunucuyu kaldır
settings-mcp-command-hint = program (stdio), ör. uvx
settings-mcp-args-hint = her satıra bir tane
settings-mcp-url = veya URL
settings-mcp-server-token = Token
settings-mcp-tools-may = Araçlarının izni
settings-mcp-ask-run = Çalıştırmadan önce sor
settings-mcp-trust-hints = Salt okunur ipuçlarına güven
settings-mcp-trust-hints-desc = yalnızca güvendiğiniz sunucular için
settings-mcp-timeout = Zaman aşımı
settings-mcp-test = Sına
settings-mcp-server-hint = sunucu adı
settings-mcp-add-server = Sunucu ekle

## Slider units (after a number)

settings-unit-chars = karakter
settings-unit-days = gün
settings-unit-lines = satır
settings-unit-rows = satır

## Accessible names of icon-only buttons
settings-a11y-remove = Kaldır
settings-a11y-move-up = Yukarı taşı
settings-a11y-move-down = Aşağı taşı
settings-a11y-record-key = Kısayol kaydet
settings-a11y-button = Düğme
