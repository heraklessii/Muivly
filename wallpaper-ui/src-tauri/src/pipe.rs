//! Client for the engine's named pipe.
//!
//! Windows exposes named pipes through the ordinary file API, so this needs
//! no Win32 code: opening `\\.\pipe\muivly` for read and write is the whole
//! connection. One connection per request keeps the engine's server simple
//! (it handles one client at a time) and means a UI crash cannot leave the
//! pipe occupied.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use serde::Serialize;

const PIPE: &str = r"\\.\pipe\muivly";

#[derive(Serialize)]
pub struct Status {
    pub fps: u32,
    pub paused: bool,
    /// None when the placeholder gradient is showing.
    pub video: Option<String>,
}

#[derive(Serialize)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub primary: bool,
    pub adapter: String,
}

fn connect() -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE)
        // The engine not running is the common case, not an exception —
        // the UI shows it as a state rather than an error.
        .map_err(|_| "engine not running".to_string())
}

/// Send one line, read one line back.
fn request(line: &str) -> Result<String, String> {
    let mut pipe = connect()?;
    writeln!(pipe, "{line}").map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(pipe);
    let mut response = String::new();
    reader.read_line(&mut response).map_err(|e| e.to_string())?;

    let response = response.trim().to_string();
    match response.strip_prefix("err ") {
        Some(message) => Err(message.to_string()),
        None => Ok(response),
    }
}

#[tauri::command]
pub fn status() -> Result<Status, String> {
    let response = request("status")?;

    let mut status = Status {
        fps: 0,
        paused: false,
        video: None,
    };

    // `ok fps=30 paused=false video=C:\some path\clip.mp4`
    // The video value can contain spaces, so it is taken as the remainder.
    for field in response.trim_start_matches("ok").trim().splitn(3, ' ') {
        match field.split_once('=') {
            Some(("fps", value)) => status.fps = value.parse().unwrap_or(0),
            Some(("paused", value)) => status.paused = value == "true",
            Some(("video", value)) if value != "-" => status.video = Some(value.to_string()),
            _ => {}
        }
    }

    Ok(status)
}

#[tauri::command]
pub fn monitors() -> Result<Vec<Monitor>, String> {
    let mut pipe = connect()?;
    writeln!(pipe, "monitors").map_err(|e| e.to_string())?;

    let reader = BufReader::new(pipe);
    let mut monitors = Vec::new();

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line == "end" {
            break;
        }

        // `monitor <name> <x> <y> <w> <h> <hz> <primary> <adapter...>`
        // The adapter name is last because it is the only field with spaces.
        let parts: Vec<&str> = line.splitn(9, ' ').collect();
        if parts.len() < 9 || parts[0] != "monitor" {
            continue;
        }

        monitors.push(Monitor {
            name: parts[1].to_string(),
            x: parts[2].parse().unwrap_or(0),
            y: parts[3].parse().unwrap_or(0),
            width: parts[4].parse().unwrap_or(0),
            height: parts[5].parse().unwrap_or(0),
            refresh_hz: parts[6].parse().unwrap_or(60),
            primary: parts[7] == "true",
            adapter: parts[8].to_string(),
        });
    }

    Ok(monitors)
}

#[tauri::command]
pub fn set_video(path: String) -> Result<(), String> {
    request(&format!("set {path}")).map(|_| ())
}

#[tauri::command]
pub fn clear_video() -> Result<(), String> {
    request("clear").map(|_| ())
}

#[tauri::command]
pub fn set_fps(fps: u32) -> Result<(), String> {
    request(&format!("fps {fps}")).map(|_| ())
}

#[tauri::command]
pub fn quit_engine() -> Result<(), String> {
    request("quit").map(|_| ())
}
