### Soyuz arayüz metinleri (tr-TR): pdf
### Anahtarlar en-US/pdf.ftl ile birebir aynı olmalı.


## PDF görüntüleyici (editors::pdf)

# $page: sayfa numarası (1'den başlar), $pages: sayfa sayısı.
pdf-page-of = Sayfa { $page } / { $pages }
# $percent: yüzde olarak yakınlaştırma, % işareti olmadan.
pdf-zoom-percent = %{ $percent }
pdf-link-not-web = Yalnızca web ve e-posta bağlantıları açılabilir
pdf-no-document = Önce görüntüleyicide bir PDF açın


## PDF görüntüleyicinin arayüzü (soyuz-app ui::editors::pdf)

pdf-ui-sidebar = Kenar çubuğu
pdf-ui-prev-page = Önceki sayfa
pdf-ui-next-page = Sonraki sayfa
# Araç çubuğundaki sayfa alanı. $page: geçerli sayfa (1'den başlar), $pages: sayfa sayısı.
pdf-ui-page-count = { $page } / { $pages }
pdf-ui-goto-hint = Sayfa
pdf-ui-goto = Sayfaya git…
pdf-ui-zoom = Yakınlaştırma
pdf-ui-zoom-out = Uzaklaştır
pdf-ui-zoom-in = Yakınlaştır
pdf-ui-fit-width = Genişliğe sığdır
pdf-ui-fit-page = Sayfaya sığdır
pdf-ui-actual-size = Gerçek boyut
pdf-ui-rotate-cw = Saat yönünde döndür
pdf-ui-rotate-ccw = Saat yönünün tersine döndür
pdf-ui-dark-pages = Koyu sayfalar
pdf-ui-find = Bul
pdf-ui-more = Diğer
pdf-ui-copy = Kopyala
pdf-ui-select-all = Tümünü seç
pdf-ui-copy-page = Sayfayı metin olarak kopyala
pdf-ui-find-selection = Seçimi bul
pdf-ui-thumbnails = Küçük resimler
pdf-ui-outline = İçindekiler
pdf-ui-no-outline = İçindekiler yok
pdf-ui-no-outline-body = Bu belgede yer imi yok.
pdf-ui-find-hint = Belgede bul
# $current: geçerli eşleşmenin sırası, $count: şimdiye dek bulunan eşleşmeler.
pdf-ui-find-of = { $current } / { $count }
# $count: şimdiye dek bulunan eşleşmeler.
pdf-ui-find-count = { $count } eşleşme
pdf-ui-find-none = Eşleşme yok
# $width, $height: milimetre olarak sayfa boyutu.
pdf-ui-status-size = { $width } × { $height } mm
# $count: seçili karakter sayısı.
pdf-ui-status-selected = { $count } karakter seçili
pdf-ui-locked = Bu PDF parolayla korunuyor
pdf-ui-locked-body = Açmak için parolasını yazın.
pdf-ui-locked-wrong = Bu parola belgeyi açmadı. Yeniden deneyin.
pdf-ui-unlock = Parolayı gir
pdf-ui-failed = PDF açılamadı
pdf-ui-page-failed = Bu sayfa çizilemedi
pdf-ui-retry = Yeniden dene
# $url: belgedeki bağlantının gösterdiği web adresi.
pdf-ui-link-ask = { $url } açılsın mı?
pdf-ui-link-open = Bağlantıyı aç
# Sayfa alanının erişilebilir adı. $name: dosya adı.
pdf-ui-document = PDF belgesi { $name }
pdf-ui-open-in-viewer = Görüntüleyicide aç
