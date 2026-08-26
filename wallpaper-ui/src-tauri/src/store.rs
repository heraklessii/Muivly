//! The library, playlists, per-monitor assignments and settings, on disk.
//!
//! Stored as one JSON document under `%APPDATA%\Muivly\state.json`. The
//! schema lives in the frontend rather than here: this side only checks that
//! what it is handed is valid JSON and writes it atomically. That keeps a UI
//! change from needing a matching Rust change, and there is nothing here the
//! engine needs to understand — the engine is told about wallpapers through
//! the pipe, not through this file.

use std::fs;
use std::path::PathBuf;

fn state_dir() -> Result<PathBuf, String> {
    let base = std::env::var("APPDATA").map_err(|_| "APPDATA is not set".to_string())?;
    let dir = PathBuf::from(base).join("Muivly");
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

fn state_file() -> Result<PathBuf, String> {
    Ok(state_dir()?.join("state.json"))
}

#[tauri::command]
pub fn load_state() -> Result<Option<String>, String> {
    let path = state_file()?;
    if !path.exists() {
        // A first run is not an error; the UI starts from its own defaults.
        return Ok(None);
    }

    let raw =
        fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;

    // A byte order mark is legal UTF-8 and illegal JSON, and everything on
    // Windows that writes text files likes to add one. Editing the state file
    // by hand should not empty the library.
    let text = raw.strip_prefix('\u{feff}').unwrap_or(&raw).to_string();

    if serde_json::from_str::<serde_json::Value>(&text).is_err() {
        // Starting fresh over an unreadable file would let the next save
        // overwrite it, so it is moved aside first. Nothing is ever lost
        // without a copy left behind.
        let quarantine = path.with_file_name(format!(
            "state.corrupt-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        ));
        let _ = fs::rename(&path, &quarantine);
        eprintln!(
            "state.json was unreadable; kept a copy at {}",
            quarantine.display()
        );
        return Ok(None);
    }

    Ok(Some(text))
}

#[tauri::command]
pub fn save_state(json: String) -> Result<(), String> {
    // Refuse to persist something that cannot be read back. Without this a
    // malformed write would only surface on the next launch, as a library
    // that has silently emptied itself.
    serde_json::from_str::<serde_json::Value>(&json)
        .map_err(|e| format!("refusing to save invalid JSON: {e}"))?;

    let path = state_file()?;
    let temp = path.with_extension("json.tmp");

    // Write then rename: a crash mid-write leaves the previous state intact
    // rather than a half-written file.
    fs::write(&temp, json).map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    fs::rename(&temp, &path).map_err(|e| format!("could not replace {}: {e}", path.display()))?;

    Ok(())
}

/// Where the state file lives, for the settings screen to show.
#[tauri::command]
pub fn state_path() -> Result<String, String> {
    Ok(state_file()?.display().to_string())
}

/// Whether a file the library points at is still there. Files get moved and
/// deleted outside the app, and a library that shows entries which cannot
/// play is worse than one that marks them.
#[tauri::command]
pub fn file_exists(path: String) -> bool {
    PathBuf::from(path).is_file()
}
