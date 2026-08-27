//! Tier classification and the resulting playback policy.
//!
//! The numbers here are the defaults the user never has to think about. They
//! are not a lock: wallpaper-ui shows the detected tier and lets the user
//! override fps and scale. See `docs/gpu_capability.md`.

use super::adapter::{AdapterClass, AdapterInfo};
use super::system::SystemInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    High,
    Mid,
    Low,
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub tier: Tier,
    pub target_fps: u32,
    pub max_scale: (u32, u32),
    /// Whether different videos may play on different monitors. Off on the
    /// low tiers: one video everywhere means one decode.
    pub allow_distinct_videos: bool,
    /// Why this tier was picked — surfaced in the UI and in the startup log.
    pub reason: String,
}

const RES_1080P: (u32, u32) = (1920, 1080);
const RES_1440P: (u32, u32) = (2560, 1440);
const RES_NATIVE: (u32, u32) = (u32::MAX, u32::MAX);

/// Pixel budget above which an integrated GPU is treated as Low regardless of
/// its class: roughly three 1440p screens.
const HEAVY_PIXEL_LOAD: u64 = 3 * 2560 * 1440;

pub fn decide(adapters: &[AdapterInfo], system: &SystemInfo) -> Recommendation {
    let Some(adapter) = super::adapter::primary(adapters) else {
        return unsupported("no adapter with a display output");
    };

    if adapter.class == AdapterClass::Software {
        return unsupported("software adapter (no GPU acceleration)");
    }
    if adapter.feature_level < 0xB000 {
        return unsupported("feature level below 11_0");
    }
    if !adapter.decode.any() {
        // Not a hard failure: the compositor still shows a static first
        // frame. It just never becomes video.
        return unsupported("no hardware video decoder");
    }

    let (tier, mut fps, max_scale, reason) = if system.total_ram_mb < 4096 {
        (
            Tier::Low,
            24,
            RES_1080P,
            format!("{} MB RAM", system.total_ram_mb),
        )
    } else if adapter.total_pixels() > HEAVY_PIXEL_LOAD && adapter.class == AdapterClass::Integrated
    {
        (
            Tier::Low,
            24,
            RES_1080P,
            format!(
                "integrated GPU driving {} MPix",
                adapter.total_pixels() / 1_000_000
            ),
        )
    } else if adapter.class == AdapterClass::Discrete && system.total_ram_mb >= 8192 {
        (Tier::High, 60, RES_NATIVE, "discrete GPU".to_string())
    } else {
        (
            Tier::Mid,
            30,
            RES_1440P,
            if adapter.class == AdapterClass::Integrated {
                "integrated GPU".to_string()
            } else {
                format!("discrete GPU, {} MB RAM", system.total_ram_mb)
            },
        )
    };

    // Rendering above the panel's refresh rate is work nobody sees.
    fps = fps.min(adapter.max_refresh());

    // On battery the wallpaper is the first thing that should give way.
    let mut reason = reason;
    if system.on_battery && fps > 30 {
        fps = 30;
        reason.push_str(", on battery");
    }

    Recommendation {
        tier,
        target_fps: fps,
        max_scale,
        allow_distinct_videos: tier == Tier::High,
        reason,
    }
}

/// The largest frame worth decoding inside a memory budget.
///
/// The budget the user sets is a number in megabytes, and the only lever
/// that actually moves it is the frame size: what a decoder holds is its
/// codec's reference frames at the size of the picture, and everything else
/// in this process is noise beside that. See docs/decisions.md for the
/// measurements the steps below come from — a 4K clip on two adapters is
/// about 700 MB of private bytes, and each halving of the pixel count takes
/// roughly a quarter of that with it.
///
/// Deliberately coarse. There are four frame sizes anybody's decoder is
/// going to land on, and a continuous function over them would only be
/// precise about something that is an estimate to begin with.
pub fn scale_for_budget(budget_mb: u32) -> (u32, u32) {
    match budget_mb {
        0 => RES_NATIVE,
        b if b >= 600 => RES_NATIVE,
        b if b >= 350 => RES_1440P,
        b if b >= 200 => RES_1080P,
        _ => (1280, 720),
    }
}

/// The frame size cap once the user's budget is taken into account.
///
/// The smaller of the two by pixel count, never the larger: a budget is a
/// ceiling the user asked for, and a tier that would allow more does not
/// get to overrule it.
pub fn capped(tier_scale: (u32, u32), budget_mb: u32) -> (u32, u32) {
    let budget = scale_for_budget(budget_mb);
    let pixels = |size: (u32, u32)| size.0 as u64 * size.1 as u64;
    if pixels(budget) < pixels(tier_scale) {
        budget
    } else {
        tier_scale
    }
}

fn unsupported(reason: &str) -> Recommendation {
    Recommendation {
        tier: Tier::Unsupported,
        target_fps: 0,
        max_scale: RES_1080P,
        allow_distinct_videos: false,
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::adapter::MonitorInfo;
    use crate::caps::decode::DecodeCaps;

    fn monitor(width: u32, height: u32, refresh_hz: u32) -> MonitorInfo {
        MonitorInfo {
            device_name: r"\.DISPLAY1".to_string(),
            x: 0,
            y: 0,
            width,
            height,
            refresh_hz,
            primary: true,
        }
    }

    fn adapter(class: AdapterClass, outputs: Vec<MonitorInfo>) -> AdapterInfo {
        AdapterInfo {
            luid: 1,
            vendor_id: 0x8086,
            device_id: 0,
            name: "test".to_string(),
            dedicated_vram: 0,
            shared_mem: 0,
            class,
            feature_level: 0xB100,
            decode: DecodeCaps {
                h264: true,
                ..Default::default()
            },
            outputs,
        }
    }

    fn system(total_ram_mb: u64, on_battery: bool) -> SystemInfo {
        SystemInfo {
            total_ram_mb,
            on_battery,
        }
    }

    #[test]
    fn integrated_gpu_defaults_to_mid() {
        let a = [adapter(
            AdapterClass::Integrated,
            vec![monitor(1920, 1080, 60)],
        )];
        let rec = decide(&a, &system(8192, false));
        assert_eq!(rec.tier, Tier::Mid);
        assert_eq!(rec.target_fps, 30);
        assert!(!rec.allow_distinct_videos);
    }

    #[test]
    fn discrete_gpu_with_enough_ram_is_high() {
        let a = [adapter(
            AdapterClass::Discrete,
            vec![monitor(1920, 1080, 144)],
        )];
        let rec = decide(&a, &system(16384, false));
        assert_eq!(rec.tier, Tier::High);
        assert_eq!(rec.target_fps, 60);
        assert!(rec.allow_distinct_videos);
    }

    #[test]
    fn fps_never_exceeds_refresh_rate() {
        // A 60fps target on a 50Hz projector is 10 frames nobody sees.
        let a = [adapter(
            AdapterClass::Discrete,
            vec![monitor(1920, 1080, 50)],
        )];
        assert_eq!(decide(&a, &system(16384, false)).target_fps, 50);
    }

    #[test]
    fn battery_caps_at_30() {
        let a = [adapter(
            AdapterClass::Discrete,
            vec![monitor(1920, 1080, 144)],
        )];
        let rec = decide(&a, &system(16384, true));
        assert_eq!(rec.target_fps, 30);
        assert!(rec.reason.contains("battery"));
    }

    #[test]
    fn low_ram_forces_low_tier() {
        let a = [adapter(
            AdapterClass::Discrete,
            vec![monitor(1920, 1080, 144)],
        )];
        let rec = decide(&a, &system(3072, false));
        assert_eq!(rec.tier, Tier::Low);
        assert_eq!(rec.max_scale, RES_1080P);
    }

    #[test]
    fn integrated_gpu_driving_many_pixels_drops_to_low() {
        let a = [adapter(
            AdapterClass::Integrated,
            vec![
                monitor(2560, 1440, 60),
                monitor(2560, 1440, 60),
                monitor(2560, 1440, 60),
                monitor(2560, 1440, 60),
            ],
        )];
        assert_eq!(decide(&a, &system(16384, false)).tier, Tier::Low);
    }

    #[test]
    fn no_hardware_decoder_is_unsupported() {
        let mut a = adapter(AdapterClass::Discrete, vec![monitor(1920, 1080, 60)]);
        a.decode = DecodeCaps::default();
        let rec = decide(&[a], &system(16384, false));
        assert_eq!(rec.tier, Tier::Unsupported);
        assert_eq!(rec.target_fps, 0);
    }

    #[test]
    fn software_adapter_is_unsupported() {
        let a = [adapter(
            AdapterClass::Software,
            vec![monitor(1920, 1080, 60)],
        )];
        assert_eq!(decide(&a, &system(16384, false)).tier, Tier::Unsupported);
    }

    #[test]
    fn no_budget_leaves_the_tier_alone() {
        assert_eq!(capped(RES_1440P, 0), RES_1440P);
    }

    #[test]
    fn a_tight_budget_overrules_a_generous_tier() {
        // A discrete GPU would otherwise decode at the clip's native size,
        // which on a 4K file is the whole 700 MB the budget was set to
        // avoid.
        assert_eq!(capped(RES_NATIVE, 250), RES_1080P);
    }

    #[test]
    fn a_generous_budget_does_not_raise_a_low_tier() {
        // The tier is a judgement about the machine. A user asking for more
        // memory is not telling us their integrated GPU got faster.
        assert_eq!(capped(RES_1080P, 4096), RES_1080P);
    }

    #[test]
    fn the_smallest_budget_still_leaves_a_watchable_frame() {
        assert_eq!(scale_for_budget(1), (1280, 720));
    }

    #[test]
    fn no_adapter_at_all_is_unsupported() {
        assert_eq!(decide(&[], &system(16384, false)).tier, Tier::Unsupported);
    }
}
