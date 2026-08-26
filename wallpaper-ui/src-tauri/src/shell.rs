//! "Set as wallpaper" in Explorer's right-click menu.
//!
//! The shortest path from a video on disk to a wallpaper on screen: right
//! click, one item, done — no window, no library, no dragging a file into a
//! panel that has to be open first.
//!
//! Registered per file type under `SystemFileAssociations`, not under the
//! extension's own key. The difference matters: the extension key belongs to
//! whichever application owns `.mp4` today, and writing there would put
//! Muivly's entry inside VLC's menu one week and lose it the next.
//! `SystemFileAssociations` is where Windows itself keeps the verbs that
//! belong to a *kind* of file rather than to an application.
//!
//! HKCU only, for the same reason autostart is: this is one user's choice
//! about their own shell, and machine-wide would need rights Muivly must
//! never ask for.

use std::path::Path;

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

/// The file types the entry is offered for. Every one of them is something
/// the engine can actually show — offering it on a `.txt` would be a menu
/// item that can only fail.
const EXTENSIONS: [&str; 10] = [
    ".mp4", ".webm", ".mkv", ".mov", ".m4v", ".avi", ".gif", ".png", ".jpg", ".jpeg",
];

/// Our verb's key name, under `SystemFileAssociations\<ext>\shell`. Named
/// with the product so it cannot collide with a verb Windows or another
/// application owns.
const VERB: &str = "Muivly.SetWallpaper";

const MENU_TEXT: &str = "Muivly duvar kağıdı yap";

#[tauri::command]
pub fn context_menu_enabled() -> bool {
    // One extension is enough to answer: they are written and removed
    // together, so a partial state is not something the UI has to show.
    let path = format!(
        r"Software\Classes\SystemFileAssociations\{}\shell\{VERB}",
        EXTENSIONS[0]
    );
    open(&path).is_some()
}

/// Add the entry to Explorer's menu, or take it away.
///
/// Rewritten rather than left alone when already on, for the same reason the
/// autostart entry is: the path changes when the app is moved or
/// reinstalled, and a menu item pointing at an executable that is gone is
/// worse than no menu item.
#[tauri::command]
pub fn set_context_menu(enabled: bool) -> Result<(), String> {
    if !enabled {
        for extension in EXTENSIONS {
            remove(&format!(
                r"Software\Classes\SystemFileAssociations\{extension}\shell\{VERB}"
            ));
        }
        return Ok(());
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // `%1` is the file Explorer was right-clicked on, quoted because the
    // path will contain spaces.
    let command = format!("\"{}\" --set \"%1\"", exe.display());

    for extension in EXTENSIONS {
        let verb = format!(r"Software\Classes\SystemFileAssociations\{extension}\shell\{VERB}");
        write_default(&verb, MENU_TEXT)?;
        // The icon shown beside the menu text: the app's own.
        write_value(&verb, "Icon", &format!("\"{}\",0", exe.display()))?;
        write_default(&format!(r"{verb}\command"), &command)?;
    }

    Ok(())
}

/// What `--set <path>` on the command line should do.
///
/// Explorer starts a whole new process for a right-click, and that process
/// has one job: hand the path to the engine that is already running and get
/// out of the way — no window, no WebView, no tray icon. If no engine is
/// running, one is started first, because "set as wallpaper" on a machine
/// where Muivly is not running should still set the wallpaper.
///
/// Returns whether this really was a `--set` invocation, so `main` knows
/// whether to carry on and open the panel.
pub fn handle_cli() -> bool {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--set") {
        return false;
    }

    let Some(path) = args.next() else {
        eprintln!("--set needs a file path");
        return true;
    };

    if !Path::new(&path).is_file() {
        eprintln!("no such file: {path}");
        return true;
    }

    if !crate::pipe::engine_running() {
        if let Err(e) = crate::engine::start_engine(None) {
            eprintln!("could not start the engine: {e}");
            return true;
        }
        // The engine claims its pipe a moment after the process starts, and
        // there is nothing to wait on from out here.
        for _ in 0..40 {
            if crate::pipe::engine_running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    if let Err(e) = crate::pipe::set_everywhere(&path) {
        eprintln!("could not set the wallpaper: {e}");
    }

    true
}

fn open(path: &str) -> Option<HKEY> {
    let wide = wide(path);
    let mut key = HKEY::default();

    unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
        .ok()
        .ok()?;
        let _ = RegCloseKey(key);
    }

    Some(key)
}

fn remove(path: &str) {
    let wide = wide(path);
    unsafe {
        // Already absent is the outcome asked for, not a failure.
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide.as_ptr()));
    }
}

/// The key's unnamed value, which is what Explorer reads as the menu text
/// and as the command line.
fn write_default(path: &str, value: &str) -> Result<(), String> {
    write(path, w!(""), value)
}

fn write_value(path: &str, name: &str, value: &str) -> Result<(), String> {
    let name = wide(name);
    write(path, PCWSTR(name.as_ptr()), value)
}

fn write(path: &str, name: PCWSTR, value: &str) -> Result<(), String> {
    let path = wide(path);
    let data = wide(value);
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };

    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()
        .map_err(|e| e.message())?;

        let result = RegSetValueExW(key, name, None, REG_SZ, Some(bytes))
            .ok()
            .map_err(|e| e.message());
        let _ = RegCloseKey(key);
        result
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
