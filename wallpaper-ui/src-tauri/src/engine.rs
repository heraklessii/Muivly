//! Starting the engine process.
//!
//! The engine is deliberately a separate process, so the UI launches it and
//! then forgets about it: closing the settings window must not take the
//! wallpaper down with it.

use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Start without a console window attached.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where to look for the engine, in order.
fn engine_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;

    let candidates = [
        // Installed: the engine sits next to the UI.
        dir.join("muivly-core.exe"),
        // Development: cargo puts both under the workspace target directory,
        // but the UI has its own workspace, so it is two levels up.
        dir.join("../../../../target/release/muivly-core.exe"),
        dir.join("../../../../target/debug/muivly-core.exe"),
    ];

    candidates.into_iter().find(|p| p.is_file())
}

#[tauri::command]
pub fn engine_installed() -> bool {
    engine_path().is_some()
}

#[tauri::command]
pub fn start_engine(video: Option<String>) -> Result<(), String> {
    let path = engine_path().ok_or("muivly-core.exe not found next to the app")?;

    let mut command = Command::new(path);
    if let Some(video) = video {
        command.arg(video);
    }

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the engine: {e}"))
}
