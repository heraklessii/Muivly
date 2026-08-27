//! GPU / system capability detection.
//!
//! Probed once at core startup, then shared as `Arc<GpuProfile>` with the
//! decoder (codec support), power (fps policy) and compositor (decode
//! grouping) modules. See `docs/gpu_capability.md`.

mod adapter;
mod decode;
mod policy;
mod system;

// Re-exported as the module API. decoder/ and power/ consume these next;
// until then the compiler sees them as unused.
#[allow(unused_imports)]
pub use adapter::{AdapterClass, AdapterInfo, MonitorInfo};
#[allow(unused_imports)]
pub use decode::DecodeCaps;
#[allow(unused_imports)]
pub use policy::{capped, scale_for_budget};
pub use policy::{Recommendation, Tier};
pub use system::SystemInfo;

use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub struct GpuProfile {
    /// Only adapters that actually own a display output. An adapter with no
    /// output cannot present a wallpaper, so it is never a render target.
    pub adapters: Vec<AdapterInfo>,
    pub system: SystemInfo,
    pub rec: Recommendation,
}

/// Probe the machine. Cheap enough (~5-15ms) to run on the startup path.
pub fn probe() -> GpuProfile {
    let system = system::probe();
    let adapters = adapter::enumerate();
    let rec = policy::decide(&adapters, &system);

    GpuProfile {
        adapters,
        system,
        rec,
    }
}

impl GpuProfile {
    /// The adapter the wallpaper renders on. See `adapter::primary`.
    /// Consumed by compositor/ once it exists.
    #[allow(dead_code)]
    pub fn primary(&self) -> Option<&AdapterInfo> {
        adapter::primary(&self.adapters)
    }

    /// Human-readable dump. Logged at startup so benchmark numbers can be
    /// tied back to a hardware class.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "system: {} MB RAM, power: {}",
            self.system.total_ram_mb,
            if self.system.on_battery {
                "battery"
            } else {
                "AC"
            }
        );

        for a in &self.adapters {
            let _ = writeln!(
                s,
                "adapter: {} [{:?}] {:04x}:{:04x} luid={:#x} \
                 vram={} MB shared={} MB fl={} decode={}",
                a.name,
                a.class,
                a.vendor_id,
                a.device_id,
                a.luid,
                a.dedicated_vram / (1024 * 1024),
                a.shared_mem / (1024 * 1024),
                a.feature_level_str(),
                a.decode.summary()
            );
            for o in &a.outputs {
                let _ = writeln!(
                    s,
                    "  output: {} {}x{} @{}Hz{}",
                    o.device_name,
                    o.width,
                    o.height,
                    o.refresh_hz,
                    if o.primary { " (primary)" } else { "" }
                );
            }
        }

        let _ = writeln!(
            s,
            "tier: {:?} -> {} fps, max {}x{}, distinct videos: {}",
            self.rec.tier,
            self.rec.target_fps,
            self.rec.max_scale.0,
            self.rec.max_scale.1,
            self.rec.allow_distinct_videos
        );
        let _ = writeln!(s, "reason: {}", self.rec.reason);
        s
    }
}
