# Project Overview

Amaç: CLAUDE.md'deki mimari özetin detayı. Yeni bir modül tasarlarken, mevcut
bir modülü değiştirirken veya "bu neden böyle yapılmış" sorusu geldiğinde oku.
Rutin kod yazımında bu dosyayı okumaya gerek yok.

## Neden Bu Mimari

Wallpaper Engine gibi araçlara kıyasla temel fark: **iki process ayrımı**
(core + ui). Bunun sebebi, ayar paneli kapansa/açılsa bile wallpaper render
motorunun bellek/performans profilinin değişmemesi. Tauri'nin WebView'i
(WRY/WebView2) her zaman ekstra RAM taşır; bunu wallpaper render'ından
tamamen izole ederek WebView maliyetini "sadece ayarlar açıkken ödenen"
bir maliyete indirgiyoruz.

## wallpaper-core Detayı

### decoder/
- Windows Media Foundation (`IMFSourceReader`) üzerinden video okuma.
- Hardware decode: D3D11 Video Decode API, GPU'nun kendi decoder'ını kullanır
  (NVDEC / Intel Quick Sync / AMD VCN — hangisi varsa otomatik seçilir, MF
  bunu zaten platform seviyesinde hallediyor).
- ffmpeg/libav'a bağımlılık YOK. Gerekirse sadece container demux için
  düşünülebilir (örn. mkv gibi MF'in native desteklemediği formatlar), ama
  decode adımı her zaman D3D11VA üzerinden gider.
- `windows-rs` crate'i ile bindings.

### compositor/
- WorkerW injection: masaüstü arka planına native pencere yerleştirme
  (`Progman` mesajlaşması ile WorkerW handle'ı alınır).
- Multi-monitor: Her monitör için ayrı decode YASAK. Tek decode → paylaşılan
  D3D11 texture (`IDXGIResource` / `IDXGIKeyedMutex` ile senkronize erişim)
  → her WorkerW instance'ı aynı texture'ı kendi swap chain'ine present eder.

### power/
- `SetWinEventHook` ile foreground window değişikliklerini izler.
- Fullscreen detection: aktif pencere tam ekran mı kontrol edilir → evetse
  render pipeline durur (pause), son frame sabit kalır.
- Occlusion detection: `DwmGetWindowAttribute` / present test ile WorkerW'in
  görünürlüğü kontrol edilir → görünmüyorsa frame rate düşürülür veya durur.
- Adaptive frame rate: GPU capability'e göre otomatik 30/60fps seçimi
  (entegre GPU → 30fps varsayılan, dedicated GPU → 60fps).

### ipc/
- Named pipe üzerinden wallpaper-ui ile haberleşme.
- Mesaj tipleri: wallpaper değiştir, monitör ata, ayar güncelle, durum sorgula.

## wallpaper-ui Detayı

- Tauri v2 + React + Vite.
- Sorumluluğu: wallpaper seçimi, monitör eşleme, ayarlar (fps limit, otomatik
  throttle aç/kapa, başlangıçta çalıştır vs).
- Wallpaper render'ı İÇERMEZ. UI kapatıldığında core process etkilenmez.

## Performans Hedefleri (kaba, ölçülecek/güncellenecek)

- Idle/occluded: ~0% GPU, minimal CPU.
- 1080p30 video, dedicated GPU: düşük tek haneli % GPU kullanımı hedefi.
- RAM: WebView2 hariç core process için düşük onlarca MB hedefi (video
  buffer boyutuna göre değişir — ring buffer birkaç frame ile sınırlı).
- Binary/kurulum boyutu: WE'nin Steam+Workshop yüküne kıyasla belirgin
  şekilde küçük (hedef: birkaç on MB, WebView2 runtime hariç).

## Açık Sorular / Henüz Karar Verilmemiş

- Wallpaper paket formatı (basit zip+manifest mi, başka bir şey mi?)
- Shader/prosedürel içerik desteği ne zaman eklenecek (v2 kapsamı olabilir).
- Linux/Mac desteği zaman çizelgesi (WorkerW eşdeğeri her platformda farklı).
- Codec kapsamı: sadece h264/vp9 mi, av1 destek ne zaman.