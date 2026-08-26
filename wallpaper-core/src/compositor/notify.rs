//! The one window Muivly owns that is not a wallpaper.
//!
//! Everything the engine draws into is a *child* window of WorkerW, and child
//! windows are not told about the things the engine most needs to know:
//! Windows broadcasts "the displays changed" and "the machine woke up" to
//! top-level windows only. Global hotkeys have the same shape — `RegisterHotKey`
//! delivers `WM_HOTKEY` to a window, and it cannot be a child of the desktop.
//!
//! So there is one hidden, zero-sized top-level window whose only job is to
//! receive those messages and leave a flag behind. It never paints, never
//! appears in Alt+Tab, and costs a window handle.
//!
//! The flags are atomics rather than a channel because the render loop reads
//! them once a pass and does not care how many times a thing happened — a
//! display that changed three times while a game was fullscreen still needs
//! exactly one rebuild.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::RegisterSuspendResumeNotification;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_M,
    VK_P, VK_RIGHT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, DEVICE_NOTIFY_WINDOW_HANDLE,
    WM_DISPLAYCHANGE, WM_HOTKEY, WM_POWERBROADCAST, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("MuivlyNotify");

/// `WM_POWERBROADCAST`'s "the machine has woken up".
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
const PBT_APMRESUMESUSPEND: usize = 0x0007;

/// Hotkey identifiers. Local to this window, so the numbers only have to be
/// distinct from each other.
const HOTKEY_NEXT: i32 = 1;
const HOTKEY_PAUSE: i32 = 2;
const HOTKEY_MUTE: i32 = 3;

static DISPLAY_CHANGED: AtomicBool = AtomicBool::new(false);
static RESUMED: AtomicBool = AtomicBool::new(false);
static NEXT_PRESSED: AtomicBool = AtomicBool::new(false);
static PAUSE_PRESSED: AtomicBool = AtomicBool::new(false);
static MUTE_PRESSED: AtomicBool = AtomicBool::new(false);

/// What the render loop should do about messages that arrived since it last
/// asked. Every field is one-shot: reading clears it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    /// A monitor was plugged in, unplugged, or had its resolution changed.
    /// Every swap chain is sized to a screen that may no longer exist.
    pub display_changed: bool,
    /// The machine came back from sleep. The device may have been reset
    /// underneath us, and the clock certainly jumped.
    pub resumed: bool,
    pub next: bool,
    pub pause: bool,
    pub mute: bool,
}

impl Signals {
    pub fn any(&self) -> bool {
        self.display_changed || self.resumed || self.next || self.pause || self.mute
    }
}

/// The hidden window, alive for as long as the engine is.
pub struct Notifier {
    hwnd: HWND,
    hotkeys: bool,
}

impl Notifier {
    pub fn create() -> windows::core::Result<Self> {
        unsafe {
            register_class()?;

            let instance = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("Muivly"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                // Not HWND_MESSAGE: a message-only window is exactly what
                // this looks like, and it is the one thing that would not
                // work — message-only windows are not sent the broadcasts
                // this window exists to receive.
                None,
                None,
                Some(instance.into()),
                None,
            )?;

            // Sleep and resume. Without this the engine wakes with swap
            // chains whose device may have been reset, and a clock that
            // believes several hours of video are owed.
            let _ = RegisterSuspendResumeNotification(
                windows::Win32::Foundation::HANDLE(hwnd.0),
                DEVICE_NOTIFY_WINDOW_HANDLE,
            );

            Ok(Self {
                hwnd,
                hotkeys: false,
            })
        }
    }

    /// Claim, or release, the three desktop-wide shortcuts.
    ///
    /// Registration fails when another application already owns a
    /// combination — which is not an error worth stopping for. The user
    /// keeps whichever ones were free, and the setting stays on.
    pub fn set_hotkeys(&mut self, enabled: bool) {
        if self.hotkeys == enabled {
            return;
        }
        self.hotkeys = enabled;

        const KEYS: [(i32, u16); 3] = [
            (HOTKEY_NEXT, VK_RIGHT.0),
            (HOTKEY_PAUSE, VK_P.0),
            (HOTKEY_MUTE, VK_M.0),
        ];

        unsafe {
            for (id, key) in KEYS {
                if enabled {
                    // NOREPEAT: holding the keys down must not skip through
                    // a playlist at the keyboard's repeat rate.
                    let modifiers: HOT_KEY_MODIFIERS = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
                    if RegisterHotKey(Some(self.hwnd), id, modifiers, key as u32).is_err() {
                        eprintln!("hotkey {id}: already taken by another application");
                    }
                } else {
                    let _ = UnregisterHotKey(Some(self.hwnd), id);
                }
            }
        }
    }

    /// Everything that has happened since the last call.
    pub fn take(&self) -> Signals {
        Signals {
            display_changed: DISPLAY_CHANGED.swap(false, Ordering::Relaxed),
            resumed: RESUMED.swap(false, Ordering::Relaxed),
            next: NEXT_PRESSED.swap(false, Ordering::Relaxed),
            pause: PAUSE_PRESSED.swap(false, Ordering::Relaxed),
            mute: MUTE_PRESSED.swap(false, Ordering::Relaxed),
        }
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        self.set_hotkeys(false);
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
    match msg {
        WM_DISPLAYCHANGE => {
            DISPLAY_CHANGED.store(true, Ordering::Relaxed);
        }

        // A resolution change often arrives as a settings change rather than
        // a display change, and a work area that moved means a taskbar that
        // moved. Both are cheap to over-report: the loop compares the new
        // layout with the old one before rebuilding anything.
        WM_SETTINGCHANGE => {
            DISPLAY_CHANGED.store(true, Ordering::Relaxed);
        }

        WM_POWERBROADCAST => {
            if wp.0 == PBT_APMRESUMEAUTOMATIC || wp.0 == PBT_APMRESUMESUSPEND {
                RESUMED.store(true, Ordering::Relaxed);
            }
        }

        WM_HOTKEY => match wp.0 as i32 {
            HOTKEY_NEXT => NEXT_PRESSED.store(true, Ordering::Relaxed),
            HOTKEY_PAUSE => PAUSE_PRESSED.store(true, Ordering::Relaxed),
            HOTKEY_MUTE => MUTE_PRESSED.store(true, Ordering::Relaxed),
            _ => {}
        },

        _ => {}
    }

    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
