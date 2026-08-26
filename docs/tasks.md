# Tasks

Amaç: Şu an ne yapılacak, sırada ne var. worklog.md geçmişi tutar, bu dosya
geleceği tutar. Bir görev tamamlandığında buradan silinir/işaretlenir VE
mimari etkisi varsa worklog.md'ye kısa girdi eklenir.

Format: öncelik grubu altında madde. Gerekirse ilgili decisions.md/
project_overview.md bölümüne referans.

---

## Şimdi (aktif faz)

- [ ] **RAM/thread düşürme — ölçüm gerekiyor.** ~265 MB ve 75 thread hâlâ
      hedefin üstünde. İki şey denendi ve girdi (bkz. decisions.md):
      `MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS`, ve processor'ın yalnız
      gerekince kurulması. **Kazancın ne olduğu henüz ölçülmedi** — sıradaki
      iş `--caps` ve Görev Yöneticisi ile önce/sonra sayısını almak.
      Denenmemiş kalanlar: buffer sayısını sınırlama, `MF_LOW_LATENCY`
      (dikkat: B-frame yeniden sıralamasını bozabilir, önce test).

## Sırada (henüz başlanmadı)

- [ ] `MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS` sonrası P010/10-bit HEVC
      yolunu gerçek dosyayla doğrula — processor'lu yeniden açma yalnız
      teoride sınandı

- [ ] **Gerçek donanımda elden geçirme.** Bu oturumda yazılanların çoğu
      derlendi ve testleri geçti ama makinede çalıştırılmadı: crossfade'in
      görüntüsü, span'in ekranlar arasında hizası, kısayolların gerçekten
      kaydolması, kabloyu çekince fps'in düşmesi, ekran takıp çıkarınca
      yeniden kurulum, sağ tık menüsünün Explorer'da görünmesi.
- [ ] `caps` sonucunun cache'lenmesi (invalidation artık var:
      `compositor/notify.rs` + yerleşim karşılaştırması)
- [ ] Paket formatına önizleme görseli yazma (okuma tarafı hazır, `preview`
      alanı manifest'te duruyor; export şu an yalnız videoyu koyuyor)
- [ ] WE ile RAM/CPU karşılaştırma benchmark videosu/görseli (pazarlama)
- [ ] winget'e ilk gönderim (manifestler `packaging/winget/`, release iş
      akışı dolduruyor — gönderim elle)

## Backlog (öncelik yok, fikir aşaması)

- [ ] Klip başı/sonu kırpma noktaları (playlist yolunda `path#start-end`
      gibi bir şey gerekiyor; protokol değişikliği)
- [ ] Shader/prosedürel içerik desteği (v2 kapsamı olabilir)
- [ ] Linux/Mac desteği araştırması

## Bloklanmış / Karar Bekliyor

(şu an yok)

## Biten

- [x] Pil politikası: pilde ayrı fps, pil tasarrufunda dondurma
- [x] Ses ducking — başka uygulama ses çalarken geri çekilme
- [x] Ekran takma/çıkarma, çözünürlük değişimi, uykudan uyanma ve Explorer
      yeniden başlaması: sahnenin tamamı yeniden kuruluyor
- [x] Monitör başına fit / görünüm / kare hızı
- [x] Tek duvar kağıdını ekranlara yayma (span)
- [x] Duvar kağıtları arası crossfade
- [x] Oynatma hızı (0.25-2.0x)
- [x] Global kısayollar + Explorer sağ tık menüsü + `--set <dosya>`
- [x] `.muivly` paket formatı (zip + manifest, elle yazıldı)
- [x] AV1/HEVC için "Store eklentisi eksik" ile "GPU çözemiyor" ayrımı
- [x] winget manifestleri
- [x] `redraw` bayrağı hiç temizlenmiyordu — duran kare her tick present
      ediliyordu
- [x] İndirmede yönlendirme allowlist'i baypas ediyordu + gövde sınırsız
      belleğe alınıyordu — ikisi de kapatıldı
- [x] IPC yarışı: üç pipe instance'ı, uzak istemci reddi, istemcide yeniden
      deneme ("motor çalışmıyor" yanılması)
- [x] Tauri komutları `(async)` — bloklayan G/Ç main thread'den çıktı
- [x] `session.txt`'te tek bozuk satır tüm oturumu siliyordu
- [x] Ses görünmeyince duruyor (eskiden sessizce çözmeye devam ediyordu)
- [x] Kitaplık ızgarası ekran dışında çözücü tutmuyor (sanallaştırma yerine
      medya elemanını sökmek — gerekçe decisions.md'de)
- [x] `.gif` `<video>`'ya gidiyordu, her GIF "okunamadı" görünüyordu
- [x] Kaydırıcılar klavye/dokunmatik ile hiçbir şey göndermiyordu
- [x] Pencere tepsideyken yoklama duruyor
- [x] Kayıp dosya işareti (`file_infos` ile; `file_exists` artık kullanılmıyor)
- [x] Görünmeyen adapter için decoder açılmıyor (`sync_decoders` `enabled`
      filtresiyle zaten sağlanıyordu — doğrulandı)
- [x] Takılan oynatma — tempolama (yüksek çözünürlüklü sayaç + videonun kendi
      kare zamanı), sample ömrü, kare değişmediyse sunum yok
- [x] Decode kendi thread'inde, iki karelik kuyrukla
- [x] Ölçek kapağı (4x eşikli — ölçüm `docs/decisions.md`'de)
- [x] Görsel + animasyonlu GIF desteği (WIC)
- [x] Ses (WASAPI, varsayılan kapalı)
- [x] Parlaklık / doygunluk / bulanıklık
- [x] Windows ile başlat + motorun oturum hatırlaması
- [x] Zengin tepsi menüsü (sonraki / duraklat / ses / çıkış)
- [x] Wallpaper Engine kitaplığı içe aktarma
- [x] Keşfet — motionbgs.com entegrasyonu
- [x] Performans göstergesi (motorun kendi CPU/RAM/fps ölçümü)
- [x] Tepsiden çıkışta motorun da kapanması + oturum başına tek motor

- [x] GPU capability detection tasarımı → `docs/gpu_capability.md`
- [x] Toolchain: rustup 1.29 + rustc 1.98 + VS 2022 Build Tools (VCTools + Win11 SDK)
- [x] `caps/` modülü implementasyonu — gerçek donanımda çalışıyor
