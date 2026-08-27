//! Taking the Windows accent colour from the wallpaper.
//!
//! Off by default and reversible by design. Everything written here is under
//! `HKEY_CURRENT_USER`, exactly as the project's rules require — no machine
//! keys, no administrator, nothing a user cannot undo. What was there before
//! is written to a file next to the session first, and put back when the
//! setting is switched off, when the engine quits, or on the next start if
//! the engine was killed before it could.
//!
//! The colour comes from the wallpaper the primary monitor is showing, read
//! back from the GPU at 16 by 9 — see `Renderer::dominant_colour`. It is
//! taken when the wallpaper changes, not when a frame is drawn.
//!
//! One honest caveat, kept out of the marketing: the palette Windows uses for
//! the taskbar and Start (`AccentPalette`) has no documented format. What is
//! written here is the layout every tool that does this has settled on, and
//! it is the reason this is a setting somebody switches on rather than
//! something Muivly does on its own.

use std::path::PathBuf;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY, REG_DWORD, REG_OPTION_NON_VOLATILE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};

/// The two keys Windows keeps its accent colour in. Both under the user.
const DWM: &str = r"Software\Microsoft\Windows\DWM";
const EXPLORER: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";

/// Every value this touches, so the backup and the restore cannot drift apart.
/// `true` marks the one value that is bytes rather than a number.
const VALUES: &[(&str, &str, bool)] = &[
    (DWM, "AccentColor", false),
    (DWM, "AccentColorInactive", false),
    (DWM, "ColorizationColor", false),
    (DWM, "ColorizationAfterglow", false),
    (EXPLORER, "AccentColorMenu", false),
    (EXPLORER, "StartColorMenu", false),
    (EXPLORER, "AccentPalette", true),
];

/// Where the values that were there before are kept.
///
/// Next to the session file, so an uninstall takes it with everything else,
/// and on disk rather than in memory so a crash does not leave somebody with
/// a colour scheme they never chose and no way back to their own.
fn backup_path() -> Option<PathBuf> {
    crate::session::path().map(|path| path.with_file_name("accent-backup.txt"))
}

/// Put this colour on the desktop's chrome.
///
/// The first call also writes the backup. Later calls do not: the first one
/// is the only one that saw the user's own colours.
pub fn apply(rgb: [u8; 3]) {
    if backup_path().is_some_and(|path| !path.exists()) {
        save_backup();
    }

    let rgb = presentable(rgb);
    let abgr = pack_abgr(rgb, 0xFF);

    write_dword(DWM, "AccentColor", abgr);
    write_dword(DWM, "AccentColorInactive", pack_abgr(dim(rgb, 0.6), 0xFF));
    // Colorization is the other way round, and carries an alpha Windows
    // treats as the intensity of the tint on title bars.
    write_dword(DWM, "ColorizationColor", pack_argb(rgb, 0xC4));
    write_dword(DWM, "ColorizationAfterglow", pack_argb(rgb, 0xC4));
    write_dword(EXPLORER, "AccentColorMenu", abgr);
    write_dword(EXPLORER, "StartColorMenu", pack_abgr(dim(rgb, 0.75), 0xFF));
    write_binary(EXPLORER, "AccentPalette", &palette(rgb));

    announce();
}

/// Put back whatever was there before Muivly touched it, and forget the
/// backup. Safe to call when nothing was ever applied.
pub fn restore() {
    let Some(path) = backup_path() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };

    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let Some((key, name, binary)) = VALUES
            .iter()
            .find(|(_, known, _)| *known == name)
            .map(|(key, name, binary)| (*key, *name, *binary))
        else {
            continue;
        };

        // A dash is how "there was no such value" is written down, and
        // deleting is the only honest way to restore that.
        if value == "-" {
            delete_value(key, name);
        } else if binary {
            match decode_hex(value) {
                Some(bytes) => write_binary(key, name, &bytes),
                None => delete_value(key, name),
            }
        } else {
            match u32::from_str_radix(value, 16) {
                Ok(number) => write_dword(key, name, number),
                Err(_) => delete_value(key, name),
            }
        }
    }

    let _ = std::fs::remove_file(&path);
    announce();
}

/// Whether a colour was applied and has not been put back yet. Read at
/// startup: an engine that was killed rather than closed left one behind.
pub fn is_applied() -> bool {
    backup_path().is_some_and(|path| path.exists())
}

fn save_backup() {
    let Some(path) = backup_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }

    let mut out = String::new();
    for (key, name, binary) in VALUES {
        let written = if *binary {
            read_binary(key, name).map(|bytes| encode_hex(&bytes))
        } else {
            read_dword(key, name).map(|number| format!("{number:08x}"))
        };
        out.push_str(&format!(
            "{name}={}\n",
            written.unwrap_or_else(|| "-".to_string())
        ));
    }

    let _ = std::fs::write(&path, out);
}

/// Tell everything on the desktop that the colours moved.
///
/// Not a guarantee: some of what Windows does with these values it does at
/// logon and nowhere else. Title bars and most applications pick it up from
/// here; the taskbar sometimes waits.
fn announce() {
    let setting: Vec<u16> = "ImmersiveColorSet"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(setting.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
    }
}

/// A colour somebody can read white text on.
///
/// The average of a wallpaper is very often nearly black or nearly white, and
/// either one as an accent means invisible chrome. This pulls the brightness
/// into a band that works against both themes and lifts a grey towards
/// whatever colour it did have, so the accent still reads as coming from the
/// picture.
///
/// A free function, and the only part of this file that is arithmetic rather
/// than registry calls — which makes it the part worth testing.
pub fn presentable(rgb: [u8; 3]) -> [u8; 3] {
    const FLOOR: f32 = 0.32;
    const CEILING: f32 = 0.70;
    /// How far a washed-out average is pushed back towards its own hue.
    const LIFT: f32 = 1.35;

    let channels = rgb.map(|c| c as f32 / 255.0);
    let mean = (channels[0] + channels[1] + channels[2]) / 3.0;

    // Away from grey first: the average of a whole photograph is nearly
    // always duller than anything actually in it.
    let saturated = channels.map(|c| (mean + (c - mean) * LIFT).clamp(0.0, 1.0));

    // The brightest channel decides, so pushing one into range does not turn
    // a blue into a grey by dragging the other two with it.
    let peak = saturated[0].max(saturated[1]).max(saturated[2]);
    let scale = if peak < FLOOR {
        // A near-black average has no direction worth keeping; anything
        // multiplied up from it is noise.
        if peak < 0.02 {
            return [0x2D, 0xD4, 0xBF];
        }
        FLOOR / peak
    } else if peak > CEILING {
        CEILING / peak
    } else {
        1.0
    };

    saturated.map(|c| ((c * scale).clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The same colour, darker. Used for the inactive title bar and for Start,
/// which Windows expects to be a shade rather than the accent itself.
fn dim(rgb: [u8; 3], amount: f32) -> [u8; 3] {
    rgb.map(|c| (c as f32 * amount).clamp(0.0, 255.0) as u8)
}

/// The eight shades Windows keeps for the taskbar and Start, dark to light.
///
/// The format is not documented anywhere Microsoft publishes. Eight entries
/// of four bytes, red first, with the last byte unused — which is what every
/// tool that writes this has converged on.
fn palette(rgb: [u8; 3]) -> [u8; 32] {
    const SHADES: [f32; 8] = [0.35, 0.5, 0.68, 0.85, 1.0, 1.2, 1.45, 0.18];

    let mut out = [0u8; 32];
    for (index, shade) in SHADES.iter().enumerate() {
        let colour = rgb.map(|c| ((c as f32 * shade).clamp(0.0, 255.0)) as u8);
        out[index * 4] = colour[0];
        out[index * 4 + 1] = colour[1];
        out[index * 4 + 2] = colour[2];
        out[index * 4 + 3] = 0;
    }
    out
}

/// `0xAABBGGRR`, which is how DWM and Explorer store a colour.
fn pack_abgr(rgb: [u8; 3], alpha: u8) -> u32 {
    (alpha as u32) << 24 | (rgb[2] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[0] as u32
}

/// `0xAARRGGBB`, which is how `ColorizationColor` stores the same thing.
fn pack_argb(rgb: [u8; 3], alpha: u8) -> u32 {
    (alpha as u32) << 24 | (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Open one of our two keys for reading and writing, creating it if Windows
/// has not.
fn open(path: &str, access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let path = wide(path);
    let mut key = HKEY::default();

    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            access,
            None,
            &mut key,
            None,
        )
    };

    (status == ERROR_SUCCESS).then_some(key)
}

fn write_dword(path: &str, name: &str, value: u32) {
    let Some(key) = open(path, KEY_WRITE) else {
        return;
    };
    let name = wide(name);
    unsafe {
        let _ = RegSetValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            REG_DWORD,
            Some(&value.to_le_bytes()),
        );
        let _ = RegCloseKey(key);
    }
}

fn write_binary(path: &str, name: &str, value: &[u8]) {
    let Some(key) = open(path, KEY_WRITE) else {
        return;
    };
    let name = wide(name);
    unsafe {
        let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_BINARY, Some(value));
        let _ = RegCloseKey(key);
    }
}

fn delete_value(path: &str, name: &str) {
    let Some(key) = open(path, KEY_WRITE) else {
        return;
    };
    let name = wide(name);
    unsafe {
        let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
        let _ = RegCloseKey(key);
    }
}

fn read_dword(path: &str, name: &str) -> Option<u32> {
    let bytes = read_binary(path, name)?;
    let [a, b, c, d] = bytes[..4].try_into().ok()?;
    Some(u32::from_le_bytes([a, b, c, d]))
}

fn read_binary(path: &str, name: &str) -> Option<Vec<u8>> {
    let key = open(path, KEY_READ)?;
    let name = wide(name);

    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || size == 0 {
        unsafe {
            let _ = RegCloseKey(key);
        }
        return None;
    }

    let mut buffer = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }

    (status == ERROR_SUCCESS).then(|| {
        buffer.truncate(size as usize);
        buffer
    })
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// `HWND_BROADCAST` wants an `HWND` and the constant is one; named here so
/// the `unsafe` block above reads as a call rather than as a cast.
const _: fn() -> HWND = || HWND_BROADCAST;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dark_wallpaper_still_gives_a_visible_accent() {
        let accent = presentable([12, 14, 20]);
        let peak = accent.iter().copied().max().unwrap_or(0);
        assert!(peak >= 80, "got {accent:?}");
    }

    #[test]
    fn a_bright_wallpaper_is_brought_down_rather_than_left_white() {
        let accent = presentable([250, 248, 245]);
        let peak = accent.iter().copied().max().unwrap_or(255);
        assert!(peak <= 190, "got {accent:?}");
    }

    /// The whole point is that the accent comes from the picture. A blue
    /// wallpaper must not come back grey, however much its brightness moved.
    #[test]
    fn the_colour_survives_being_brought_into_range() {
        let accent = presentable([20, 30, 90]);
        assert!(accent[2] > accent[0], "got {accent:?}");
        assert!(accent[2] > accent[1], "got {accent:?}");
    }

    /// Black has no hue to keep, and multiplying it up produces whatever
    /// rounding error happened to be in the average.
    #[test]
    fn a_black_wallpaper_falls_back_to_the_product_colour() {
        assert_eq!(presentable([0, 0, 0]), [0x2D, 0xD4, 0xBF]);
    }

    #[test]
    fn the_two_packings_are_not_the_same_one_twice() {
        let rgb = [0x11, 0x22, 0x33];
        assert_eq!(pack_abgr(rgb, 0xFF), 0xFF_33_22_11);
        assert_eq!(pack_argb(rgb, 0xC4), 0xC4_11_22_33);
    }

    #[test]
    fn the_palette_is_eight_shades_of_the_one_colour() {
        let bytes = palette([100, 50, 25]);
        assert_eq!(bytes.len(), 32);
        // Dark to light across the first five entries: the ramp is what
        // Windows reads as one accent rather than eight colours.
        for index in 0..4 {
            assert!(bytes[index * 4] < bytes[(index + 1) * 4], "at {index}");
        }
    }

    /// The backup is written as text and read back as bytes. A round trip
    /// that loses a byte is a colour scheme somebody cannot get back.
    #[test]
    fn a_backup_value_round_trips() {
        let bytes: Vec<u8> = (0..32).collect();
        assert_eq!(decode_hex(&encode_hex(&bytes)), Some(bytes));
        assert_eq!(decode_hex("odd"), None);
    }
}
