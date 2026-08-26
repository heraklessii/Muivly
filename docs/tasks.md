# Tasks

Amaç: Şu an ne yapılacak, sırada ne var. worklog.md geçmişi tutar, bu dosya
geleceği tutar. Bir görev tamamlandığında buradan silinir/işaretlenir VE
mimari etkisi varsa worklog.md'ye kısa girdi eklenir.

Format: öncelik grubu altında madde. Gerekirse ilgili decisions.md/
project_overview.md bölümüne referans.

---

## Şimdi (aktif faz)

- [ ] **RAM/thread düşürme.** ~265 MB ve 75 thread, hedefin çok üstünde.
      Nereye gittiğini ölç: MF source reader thread havuzu mu, decode
      buffer sayısı mı, iki decoder mı. Denenecekler: async callback yerine
      sync okuma, `MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS`, buffer sayısını
      sınırlama.
- [ ] Görünmeyen adapter için decoder hiç açılmasın (şu an iki adapter için
      iki decoder açılıyor, biri hep kapalı olsa bile)

## Sırada (henüz başlanmadı)


- [ ] `caps` sonucunun cache'lenmesi + `WM_DISPLAYCHANGE` ile invalidation
- [ ] Monitör takma/çıkarma sırasında yüzeylerin yeniden oluşturulması
- [ ] wallpaper-core ↔ wallpaper-ui named pipe IPC protokolü taslağı
- [ ] Tauri UI iskeleti: wallpaper seçici, monitör eşleme ekranı
      (tema: `docs/design_system.md`, Outfit fontu Muita'dan kopyalanacak)
- [ ] Installer (`Muivly-x.y.z-setup.exe`) — Tauri UI çıkınca release.yml'e ikinci job
- [ ] WE ile RAM/CPU karşılaştırma benchmark videosu/görseli (pazarlama)

## Backlog (öncelik yok, fikir aşaması)

- [ ] Wallpaper paket formatı tasarımı (zip+manifest?)
- [ ] Shader/prosedürel içerik desteği (v2 kapsamı olabilir)
- [ ] Linux/Mac desteği araştırması
- [ ] AV1 codec desteği

## Bloklanmış / Karar Bekliyor

(şu an yok)

## Biten

- [x] GPU capability detection tasarımı → `docs/gpu_capability.md`
- [x] Toolchain: rustup 1.29 + rustc 1.98 + VS 2022 Build Tools (VCTools + Win11 SDK)
- [x] `caps/` modülü implementasyonu — gerçek donanımda çalışıyor
