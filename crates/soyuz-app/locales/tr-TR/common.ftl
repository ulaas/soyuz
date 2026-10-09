### Soyuz arayüz metinleri (tr-TR): common
### Anahtarlar en-US/common.ftl ile birebir aynı olmalı.

## Ortak düğmeler ve sözcükler. Alana özgü metinler kendi dosyalarına.

common-ok = Tamam
common-cancel = İptal
common-close = Kapat
common-apply = Uygula
common-save = Kaydet
common-delete = Sil
common-rename = Yeniden Adlandır
common-copy = Kopyala
common-move = Taşı
common-undo = Geri Al
common-redo = Yinele
common-yes = Evet
common-no = Hayır
common-retry = Yeniden Dene
common-skip = Atla
common-reset = Sıfırla
common-loading = Yükleniyor…

# Türkçede sayıdan sonra ad tekil kalır ("3 öğe"); yine de CLDR "one"/"other" ayrımı var.
common-items = { $count ->
    [one] { $count } öğe
   *[other] { $count } öğe
}
common-folders = { $count ->
    [one] { $count } klasör
   *[other] { $count } klasör
}
common-files = { $count ->
    [one] { $count } dosya
   *[other] { $count } dosya
}
common-kind = { $kind ->
    [file] Dosya
    [folder] Klasör
    [link] Bağlantı
   *[other] Öğe
}

## Dropdown widget
dropdown-search = Ara…
dropdown-no-matches = Eşleşme yok
dropdown-use-typed = “{ $text }” kullan
dropdown-custom = Özel…
