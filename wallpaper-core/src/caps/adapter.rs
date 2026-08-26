//! DXGI adapter enumeration, output mapping and integrated/discrete
//! classification.

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG,
    DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_GPU_PREFERENCE_MINIMUM_POWER,
};
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};

use super::decode::{self, DecodeCaps};

const VENDOR_NVIDIA: u32 = 0x10DE;
const VENDOR_MICROSOFT: u32 = 0x1414; // Basic Render Driver / WARP

/// 1 GiB. Above this an adapter has its own memory pool, which in practice
/// means a discrete card (Intel Arc included — that is why VRAM is checked
/// before vendor id).
const DISCRETE_VRAM_THRESHOLD: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterClass {
    Software,
    Integrated,
    Discrete,
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub device_name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub primary: bool,
}

#[derive(Debug, Clone)]
pub struct AdapterInfo {
    pub luid: i64,
    pub vendor_id: u32,
    pub device_id: u32,
    pub name: String,
    pub dedicated_vram: u64,
    pub shared_mem: u64,
    pub class: AdapterClass,
    pub feature_level: u32,
    pub decode: DecodeCaps,
    pub outputs: Vec<MonitorInfo>,
}

impl AdapterInfo {
    pub fn feature_level_str(&self) -> String {
        format!(
            "{}.{}",
            (self.feature_level >> 12) & 0xF,
            (self.feature_level >> 8) & 0xF
        )
    }

    /// Total pixels this adapter has to fill every frame. Drives the Low tier
    /// check: three 1440p screens on an iGPU is a different job from one.
    pub fn total_pixels(&self) -> u64 {
        self.outputs
            .iter()
            .map(|o| o.width as u64 * o.height as u64)
            .sum()
    }

    pub fn max_refresh(&self) -> u32 {
        self.outputs
            .iter()
            .map(|o| o.refresh_hz)
            .max()
            .unwrap_or(60)
    }
}

/// Enumerate adapters that own at least one output, ordered by MINIMUM_POWER.
///
/// Adapters without an output are skipped: they cannot present a wallpaper,
/// and on a hybrid laptop the dGPU is usually exactly that. Picking the
/// output owner keeps the pipeline zero-copy (see decisions.md).
pub fn enumerate() -> Vec<AdapterInfo> {
    let mut out = Vec::new();

    unsafe {
        let factory: IDXGIFactory6 = match CreateDXGIFactory1() {
            Ok(f) => f,
            Err(_) => return out,
        };

        for i in 0.. {
            let adapter: IDXGIAdapter1 =
                match factory.EnumAdapterByGpuPreference(i, DXGI_GPU_PREFERENCE_MINIMUM_POWER) {
                    Ok(a) => a,
                    Err(_) => break, // DXGI_ERROR_NOT_FOUND: list exhausted
                };

            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };

            let outputs = outputs_of(&adapter);
            if outputs.is_empty() {
                continue;
            }

            let name = String::from_utf16_lossy(&desc.Description)
                .trim_end_matches('\0')
                .to_string();

            let is_software = DXGI_ADAPTER_FLAG(desc.Flags as i32) == DXGI_ADAPTER_FLAG_SOFTWARE
                || desc.VendorId == VENDOR_MICROSOFT;

            let class = classify(is_software, desc.VendorId, desc.DedicatedVideoMemory as u64);

            let (feature_level, decode) = probe_device(&adapter);

            out.push(AdapterInfo {
                luid: (desc.AdapterLuid.HighPart as i64) << 32 | desc.AdapterLuid.LowPart as i64,
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
                name,
                dedicated_vram: desc.DedicatedVideoMemory as u64,
                shared_mem: desc.SharedSystemMemory as u64,
                class,
                feature_level,
                decode,
                outputs,
            });
        }
    }

    out
}

/// VRAM first, vendor only as a tiebreak — that ordering classifies Intel Arc
/// (vendor 0x8086 but discrete) and AMD dGPUs correctly without maintaining a
/// device id list.
fn classify(is_software: bool, vendor_id: u32, dedicated_vram: u64) -> AdapterClass {
    if is_software {
        AdapterClass::Software
    } else if dedicated_vram >= DISCRETE_VRAM_THRESHOLD || vendor_id == VENDOR_NVIDIA {
        AdapterClass::Discrete
    } else {
        AdapterClass::Integrated
    }
}

unsafe fn outputs_of(adapter: &IDXGIAdapter1) -> Vec<MonitorInfo> {
    let mut monitors = Vec::new();

    for i in 0.. {
        let output = match adapter.EnumOutputs(i) {
            Ok(o) => o,
            Err(_) => break,
        };

        let Ok(desc) = output.GetDesc() else {
            continue;
        };

        // Current mode via GDI rather than IDXGIOutput::GetDisplayModeList:
        // we want what the display is running at right now, not the full
        // supported list, and the list call is comparatively expensive.
        let mut devmode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let ok = EnumDisplaySettingsW(
            windows::core::PCWSTR(desc.DeviceName.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut devmode,
        )
        .as_bool();

        // Physical pixels come from DEVMODEW. DXGI_OUTPUT_DESC's desktop
        // coordinates are DPI-virtualized, so a 2560x1440 panel at 125%
        // scaling reports 2048x1152 there — wrong basis for a pixel budget.
        let rect = desc.DesktopCoordinates;
        monitors.push(MonitorInfo {
            device_name: String::from_utf16_lossy(&desc.DeviceName)
                .trim_end_matches('\0')
                .to_string(),
            width: if ok {
                devmode.dmPelsWidth
            } else {
                (rect.right - rect.left).unsigned_abs()
            },
            height: if ok {
                devmode.dmPelsHeight
            } else {
                (rect.bottom - rect.top).unsigned_abs()
            },
            refresh_hz: if ok && devmode.dmDisplayFrequency > 1 {
                devmode.dmDisplayFrequency
            } else {
                60 // 0 and 1 mean "hardware default" in DEVMODEW
            },
            // The primary display is the one whose desktop origin is (0,0).
            primary: rect.left == 0 && rect.top == 0,
        });
    }

    monitors
}

/// Create a throwaway D3D11 device to read the feature level and the video
/// decode profiles. The device is dropped immediately; this is a probe, not
/// the render device.
unsafe fn probe_device(adapter: &IDXGIAdapter1) -> (u32, DecodeCaps) {
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut level = D3D_FEATURE_LEVEL::default();

    let created = D3D11CreateDevice(
        adapter,
        D3D_DRIVER_TYPE_UNKNOWN, // must be UNKNOWN when an adapter is given
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        Some(&levels),
        D3D11_SDK_VERSION,
        Some(&mut device),
        Some(&mut level),
        None,
    );

    match (created, device) {
        (Ok(()), Some(device)) => {
            let caps = device
                .cast()
                .map(|video| decode::probe(&video))
                .unwrap_or_default();
            (level.0 as u32, caps)
        }
        _ => (0, DecodeCaps::default()),
    }
}

/// The adapter the wallpaper renders on: the one owning the primary output.
/// Enumeration is already ordered by MINIMUM_POWER, so on a hybrid laptop
/// this lands on the iGPU.
pub fn primary(adapters: &[AdapterInfo]) -> Option<&AdapterInfo> {
    adapters
        .iter()
        .find(|a| a.outputs.iter().any(|o| o.primary))
        .or_else(|| adapters.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VENDOR_INTEL: u32 = 0x8086;
    const VENDOR_AMD: u32 = 0x1002;
    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn software_adapter_wins_over_everything() {
        // WARP reports plenty of "video memory"; the flag decides.
        assert_eq!(
            classify(true, VENDOR_NVIDIA, 8 * GIB),
            AdapterClass::Software
        );
    }

    #[test]
    fn intel_arc_is_discrete_despite_intel_vendor_id() {
        // The reason VRAM is checked before the vendor id.
        assert_eq!(
            classify(false, VENDOR_INTEL, 8 * GIB),
            AdapterClass::Discrete
        );
    }

    #[test]
    fn amd_apu_is_integrated() {
        // Measured on a Radeon 780M: 419 MB of "dedicated" memory carved out
        // of system RAM.
        assert_eq!(
            classify(false, VENDOR_AMD, 419 * 1024 * 1024),
            AdapterClass::Integrated
        );
    }

    #[test]
    fn nvidia_is_discrete_even_with_little_reported_vram() {
        assert_eq!(
            classify(false, VENDOR_NVIDIA, 256 * 1024 * 1024),
            AdapterClass::Discrete
        );
    }
}
