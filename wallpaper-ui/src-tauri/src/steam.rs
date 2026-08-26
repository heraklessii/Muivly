//! Importing what the user already owns in Wallpaper Engine.
//!
//! Nothing is copied, converted or phoned home about. Workshop items are
//! ordinary folders with an ordinary `project.json` and an ordinary video
//! next to it, and importing means adding that path to the Muivly library —
//! the same thing the "add files" button does, minus the hunting through
//! Steam folders nobody should have to do by hand.
//!
//! Only video and image wallpapers can come across. Wallpaper Engine also
//! has scene and web types, which are its own formats running its own
//! engine; those are skipped rather than imported and then found broken.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

/// Wallpaper Engine's Steam application id. Its workshop content lives under
/// a folder of this name in every Steam library.
const APP_ID: &str = "431960";

/// What the picker shows for one importable wallpaper.
#[derive(Serialize)]
pub struct Found {
    pub title: String,
    pub path: String,
    /// The preview image Wallpaper Engine generated, if there is one.
    pub preview: Option<String>,
}

/// The fields of `project.json` this needs. Everything else in that file
/// belongs to Wallpaper Engine.
#[derive(Deserialize)]
struct Project {
    file: Option<String>,
    title: Option<String>,
    preview: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

/// Every video or image wallpaper found in the Wallpaper Engine workshop
/// folders, across all Steam libraries.
#[tauri::command(async)]
pub fn scan_wallpaper_engine() -> Result<Vec<Found>, String> {
    let libraries = steam_libraries();
    if libraries.is_empty() {
        return Err("Steam not found on this machine".to_string());
    }

    let mut found = Vec::new();
    for library in libraries {
        let workshop = library
            .join("steamapps")
            .join("workshop")
            .join("content")
            .join(APP_ID);

        let Ok(entries) = std::fs::read_dir(&workshop) else {
            continue;
        };

        for entry in entries.flatten() {
            if let Some(item) = read_project(&entry.path()) {
                found.push(item);
            }
        }
    }

    // Alphabetical: the workshop ids these arrive in are meaningless to a
    // person looking for one wallpaper they remember by name.
    found.sort_by_key(|item| item.title.to_lowercase());
    Ok(found)
}

/// Read one workshop folder, or `None` if it holds nothing Muivly can play.
fn read_project(folder: &Path) -> Option<Found> {
    let text = std::fs::read_to_string(folder.join("project.json")).ok()?;
    let project: Project = serde_json::from_str(&text).ok()?;

    // Scene and web wallpapers are Wallpaper Engine's own runtime; there is
    // nothing here that could play them.
    if !matches!(
        project.kind.as_deref(),
        Some("video") | Some("image") | None
    ) {
        return None;
    }

    let file = folder.join(project.file?);
    if !file.is_file() {
        return None;
    }
    // A scene wallpaper can still name a `.pkg` in `file`; the extension is
    // the honest test, not the declared type.
    if !playable(&file) {
        return None;
    }

    let title = project.title.unwrap_or_else(|| {
        folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let preview = project
        .preview
        .map(|name| folder.join(name))
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string());

    Some(Found {
        title,
        path: file.display().to_string(),
        preview,
    })
}

fn playable(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "mp4" | "webm" | "mkv" | "mov" | "m4v" | "avi" | "gif" | "png" | "jpg" | "jpeg"
    )
}

/// Every Steam library folder, starting with the install itself.
///
/// Steam keeps its own list in `libraryfolders.vdf`, which is where games on
/// a second drive live. Reading only the install folder would miss most
/// people's workshop content.
fn steam_libraries() -> Vec<PathBuf> {
    let Some(root) = steam_path() else {
        return Vec::new();
    };

    let mut libraries = vec![root.clone()];

    let manifest = root.join("steamapps").join("libraryfolders.vdf");
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return libraries;
    };

    // VDF is Valve's own key-value format. Parsing it properly would be a
    // dependency for one line: every library folder is a `"path"` entry, and
    // that is the only thing here worth reading.
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("\"path\"") else {
            continue;
        };
        let Some(start) = rest.find('"') else {
            continue;
        };
        let Some(end) = rest[start + 1..].find('"') else {
            continue;
        };

        let path = PathBuf::from(rest[start + 1..start + 1 + end].replace("\\\\", "\\"));
        if path.is_dir() && !libraries.contains(&path) {
            libraries.push(path);
        }
    }

    libraries
}

fn steam_path() -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;

    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Valve\Steam"),
            PCWSTR(w!("SteamPath").as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }

    let length = (size as usize / 2).saturating_sub(1);
    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
    path.is_dir().then_some(path)
}
