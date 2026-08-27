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

use serde::{Deserialize, Serialize};

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

/// One automation rule as the frontend sees it.
///
/// The engine's wire form is terse (`t420|C:\\x.mp4`); this is the same
/// thing with the parts named, because a settings panel needs the pieces
/// separately and string surgery in TypeScript is where the bugs live.
#[derive(Serialize, Deserialize, Clone)]
pub struct Rule {
    /// `time` or `theme`.
    pub kind: String,
    /// Minutes since midnight for `time`; 1 for the dark theme, 0 for light.
    pub value: u32,
    pub items: Vec<String>,
}

/// One saved arrangement of wallpapers across the screens.
#[derive(Serialize, Deserialize, Clone)]
pub struct Scene {
    pub name: String,
    /// Device name, and what that screen was showing.
    pub monitors: Vec<(String, Vec<String>)>,
}

/// One setting a shader file declares for itself, and where it is set.
#[derive(Serialize, Clone)]
pub struct ShaderParam {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub value: f32,
    pub label: String,
}

/// A shader on screen, with the settings it asked for.
#[derive(Serialize, Clone)]
pub struct ShaderFile {
    pub path: String,
    pub params: Vec<ShaderParam>,
}

/// A rewrite in progress, or the one that just finished.
#[derive(Serialize)]
pub struct Optimize {
    pub source: String,
    /// 0-100.
    pub percent: u32,
    /// Where the smaller copy landed, once it has.
    pub output: Option<String>,
    pub error: Option<String>,
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
    /// How long out of sight before the engine hands its decoders back;
    /// 0 keeps them open.
    pub hibernate_secs: u64,
    /// Whether they are handed back right now.
    pub hibernating: bool,
    /// How far the wallpaper answers to sound, and to the cursor. 0-1 each.
    pub reactive: f32,
    pub parallax: f32,
    /// A memory budget in megabytes; 0 is none.
    pub memory_mb: u32,
    /// Applications that freeze the wallpaper while they are in front.
    pub apps: Vec<String>,
    /// Wallpapers that change themselves, by clock or by theme.
    pub rules: Vec<Rule>,
    /// Named arrangements of wallpapers across the screens.
    pub scenes: Vec<Scene>,
    /// The settings each shader on screen declares for itself.
    pub shaders: Vec<ShaderFile>,
    /// How long the machine may sit untouched before the wallpaper stands
    /// still, and whether it is standing still now.
    pub idle_secs: u64,
    pub away: bool,
    /// The frame rate while the machine is busy with something else, whether
    /// it is, and how busy the last sample found it.
    pub busy_fps: u32,
    pub busy: bool,
    pub load: f32,
    /// Whether Windows' reduce-motion setting is honoured.
    pub reduce_motion: bool,
    /// How far a photograph drifts on its own, 0-1.
    pub drift: f32,
    /// Whether the Windows accent colour follows the wallpaper.
    pub accent: bool,
    /// How long the engine has been up, and how much of that it spent
    /// drawing nothing at all.
    pub uptime_secs: u64,
    pub resting_secs: u64,
    /// A clip being rewritten smaller, if one is.
    pub optimize: Option<Optimize>,
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
            hibernate_secs: 20,
            hibernating: false,
            reactive: 0.0,
            parallax: 0.0,
            memory_mb: 0,
            apps: Vec::new(),
            rules: Vec::new(),
            scenes: Vec::new(),
            shaders: Vec::new(),
            idle_secs: 300,
            away: false,
            busy_fps: 10,
            busy: false,
            load: 0.0,
            reduce_motion: true,
            drift: 0.0,
            accent: false,
            uptime_secs: 0,
            resting_secs: 0,
            optimize: None,
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
                    Some(("hibernate", v)) => status.hibernate_secs = v.parse().unwrap_or(20),
                    Some(("hibernating", v)) => status.hibernating = v == "true",
                    Some(("reactive", v)) => status.reactive = v.parse().unwrap_or(0.0),
                    Some(("parallax", v)) => status.parallax = v.parse().unwrap_or(0.0),
                    Some(("memory", v)) => status.memory_mb = v.parse().unwrap_or(0),
                    Some(("batfps", v)) => status.battery_fps = v.parse().unwrap_or(24),
                    Some(("batfreeze", v)) => status.pause_on_saver = v == "true",
                    Some(("battery", v)) => status.on_battery = v == "true",
                    Some(("saver", v)) => status.saver = v == "true",
                    Some(("charge", v)) => status.battery_percent = v.parse().unwrap_or(100),
                    Some(("cpu", v)) => status.cpu = v.parse().unwrap_or(0.0),
                    Some(("ram", v)) => status.ram_mb = v.parse().unwrap_or(0),
                    Some(("realfps", v)) => status.real_fps = v.parse().unwrap_or(0.0),
                    Some(("idle", v)) => status.idle_secs = v.parse().unwrap_or(300),
                    Some(("away", v)) => status.away = v == "true",
                    Some(("busyfps", v)) => status.busy_fps = v.parse().unwrap_or(10),
                    Some(("busy", v)) => status.busy = v == "true",
                    Some(("load", v)) => status.load = v.parse().unwrap_or(0.0),
                    Some(("reducemotion", v)) => status.reduce_motion = v == "true",
                    Some(("drift", v)) => status.drift = v.parse().unwrap_or(0.0),
                    Some(("accent", v)) => status.accent = v == "true",
                    Some(("uptime", v)) => status.uptime_secs = v.parse().unwrap_or(0),
                    Some(("resting", v)) => status.resting_secs = v.parse().unwrap_or(0),
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

        // `apps <name>|<name>` and `rules <rule>;<rule>`, absent when empty.
        if let Some(list) = line.strip_prefix("apps ") {
            status.apps = list
                .split('|')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect();
            continue;
        }

        if let Some(text) = line.strip_prefix("rules ") {
            status.rules = parse_rules(text);
            continue;
        }

        // `scene <name>;<monitor>=<path>|<path>;...`, one line each.
        if let Some(text) = line.strip_prefix("scene ") {
            if let Some(scene) = parse_scene(text) {
                status.scenes.push(scene);
            }
            continue;
        }

        // `shader <path>|<name>,<min>,<max>,<default>,<value>,<label>|...`
        if let Some(text) = line.strip_prefix("shader ") {
            if let Some(shader) = parse_shader(text) {
                status.shaders.push(shader);
            }
            continue;
        }

        // `optimize <state> <percent> <source>|<detail>`
        if let Some(rest) = line.strip_prefix("optimize ") {
            status.optimize = parse_optimize(rest);
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
pub fn set_hibernate(seconds: u64) -> Result<(), String> {
    request(&format!("hibernate {seconds}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_motion(reactive: f32, parallax: f32) -> Result<(), String> {
    request(&format!("motion {reactive} {parallax}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_memory(megabytes: u32) -> Result<(), String> {
    request(&format!("memory {megabytes}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_apps(names: Vec<String>) -> Result<(), String> {
    request(&format!("apps {}", names.join("|"))).map(|_| ())
}

/// The whole rule list at once. Sending one rule at a time would need a way
/// to say which one, and the list is short enough that the whole panel is
/// the simpler message.
#[tauri::command(async)]
pub fn set_rules(rules: Vec<Rule>) -> Result<(), String> {
    let text = rules
        .iter()
        .filter(|rule| !rule.items.is_empty())
        .map(|rule| {
            let head = if rule.kind == "theme" {
                format!("d{}", if rule.value == 1 { 1 } else { 0 })
            } else {
                format!("t{}", rule.value.min(24 * 60 - 1))
            };
            format!("{head}|{}", rule.items.join("|"))
        })
        .collect::<Vec<_>>()
        .join(";");

    request(&format!("rules {text}")).map(|_| ())
}

/// Ask the engine to rewrite a clip at the size of the largest screen.
#[tauri::command(async)]
pub fn optimize(path: String) -> Result<(), String> {
    request(&format!("optimize {path}")).map(|_| ())
}

/// How long the machine may sit untouched before the wallpaper stands still.
#[tauri::command(async)]
pub fn set_idle(seconds: u64) -> Result<(), String> {
    request(&format!("idle {seconds}")).map(|_| ())
}

/// The frame rate to fall to while the machine is busy with something else.
#[tauri::command(async)]
pub fn set_busy_fps(fps: u32) -> Result<(), String> {
    request(&format!("busy {fps}")).map(|_| ())
}

#[tauri::command(async)]
pub fn set_reduce_motion(enabled: bool) -> Result<(), String> {
    request(&format!(
        "reducemotion {}",
        if enabled { "on" } else { "off" }
    ))
    .map(|_| ())
}

/// How far a photograph drifts on its own.
#[tauri::command(async)]
pub fn set_drift(drift: f32) -> Result<(), String> {
    request(&format!("drift {drift}")).map(|_| ())
}

/// Whether the Windows accent colour follows the wallpaper.
#[tauri::command(async)]
pub fn set_accent(enabled: bool) -> Result<(), String> {
    request(&format!("accent {}", if enabled { "on" } else { "off" })).map(|_| ())
}

/// One shader file's own settings, all of them at once.
#[tauri::command(async)]
pub fn set_shader_params(path: String, values: Vec<(String, f32)>) -> Result<(), String> {
    if path.contains(['\n', '\r', '|']) {
        return Err("a shader path cannot contain a line break or a pipe".to_string());
    }
    if values.is_empty() {
        return Ok(());
    }

    let fields: Vec<String> = values
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    request(&format!("shader {path}|{}", fields.join("|"))).map(|_| ())
}

/// Save what is on every screen under a name, recall it, or forget it.
#[tauri::command(async)]
pub fn scene(action: String, name: String) -> Result<(), String> {
    if !matches!(action.as_str(), "save" | "load" | "delete") {
        return Err("a scene is saved, loaded or deleted".to_string());
    }
    if name.is_empty() || name.contains([';', '|', '=', '\n', '\r']) {
        return Err("a scene name cannot contain ; | = or a line break".to_string());
    }
    request(&format!("scene {action} {name}")).map(|_| ())
}

/// `t420|C:\\a.mp4;d1|C:\\b.mp4` as something a settings panel can edit.
fn parse_rules(text: &str) -> Vec<Rule> {
    text.split(';')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let mut parts = entry.split('|');
            let head = parts.next()?;
            let (kind, value) = head.split_at_checked(1)?;
            let items: Vec<String> = parts
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect();

            Some(Rule {
                kind: match kind {
                    "t" => "time".to_string(),
                    "d" => "theme".to_string(),
                    _ => return None,
                },
                value: value.parse().ok()?,
                items,
            })
        })
        .collect()
}

/// `<name>;<monitor>=<path>|<path>;<monitor>=` as something a panel can show.
///
/// A monitor with nothing assigned is kept: clearing a screen is part of
/// what recalling an arrangement does.
fn parse_scene(text: &str) -> Option<Scene> {
    let mut parts = text.split(';');
    let name = parts.next()?.trim();
    if name.is_empty() {
        return None;
    }

    Some(Scene {
        name: name.to_string(),
        monitors: parts
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| {
                let (monitor, list) = entry.split_once('=')?;
                let items = list
                    .split('|')
                    .filter(|path| !path.is_empty())
                    .map(str::to_string)
                    .collect();
                Some((monitor.to_string(), items))
            })
            .collect(),
    })
}

/// `<path>|<name>,<min>,<max>,<default>,<value>,<label>|...`
///
/// The label is last inside each field because it is the only part that may
/// contain a comma or a space; everything before it is a number.
fn parse_shader(text: &str) -> Option<ShaderFile> {
    let mut parts = text.split('|');
    let path = parts.next().filter(|p| !p.is_empty())?;

    let params: Vec<ShaderParam> = parts
        .filter_map(|field| {
            let fields: Vec<&str> = field.splitn(6, ',').collect();
            let [name, min, max, default, value, label] = fields[..] else {
                return None;
            };
            Some(ShaderParam {
                name: name.to_string(),
                min: min.parse().ok()?,
                max: max.parse().ok()?,
                default: default.parse().ok()?,
                value: value.parse().ok()?,
                label: label.to_string(),
            })
        })
        .collect();

    (!params.is_empty()).then(|| ShaderFile {
        path: path.to_string(),
        params,
    })
}

/// `<state> <percent> <source>|<detail>`.
fn parse_optimize(rest: &str) -> Option<Optimize> {
    let mut parts = rest.splitn(3, ' ');
    let state = parts.next()?;
    let percent = parts.next()?.parse().unwrap_or(0);
    let (source, detail) = parts.next()?.split_once('|')?;

    Some(Optimize {
        source: source.to_string(),
        percent,
        // The detail field is the output path or the reason, depending on
        // the state — one slot, because only one of them is ever set.
        output: (state == "done").then(|| detail.to_string()),
        error: (state == "failed").then(|| detail.to_string()),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_rule_keeps_its_minute() {
        let rules = parse_rules(r"t420|C:\clips\day.mp4");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].kind, "time");
        assert_eq!(rules[0].value, 420);
        assert_eq!(rules[0].items, vec![r"C:\clips\day.mp4".to_string()]);
    }

    #[test]
    fn a_theme_rule_says_which_theme() {
        let rules = parse_rules(r"d1|C:\clips\dark.mp4;d0|C:\clips\light.mp4");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].kind, "theme");
        assert_eq!(rules[0].value, 1);
        assert_eq!(rules[1].value, 0);
    }

    #[test]
    fn a_playlist_rule_keeps_every_item_in_order() {
        // Paths are separated by `|`, which is also what separates them from
        // the trigger — so a list is where an off-by-one here would show.
        let rules = parse_rules(r"t0|a.mp4|b.mp4|c.mp4");
        assert_eq!(rules[0].items, vec!["a.mp4", "b.mp4", "c.mp4"]);
    }

    #[test]
    fn nonsense_is_skipped_rather_than_fatal() {
        // A malformed rule must not cost the user the rules around it: this
        // list is polled and re-rendered, not saved from here.
        assert!(parse_rules("").is_empty());
        assert!(parse_rules("wat|x.mp4").is_empty());
        assert_eq!(parse_rules("wat|x.mp4;t60|good.mp4").len(), 1);
    }

    #[test]
    fn a_scene_keeps_every_screen_including_the_empty_ones() {
        let scene = parse_scene(r"Gece;\\.\DISPLAY1=C:\a b.mp4|C:\c.mp4;\\.\DISPLAY2=").unwrap();
        assert_eq!(scene.name, "Gece");
        assert_eq!(scene.monitors.len(), 2);
        assert_eq!(scene.monitors[0].1.len(), 2);
        assert!(scene.monitors[1].1.is_empty());
    }

    #[test]
    fn a_scene_needs_a_name() {
        assert!(parse_scene("").is_none());
        assert!(parse_scene(";DISPLAY1=a.mp4").is_none());
    }

    /// The label is the only field with spaces or commas in it, which is why
    /// it is last and why the split has a limit.
    #[test]
    fn a_shader_setting_keeps_a_label_with_commas_in_it() {
        let shader =
            parse_shader(r"C:\s\bars.hlsl|glow,0,1,0.35,0.5,How far it spreads, roughly").unwrap();
        assert_eq!(shader.path, r"C:\s\bars.hlsl");
        assert_eq!(shader.params[0].name, "glow");
        assert_eq!(shader.params[0].value, 0.5);
        assert_eq!(shader.params[0].label, "How far it spreads, roughly");
    }

    #[test]
    fn a_shader_with_no_settings_is_not_reported_at_all() {
        assert!(parse_shader(r"C:\s\plain.hlsl").is_none());
        assert!(parse_shader("").is_none());
    }

    #[test]
    fn a_rewrite_in_progress_has_neither_an_output_nor_an_error() {
        let job = parse_optimize(r"running 42 C:\clips\rain.mp4|").unwrap();
        assert_eq!(job.percent, 42);
        assert_eq!(job.source, r"C:\clips\rain.mp4");
        assert!(job.output.is_none());
        assert!(job.error.is_none());
    }

    #[test]
    fn a_finished_rewrite_carries_where_it_landed() {
        let job = parse_optimize(r"done 100 C:\clips\rain.mp4|C:\out\rain 1920x1080.mp4").unwrap();
        assert_eq!(job.output.as_deref(), Some(r"C:\out\rain 1920x1080.mp4"));
        assert!(job.error.is_none());
    }

    #[test]
    fn a_failed_rewrite_carries_the_reason() {
        // The reason has spaces in it, which is why the detail field is last
        // and split off by `|` rather than by whitespace.
        let job = parse_optimize(r"failed 0 C:\clips\rain.mp4|no hardware encoder here").unwrap();
        assert_eq!(job.error.as_deref(), Some("no hardware encoder here"));
        assert!(job.output.is_none());
    }
}
