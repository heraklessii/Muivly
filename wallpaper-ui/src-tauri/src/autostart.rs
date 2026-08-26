//! Starting with Windows.
//!
//! The engine goes in the Run key, not the settings panel. The wallpaper is
//! what the user wants back at logon; a settings window they did not ask for
//! is an annoyance, and its WebView is a hundred megabytes charged to boot
//! for a panel nobody opened. The engine restores its own last session, so
//! it needs nothing from the UI to come up with the right wallpaper.
//!
//! HKCU, never HKLM: this is one user's choice about their own desktop, and
//! writing it machine-wide would need administrator rights Muivly should
//! never ask for.

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("Muivly");

#[tauri::command]
pub fn autostart_enabled() -> bool {
    read_run_value().is_some()
}

/// Turn the autostart entry on or off.
///
/// Rewritten rather than left alone when already on: the path changes when
/// the user moves or reinstalls the app, and a Run entry pointing at an
/// executable that is no longer there is worse than none.
#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if !enabled {
        return remove_run_value();
    }

    let engine = crate::engine::engine_path().ok_or("muivly-core.exe not found next to the app")?;
    // Quoted: the install path contains spaces on any normal machine, and an
    // unquoted Run entry is read as a command plus arguments.
    write_run_value(&format!("\"{}\"", engine.display()))
}

fn read_run_value() -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;

    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }

    let length = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buffer[..length]))
}

fn write_run_value(command: &str) -> Result<(), String> {
    let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };

    with_key(KEY_WRITE, |key| unsafe {
        RegSetValueExW(key, VALUE, None, REG_SZ, Some(bytes))
            .ok()
            .map_err(|e| e.message())
    })
}

fn remove_run_value() -> Result<(), String> {
    with_key(KEY_WRITE, |key| unsafe {
        // Already absent is the outcome asked for, not a failure.
        let _ = RegDeleteValueW(key, VALUE);
        Ok(())
    })
}

/// Open the Run key, hand it to `body`, and close it however that goes.
fn with_key<T>(
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
    body: impl FnOnce(HKEY) -> Result<T, String>,
) -> Result<T, String> {
    let mut key = HKEY::default();

    unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            None,
            access | KEY_READ,
            &mut key,
        )
        .ok()
        .map_err(|e| e.message())?;
    }

    let result = body(key);
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}
