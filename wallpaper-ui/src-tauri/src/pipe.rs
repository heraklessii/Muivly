//! Client for the engine's named pipe.
//!
//! Windows exposes named pipes through the ordinary file API, so this needs
//! no Win32 code: opening `\\.\pipe\muivly` for read and write is the whole
//! connection. One connection per request keeps the engine's server simple
//! and means a UI crash cannot leave the pipe occupied.
//!
//! Every command here is declared `#[tauri::command(async)]`. Tauri runs a
//! plain synchronous command on the main thread, and each of these blocks on
//! pipe I/O — so on the default they hold the window's own thread for as
//! long as the engine takes to answer. The engine is fast, right up until it
//! is not (a display change, a file being opened off a sleeping drive), and
//! then the whole settings window stops repainting. Nothing in this file
//! touches the window, so none of it belongs on that thread.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};

const PIPE: &str = r"\\.\pipe\muivly";

/// Windows' "the pipe exists but every instance is in use".
const ERROR_PIPE_BUSY: i32 = 231;

/// How long to keep trying a busy pipe before calling it a failure.
///
/// The engine listens on several instances, so this is the second line of
/// defence rather than the first — but "engine not running" is what the user
/// sees when it is wrong, and that is too heavy a conclusion to draw from
/// two requests overlapping by a millisecond.
const BUSY_RETRIES: u32 = 10;
const BUSY_WAIT: std::time::Duration = std::time::Duration::from_millis(20);

use serde::Serialize;

/// What one monitor has chosen not to share with the others.
///
/// `None` in a field means "follow the desktop", which is what every monitor
/// does until someone opens the per-screen panel.
#[derive(Serialize, Default, Clone)]
pub struct Overrides {
    pub fit: Option<String>,
    pub fps: Option<u32>,
    pub brightness: Option<f32>,
    pub saturation: Option<f32>,
    pub blur: Option<f32>,
}

#[derive(Serialize)]
pub struct MonitorState {
    pub name: String,
    pub enabled: bool,
    /// Which item of the playlist is on screen.
    pub index: usize,
    pub items: Vec<String>,
    pub overrides: Overrides,
}

#[derive(Serialize)]
pub struct Status {
    pub fps: u32,
    pub paused: bool,
    /// Stopped on purpose rather than because nothing is visible.
    pub frozen: bool,
    pub fit: String,
    pub interval_secs: u64,
    pub brightness: f32,
    pub saturation: f32,
    pub blur: f32,
    pub sound: bool,
    pub volume: f32,
    /// Whether the soundtrack stands down for other applications...
    pub duck: bool,
    /// ...and whether it is doing so right now.
    pub ducking: bool,
    pub speed: f32,
    pub fade_ms: u64,
    pub span: bool,
    pub hotkeys: bool,
    /// The frame rate cap while unplugged; 0 means the same as plugged in.
    pub battery_fps: u32,
    pub pause_on_saver: bool,
    /// What the machine is running on at the moment.
    pub on_battery: bool,
    pub saver: bool,
    pub battery_percent: u8,
    /// Share of one core, 0-100.
    pub cpu: f32,
    pub ram_mb: u32,
    /// Frames actually presented per second, which is not the target fps.
    pub real_fps: f32,
    /// The last thing the engine could not do, in words meant for the user.
    pub error: Option<String>,
    pub monitors: Vec<MonitorState>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            fps: 0,
            paused: false,
            frozen: false,
            fit: "cover".to_string(),
            interval_secs: 0,
            brightness: 1.0,
            saturation: 1.0,
            blur: 0.0,
            sound: false,
            volume: 0.5,
            duck: true,
            ducking: false,
            speed: 1.0,
            fade_ms: 400,
            span: false,
            hotkeys: true,
            battery_fps: 24,
            pause_on_saver: true,
            on_battery: false,
            saver: false,
            battery_percent: 100,
            cpu: 0.0,
            ram_mb: 0,
            real_fps: 0.0,
            error: None,
            monitors: Vec::new(),
        }
    }
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
    for _ in 0..BUSY_RETRIES {
        match OpenOptions::new().read(true).write(true).open(PIPE) {
            Ok(file) => return Ok(file),
            // Busy means the engine is there and answering someone else —
            // the opposite of not running. Waiting a moment is the whole fix.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                std::thread::sleep(BUSY_WAIT);
            }
            Err(_) => break,
        }
    }

    // The engine not running is the common case, not an exception — the UI
    // shows it as a state rather than an error.
    Err("engine not running".to_string())
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

#[tauri::command(async)]
pub fn status() -> Result<Status, String> {
    let lines = request_lines("status")?;
    let mut status = Status::default();

    for line in lines {
        if let Some(rest) = line.strip_prefix("ok ") {
            // `fps=30 paused=false fit=cover interval=0`
            for field in rest.split(' ') {
                match field.split_once('=') {
                    Some(("fps", v)) => status.fps = v.parse().unwrap_or(0),
                    Some(("paused", v)) => status.paused = v == "true",
                    Some(("fit", v)) => status.fit = v.to_string(),
                    Some(("interval", v)) => status.interval_secs = v.parse().unwrap_or(0),
                    Some(("brightness", v)) => status.brightness = v.parse().unwrap_or(1.0),
                    Some(("saturation", v)) => status.saturation = v.parse().unwrap_or(1.0),
                    Some(("blur", v)) => status.blur = v.parse().unwrap_or(0.0),
                    Some(("sound", v)) => status.sound = v == "true",
                    Some(("volume", v)) => status.volume = v.parse().unwrap_or(0.5),
                    Some(("frozen", v)) => status.frozen = v == "true",
                    Some(("duck", v)) => status.duck = v == "true",
                    Some(("ducking", v)) => status.ducking = v == "true",
                    Some(("speed", v)) => status.speed = v.parse().unwrap_or(1.0),
                    Some(("fade", v)) => status.fade_ms = v.parse().unwrap_or(400),
                    Some(("span", v)) => status.span = v == "true",
                    Some(("hotkeys", v)) => status.hotkeys = v == "true",
                    Some(("batfps", v)) => status.battery_fps = v.parse().unwrap_or(24),
                    Some(("batfreeze", v)) => status.pause_on_saver = v == "true",
                    Some(("battery", v)) => status.on_battery = v == "true",
                    Some(("saver", v)) => status.saver = v == "true",
                    Some(("charge", v)) => status.battery_percent = v.parse().unwrap_or(100),
                    Some(("cpu", v)) => status.cpu = v.parse().unwrap_or(0.0),
                    Some(("ram", v)) => status.ram_mb = v.parse().unwrap_or(0),
                    Some(("realfps", v)) => status.real_fps = v.parse().unwrap_or(0.0),
                    _ => {}
                }
            }
            continue;
        }

        // `error <message>` — its own line because a message has spaces in
        // it and the key=value header above cannot carry one.
        if let Some(message) = line.strip_prefix("error ") {
            status.error = Some(message.to_string());
            continue;
        }

        // `own <name> <fit|-> <fps> <brightness|-1> <saturation> <blur>`
        //
        // Applied to a monitor already parsed, because the engine sends the
        // monitor lines first. A line for a monitor that is not there is
        // ignored rather than being an error: the two lists are built from
        // the same map, so it cannot happen, and if it did the wallpaper
        // would still be fine.
        if let Some(rest) = line.strip_prefix("own ") {
            let fields: Vec<&str> = rest.split(' ').collect();
            let [name, fit, fps, brightness, saturation, blur] = fields[..] else {
                continue;
            };
            let Some(monitor) = status.monitors.iter_mut().find(|m| m.name == name) else {
                continue;
            };

            monitor.overrides.fit = (fit != "-").then(|| fit.to_string());
            monitor.overrides.fps = fps.parse().ok().filter(|n| *n > 0);
            // Negative is how the engine writes "this screen has no grade of
            // its own"; a real brightness is never below zero.
            if let Some(value) = brightness.parse::<f32>().ok().filter(|b| *b >= 0.0) {
                monitor.overrides.brightness = Some(value);
                monitor.overrides.saturation = saturation.parse().ok();
                monitor.overrides.blur = blur.parse().ok();
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
            overrides: Overrides::default(),
        });
    }

    Ok(status)
}

#[tauri::command(async)]
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

#[tauri::command(async)]
pub fn set_playlist(monitor: String, items: Vec<String>) -> Result<(), String> {
    // The protocol is one message per line, so a path containing a newline
    // would not be one argument — it would be the rest of the message plus a
    // second command of the sender's choosing. Windows will not create such
    // a name, but these paths come out of a state file a user can edit, and
    // "cannot happen" is not the same as "is checked".
    if items.iter().any(|path| path.contains(['\n', '\r'])) {
        return Err("a wallpaper path cannot contain a line break".to_string());
    }

    request(&format!("set {monitor} {}", items.join("|"))).map(|_| ())
}

#[tauri::command(async)]
pub fn next_item(monitor: String) -> Result<(), String> {
    request(&format!("next {monitor}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_monitor_enabled(monitor: String, enabled: bool) -> Result<(), String> {
    request(&format!("enable {monitor} {enabled}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_fps(fps: u32) -> Result<(), String> {
    request(&format!("fps {fps}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_fit(fit: String) -> Result<(), String> {
    request(&format!("fit {fit}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_interval(seconds: u64) -> Result<(), String> {
    request(&format!("interval {seconds}")).map(|_| ())
}

#[tauri::command(async)]
pub fn quit_engine() -> Result<(), String> {
    request("quit").map(|_| ())
}

/// Whether an engine is listening. Cheaper than a request and it answers the
/// only question the launcher has.
pub fn engine_running() -> bool {
    connect().is_ok()
}

#[tauri::command(async)]
pub fn set_visual(brightness: f32, saturation: f32, blur: f32) -> Result<(), String> {
    request(&format!("visual {brightness} {saturation} {blur}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_sound(enabled: bool, volume: f32, duck: bool) -> Result<(), String> {
    let state = if enabled { "on" } else { "off" };
    request(&format!("sound {state} {volume} {duck}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_power(battery_fps: u32, pause_on_saver: bool) -> Result<(), String> {
    request(&format!("power {battery_fps} {pause_on_saver}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_speed(speed: f32) -> Result<(), String> {
    request(&format!("speed {speed}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_fade(milliseconds: u64) -> Result<(), String> {
    request(&format!("fade {milliseconds}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_span(span: bool) -> Result<(), String> {
    request(&format!("span {}", if span { "on" } else { "off" })).map(|_| ())
}

#[tauri::command(async)]
pub fn set_hotkeys(enabled: bool) -> Result<(), String> {
    request(&format!("hotkeys {}", if enabled { "on" } else { "off" })).map(|_| ())
}

#[tauri::command(async)]
pub fn set_frozen(frozen: bool) -> Result<(), String> {
    request(&format!("freeze {}", if frozen { "on" } else { "off" })).map(|_| ())
}

/// One monitor's own settings, all six fields at once.
///
/// `None` means "follow the desktop" and is sent as `-` (or 0 for the frame
/// rate). Sending the whole panel rather than a field at a time is what stops
/// a monitor ending up half-overridden when one call fails.
#[tauri::command(async)]
pub fn set_overrides(
    monitor: String,
    fit: Option<String>,
    fps: Option<u32>,
    brightness: Option<f32>,
    saturation: Option<f32>,
    blur: Option<f32>,
) -> Result<(), String> {
    let fit = fit.unwrap_or_else(|| "-".to_string());
    let fps = fps.unwrap_or(0);
    let (brightness, saturation, blur) = match brightness {
        Some(b) => (b, saturation.unwrap_or(1.0), blur.unwrap_or(0.0)),
        None => (-1.0, 1.0, 0.0),
    };

    request(&format!(
        "own {monitor} {fit} {fps} {brightness} {saturation} {blur}"
    ))
    .map(|_| ())
}

/// Skip every monitor to the next item in its playlist.
///
/// Used from the tray, where there is no room to ask which screen was meant
/// and the honest answer for most people is "all of them".
pub fn next_all() {
    let Ok(status) = status() else {
        return;
    };
    for monitor in status.monitors {
        let _ = request(&format!("next {}", monitor.name));
    }
}

/// Turn every monitor off, or back on again.
///
/// Off means the Windows wallpaper shows through and the engine stops
/// drawing entirely — which is what someone reaching for "pause" wants,
/// whether they are recording their screen or just want it still.
pub fn toggle_all() {
    let Ok(status) = status() else {
        return;
    };

    // Mixed states resolve to off: if any screen is still playing, the thing
    // being asked for is quiet, not more of it.
    let target = !status.monitors.iter().any(|m| m.enabled);
    for monitor in status.monitors {
        let _ = request(&format!("enable {} {target}", monitor.name));
    }
}

pub fn toggle_sound() {
    let Ok(status) = status() else {
        return;
    };
    let state = if status.sound { "off" } else { "on" };
    let _ = request(&format!("sound {state} {} {}", status.volume, status.duck));
}

/// Stop the wallpaper where it stands, or start it again.
///
/// Different from the tray's "pause", which switches every monitor off and
/// shows the Windows wallpaper. This one leaves the picture exactly where it
/// is and stops it moving — what someone recording their screen wants, and
/// what the Ctrl+Alt+P shortcut does.
pub fn toggle_freeze() {
    let _ = request("freeze toggle");
}

/// Point every monitor at one file. Used by the Explorer context menu, where
/// the user right-clicked a video and there was no room to ask which screen.
pub fn set_everywhere(path: &str) -> Result<(), String> {
    if path.contains(['\n', '\r', '|']) {
        return Err("a wallpaper path cannot contain a line break or a pipe".to_string());
    }

    let status = status()?;
    for monitor in status.monitors {
        request(&format!("set {} {path}", monitor.name))?;
    }
    Ok(())
}
