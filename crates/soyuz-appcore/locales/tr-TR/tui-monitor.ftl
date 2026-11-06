### Soyuz arayüz metinleri (tr-TR): tui-monitor
### Terminal arayüzü: sistem izleyici sayfaları.
### Anahtarlar en-US/tui-monitor.ftl ile aynı kalmalıdır.

## Üst şerit

tui-monitor-tab-overview = Özet
tui-monitor-tab-processes = Süreç
tui-monitor-tab-cpu = CPU
tui-monitor-tab-memory = Bell
tui-monitor-tab-disks = Disk
tui-monitor-tab-network = Ağ
tui-monitor-tab-gpu = GPU
tui-monitor-tab-sensors = Sens
tui-monitor-tab-connections = Bağl
tui-monitor-tab-services = Hizm
tui-monitor-tab-startup = Baş
tui-monitor-tab-history = Geçm
tui-monitor-tab-snapshots = Anlık
tui-monitor-status-hint = ? tuşlar · Tab sayfalar · Ctrl+Q çıkış
tui-monitor-paused = DURAKLATILDI
tui-monitor-interval = { $secs } sn'de bir
tui-monitor-page-of = { $page }, sayfa { $index } / { $total }

## Ortak sözcükler

tui-monitor-loading = Okunuyor…
tui-monitor-no-matches = Filtreyle eşleşen bir şey yok
tui-monitor-match-count = { $count } / { $total }
tui-monitor-row-of = satır { $index } / { $total }
tui-monitor-list-count = { $title }: { $count }
tui-monitor-list-empty = Bu sistem burada bir şey bildirmiyor
tui-monitor-total = Toplam
tui-monitor-system = Sistem
tui-monitor-read = Okuma
tui-monitor-write = Yazma
tui-monitor-download = İndirme
tui-monitor-upload = Yükleme
tui-monitor-swap = Takas
tui-monitor-temperature = Sıcaklık
tui-monitor-processes = Süreçler
tui-monitor-top-cpu = En çok CPU
tui-monitor-top-memory = En çok bellek

## Özet

tui-monitor-cores = { $cores } CPU
tui-monitor-cores-threads = { $cores } çekirdek, { $threads } iş parçacığı
tui-monitor-ram = { $size } RAM
tui-monitor-uptime = açık kalma { $time }
tui-monitor-load = yük { $one } { $five } { $fifteen }
tui-monitor-user-system = kullanıcı { $user }, sistem { $system }
tui-monitor-read-write = okuma { $read }, yazma { $write }
tui-monitor-down-up = iniş { $down }, çıkış { $up }
tui-monitor-threads = { $count } iş parçacığı

## CPU, bellek ve aygıtlar

tui-monitor-iowait = bekleme { $value }
tui-monitor-ctx-switches = { $count } geçiş/sn
tui-monitor-logical-cpus = Mantıksal CPU'lar
tui-monitor-cpu-n = CPU { $n }
tui-monitor-memory-used = Bellek: { $total } içinde { $used } kullanımda ({ $percent })
tui-monitor-used = Kullanılan
tui-monitor-cached = Önbellek
tui-monitor-buffers = Tampon
tui-monitor-free = Boş
tui-monitor-available = Kullanılabilir
tui-monitor-shared = Paylaşılan
tui-monitor-kernel = Çekirdek
tui-monitor-compressed = Sıkıştırılmış
tui-monitor-commit = Ayrılan
tui-monitor-paging = Sayfalama: giriş { $page_in }, çıkış { $page_out }
tui-monitor-busy = Meşgul
tui-monitor-queue = Kuyruk
tui-monitor-disk-ssd = SSD
tui-monitor-disk-hdd = HDD
tui-monitor-disk-foot = { $read } okuma/sn, { $write } yazma/sn · { $read_total } okundu, { $write_total } yazıldı
tui-monitor-net-up = bağlı
tui-monitor-net-down = kapalı
tui-monitor-net-foot = { $down } alındı, { $up } gönderildi · { $errors } hata
tui-monitor-load-label = Yük
tui-monitor-gpu-memory = Bellek
tui-monitor-fan = Fan
tui-monitor-power = Güç
tui-monitor-voltage = Gerilim
tui-monitor-current = Akım
tui-monitor-battery-charging = şarj oluyor
tui-monitor-battery-plugged = prize takılı
tui-monitor-battery-discharging = pilde
tui-monitor-battery-health = sağlık { $value }
tui-monitor-sensors-other = Diğer
tui-monitor-no-disks = Bu sistem disk bildirmiyor
tui-monitor-no-networks = Gösterilecek ağ arabirimi yok
tui-monitor-no-gpus = Bu sistem grafik işlemcisi bildirmiyor
tui-monitor-no-sensors = Bu sistem algılayıcı bildirmiyor

## Süreçler

tui-monitor-scale-machine = %100 = tüm makine
tui-monitor-scale-core = %100 = bir CPU
tui-monitor-proc-count = { $count } süreç, { $running } çalışıyor
tui-monitor-marked = { $count } işaretli
tui-monitor-columns-hidden = { $count } sütun sığmıyor
tui-monitor-unknown-user = Bilinmeyen kullanıcı
tui-monitor-filter-regex = regex
tui-monitor-announce-proc = { $name }, süreç { $pid }, CPU { $cpu }, bellek { $memory }, { $state }, { $at }
tui-monitor-announce-group = { $name }, { $count } öğeli grup, { $at }
tui-monitor-details-general = Genel
tui-monitor-details-threads = İş parçacıkları
tui-monitor-details-modules = Modüller
tui-monitor-details-files = Dosyalar
tui-monitor-details-environment = Ortam
tui-monitor-details-sockets = Soketler
tui-monitor-details-none = Ayrıntılarını görmek için bir süreç seçin
tui-monitor-details-group = Bu satır bir grup: süreçlerinden birini seçin
tui-monitor-details-empty = Gösterilecek bir şey yok

## Süreç menüsü

tui-monitor-end-tree = Süreç ağacını sonlandır
tui-monitor-menu-priority = Öncelik
tui-monitor-menu-affinity = İşlemciler…
tui-monitor-menu-open-location = Dosya konumunu aç
tui-monitor-menu-copy = Kopyala
tui-monitor-menu-mark = İşaretle
tui-monitor-menu-details = Ayrıntılar paneli
tui-monitor-menu-columns = Sütunlar…
tui-monitor-menu-export = CSV olarak dışa aktar
tui-monitor-menu-kernel = Çekirdek iş parçacıklarını göster veya gizle
tui-monitor-menu-other-users = Diğer kullanıcıları göster veya gizle
tui-monitor-menu-heat = Isı renklerini ve çubukları aç veya kapat
tui-monitor-menu-scale = CPU ölçeğini değiştir

## İletişim kutuları

tui-monitor-columns-title = Süreç tablosu sütunları
tui-monitor-columns-hints = Boşluk göster veya gizle · Shift+Yukarı/Aşağı taşı · r sıfırla · Enter bitti
tui-monitor-affinity-title = { $name } işlemcileri
tui-monitor-affinity-hints = Boşluk işaretle · Enter uygula · Esc iptal
tui-monitor-snapshot-new = Yeni anlık görüntü
tui-monitor-snapshot-rename = Anlık görüntüyü yeniden adlandır
tui-monitor-snapshot-name = Ad
tui-monitor-snapshot-hints = Enter kaydet · Esc iptal

## Listeler

tui-monitor-col-proto = Protokol
tui-monitor-col-local = Yerel
tui-monitor-col-remote = Uzak
tui-monitor-col-state = Durum
tui-monitor-col-process = Süreç
tui-monitor-col-service = Hizmet
tui-monitor-col-start = Başlangıç
tui-monitor-col-unit = Ad
tui-monitor-col-name = Ad
tui-monitor-col-enabled = Etkin
tui-monitor-col-source = Kaynak
tui-monitor-col-command = Komut
tui-monitor-col-threads = İş parçacığı
tui-monitor-col-handles = Tutamaç

## Geçmiş

tui-monitor-range-minutes = { $count } dk
tui-monitor-range-hours = { $count } sa
tui-monitor-range-days = { $count } g
tui-monitor-history-live = canlı
tui-monitor-history-range = Son { $range } · { $when }

## Anlık görüntüler ve karşılaştırma

tui-monitor-compare-sides = A: { $a } · B: { $b }
tui-monitor-side-none = seçilmedi
tui-monitor-side-now = şimdi
tui-monitor-no-snapshots = Henüz anlık görüntü yok: almak için n'ye basın
tui-monitor-col-snapshot = Anlık görüntü
tui-monitor-col-taken = Alındığı zaman
tui-monitor-auto = otomatik
tui-monitor-comparing = Karşılaştırılıyor…
tui-monitor-compare-summary = +{ $added } -{ $removed } ~{ $changed } · { $by } · { $same }
tui-monitor-match-process = sürece göre
tui-monitor-match-program = programa göre
tui-monitor-same-shown = değişmeyenler gösteriliyor
tui-monitor-same-hidden = değişmeyenler gizli
tui-monitor-change-added = eklendi
tui-monitor-change-removed = kaldırıldı
tui-monitor-change-changed = değişti
tui-monitor-change-same = değişmedi
tui-monitor-export-json = JSON olarak dışa aktar
tui-monitor-export-csv = Süreçleri CSV olarak dışa aktar

## Sayfa başlık satırlarındaki tuş ipuçları

tui-monitor-hint-processes = / filtre · V görünüm · c sütunlar · ? tuşlar
tui-monitor-hint-cpu = v görünüm · ? tuşlar
tui-monitor-hint-network = v sanal · ? tuşlar
tui-monitor-hint-connections = / filtre · < > sırala · Enter süreç
tui-monitor-hint-services = x başlat · X durdur · R yeniden başlat · / filtre
tui-monitor-hint-lists = / filtre · < > sırala · ? tuşlar
tui-monitor-hint-history = r kaydet · Boşluk duraklat · + - aralık · oklar kaydır · L canlı · , . imleç
tui-monitor-hint-snapshots = n yeni · a b taraflar · Enter karşılaştır · ? tuşlar
tui-monitor-hint-compare = u değişmeyen · m eşleşme · / filtre · Esc kapat

## Yardım katmanı

tui-monitor-help-title = Bu sayfanın tuşları
tui-monitor-help-all-title = Her sayfada
tui-monitor-help-all =
    Tab, Shift+Tab, 1-9, 0   diğer sayfalar
    P                        ölçümleri duraklat veya sürdür
    i                        sonraki ölçüm aralığı
    oklar, Page Up/Down      taşı veya kaydır
    ? veya Esc               bu yardımı kapat
tui-monitor-help-overview =
    Bir kutuya tıklayın      sayfasına gider
tui-monitor-help-processes =
    / ve Esc                 filtrele, filtreyi temizle
    Ctrl+R                   filtrenin tamamı düzenli ifade olur
    V                        liste, ağaç, programa göre, kullanıcıya göre
    Sol, Sağ                 satırı daralt veya genişlet
    + ve -                   tüm satırları genişlet veya daralt
    Boşluk                   süreci işaretle
    < > ve I                 sıralama sütunu, sırayı ters çevir
    c                        sütunları seç
    d, [ ve ]                ayrıntılar paneli, sekmeleri
    Shift+[ ve Shift+]       ayrıntıları kaydır
    F9 veya k                süreci sonlandır (önce sorar)
    K                        süreci öldür (önce sorar)
    t                        süreci ve başlattıklarını sonlandır
    s ve r                   askıya al, sürdür
    p ve a                   öncelik, işlemciler
    o                        programın klasörünü aç
    y                        pid, ad, yol veya komut satırını kopyala
    e                        tabloyu CSV olarak dışa aktar
    Enter                    sürecin menüsü
tui-monitor-help-cpu =
    v                        CPU başına grafik, ısı ızgarası veya çubuk
tui-monitor-help-devices =
    Yukarı, Aşağı            kaydır
tui-monitor-help-network =
    v                        sanal arabirimleri göster veya gizle
tui-monitor-help-connections =
    / ve Esc                 filtrele, filtreyi temizle
    < > ve I                 sıralama sütunu, sırayı ters çevir
    Enter                    bağlantının sürecine git
tui-monitor-help-services =
    / ve Esc                 filtrele, filtreyi temizle
    < > ve I                 sıralama sütunu, sırayı ters çevir
    x, X, R                  başlat, durdur, yeniden başlat (önce sorar)
tui-monitor-help-lists =
    / ve Esc                 filtrele, filtreyi temizle
    < > ve I                 sıralama sütunu, sırayı ters çevir
tui-monitor-help-history =
    r                        kaydı başlat veya durdur
    Boşluk                   kaydı duraklat veya sürdür
    + ve -                   daha uzun veya kısa aralık
    Sol, Sağ                 zamanda geri veya ileri git
    L                        canlıya dön
    , ve .                   imleci taşı: o andaki değerler ve en üst süreçler
    Esc                      imleci gizle
tui-monitor-help-snapshots =
    n                        yeni anlık görüntü
    F2 veya r                yeniden adlandır
    Del                      sil (önce sorar)
    e                        dışa aktar
    a                        sol taraf (A) olarak kullan
    b                        sağ taraf (B) olarak kullan, tekrar basınca şimdi
    Shift+B                  şimdiyle karşılaştır
    Enter                    karşılaştır
    Karşılaştırmada:
    < > ve I                 sıralama sütunu, sırayı ters çevir
    u                        değişmeyen süreçleri göster veya gizle
    m                        sürece veya programa göre eşleştir
    / ve Esc                 filtrele, kapat
tui-monitor-help-query-title = Filtre
tui-monitor-help-query =
    sözcük                   ad, komut satırı, yol, kullanıcı veya pid içinde
    "iki sözcük"             tek terim; tüm terimler eşleşmeli
    -sözcük                  eşleşmemeli
    a|b                      ikisinden biri
    /ifade/ ve /ifade/i      düzenli ifade
    user:root                tek sütunda: user name cmd path state pid ppid cgroup arch
    cpu>5  mem>100M          karşılaştırma: cpu mem rss private threads handles read write gpu nice

## Geçmiş kaydı

tui-monitor-recording = Kaydediliyor
tui-monitor-recording-paused = Kayıt duraklatıldı
tui-monitor-not-recording = Kayıt yok
tui-monitor-rec = kayıt
tui-monitor-rec-paused = kayıt duraklatıldı
