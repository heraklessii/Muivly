//! Finding the window that lives behind the desktop icons.
//!
//! Windows draws the desktop in a window called Progman. Sending it an
//! undocumented message makes it split the desktop into an icon layer
//! (`SHELLDLL_DefView`) and a wallpaper layer (class `WorkerW`). Anything
//! parented to the wallpaper layer appears as the wallpaper, with icons still
//! on top and still clickable.
//!
//! Where that WorkerW ends up differs by Windows version, and getting it
//! wrong fails *silently* — the window is created, rendering succeeds, and
//! nothing appears on screen. Two layouts exist in the wild:
//!
//! - **Child layout** (seen on Windows 11): Progman owns both
//!   `SHELLDLL_DefView` and a full-desktop `WorkerW` as children.
//! - **Sibling layout** (Windows 7 through 10): the WorkerW is a *top-level*
//!   window sitting immediately behind the one that owns `SHELLDLL_DefView`.
//!
//! Both are tried, and the result is checked against the desktop size before
//! being accepted: a machine can have a dozen top-level WorkerW windows, and
//! all but one are invisible stubs a few pixels across.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{w, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetWindowRect, IsWindow, IsWindowVisible,
    SendMessageTimeoutW, SMTO_NORMAL,
};

/// Whether the window the wallpaper is parented to still exists.
///
/// Explorer restarting — after a crash, or after the user kills it — takes
/// WorkerW with it and leaves every surface parented to a handle that is
/// gone. The wallpaper vanishes and nothing reports an error, because
/// nothing failed: the windows are simply orphans. Checking is one call.
pub fn is_alive(hwnd: HWND) -> bool {
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

/// The undocumented message that asks Progman to split the desktop into an
/// icon window and a wallpaper window.
const WM_SPAWN_WORKER: u32 = 0x052C;

/// Where a wallpaper surface can be parented, and how it was found.
pub struct Target {
    pub hwnd: HWND,
    pub how: &'static str,
}

/// Locate the wallpaper layer, asking Progman to create it if needed.
pub fn find() -> Option<Target> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok()?;

        // Two argument forms are in circulation. The (0xD, 0x1) form is what
        // Explorer itself sends on Windows 10/11; the (0, 0) form is the
        // older one. Sending both costs nothing and covers every build seen
        // in the wild. The timeout matters: a hung Explorer must not hang us.
        for (wparam, lparam) in [(0xD_usize, 0x1_isize), (0, 0)] {
            let _ = SendMessageTimeoutW(
                progman,
                WM_SPAWN_WORKER,
                WPARAM(wparam),
                LPARAM(lparam),
                SMTO_NORMAL,
                1000,
                None,
            );
        }

        if let Ok(child) = FindWindowExW(Some(progman), None, w!("WorkerW"), None) {
            if covers_desktop(child) {
                return Some(Target {
                    hwnd: child,
                    how: "WorkerW (child of Progman)",
                });
            }
        }

        if let Some(sibling) = find_sibling_workerw() {
            if covers_desktop(sibling) {
                return Some(Target {
                    hwnd: sibling,
                    how: "WorkerW (sibling of the icon layer)",
                });
            }
        }

        // Some configurations never produce a usable WorkerW — a few Windows
        // 11 builds, and desktops running a shell replacement. Progman itself
        // also sits behind the icons; we just share a window with the shell.
        Some(Target {
            hwnd: progman,
            how: "Progman (no usable WorkerW)",
        })
    }
}

/// Reject the invisible stubs. The real wallpaper window spans the whole
/// virtual desktop; the decoys are typically 166x47 and hidden.
fn covers_desktop(hwnd: HWND) -> bool {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return false;
    }

    // A real desktop is never this small, and a stub never this large.
    (rect.right - rect.left) >= 640 && (rect.bottom - rect.top) >= 480
}

static FOUND: AtomicIsize = AtomicIsize::new(0);

fn find_sibling_workerw() -> Option<HWND> {
    FOUND.store(0, Ordering::Relaxed);
    let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM(0)) };

    match FOUND.load(Ordering::Relaxed) {
        0 => None,
        handle => Some(HWND(handle as *mut _)),
    }
}

unsafe extern "system" fn enum_proc(top: HWND, _: LPARAM) -> BOOL {
    let has_icons = unsafe { FindWindowExW(Some(top), None, w!("SHELLDLL_DefView"), None) }.is_ok();

    if has_icons {
        // The sibling immediately behind `top` in z-order.
        if let Ok(worker) = unsafe { FindWindowExW(None, Some(top), w!("WorkerW"), None) } {
            FOUND.store(worker.0 as isize, Ordering::Relaxed);
            return false.into(); // stop enumerating
        }
    }

    true.into()
}
