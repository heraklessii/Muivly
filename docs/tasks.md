# Tasks

Amaç: Şu an ne yapılacak, sırada ne var. worklog.md geçmişi tutar, bu dosya
geleceği tutar. Bir görev tamamlandığında buradan silinir/işaretlenir VE
mimari etkisi varsa worklog.md'ye kısa girdi eklenir.

Format: öncelik grubu altında madde. Gerekirse ilgili decisions.md/
project_overview.md bölümüne referans.

---

## Şimdi (aktif faz)

- [ ] WorkerW injection + shared D3D11 texture prototipi. (bkz.
      decisions.md → "Multi-monitor: paylaşılan texture")
- [ ] Cross-adapter yolu: İlker'in makinesi tam da bu durum (iGPU + dGPU'da
      birer monitör), prototip bunu ilk günden test edebilir.

## Sırada (henüz başlanmadı)

- [ ] Media Foundation decoder iskeleti (`IMFSourceReader` + D3D11VA wrapper)
- [ ] Fullscreen/occlusion detection (`SetWinEventHook`, `DwmGetWindowAttribute`)
- [ ] `caps` sonucunun cache'lenmesi + `WM_DISPLAYCHANGE` ile invalidation
      (tasarımda var, henüz kodda yok)
- [ ] wallpaper-core ↔ wallpaper-ui named pipe IPC protokolü taslağı
- [ ] Tauri UI iskeleti: wallpaper seçici, monitör eşleme ekranı
      (tema: `docs/design_system.md`, Outfit fontu Muita'dan kopyalanacak)
- [ ] WE ile RAM/CPU karşılaştırma benchmark videosu/görseli (pazarlama)
- [ ] Installer (`Muivly-x.y.z-setup.exe`) — Tauri UI çıkınca release.yml'e ikinci job

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
