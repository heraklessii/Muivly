# GPU Capability Detection — Tasarım

Durum: tasarım onaylandı, implementasyon bekliyor.
İlgili: `docs/project_overview.md` → power/, `docs/decisions.md` → "Hedef kitle".

Amaç: Core başlarken donanımı **bir kez** yoklayıp, tüm pipeline'ın (decode
hedefi, fps, ölçekleme, kaç ayrı video) dayanacağı tek bir `GpuProfile`
üretmek. Kullanıcıya sorulmadan makul bir varsayılan; UI'dan override edilebilir.

Neden önemli: hedef kitle entegre GPU / 4-8GB RAM. Yanlış varsayılan (ör.
iGPU'da 60fps 4K) "hafif" iddiasını ilk açılışta çürütür.

---

## 1. Hangi adapter'da çalışacağız (kritik karar)

Hibrit laptop'ta (iGPU + dGPU) wallpaper'ı **dGPU'da çalıştırmak yanlış**:
pil ömrünü yakar, dGPU'yu sürekli uyanık tutar ve çıkışlar zaten iGPU'ya
bağlıysa cross-adapter kopya doğurur (zero-copy kuralı bozulur).

Kural: **çıkışın (output) sahibi olan adapter seçilir.**
- `IDXGIFactory6::EnumAdapterByGpuPreference(DXGI_GPU_PREFERENCE_MINIMUM_POWER)`
  ile sırala, `IDXGIAdapter::EnumOutputs` ile hangi monitörün hangi adapter'a
  bağlı olduğunu çıkar.
- Monitörler tek adapter'daysa: tek decode, tek paylaşılan texture (mevcut kural).
- Monitörler **farklı adapter'lara** dağılmışsa: adapter başına gruplanır,
  grup başına bir decode. → **Açık soru, aşağıda.**

## 2. Ölçülen sinyaller (tek seferlik probe, ~5-15ms)

| Sinyal | API | Ne için |
|---|---|---|
| Adapter listesi + güç tercihi | `IDXGIFactory6::EnumAdapterByGpuPreference` | Hangi GPU |
| Output→adapter eşlemesi | `IDXGIAdapter::EnumOutputs` | Decode gruplama |
| VendorId / DeviceId / LUID | `DXGI_ADAPTER_DESC1` | Sınıflandırma, cache anahtarı |
| DedicatedVideoMemory / SharedSystemMemory | `DXGI_ADAPTER_DESC1` | iGPU vs dGPU |
| `DXGI_ADAPTER_FLAG_SOFTWARE` | `desc1.Flags` | WARP / Basic Render'ı ele |
| Feature level | `D3D11CreateDevice` | 11_0 altı → desteklenmiyor |
| Decode profilleri | `ID3D11VideoDevice::GetVideoDecoderProfileCount` + `CheckVideoDecoderFormat(NV12)` | H264/HEVC/VP9/AV1 HW var mı |
| Monitör çözünürlük + refresh | `IDXGIOutput1::GetDesc1`, `GetDisplayModeList1` | fps tavanı, toplam piksel yükü |
| Toplam RAM | `GlobalMemoryStatusEx` | 4GB / 8GB sınıfı |
| Güç kaynağı | `GetSystemPowerStatus` | Pilde throttle |

Not: decode desteği **profil GUID'i + format kontrolü** ile doğrulanır;
sürücünün profili listelemesi tek başına yeterli sayılmaz. **Format profile göre
değişir**: 8-bit profiller NV12, 10-bit olanlar (HEVC Main10) P010. Main10'u
NV12 ile sorgulamak destekleyen donanımda bile "yok" cevabı veriyor — RTX 4050
üzerinde ölçüldü.

Ayrıca: `DXGI_OUTPUT_DESC.DesktopCoordinates` **DPI ile sanallaştırılmış**
değer döner (%125 ölçekte 2560x1440 panel 2048x1152 görünür). Gerçek piksel
için ya `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` çağrılmalı ya da
`DEVMODEW.dmPelsWidth/dmPelsHeight` kullanılmalı. Kod ikisini de yapıyor.

## 3. Adapter sınıflandırma

Sıra önemli — VRAM birinci, vendor sadece eşitlik bozucu:

1. `DXGI_ADAPTER_FLAG_SOFTWARE` veya VendorId `0x1414` (MS Basic Render) → **Software**
2. `DedicatedVideoMemory >= 1 GiB` → **Discrete**
3. VendorId `0x10DE` (NVIDIA) → **Discrete**
4. Aksi halde → **Integrated** (Intel `0x8086`, AMD APU `0x1002`, Qualcomm `0x5143`)

VRAM'ı önce kontrol etmek Intel Arc (`0x8086` ama dGPU) ve AMD dGPU
durumlarını vendor listesi şişirmeden doğru sınıflandırır.

## 4. Tier → politika tablosu

| Tier | Koşul | Hedef fps | Ölçek | Not |
|---|---|---|---|---|
| **High** | Discrete + HW decode + RAM >= 8GB | 60 (refresh'i geçmez) | native | |
| **Mid** | Integrated + HW decode | 30 | native, >1440p ise 1440p'ye downscale | Varsayılan kitle |
| **Low** | RAM < 4GB, ya da toplam piksel yükü çok yüksek | 24 | 1080p'ye downscale | |
| **Unsupported** | Software adapter veya feature level < 11_0 | statik | — | Wallpaper statik görsel olarak gösterilir |

Uygulanan ek kısıtlar:
- Efektif fps = `min(tier_fps, monitörün refresh rate'i)`.
- Pilde: fps tavanı 30, `Low` ise 24.
- `Low` ve `Mid`'de aynı anda **birden fazla farklı video** varsayılan olarak
  kapalı (tüm monitörlerde aynı video → tek decode, zaten serbest).

## 5. HW decode yoksa ne olur

CPU decode yasak (CLAUDE.md). Dolayısıyla codec'in HW desteği yoksa video
oynatılmaz; ilk frame statik gösterilir ve UI'da neden bildirilir.
→ **Açık soru, aşağıda.**

## 6. Modül yeri ve API şekli

Hem `decoder/` (codec desteği) hem `power/` (fps) bu bilgiye ihtiyaç duyuyor;
`decoder`'ın `power`'a bağımlı olması ters olacağı için ayrı modül:
`wallpaper-core/src/caps/`. → **Açık soru (CLAUDE.md modül listesini etkiler).**

```rust
pub struct GpuProfile {
    pub adapters: Vec<AdapterInfo>,   // sadece output sahibi olanlar
    pub system:   SystemInfo,
    pub rec:      Recommendation,     // türetilmiş politika
}

pub struct AdapterInfo {
    pub luid: i64, pub vendor_id: u32, pub device_id: u32, pub name: String,
    pub dedicated_vram: u64, pub shared_mem: u64,
    pub class: AdapterClass,          // Software | Integrated | Discrete
    pub feature_level: u32,
    pub decode: DecodeCaps,
    pub outputs: Vec<MonitorInfo>,
}

pub struct DecodeCaps {
    pub h264: bool, pub hevc_main: bool, pub hevc_main10: bool,
    pub vp9: bool, pub av1: bool,
    pub max_width: u32, pub max_height: u32,
}

pub struct Recommendation {
    pub tier: Tier,                   // High | Mid | Low | Unsupported
    pub target_fps: u32,
    pub max_scale: (u32, u32),
    pub allow_distinct_videos: bool,
}

pub fn probe() -> GpuProfile;         // core başlangıcında bir kez
```

Core start'ta bir kez çağrılır, `Arc<GpuProfile>` olarak decoder/power/
compositor'a geçirilir.

## 7. Cache ve invalidation

- Sonuç config yanında cache'lenir; anahtar = adapter LUID + driver sürümü.
- `IDXGIFactory::IsCurrent() == false` veya `WM_DISPLAYCHANGE` → yeniden probe.
- Monitör takılıp çıkarılması refresh/çözünürlük tavanını değiştirebilir,
  bu yüzden tier yeniden hesaplanır.

## 8. Kullanıcı override

Auto-detect **varsayılan**, kilit değil. UI algılanan tier'i ve gerekçesini
gösterir ("Entegre GPU algılandı → 30fps"), fps/ölçek elle değiştirilebilir.
Override config'te saklanır; donanım değişirse (LUID değişimi) sıfırlanır.

Probe çıktısı log'lanır — WE karşılaştırma benchmark'ında (bkz. tasks.md)
ölçümleri donanım sınıfına bağlayabilmek için gerekli.

---

## Karara Bağlanan Noktalar (2026-08-26, onaylandı)

1. **Farklı GPU'lardaki monitörler** → adapter başına bir decode. "Monitör
   başına decode yasak" kuralının yazılı istisnası.
2. **HW decode olmayan codec** → statik ilk frame + UI'da neden bildirimi.
   CPU fallback yok.
3. **`caps/` ayrı modül** olarak `wallpaper-core/src/caps/` altında.

Üçü de `docs/decisions.md`'ye işlendi, CLAUDE.md güncellendi.
