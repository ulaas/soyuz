### Soyuz arayüz metinleri (tr-TR): dialogs
### Anahtarlar en-US/dialogs.ftl ile birebir aynı olmalı.

## Shared

dialogs-done = Bitti
dialogs-show = Göster
dialogs-hide = Gizle

## Select by pattern

dialogs-pattern-title = Desene göre seç
dialogs-pattern-hint = Joker desen (*.rs, IMG_??.jpg) veya düz metin
dialogs-pattern-add = Geçerli seçime ekle
dialogs-pattern-select = Seç
dialogs-pattern-selected = { $count } öğe seçildi

## Columns

dialogs-columns-title = Sütunlar

## About

dialogs-about-title = Soyuz hakkında
dialogs-about-version-line = Sürüm { $version }
dialogs-about-tagline = Klavye öncelikli, hiç takılmayan bir dosya gezgini.
dialogs-about-version = Sürüm
dialogs-about-build = Derleme
dialogs-about-platform = Platform
dialogs-about-renderer = Oluşturucu
dialogs-about-config = Yapılandırma
dialogs-about-memory = Bellek
dialogs-about-memory-line = küçük resimler { $thumbs } ({ $thumb_count }) · önizlemeler { $previews } ({ $preview_count })
dialogs-about-license = MIT VEYA Apache-2.0 lisansıyla dağıtılır. Yazı tipleri: Inter, JetBrains Mono (OFL). Simgeler: Phosphor (MIT).
dialogs-about-copy-info = Bilgileri kopyala

## Keyboard shortcuts

dialogs-keys-title = Klavye kısayolları
dialogs-keys-filter = Kısayolları filtrele

## Record shortcut

dialogs-record-title = Kısayol kaydet
dialogs-record-press = Bir tuş bileşimine basın…
dialogs-record-sequence-hint = En fazla üç tuş bileşimi bir dizi oluşturur (örn. G ardından G).
dialogs-record-unused = Başka hiçbir şey tarafından kullanılmıyor.
dialogs-record-user = { $action } ({ $context })
dialogs-record-conflict = Şu anda şuna atanmış: { $users }. Kaydederseniz buraya taşınır.
dialogs-record-keys-hint = Enter kaydeder · Esc iptal eder
dialogs-record-clear = Temizle

## Delete confirmation

dialogs-delete-trash-one = “{ $name }” Çöp Kutusu'na taşınsın mı?
dialogs-delete-trash-many = { $count } öğe Çöp Kutusu'na taşınsın mı?
dialogs-delete-permanent-one = “{ $name }” kalıcı olarak silinsin mi?
dialogs-delete-permanent-many = { $count } öğe kalıcı olarak silinsin mi?
dialogs-delete-wipe-one = “{ $name }” güvenli biçimde silinsin mi?
dialogs-delete-wipe-many = { $count } öğe güvenli biçimde silinsin mi?
dialogs-delete-empty-title = Çöp Kutusu boşaltılsın mı?
dialogs-delete-trash-body = Öğeleri daha sonra Çöp Kutusu'ndan geri yükleyebilirsiniz.
dialogs-delete-trash-confirm = Çöp Kutusu'na taşı
dialogs-delete-permanent-body = Kalıcı olarak silinen öğeler geri yüklenemez. Bu işlem geri alınamaz.
dialogs-delete-wipe-body = Veriler silinmeden önce üzerine yazılır. SSD'lerde ve yazarken kopyalayan (copy-on-write) dosya sistemlerinde eski blokların tamamen yok olduğu garanti edilemez.
dialogs-delete-wipe-confirm = Güvenli sil
dialogs-delete-purge-body = Bu öğeler Çöp Kutusu'ndan kalıcı olarak kaldırılacak.
dialogs-delete-empty-body = Çöp Kutusu'ndaki { $count } öğenin tümü kalıcı olarak silinecek.
dialogs-delete-empty-confirm = Çöp Kutusu'nu boşalt
dialogs-delete-and-more = ve { $count } öğe daha

## Quit while jobs run

dialogs-quit-title = { $count ->
    [one] Bir dosya işlemi hâlâ sürüyor
   *[other] { $count } dosya işlemi hâlâ sürüyor
}
dialogs-quit-body = Şimdi çıkarsanız işlemler iptal edilir ve yarım kopyalanmış dosyalar kalabilir. Soyuz'u açık tutabilir ya da işlemler bitince kendiliğinden kapanmasını sağlayabilirsiniz.
dialogs-quit-cancel-jobs = İşleri iptal et ve çık
dialogs-quit-when-finished = Bitince çık
dialogs-quit-keep-open = Açık tut

## Job prompts (conflict, error, space, trash, read-only, password)

dialogs-prompt-waiting = 1 / { $total } bekliyor
dialogs-prompt-cancel-op = İşlemi iptal et
dialogs-conflict-title-file = Değiştirilsin mi, atlansın mı?
dialogs-conflict-title-dir = Klasörler birleştirilsin mi?
dialogs-conflict-title-kind = Bu ad zaten kullanılıyor
dialogs-conflict-title-same = Kaynak ve hedef aynı
dialogs-conflict-title-into-self = Klasör kendi içine taşınamaz
dialogs-conflict-title-case = Ad yalnızca büyük/küçük harf farklı
dialogs-prompt-title-archive-limit = Arşiv güvenlik sınırını aşıyor
dialogs-prompt-title-error = Bir sorun oluştu
dialogs-prompt-title-space = Yeterli alan yok
dialogs-prompt-title-trash = Çöp Kutusu'na taşınamıyor
dialogs-prompt-title-read-only = Salt okunur öğe
dialogs-prompt-title-password = Parola gerekli
dialogs-conflict-file-exists = Hedefte “{ $name }” adlı bir dosya zaten var.
dialogs-conflict-dir-exists = Hedefte “{ $name }” adlı bir klasör zaten var.
dialogs-conflict-kind-differs = “{ $name }” adlı öğe farklı türde bir öğe olarak zaten var.
dialogs-conflict-same = Kaynak ve hedef aynı öğe.
dialogs-conflict-into-self = “{ $name }” adlı klasör kendi içine taşınamaz.
dialogs-conflict-case = “{ $name }” adı yalnızca büyük/küçük harf bakımından farklı.
dialogs-conflict-source = Kaynak
dialogs-conflict-destination = Hedef
dialogs-conflict-folder = Klasör
dialogs-conflict-larger = Daha büyük
dialogs-conflict-newer = Daha yeni
dialogs-conflict-new-name = Yeni ad
dialogs-conflict-apply-all = Bu işlemdeki kalan tüm çakışmalar için bunu uygula
dialogs-conflict-back = Geri
dialogs-conflict-keep-both = İkisini de tut
dialogs-conflict-merge = Birleştir
dialogs-conflict-replace = Değiştir
dialogs-conflict-more = Diğer…
dialogs-conflict-replace-folder = Klasörü değiştir
dialogs-conflict-replace-newer = Daha yeniyse değiştir
dialogs-conflict-replace-larger = Daha büyükse değiştir
dialogs-conflict-replace-different = Boyutu farklıysa değiştir
dialogs-conflict-rename = Yeniden adlandır…
dialogs-prompt-archive-limit = “{ $name }” açıldığında güvenlik sınırını aşıyor.
dialogs-prompt-archive-limit-hint = Diski doldurmak için hazırlanmış arşivler (sıkıştırma bombaları) böyle görünür. Sınırlar Ayarlar › Dosya işlemleri bölümündedir.
dialogs-prompt-abort = Durdur
dialogs-prompt-abort-extract-tip = Ayıklamayı durdur
dialogs-prompt-abort-tip = Tüm işlemi durdur
dialogs-prompt-continue = Devam et
dialogs-prompt-continue-tip = Bu ayıklama için sınırı yok say
dialogs-prompt-error = “{ $name }” işlenemedi.
dialogs-prompt-skip-all = Tümünü atla
dialogs-prompt-skip-all-tip = Bu hatayı ve sonraki tüm hataları atla
dialogs-prompt-space = “{ $name }” için { $needed } gerekiyor, ancak yalnızca { $free } boş alan var.
dialogs-prompt-space-hint = Disk dolduğunda bazı öğeler başarısız olacak.
dialogs-prompt-continue-anyway = Yine de devam et
dialogs-prompt-trash = “{ $name }” Çöp Kutusu'na taşınamıyor.
dialogs-prompt-trash-permanent = Bunun yerine kalıcı olarak silinsin mi? Bu işlem geri alınamaz.
dialogs-prompt-apply-all-items = Bu işlemdeki tüm öğeler için bunu uygula
dialogs-prompt-delete-permanently = Kalıcı olarak sil
dialogs-prompt-read-only = “{ $name }” salt okunur.
dialogs-prompt-read-only-ask = Yine de değiştirilsin mi?
dialogs-prompt-apply-all-read-only = Bu işlemdeki tüm salt okunur öğeler için bunu uygula
dialogs-prompt-change-anyway = Yine de değiştir
dialogs-prompt-encrypted = “{ $name }” şifreli.
dialogs-prompt-wrong-password = Parola işe yaramadı. Yeniden deneyin.
dialogs-prompt-password = Parola
dialogs-prompt-unlock = Kilidi aç

## Properties

dialogs-menu-batch-rename = Toplu yeniden adlandır…
dialogs-menu-attributes = Öznitelikler ve zaman damgaları…
dialogs-menu-checksum = Sağlama toplamı
dialogs-menu-compare = Şununla karşılaştır…
dialogs-props-owner-changed = Sahiplik değiştirildi
dialogs-props-owner-failed = Sahiplik değiştirilemedi
dialogs-props-unavailable = Özellikler yalnızca diskteki dosya ve klasörler için kullanılabilir
dialogs-props-select-first = Önce bir veya daha fazla öğe seçin
dialogs-props-tab-general = Genel
dialogs-props-tab-attributes = Öznitelikler
dialogs-props-tab-security = Güvenlik
dialogs-props-tab-checksums = Sağlama toplamları
dialogs-props-tab-details = Ayrıntılar
dialogs-props-tab-versions = Önceki sürümler
dialogs-props-title-one = { $name } özellikleri
dialogs-props-created = Oluşturulma
dialogs-props-modified = Değiştirilme
dialogs-props-accessed = Erişim
dialogs-props-attributes = Öznitelikler
dialogs-props-timestamps = Zaman damgaları
dialogs-props-exact-size = { $size } ({ $bytes } bayt)
dialogs-props-kind-size = { $kind } · { $size }
dialogs-props-selected = { $count } öğe seçili
dialogs-props-all-kind = Tümü: { $kind }
dialogs-props-mixed-types = Karışık türler
dialogs-props-summary = { $counts } · { $kinds }
dialogs-props-type = Tür
dialogs-props-location = Konum
dialogs-props-size = Boyut
dialogs-props-size-on-disk = Diskteki boyut
dialogs-props-contains = { $files } dosya, { $dirs } klasör
dialogs-props-contains-errors = { $files } dosya, { $dirs } klasör · { $errors } okunamadı
dialogs-props-contains-label = İçerik
dialogs-props-total = Toplam
dialogs-props-link-target = Bağlantı hedefi
dialogs-props-dates = Tarihler
dialogs-props-touch-tip = Değiştirilme ve erişim zamanını şimdi yap
dialogs-props-multiple = Birden çok değer
dialogs-props-unknown = Bilinmiyor
dialogs-props-changed = değişti
dialogs-props-revert = Geri döndür
dialogs-props-no-created-linux = Linux dosya sistemleri oluşturulma zamanının değiştirilmesine izin vermez.
dialogs-props-reading = Meta veriler okunuyor…
dialogs-props-copy-path = Tam yolu kopyala
dialogs-props-invalid-name = Geçersiz ad: { $error }
dialogs-props-unsaved = Kaydedilmemiş değişiklikler
dialogs-props-undo-note = Değişiklikler arka planda bir iş olarak uygulanır ve geri alınabilir (Ctrl+Z).

## Attributes

dialogs-attr-readonly = Salt okunur
dialogs-attr-readonly-hint = Programlar dosyayı okuyabilir ama değiştiremez
dialogs-attr-hidden = Gizli
dialogs-attr-hidden-hint = Gizli dosyalar gösterilmedikçe görünmez
dialogs-attr-hidden-finder-hint = Finder'da gizli
dialogs-attr-system = Sistem
dialogs-attr-system-hint = İşletim sistemi tarafından kullanılır
dialogs-attr-archive = Arşiv
dialogs-attr-archive-hint = Son yedeklemeden bu yana değişti
dialogs-attr-not-indexed = İçeriği dizinlenmesin
dialogs-attr-not-indexed-hint = Arama dizininin dışında tutulur
dialogs-attr-temporary = Geçici
dialogs-attr-temporary-hint = Mümkünse bellekte tutulur
dialogs-attr-offline = Çevrimdışı
dialogs-attr-offline-hint = Veriler hemen kullanılamaz
dialogs-attr-compressed = Sıkıştırılmış (NTFS)
dialogs-attr-compressed-hint = Dosya sistemi tarafından sıkıştırılmış olarak saklanır
dialogs-attr-locked = Kilitli (değiştirilemez)
dialogs-attr-locked-hint = Değiştirilemez, yeniden adlandırılamaz veya silinemez
dialogs-attr-immutable = Değiştirilemez
dialogs-attr-immutable-hint = Değiştirilemez, yeniden adlandırılamaz veya silinemez (root gerekir)
dialogs-attr-encrypted = Şifreli
dialogs-attr-sparse = Seyrek
dialogs-attr-reparse = Yeniden ayrıştırma noktası
dialogs-attr-cloud = Bulut yer tutucusu
dialogs-attrs-title-one = { $name } öznitelikleri
dialogs-attrs-title-many = { $count } öğenin öznitelikleri
dialogs-attrs-collecting = Klasör içerikleri toplanıyor…
dialogs-attrs-all-now = Tümünü şimdi yap
dialogs-attrs-recursive = Seçili klasörlerin içindeki her şeye de uygula

## Unix permissions

dialogs-mode-permissions = İzinler
dialogs-mode-read = Okuma
dialogs-mode-write = Yazma
dialogs-mode-execute = Çalıştırma
dialogs-mode-owner = Sahip
dialogs-mode-group = Grup
dialogs-mode-others = Diğerleri
dialogs-mode-setuid = Kullanıcı kimliği ata
dialogs-mode-setgid = Grup kimliği ata
dialogs-mode-sticky = Yapışkan
dialogs-mode-octal = Sekizlik
dialogs-mode-mixed = karışık

## Security

dialogs-sec-full-control = Tam denetim
dialogs-sec-special = Özel
dialogs-sec-unknown-user = bilinmeyen kullanıcı “{ $name }”
dialogs-sec-unknown-group = bilinmeyen grup “{ $name }”
dialogs-sec-ownership = Sahiplik
dialogs-sec-owner-admin = Sahipliği değiştirmek genellikle yönetici hakları gerektirir.
dialogs-sec-reading = İzinler okunuyor…
dialogs-sec-unavailable = İzinler kullanılamıyor
dialogs-sec-your-access = Erişiminiz
dialogs-sec-entries = Erişim girdileri
dialogs-sec-no-entries = Girdi yok (herkesin tam erişimi var)
dialogs-sec-inherited = devralındı
dialogs-sec-read-only-view = Salt okunur görünüm. Gelişmiş izin düzenleme kullanılamıyor.

## Checksums

dialogs-hash-no-thread = çalışan iş parçacığı başlatılamadı
dialogs-hash-compute-all = Tümünü hesapla
dialogs-hash-not-computed = Hesaplanmadı
dialogs-hash-compute = Hesapla
dialogs-hash-percent = %{ $percent }
dialogs-hash-compare = Karşılaştır
dialogs-hash-compare-hint = Doğrulamak için bir sağlama toplamı yapıştırın (MD5, SHA-1, SHA-256, BLAKE3 veya CRC-32)
dialogs-hash-not-hex = Onaltılık bir sağlama toplamı değil
dialogs-hash-bad-length = Tanınmayan sağlama toplamı uzunluğu
dialogs-hash-computing = Hesaplanıyor…
dialogs-hash-match = Eşleşiyor · { $algo }
dialogs-hash-mismatch = Eşleşmiyor · dosya farklı

## Details

dialogs-details-full-path = Tam yol
dialogs-details-resolves-to = Çözümlendiği yer
dialogs-details-target-missing = Hedef yok
dialogs-details-hard-links = Sabit bağlantılar
dialogs-details-file-id = Dosya kimliği
dialogs-details-inode = Inode
dialogs-details-volume-serial = Birim seri numarası
dialogs-details-device = Aygıt
dialogs-details-reparse-tag = Yeniden ayrıştırma etiketi
dialogs-details-mode = Mod
dialogs-details-file-system = Dosya sistemi
dialogs-details-shortcut = Kısayol
dialogs-details-target = Hedef
dialogs-details-dimensions = Boyutlar
dialogs-details-megapixels = Megapiksel
dialogs-details-image = Görüntü
dialogs-details-media = Medya
dialogs-exif-camera = Kamera
dialogs-exif-lens = Objektif
dialogs-exif-taken = Çekim tarihi
dialogs-exif-exposure = Pozlama
dialogs-exif-aperture = Diyafram
dialogs-exif-focal = Odak uzaklığı
dialogs-exif-orientation = Yön
dialogs-exif-software = Yazılım
dialogs-exif-orient-2 = Aynalanmış
dialogs-exif-orient-3 = 180° döndürülmüş
dialogs-exif-orient-4 = Dikey aynalanmış
dialogs-exif-orient-5 = Aynalanmış, saat yönünün tersine 90° döndürülmüş
dialogs-exif-orient-6 = Saat yönünde 90° döndürülmüş
dialogs-exif-orient-7 = Aynalanmış, saat yönünde 90° döndürülmüş
dialogs-exif-orient-8 = Saat yönünün tersine 90° döndürülmüş

## Date picker

dialogs-date-pick = Tarih seç
dialogs-date-prev-month = Önceki ay
dialogs-date-next-month = Sonraki ay
dialogs-date-now = Şimdi

## Create link

dialogs-link-title = Bağlantı oluştur
dialogs-link-symlink = Sembolik bağlantı
dialogs-link-hardlink = Sabit bağlantı
dialogs-link-junction = Bağlantı noktası
dialogs-link-symlink-hint = Bir dosyayı veya klasörü yoluyla gösterir ve birimler arasında çalışabilir. Windows'ta Geliştirici Modu veya yönetici yetkisi gerekir.
dialogs-link-hardlink-hint = Aynı dosya verisi için ikinci bir ad. Yalnızca dosyalar, aynı birimde.
dialogs-link-junction-hint = Özel yetki gerektirmeyen bir Windows klasör bağlantısı. Yalnızca klasörler, yerel birimler.
dialogs-link-need-folder = Bağlantılar yalnızca diskteki bir klasörde oluşturulabilir
dialogs-link-parent = Üst klasör
dialogs-link-home = Giriş klasörü
dialogs-link-use-folder = Bu klasörü kullan
dialogs-link-empty-folder = Boş klasör
dialogs-link-picker-hint = Seçmek için tıklayın, bir klasörü açmak için çift tıklayın.
dialogs-link-target = Hedef
dialogs-link-target-hint = Bağlanılacak dosyanın veya klasörün yolu
dialogs-link-browse = Gözat
dialogs-link-checking = Hedef denetleniyor…
dialogs-link-missing = Hedef yok
dialogs-link-file = Dosya
dialogs-link-relative = Göreli yol sakla (ikisi birlikte taşındığında bağlantı çalışmaya devam eder)
dialogs-link-name = Bağlantı adı
dialogs-link-name-hint = Yeni bağlantının adı
dialogs-link-created-in = Oluşturulacağı yer: { $dir }
dialogs-link-in = Konum: { $dir }
dialogs-link-choose-target = Bir hedef seçin.
dialogs-link-need-target = Bir hedef yolu girin.
dialogs-link-exists = “{ $name }” burada zaten var.
dialogs-link-hard-files-only = Sabit bağlantılar yalnızca dosyaları gösterebilir.
dialogs-link-hard-needs-file = Sabit bağlantı için var olan bir dosya gerekir.
dialogs-link-hard-same-volume = Sabit bağlantılar hedefle aynı birimde olmalıdır.
dialogs-link-junction-folders-only = Bağlantı noktaları yalnızca klasörleri gösterebilir.
dialogs-link-junction-needs-folder = Bağlantı noktası için var olan bir klasör gerekir.
dialogs-link-create = Oluştur

## Copy to / Move to

dialogs-transfer-move-title = Klasöre taşı
dialogs-transfer-copy-title = Klasöre kopyala
dialogs-transfer-move-one = “{ $name }” şuraya taşınacak:
dialogs-transfer-copy-one = “{ $name }” şuraya kopyalanacak:
dialogs-transfer-move-many = { $count } öğe şuraya taşınacak:
dialogs-transfer-copy-many = { $count } öğe şuraya kopyalanacak:
dialogs-transfer-dest = Hedef klasör
dialogs-transfer-dest-hint = Klasör yolu

## Open with

dialogs-openwith-title = Birlikte aç: “{ $name }”
dialogs-openwith-looking = Uygulamalar aranıyor…
dialogs-openwith-none = Bu dosya türü için kayıtlı uygulama yok
dialogs-openwith-default-handler = { $app } (varsayılan işleyiciyle açılır)
dialogs-openwith-other = Ya da başka bir program (tam yol):
dialogs-openwith-open = Aç
dialogs-openwith-not-program = { $path } bir program dosyası değil
dialogs-openwith-choose = Bir uygulama seçin veya bir program yolu yazın

## MCP approval

dialogs-mcp-title-destructive = { $client } dosyaları silmek veya üzerine yazmak istiyor
dialogs-mcp-title = { $client } dosyaları değiştirmek istiyor
dialogs-mcp-title-unknown = Bir yapay zekâ istemcisi dosyaları değiştirmek istiyor
dialogs-mcp-note = Onaylanan planlar duraklatabileceğiniz, iptal edebileceğiniz ve geri alabileceğiniz normal arka plan işleri olarak çalışır.

## Crash recovery

dialogs-recovery-title = { $count ->
    [one] Soyuz bir işlem sırasında kapandı
   *[other] Soyuz { $count } işlem sırasında kapandı
}
dialogs-recovery-done = %{ $percent } tamamlandı
dialogs-recovery-progress-unknown = ilerleme bilinmiyor
dialogs-recovery-job-line = Başlangıç: { $started } · { $done }
dialogs-recovery-job-line-undo = Başlangıç: { $started } · { $done } · tamamlanan { $count } bölüm geri alınabilir
dialogs-recovery-body = Bu işlemler Soyuz durduğunda hâlâ sürüyordu. Onları sürdürebilir, yaptıklarını geri alabilir (mümkün olduğunda Çöp Kutusu'na taşınır) ya da her şeyi olduğu gibi bırakabilirsiniz.
dialogs-recovery-scanning = Artık geçici dosyalar aranıyor…
dialogs-recovery-no-leftovers = Artık geçici dosya bulunamadı.
dialogs-recovery-clean = { $count } artık geçici dosyayı Çöp Kutusu'na taşı
dialogs-recovery-later = Sonra karar ver
dialogs-recovery-later-tip = Soyuz bir dahaki açılışında yeniden sor
dialogs-recovery-resume = Sürdür
dialogs-recovery-resume-tip = Kalanı tamamla
dialogs-recovery-rollback = Geri al
dialogs-recovery-rollback-tip = Tamamlanan bölümleri geri al
dialogs-recovery-rollback-none = Henüz hiçbir şey tamamlanmadı
dialogs-recovery-discard = Yok say
dialogs-recovery-discard-tip = Her şeyi olduğu gibi bırak
dialogs-crash-title = { $count ->
    [one] Soyuz beklenmedik şekilde kapandı
   *[other] Soyuz { $count } çökme raporu kaydetti
}
dialogs-crash-truncated = … (devamını görmek için dosyayı açın)
dialogs-crash-body = Bu bilgisayara bir çökme raporu kaydedildi. Raporda sürüm, işletim sistemi, sorunun oluştuğu yer ve son günlük satırları bulunur. Hiçbir yere gönderilmez.
dialogs-crash-view = Görüntüle
dialogs-crash-delete = Raporları sil
dialogs-crash-open-folder = Klasörü aç
dialogs-crash-open-folder-tip = Çökme raporları klasörünü yeni bir sekmede göster
dialogs-crash-safe-mode = Güvenli modda yeniden başlat
dialogs-crash-safe-mode-tip = Varsayılan ayarlar; oturum geri yükleme, izleyiciler, küçük resimler ve Git olmadan

## Batch rename

dialogs-rename-unavailable = Toplu yeniden adlandırma yalnızca diskteki dosya ve klasörlerde çalışır
dialogs-rename-nothing = Yeniden adlandırılacak bir şey yok
dialogs-rename-title = { $count } öğeyi yeniden adlandır
dialogs-rename-rules = Kurallar
dialogs-rename-editor = Metin düzenleyici
dialogs-rename-presets = Hazır ayarlar
dialogs-rename-builtin = Yerleşik
dialogs-rename-saved = Kaydedilenler
dialogs-rename-preset-name = Hazır ayar adı
dialogs-rename-delete-preset = Kaydedilen hazır ayarı sil
dialogs-rename-edit = Düzenlenecek kısım
dialogs-rename-remove-rule = Kuralı kaldır
dialogs-rename-move-down = Aşağı taşı
dialogs-rename-move-up = Yukarı taşı
dialogs-rename-add-rule = + Kural ekle
dialogs-rename-insert-prefix = Ön ek ekle
dialogs-rename-insert-suffix = Son ek ekle
dialogs-rename-counter = Sayaç  { $token }
dialogs-rename-tokens-hint = Belirteçler: { $tokens }. Düzenli ifade değiştirmelerinde { $group } veya { $named } kullanılabilir.
dialogs-rename-one-per-line = Her satıra bir ad
dialogs-rename-editor-hint = Dilediğiniz gibi düzenleyin; N. satır N. öğeyi yeniden adlandırır. Elektronik tablodan yapıştırmak da olur.
dialogs-rename-reset-names = Geçerli adlara sıfırla
dialogs-rename-current-name = Geçerli ad
dialogs-rename-duplicate = Başka bir öğe de aynı adı alıyor
dialogs-rename-exists = Bu adda bir dosya zaten var
dialogs-rename-position = Konum
dialogs-rename-pos-start = Başa
dialogs-rename-pos-end = Sona
dialogs-rename-pos-at = Konuma
dialogs-rename-pos-index-tip = Karakter sırası; negatif değerler sondan sayılır
dialogs-rename-find = Bul
dialogs-rename-find-hint = aranacak metin
dialogs-rename-find-regex-hint = örn. IMG_(\d+)
dialogs-rename-replace = Değiştir
dialogs-rename-replace-hint = yeni metin (belirteçler kullanılabilir)
dialogs-rename-replace-regex-hint = örn. Fotoğraf $1
dialogs-rename-regex = Düzenli ifade
dialogs-rename-match-case = Büyük/küçük harf duyarlı
dialogs-rename-first-only = Yalnızca ilki
dialogs-rename-text = Metin
dialogs-rename-text-hint = örn. { $token } veya Taslak
dialogs-rename-remove-count = Kaldır:
dialogs-rename-remove-start = karakter, başlangıç konumu
dialogs-rename-from-end = sondan
dialogs-rename-separator = Ayırıcı
dialogs-rename-separator-hint = örn. _ veya boşluk
dialogs-rename-pattern = Desen
dialogs-rename-collapse = Art arda gelen boşlukları da teke indir
dialogs-rename-counter-start = Başlangıç
dialogs-rename-counter-step = Artış
dialogs-rename-counter-digits = Basamak
dialogs-rename-per-folder = Klasör başına
dialogs-rename-to-rename = { $count } öğe yeniden adlandırılacak
dialogs-rename-problems = { $count } sorun
dialogs-rename-cycles = { $count } değiş tokuş döngüsü · geçici adlar kullanılıyor
dialogs-rename-go = { $count } öğeyi yeniden adlandır
dialogs-rename-target-stem = Yalnızca ad
dialogs-rename-target-ext = Yalnızca uzantı
dialogs-rename-target-whole = Ad + uzantı
dialogs-rename-case-lower = küçük harf
dialogs-rename-case-upper = BÜYÜK HARF
dialogs-rename-case-title = Baş Harfler Büyük
dialogs-rename-case-sentence = Cümle düzeni
dialogs-rename-case-invert = hARFLERİ tERS çEVİR
dialogs-rename-rule-regex = Değiştir (düzenli ifade)
dialogs-rename-rule-replace = Değiştir
dialogs-rename-rule-case = Harf durumunu değiştir
dialogs-rename-rule-insert = Metin ekle
dialogs-rename-rule-remove = Karakter kaldır
dialogs-rename-rule-number = Numaralandırma
dialogs-rename-rule-template = Şablon
dialogs-rename-rule-trim = Boşlukları kırp
dialogs-rename-preset-photos = Fotoğraflar çekim tarihine göre
dialogs-rename-preset-folder-number = Klasör adı + numara
dialogs-rename-preset-underscores = Boşlukları alt çizgiye çevir
dialogs-rename-preset-lower-ext = Uzantıları küçük harfe çevir
dialogs-rename-preset-date-prefix = Değiştirilme tarihini başa ekle
dialogs-rename-err-date-format = geçersiz tarih biçimi “{ $format }”
dialogs-rename-err-unmatched = eşi olmayan “{"}"}”
dialogs-rename-err-unclosed = kapatılmamış “{"{"}”
dialogs-rename-err-padding = {"{"}{ $token }{"}"} içinde hatalı basamak sayısı
dialogs-rename-err-token = bilinmeyen belirteç {"{"}{ $token }{"}"}
dialogs-rename-err-rule = kural { $rule }: { $error }
dialogs-rename-err-no-target = hedef yok
dialogs-rename-err-lines = { $count } öğe için { $lines } satır var: her satırda tam olarak bir ad bırakın
