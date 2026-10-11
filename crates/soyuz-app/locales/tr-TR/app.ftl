### Soyuz arayüz metinleri (tr-TR): app
### Anahtarlar en-US/app.ftl ile birebir aynı olmalı.

## Dosya bağlam menüsü (ui/context_menu.rs)

app-ctx-open = Aç
app-ctx-open-in-new-tab = Yeni sekmede aç
app-ctx-open-in-other-pane = Diğer bölmede aç
app-ctx-open-with = Birlikte aç
app-ctx-open-with-looking = Uygulamalar aranıyor…
app-ctx-open-with-none = Kayıtlı uygulama bulunamadı
app-ctx-open-with-choose = Başka bir uygulama seç…
app-ctx-edit-in-soyuz = Soyuz'da düzenle
app-ctx-cut = Kes
app-ctx-copy = Kopyala
app-ctx-paste-into-folder = Klasörün içine yapıştır
app-ctx-previous-versions = Önceki sürümü geri yükle…
app-ctx-copy-to-other-pane = Diğer bölmeye kopyala
app-ctx-move-to-other-pane = Diğer bölmeye taşı
app-ctx-rename = Yeniden adlandır
app-ctx-duplicate = Çoğalt
app-ctx-create-link = Bağlantı oluştur…
app-ctx-move-to-trash = Çöp Kutusu'na taşı
app-ctx-delete-permanently = Kalıcı olarak sil
app-ctx-copy-path = Yolu kopyala
app-ctx-copy-paths = Yolları kopyala
app-ctx-copy-name = Adı kopyala
app-ctx-copy-names = Adları kopyala
app-ctx-reveal-in-system = Sistem dosya yöneticisinde göster
app-ctx-open-terminal-here = Terminali burada aç
app-ctx-open-external-terminal = Harici terminali burada aç
app-ctx-add-to-favorites = Sık kullanılanlara ekle
app-ctx-folder-sizes = Klasör boyutlarını hesapla
app-ctx-properties = Özellikler
app-ctx-view = Görünüm
app-ctx-sort-by = Sıralama ölçütü
app-ctx-group-by = Gruplama ölçütü
app-ctx-refresh = Yenile
app-ctx-paste = Yapıştır
app-ctx-paste-as-link = Bağlantı olarak yapıştır
app-ctx-paste-and-verify = Yapıştır ve doğrula
app-ctx-new-folder = Yeni klasör
app-ctx-new-file = Yeni dosya
app-ctx-new-link = Yeni bağlantı…
app-ctx-copy-folder-path = Klasör yolunu kopyala
app-ctx-show-hidden = Gizli dosyaları göster
app-ctx-item-checkboxes = Öğe onay kutuları
app-ctx-select-all = Tümünü seç
app-ctx-columns = Sütunlar
app-ctx-reset-columns = Sütunları sıfırla

## Görünüm modları ve sıralama

app-view-details = Ayrıntılar
app-view-list = Liste
app-view-icons = Simgeler
app-view-gallery = Galeri
app-view-columns = Sütunlar
app-sort-ascending = Artan
app-sort-descending = Azalan

## Klasör renkleri ve amblemler (ui/looks.rs)

app-looks-folder-color = Klasör rengi
# Renk noktasının ekran okuyucu adı; $color çevrilmiş renk adıdır.
app-looks-color-folder = { $color } klasör
app-looks-no-color = Renk yok
app-looks-emblem = Amblem
app-looks-no-emblem = Amblem yok
app-color-red = Kırmızı
app-color-orange = Turuncu
app-color-yellow = Sarı
app-color-green = Yeşil
app-color-cyan = Camgöbeği
app-color-blue = Mavi
app-color-purple = Mor
app-color-gray = Gri
app-emblem-star = Yıldız
app-emblem-heart = Kalp
app-emblem-code = Kod
app-emblem-image = Resimler
app-emblem-music = Müzik
app-emblem-film = Video
app-emblem-briefcase = İş
app-emblem-book = Kitaplar
app-emblem-cloud = Bulut
app-emblem-lock = Özel
app-emblem-game = Oyunlar
app-emblem-archive = Arşiv

## Araç çubuğu (ui/toolbar.rs)

app-toolbar-back = Geri
app-toolbar-forward = İleri
app-toolbar-up = Yukarı
app-toolbar-settings = Ayarlar
app-toolbar-dual-pane = Çift bölme (sağ tık: düzenler, eşitleme)
app-toolbar-preview-pane = Önizleme bölmesi
app-toolbar-search = Alt klasörlerde ara
app-toolbar-command-palette = Komut paleti
app-toolbar-sort = Sırala
app-toolbar-view = Görünüm
app-toolbar-no-history = Geçmiş yok
app-toolbar-layout = Düzen
app-toolbar-hidden-files = Gizli dosyalar
app-toolbar-sidebar = Kenar çubuğu
app-toolbar-choose-columns = Sütunları seç…
app-toolbar-reset-folder-view = Klasör görünümünü sıfırla
app-toolbar-apply-view-to-all = Görünümü tüm klasörlere uygula…
app-toolbar-folders-first = Önce klasörler
app-toolbar-natural-order = Doğal sıralama (dosya2 < dosya10)
app-toolbar-sync-browsing = Eşitlenmiş gezinme
app-toolbar-swap-panes = Bölmeleri değiştir
app-toolbar-mirror-pane = Diğer bölmeye yansıt
app-toolbar-focus-other-pane = Diğer bölmeye geç
app-toolbar-filter = Filtrele

## Durum çubuğu (ui/statusbar.rs)

app-status-items = { $count } öğe
# Canlı filtre etkin: $count öğeden $visible tanesi eşleşiyor.
app-status-items-filtered = { $visible } / { $count } öğe
app-status-hidden = · { $count } gizli
app-status-selected = { $count } seçili
# $size biçimlendirilmiş boyuttur ("1,5 MB").
app-status-selected-size = { $count } seçili  ({ $size })
# $size piksel cinsinden simge boyutudur.
app-status-icon-size = Simge boyutu { $size }
app-status-free = { $free } boş
app-status-free-of = { $total } alanın { $free } kadarı boş
app-status-theme-system = Sistem temasını kullan
app-status-theme-light = Açık tema
app-status-theme-dark = Koyu tema

## Kenar çubuğu (ui/sidebar.rs)

app-sidebar-favorites = Sık kullanılanlar
app-sidebar-devices = Aygıtlar
app-sidebar-cloud = Bulut
app-sidebar-tags = Etiketler
app-sidebar-network = Ağ
app-sidebar-other = Diğer
app-sidebar-folders = Disk ağacı
app-sidebar-smart = Akıllı klasörler
app-sidebar-settings = Ayarlar
# $key bir klavye kısayoludur ("Ctrl+,").
app-sidebar-settings-tip = Ayarlar  ({ $key })
app-sidebar-remove-favorite = Sık kullanılanlardan kaldır
app-sidebar-this-pc = Bu bilgisayar
app-sidebar-computer = Bilgisayar
# $path bağlama noktasıdır; $free / $total biçimlendirilmiş boyutlardır.
app-sidebar-volume-tip = { $path }
    { $total } alanın { $free } kadarı boş
app-sidebar-scanning = Taranıyor…
app-sidebar-trash = Çöp Kutusu
# Kenar çubuğunun üstündeki simge çubuğu: bölümler ile disk ağacı arasında geçiş.
app-sidebar-view-bar = Kenar çubuğu görünümü
app-sidebar-view-places = Konumlar
app-sidebar-view-places-tip = Konumlar: sık kullanılanlar, aygıtlar, portal, etiketler ve daha fazlası
app-sidebar-view-tree = Disk ağacı
app-sidebar-view-tree-tip = Disk ağacı: sürücüleriniz ve giriş klasörünüz tek bir klasör ağacında
# $tip yukarıdaki ipuçlarından biri, $key bir klavye kısayoludur ("Ctrl+Shift+E").
app-sidebar-view-tip-key = { $tip }  ({ $key })

## Portal: kenar çubuğu bölümüne bırakılarak sabitlenen klasörler (ui/portal.rs)

app-sidebar-portal = Portal
app-portal-empty = Sabitlemek için klasörleri buraya bırakın
app-portal-missing = bulunamadı
# $path sabitlenen klasördür.
app-portal-missing-tip = { $path }
    Klasör bulunamadı (taşınmış, yeniden adlandırılmış ya da bağlı olmayan bir sürücüde olabilir).
app-portal-drop-badge = Portala sabitle
app-portal-open = Aç
app-portal-rename = Etiketi yeniden adlandır…
app-portal-reset-name = Klasörün adını kullan
app-portal-unpin = Portaldan kaldır
# $name bir klasör adıdır.
app-portal-pinned = “{ $name }” portala sabitlendi
app-portal-pinned-many = { $count } klasör portala sabitlendi
app-portal-folders-only = Portala yalnızca klasörler sabitlenebilir
app-portal-already = Zaten portala sabitlenmiş
app-place-home = Giriş klasörü
app-place-desktop = Masaüstü
app-place-documents = Belgeler
app-place-downloads = İndirilenler
app-place-pictures = Resimler
app-place-music = Müzik
app-place-videos = Videolar

## Adres çubuğu (ui/breadcrumb.rs)

app-crumb-address = Adres
app-crumb-hidden-parents = Gizlenen üst klasörler
app-crumb-edit-path = { $name } (yolu düzenle)
app-crumb-go-to = Git: { $name }
app-crumb-refresh = Yenile
app-crumb-refresh-tip = Yenile (F5)
app-crumb-subfolders = Alt klasörler
app-crumb-cant-read = Bu klasör okunamıyor
app-crumb-no-folders = Klasör yok

## Başlık çubuğu (ui/titlebar.rs)

app-titlebar-menu = Soyuz menüsü
app-titlebar-new-window = Yeni pencere
app-titlebar-settings = Ayarlar…
app-titlebar-shortcuts = Klavye kısayolları
app-titlebar-about = Soyuz hakkında
app-titlebar-quit = Çık
app-titlebar-minimize = Simge durumuna küçült
app-titlebar-maximize = Ekranı kapla
app-titlebar-restore = Önceki boyutuna getir
app-titlebar-close = Kapat

## Sekme şeridi (ui/tabstrip.rs)

app-tabs-list = Sekmeler
app-tabs-group = Sekme grubu
app-tabs-group-named = Sekme grubu: { $name }
app-tabs-expand-group = Grubu genişlet
app-tabs-collapse-group = Grubu daralt (diğer seçenekler için sağ tıklayın)
app-tabs-close-named = Sekmeyi kapat: { $title }
app-tabs-new = Yeni sekme
# $key bir klavye kısayoludur ("Ctrl+T").
app-tabs-new-tip = Yeni sekme ({ $key })
app-tabs-drop-new-window = Yeni pencerede açmak için bırakın
app-tabs-drop-other-window = O pencereye taşımak için bırakın
app-tabs-drop-keep-one = Bir pencerede en az bir sekme kalır
app-tabs-group-name = Grup adı
app-tabs-duplicate = Sekmeyi çoğalt
app-tabs-pin = Sekmeyi sabitle
app-tabs-unpin = Sekmenin sabitlemesini kaldır
app-tabs-close = Sekmeyi kapat
app-tabs-close-others = Diğer sekmeleri kapat
app-tabs-close-right = Sağdaki sekmeleri kapat
app-tabs-reopen-closed = Kapatılan sekmeyi yeniden aç

## Karşılama kartı (ui/welcome.rs)

app-welcome-title = Soyuz'a hoş geldiniz
app-welcome-dismiss = Kapat
app-welcome-intro = Klavye odaklı ve hiç takılmaz. Başlamak için birkaç kısayol:
app-welcome-tip-palette = Tüm komutlar, aranabilir
app-welcome-tip-jump = Ziyaret ettiğiniz herhangi bir klasöre atlayın
app-welcome-tip-search = Adlarda ve içeriklerde arayın
app-welcome-tip-dual = Yan yana iki klasör
app-welcome-tip-terminal = Sizi takip eden bir terminal
app-welcome-tip-preview = Seçili dosyayı önizleyin
app-welcome-got-it = Anladım
app-welcome-settings = Ayarlar

## Yol seçici (ui/path_picker.rs)

app-picker-parent = Üst klasör
app-picker-folder-path = Klasör yolu
app-picker-file-name = Dosya adı
# $name bir dosya adıdır.
app-picker-replace = “{ $name }” zaten var. Üzerine yazmak için Kaydet'e yeniden basın.
app-picker-open = Aç
app-picker-no-items = Eşleşen öğe yok
# Satırın ekran okuyucu adı; $kind "folder" ya da "file".
app-picker-row = { $kind ->
    [folder] { $name }, klasör
   *[other] { $name }, dosya
}

## Ortak bileşenler, bildirimler, uygulama (ui/widgets.rs, ui/toasts.rs, app.rs)

app-hidden-controls-tip = Ad, gizli yön/biçim karakterleri içeriyor
app-icon-size = Simge boyutu
app-toast-dismiss = Kapat
app-quit-when-finished = İşlemler bitince Soyuz kapanacak

## Linux (data)

app-device-mount = Bağla
app-device-mount-failed = { $name } bağlanamadı
app-device-unmount = Bağlantıyı kes
app-device-unmounted = { $name } bağlantısı kesildi
app-device-unmount-failed = { $name } bağlantısı kesilemedi
app-device-eject = Çıkar
app-device-ejected = { $name } güvenle çıkarılabilir
app-device-eject-failed = { $name } çıkarılamadı
app-device-not-authorized = İzin verilmedi. Bir polkit kimlik doğrulama aracısı çalışıyor mu?
app-device-busy = Aygıt kullanımda. Onu kullanan dosyaları ve programları kapatıp yeniden deneyin.
app-device-no-udisks = Bu sistemde UDisks2 kullanılamıyor.

## Linux (devices)
app-device-mount-ro = Salt okunur bağla
app-device-unlock = Kilidi aç…
app-device-lock = Kilitle
app-device-hide = Kenar çubuğunda gizle
app-device-copy-path = Aygıt yolunu kopyala
# $name: device name.
app-device-locked-done = { $name } kilitlendi
app-device-lock-failed = { $name } kilitlenemedi
app-device-auth-dismissed = Kimlik doğrulama istemi iptal edildi.
app-device-hibernated = Windows tam olarak kapatılmamış (hazırda bekletme veya Hızlı Başlangıç); birim yalnızca salt okunur bağlanabilir.
# $fs: filesystem name (APFS, ReFS…).
app-device-unsupported-fs = Linux, { $fs } birimlerini bağlayamaz.
app-device-wrong-passphrase = Parola veya kurtarma anahtarı yanlış.
app-device-hibernated-title = { $name } Windows tarafından kullanılıyor
app-device-hibernated-body = Windows hazırda bekletildi veya Hızlı Başlangıç ile kapatıldı; dosyaları orada hâlâ açık olabilir. Birime şimdi yazmak ona zarar verebilir. Birimi salt okunur bağlayabilir ya da normal kullanmak için Windows'u başlatıp tamamen kapatabilirsiniz (Denetim Masası › Güç Seçenekleri › Güç düğmelerinin yapacaklarını seçin bölümünden Hızlı Başlangıç'ı kapatın).
app-device-hidden = Aygıt kenar çubuğunda gizlendi
app-device-hidden-body = Yeniden göstermek için: Ayarlar › Görünümler ve düzen › Kenar çubuğu bölümleri.
# $size: e.g. "1.0 TB"; $fs: filesystem (NTFS, exFAT…).
app-device-volume-name = { $size } { $fs } birimi
app-device-volume-name-nosize = { $fs } birimi
app-device-removable = çıkarılabilir
app-device-internal = dahili disk
app-device-click-mount = Bağlamak için tıklayın
app-device-click-unlock = Kilidi açmak için tıklayın
app-device-unsupported-hint = Linux bu dosya sistemini bağlayamaz
# Tooltip of an unmounted device: $name, $device (/dev/sdb1), $fs, $size, $drive (model · internal disk), $hint.
app-device-candidate-tip = { $name }
    { $device } · { $fs } · { $size }
    { $drive }
    { $hint }
app-device-status-unmounted = Bağlı değil · { $size }
app-device-status-locked = Kilitli · { $size }
app-device-status-unsupported = Desteklenmiyor · { $size }
app-device-status-mounting = Bağlanıyor…
app-device-status-unlocking = Kilit açılıyor…
app-device-status-locking = Kilitleniyor…
app-device-status-unmounting = Bağlantı kesiliyor…

## a11y
app-a11y-resize-sidebar = Kenar çubuğunu yeniden boyutlandır
app-a11y-resize-activity = Etkinlik panelini yeniden boyutlandır
app-a11y-resize-shelf = Rafı yeniden boyutlandır
app-a11y-resize-preview = Önizleme bölmesini yeniden boyutlandır
app-a11y-resize-assistant = Asistan panelini yeniden boyutlandır
app-a11y-resize-panes = Bölmeleri yeniden boyutlandır
