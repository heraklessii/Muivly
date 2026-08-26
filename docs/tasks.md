# Tasks

Amaç: Şu an ne yapılacak, sırada ne var. worklog.md geçmişi tutar, bu dosya
geleceği tutar. Bir görev tamamlandığında buradan silinir/işaretlenir VE
mimari etkisi varsa worklog.md'ye kısa girdi eklenir.

Format: öncelik grubu altında madde. Gerekirse ilgili decisions.md/
project_overview.md bölümüne referans.

---

## Şimdi (aktif faz)

- [ ] Media Foundation decoder iskeleti (`IMFSourceReader` + D3D11VA wrapper)
      — artık ekrana çizecek yer var, sıradaki büyük parça bu
- [ ] Aynı adapter'daki monitörler için paylaşılan texture (şu an her
      monitör kendi swap chain'ine aynı shader'ı çiziyor; video gelince
      tek decode → paylaşılan texture olmalı)

## Sırada (henüz başlanmadı)

- [ ] RAM'i düşürme: ~82 MB private hedefin üstünde, nereye gittiğini ölç
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
