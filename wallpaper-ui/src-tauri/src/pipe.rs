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
pub struct MonitorState {
    pub name: String,
    pub enabled: bool,
    /// Which item of the playlist is on screen.
    pub index: usize,
    pub items: Vec<String>,
}

#[derive(Serialize)]
pub struct Status {
    pub fps: u32,
    pub paused: bool,
    pub fit: String,
    pub interval_secs: u64,
    pub monitors: Vec<MonitorState>,
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

/// Send one line, read lines until `end`.
fn request_lines(line: &str) -> Result<Vec<String>, String> {
    let mut pipe = connect()?;
    writeln!(pipe, "{line}").map_err(|e| e.to_string())?;

    let reader = BufReader::new(pipe);
    let mut lines = Vec::new();

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim().to_string();
        if line == "end" {
            break;
        }
        if let Some(message) = line.strip_prefix("err ") {
            return Err(message.to_string());
        }
        lines.push(line);
    }

    Ok(lines)
}

#[tauri::command]
pub fn status() -> Result<Status, String> {
    let lines = request_lines("status")?;
    let mut status = Status {
        fps: 0,
        paused: false,
        fit: "cover".to_string(),
        interval_secs: 0,
        monitors: Vec::new(),
    };

    for line in lines {
        if let Some(rest) = line.strip_prefix("ok ") {
            // `fps=30 paused=false fit=cover interval=0`
            for field in rest.split(' ') {
                match field.split_once('=') {
                    Some(("fps", v)) => status.fps = v.parse().unwrap_or(0),
                    Some(("paused", v)) => status.paused = v == "true",
                    Some(("fit", v)) => status.fit = v.to_string(),
                    Some(("interval", v)) => status.interval_secs = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
            continue;
        }

        // `monitor <name> <enabled> <index> <path>|<path>`
        // The list is last because paths contain spaces; it is split on `|`,
        // which Windows paths cannot contain.
        let parts: Vec<&str> = line.splitn(5, ' ').collect();
        if parts.len() < 4 || parts[0] != "monitor" {
            continue;
        }

        status.monitors.push(MonitorState {
            name: parts[1].to_string(),
            enabled: parts[2] == "true",
            index: parts[3].parse().unwrap_or(0),
            items: parts
                .get(4)
                .map(|list| {
                    list.split('|')
                        .filter(|p| !p.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }

    Ok(status)
}

#[tauri::command]
pub fn monitors() -> Result<Vec<Monitor>, String> {
    let lines = request_lines("monitors")?;
    let mut monitors = Vec::new();

    for line in lines {
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
pub fn set_playlist(monitor: String, items: Vec<String>) -> Result<(), String> {
    request(&format!("set {monitor} {}", items.join("|"))).map(|_| ())
}

#[tauri::command]
pub fn next_item(monitor: String) -> Result<(), String> {
    request(&format!("next {monitor}")).map(|_| ())
}

#[tauri::command]
pub fn set_monitor_enabled(monitor: String, enabled: bool) -> Result<(), String> {
    request(&format!("enable {monitor} {enabled}")).map(|_| ())
}

#[tauri::command]
pub fn set_fps(fps: u32) -> Result<(), String> {
    request(&format!("fps {fps}")).map(|_| ())
}

#[tauri::command]
pub fn set_fit(fit: String) -> Result<(), String> {
    request(&format!("fit {fit}")).map(|_| ())
}

#[tauri::command]
pub fn set_interval(seconds: u64) -> Result<(), String> {
    request(&format!("interval {seconds}")).map(|_| ())
}

#[tauri::command]
pub fn quit_engine() -> Result<(), String> {
    request("quit").map(|_| ())
}
