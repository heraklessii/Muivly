//! Desktop window tree dump.
//!
//! The WorkerW arrangement differs between Windows builds and between
//! machines with the same build, and getting it wrong fails silently: the
//! windows are created, rendering succeeds, and nothing appears. This makes
//! the arrangement visible instead of guessed at.

use std::sync::Mutex;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetWindowRect, IsWindowVisible,
};

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Print every top-level window that takes part in drawing the desktop,
/// with its children.
pub fn dump() {
    println!("--- desktop window tree ---");

    LINES.lock().unwrap().clear();
    unsafe {
        let _ = EnumWindows(Some(top_level), LPARAM(0));
    }

    for line in LINES.lock().unwrap().iter() {
        println!("{line}");
    }
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len as usize])
}

fn describe(hwnd: HWND, indent: &str) -> String {
    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();

    format!(
        "{indent}{:>10} {:<20} visible={} rect=({},{})-({},{})",
        format!("{:#x}", hwnd.0 as isize),
        class_of(hwnd),
        visible,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom
    )
}

unsafe extern "system" fn top_level(hwnd: HWND, _: LPARAM) -> BOOL {
    let class = class_of(hwnd);

    if class == "WorkerW" || class == "Progman" || class == "MuivlySurface" {
        LINES.lock().unwrap().push(describe(hwnd, ""));
        unsafe {
            let _ = EnumChildWindows(Some(hwnd), Some(child), LPARAM(0));
        }
    }

    true.into()
}

unsafe extern "system" fn child(hwnd: HWND, _: LPARAM) -> BOOL {
    LINES.lock().unwrap().push(describe(hwnd, "    "));
    true.into()
}
