//! Deciding when a monitor is not worth drawing to, and when the machine
//! cannot afford to draw at all.
//!
//! Coverage is here in `mod.rs`; the power source and what to do about it is
//! in `battery.rs`. They answer the same question from two directions — is
//! this frame worth what it costs — which is why they live together.
//!
//! Coverage, in two signals, because neither is sufficient on its own:
//!
//! - **DXGI occlusion** (handled in `compositor::render`): the driver tells
//!   us a swap chain is fully covered. Accurate when it fires, but it does
//!   not fire reliably for the child windows a wallpaper lives in.
//! - **Window geometry** (here): if any ordinary window covers a monitor,
//!   nothing behind it is visible. This is the case that matters — a
//!   fullscreen game, a maximised editor.
//!
//! The foreground window alone is not enough: only one window is foreground
//! at a time, so on a second monitor a maximised window that does not have
//! focus would be missed and that monitor would keep rendering into nothing.
//! So every top-level window is examined.

pub mod apps;
pub mod battery;
pub mod idle;
pub mod load;

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONULL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, IsIconic, IsWindowVisible,
};

use crate::caps::MonitorInfo;

/// Enumerating windows is cheap but not free, and coverage does not change
/// between one frame and the next. Four checks a second is plenty to catch an
/// alt-tab without a user noticing the delay.
const RECHECK_AFTER: Duration = Duration::from_millis(250);

thread_local! {
    static CACHE: RefCell<Vec<(String, bool, Instant)>> = const { RefCell::new(Vec::new()) };
}

/// Whether this monitor is completely hidden behind some window.
pub fn is_covered(monitor: &MonitorInfo) -> bool {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let now = Instant::now();

        if let Some(entry) = cache.iter_mut().find(|e| e.0 == monitor.device_name) {
            if now.duration_since(entry.2) < RECHECK_AFTER {
                return entry.1;
            }
            entry.1 = compute(monitor);
            entry.2 = now;
            return entry.1;
        }

        let covered = compute(monitor);
        cache.push((monitor.device_name.clone(), covered, now));
        covered
    })
}

fn compute(monitor: &MonitorInfo) -> bool {
    let Some(work_area) = work_area_of(monitor) else {
        return false;
    };

    // The work area, not the full monitor: a maximised window stops at the
    // taskbar, and the strip the taskbar sits on is not wallpaper anyone can
    // see. Requiring full-monitor coverage would mean maximised windows never
    // pause anything.
    TARGET.with(|t| {
        *t.borrow_mut() = Some(Candidate {
            work_area,
            covered: false,
        });
    });

    let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM(0)) };

    TARGET.with(|t| t.borrow_mut().take().map(|c| c.covered).unwrap_or(false))
}

struct Candidate {
    work_area: RECT,
    covered: bool,
}

thread_local! {
    static TARGET: RefCell<Option<Candidate>> = const { RefCell::new(None) };
}

unsafe extern "system" fn enum_proc(hwnd: HWND, _: LPARAM) -> BOOL {
    if !covers_anything(hwnd) {
        return true.into();
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return true.into();
    }

    TARGET.with(|t| {
        let mut slot = t.borrow_mut();
        if let Some(candidate) = slot.as_mut() {
            if contains(&rect, &candidate.work_area) {
                candidate.covered = true;
                return false.into(); // found one; stop looking
            }
        }
        true.into()
    })
}

/// Filter out the windows that exist but cannot hide anything.
fn covers_anything(hwnd: HWND) -> bool {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    // A minimised window keeps the bounds it had when restored, so its rect
    // would otherwise look like full coverage.
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return false;
    }

    // Cloaked windows are the trap here: a suspended UWP app stays "visible"
    // by IsWindowVisible and often carries a full-screen rect, which would
    // pause the wallpaper for a window nobody can see.
    let mut cloaked = 0u32;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            std::mem::size_of::<u32>() as u32,
        )
    };
    if ok.is_ok() && cloaked != 0 {
        return false;
    }

    // The desktop itself, and our own surfaces, are not "coverage".
    !matches!(
        class_of(hwnd).as_str(),
        "Progman" | "WorkerW" | "MuivlySurface"
    )
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len as usize])
}

/// Ask Windows for the monitor at this monitor's centre, and its work area.
fn work_area_of(monitor: &MonitorInfo) -> Option<RECT> {
    let centre = POINT {
        x: monitor.x + monitor.width as i32 / 2,
        y: monitor.y + monitor.height as i32 / 2,
    };

    let handle = unsafe { MonitorFromPoint(centre, MONITOR_DEFAULTTONULL) };
    if handle.is_invalid() {
        return None;
    }

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    if unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
        Some(info.rcWork)
    } else {
        None
    }
}

fn contains(outer: &RECT, inner: &RECT) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn exact_cover_counts_as_covered() {
        let monitor = rect(0, 0, 1920, 1080);
        assert!(contains(&monitor, &monitor));
    }

    #[test]
    fn a_window_larger_than_the_screen_covers_it() {
        // Fullscreen games routinely report a rect slightly outside the
        // monitor bounds.
        assert!(contains(&rect(-8, -8, 1928, 1088), &rect(0, 0, 1920, 1080)));
    }

    #[test]
    fn one_pixel_short_is_not_covered() {
        assert!(!contains(&rect(0, 0, 1920, 1079), &rect(0, 0, 1920, 1080)));
    }

    #[test]
    fn a_window_on_another_monitor_covers_nothing() {
        let second = rect(1920, 0, 3840, 1080);
        assert!(!contains(&second, &rect(0, 0, 1920, 1080)));
    }
}
