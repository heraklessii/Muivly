//! Memory and power state — the non-GPU half of the profile.

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

#[derive(Debug, Clone, Copy)]
pub struct SystemInfo {
    pub total_ram_mb: u64,
    pub on_battery: bool,
}

pub fn probe() -> SystemInfo {
    SystemInfo {
        total_ram_mb: total_ram_mb(),
        on_battery: on_battery(),
    }
}

fn total_ram_mb() -> u64 {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };

    unsafe {
        if GlobalMemoryStatusEx(&mut status).is_ok() {
            status.ullTotalPhys / (1024 * 1024)
        } else {
            // Assume the low end rather than the high end: a wrong guess
            // downward only costs some frames, upward costs stutter.
            4096
        }
    }
}

fn on_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();

    unsafe {
        if GetSystemPowerStatus(&mut status).is_ok() {
            // 0 = offline (battery), 1 = online (AC), 255 = unknown.
            status.ACLineStatus == 0
        } else {
            false
        }
    }
}
