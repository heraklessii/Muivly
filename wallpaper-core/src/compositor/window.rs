//! One borderless child window per monitor, parented to WorkerW.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, RegisterClassW, ShowWindow,
    HCURSOR, HICON, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNA, WNDCLASSW, WS_CHILD,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

use crate::caps::MonitorInfo;

const CLASS_NAME: PCWSTR = w!("MuivlySurface");

/// A window covering exactly one monitor, living behind the desktop icons.
///
/// Created hidden. A monitor only gets ours in front of the desktop once it
/// has something to play; until then Windows' own wallpaper is what should be
/// on screen.
pub struct Surface {
    pub hwnd: HWND,
    parent: HWND,
    pub monitor: MonitorInfo,
}

impl Surface {
    pub fn create(parent: HWND, monitor: &MonitorInfo) -> windows::core::Result<Self> {
        register_class()?;

        // Child coordinates are relative to WorkerW's client area, whose
        // origin is the top-left of the *virtual* desktop — not of the
        // primary monitor. A screen positioned left of the primary one has a
        // negative desktop x, and without this shift it would be placed off
        // the edge of the parent.
        let (vx, vy) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            )
        };

        let hwnd = unsafe {
            CreateWindowExW(
                // NOACTIVATE and TOOLWINDOW keep it out of Alt+Tab and stop
                // it from ever stealing focus. TRANSPARENT keeps mouse input
                // going to the desktop, so icons and right-click still work.
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                CLASS_NAME,
                w!("Muivly"),
                WS_CHILD,
                monitor.x - vx,
                monitor.y - vy,
                monitor.width as i32,
                monitor.height as i32,
                Some(parent),
                None,
                None,
                None,
            )?
        };

        Ok(Self {
            hwnd,
            parent,
            monitor: monitor.clone(),
        })
    }

    /// Put the surface in front of the desktop, or take it away.
    ///
    /// Hiding uncovers whatever the shell painted underneath. WorkerW does
    /// not always notice on its own, so the exposed area is invalidated —
    /// without that the last frame stays on screen and the monitor looks like
    /// it never turned off.
    pub fn set_visible(&self, visible: bool) {
        unsafe {
            let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            if !visible {
                let _ = InvalidateRect(Some(self.parent), None, true);
            }
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn register_class() -> windows::core::Result<()> {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    static mut RESULT: windows::core::Result<()> = Ok(());

    ONCE.call_once(|| unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => {
                RESULT = Err(e);
                return;
            }
        };

        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            // No background brush: every pixel comes from D3D. A brush here
            // would repaint the window white between frames and flicker.
            hbrBackground: HBRUSH::default(),
            hCursor: HCURSOR::default(),
            hIcon: HICON::default(),
            ..Default::default()
        };

        if RegisterClassW(&class) == 0 {
            RESULT = Err(windows::core::Error::from_thread());
        }
    });

    #[allow(static_mut_refs)]
    unsafe {
        RESULT.clone()
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
