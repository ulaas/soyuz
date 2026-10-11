### Soyuz arayüz metinleri (tr-TR): ops
### Anahtarlar en-US/ops.ftl ile birebir aynı olmalı.

## Gezinme, başlatma ve uygulama komutları (ops.rs)

ops-bad-address = Bu adres açılamıyor
ops-open-failed = { $name } açılamadı
ops-launcher-failed = Başlatıcı çalıştırılamadı
ops-folder-gone = Klasör artık yok
ops-folder-gone-body = { $name } kaldırıldı veya bağlantısı kesildi.
ops-coming-soon = Yakında: { $action }
ops-coming-soon-body = Dosya işlemleri, iş motoru entegrasyonuyla birlikte gelecek.
ops-open-with-failed = Şu uygulamayla açılamadı: { $app }
ops-copied-paths = { $count } yol kopyalandı
ops-copied-to-clipboard = Panoya kopyalandı
ops-reveal-failed = Sistem dosya yöneticisinde gösterilemedi
ops-terminal-failed = Terminal açılamadı
ops-favorite-added = “{ $name }” sık kullanılanlara eklendi
ops-view-applied = Görünüm tüm klasörlere uygulandı

## İş başlıkları (kartlar, bildirimler, geçmiş). $state: "active" | "done".

ops-job-copy-to = { $state ->
    [active] Kopyalanıyor: { $what } → { $dest }
   *[done] Kopyalandı: { $what } → { $dest }
}
ops-job-move-to = { $state ->
    [active] Taşınıyor: { $what } → { $dest }
   *[done] Taşındı: { $what } → { $dest }
}
ops-job-rename-to = { $state ->
    [active] Yeniden adlandırılıyor: { $what } → { $dest }
   *[done] Yeniden adlandırıldı: { $what } → { $dest }
}
ops-job-copy = { $state ->
    [active] { $what } kopyalanıyor
   *[done] { $what } kopyalandı
}
ops-job-move = { $state ->
    [active] { $what } taşınıyor
   *[done] { $what } taşındı
}
ops-job-rename = { $state ->
    [active] { $what } yeniden adlandırılıyor
   *[done] { $what } yeniden adlandırıldı
}
ops-job-duplicate = { $state ->
    [active] { $what } çoğaltılıyor
   *[done] { $what } çoğaltıldı
}
ops-job-trash = { $state ->
    [active] { $what } Çöp Kutusu'na taşınıyor
   *[done] { $what } Çöp Kutusu'na taşındı
}
ops-job-delete = { $state ->
    [active] { $what } siliniyor
   *[done] { $what } silindi
}
ops-job-wipe = { $state ->
    [active] { $what } güvenli şekilde siliniyor
   *[done] { $what } güvenli şekilde silindi
}
ops-job-set-meta = { $state ->
    [active] Özellikler güncelleniyor: { $what }
   *[done] Özellikler güncellendi: { $what }
}
ops-job-restore = { $state ->
    [active] { $what } geri yükleniyor
   *[done] { $what } geri yüklendi
}
ops-job-empty-trash = { $state ->
    [active] Çöp Kutusu boşaltılıyor
   *[done] Çöp Kutusu boşaltıldı
}
ops-job-new-folder = { $state ->
    [active] { $has_name ->
        [yes] Klasör oluşturuluyor: { $name }
       *[no] Klasör oluşturuluyor
    }
   *[done] { $has_name ->
        [yes] Klasör oluşturuldu: { $name }
       *[no] Klasör oluşturuldu
    }
}
ops-job-new-file = { $state ->
    [active] { $has_name ->
        [yes] Dosya oluşturuluyor: { $name }
       *[no] Dosya oluşturuluyor
    }
   *[done] { $has_name ->
        [yes] Dosya oluşturuldu: { $name }
       *[no] Dosya oluşturuldu
    }
}
ops-job-link = { $state ->
    [active] { $has_name ->
        [yes] Bağlantı oluşturuluyor: { $name }
       *[no] Bağlantı oluşturuluyor
    }
   *[done] { $has_name ->
        [yes] Bağlantı oluşturuldu: { $name }
       *[no] Bağlantı oluşturuldu
    }
}
ops-job-undo = { $state ->
    [active] Geri alınıyor: { $label }
   *[done] Geri alındı: { $label }
}
ops-job-redo = { $state ->
    [active] Yineleniyor: { $label }
   *[done] Yinelendi: { $label }
}
ops-job-retry = { $state ->
    [active] Yeniden deneniyor: { $label }
   *[done] Yeniden denendi: { $label }
}
ops-job-other = { $state ->
    [active] İşleniyor: { $label }
   *[done] Tamamlandı: { $label }
}

## İş sonuçları

ops-outcome-failed = Başarısız: { $error }
ops-outcome-cancelled = İptal edildi
ops-summary-done-in = { $time } içinde tamamlandı
ops-summary-skipped = { $count } atlandı
ops-summary-failed = { $count } başarısız
ops-job-failed = Başarısız: { $what }
ops-details = Ayrıntılar

## Varsayılan adlar

ops-link-default-name = { $stem } - bağlantı{ $ext }
ops-default-folder-name = Yeni klasör
ops-default-file-name = Yeni dosya.txt

## Pano, yapıştırma, geri alma (fileops.rs)

ops-clip-copied = { $count } öğe kopyalandı
ops-clip-cut = { $count } öğe kesildi
ops-clip-paste-hint = Ctrl+V ile yapıştırın.
ops-nothing-to-paste = Yapıştırılacak bir şey yok
ops-nothing-to-paste-body = Önce dosyaları kopyalayın veya kesin (Ctrl+C / Ctrl+X).
ops-no-other-pane = Diğer bölme yok
ops-no-other-pane-body = Klasörler arasında kopyalamak veya taşımak için çift bölmeli görünümü açın (F9).
ops-drop-into-itself = Bir klasör kendi içine bırakılamaz
ops-drop-onto-folder = Kopyalamak için dosyaları bir klasör görünümüne bırakın
ops-nothing-to-undo = Bu oturumda geri alınacak bir şey yok
ops-still-undoing = Bu işlem hâlâ geri alınıyor
ops-nothing-to-redo = Yinelenecek bir şey yok
ops-not-available = Şu anda kullanılamıyor

## Sürükle ve bırak

ops-dnd-move-here = Buraya taşı
ops-dnd-copy-here = Buraya kopyala
ops-dnd-link-here = Buraya bağlantı oluştur
ops-dnd-trash = Çöp Kutusu
ops-dnd-badge-trash = Taşı: { $target }
ops-dnd-badge-copy = Kopyala: { $target }
ops-dnd-badge-move = Taşı: { $target }
ops-dnd-badge-link = Bağlantı oluştur: { $target }
ops-dnd-badge-drop = Bırak: { $target }

## Yeniden adlandırma denetimi

ops-rename-required = Bir ad girilmelidir.
ops-rename-reserved = “{ $name }” ayrılmış bir addır.
ops-rename-invalid-char = Adlar “{ $char }” karakterini içeremez.
ops-rename-trailing-dot = Adlar nokta veya boşlukla bitemez.
ops-rename-too-long = Ad çok uzun ({ $count } karakter).
ops-rename-exists = Burada “{ $name }” adlı bir öğe zaten var.

## Pencereler ve yönetici olarak çalıştırma (ops/window.rs)

ops-new-window-failed = Yeni pencere açılamadı
ops-window-keeps-tab = Bir pencerede en az bir sekme kalmalıdır
ops-elevate-select = Yönetici olarak çalıştırılacak bir program seçin
ops-elevate-needs = { $platform ->
    [windows] Yönetici olarak çalıştırmak için bir .exe, .msi, .bat veya .cmd dosyası gerekir
    [linux] Yönetici olarak çalıştırmak için çalıştırma izni olan bir program dosyası gerekir (pkexec ile çalıştırılır)
   *[other] Yönetici olarak çalıştırmak için çalıştırılabilir bir dosya gerekir
}
ops-elevate-failed = { $name } yönetici olarak çalıştırılamadı

## Yer adları (adres çubuğu, sekme ve pencere başlıkları) ve zaman çizelgesi grupları

ops-place-home = Giriş
ops-place-trash = Çöp Kutusu
ops-place-computer = Bu bilgisayar
ops-place-recent = Son kullanılanlar
ops-place-timeline = Zaman çizelgesi
ops-place-tag = Etiket: { $name }
ops-place-search = Arama
ops-bucket-today = Bugün
ops-bucket-yesterday = Dün
ops-bucket-this-week = Bu hafta
ops-bucket-earlier = Daha önce

## Linux (platform)

ops-elevate-no-pkexec = Programları yönetici olarak çalıştırmak için polkit'in pkexec aracı kullanılır, ancak kurulu değil. polkit paketini kurun (pkexec onunla gelir) ve yeniden deneyin.
