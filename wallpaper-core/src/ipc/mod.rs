//! Named pipe server. The settings UI is the only client.
//!
//! The protocol is one UTF-8 line per message, because the message set is
//! small enough that a serialisation library would cost more (binary size, a
//! dependency, a schema to keep in sync) than it saves. If this grows much
//! further, that trade flips — see docs/decisions.md.
//!
//! Requests:
//!   status                    -> `ok <key>=<value> ...`, then one
//!                                `monitor <name> <enabled> <index> <path>|...`
//!                                line per display, then one
//!                                `own <name> <fit> <fps> <bright> <sat> <blur>`
//!                                line per display with settings of its own,
//!                                then an optional `error <message>` line,
//!                                then `end`
//!   monitors                  -> one `monitor <name> <x> <y> <w> <h> <hz> <primary> <adapter>`
//!                                line per display, then `end`
//!   set <monitor> <path>|...  -> `ok`   (empty list clears; `|` separates a playlist)
//!   next <monitor>            -> `ok`
//!   enable <monitor> <bool>   -> `ok`
//!   fps <n>                   -> `ok`
//!   fit <cover|contain|stretch> -> `ok`
//!   interval <seconds>        -> `ok`   (0 = advance when the clip ends)
//!   shuffle <on|off>          -> `ok`   (play a list in a drawn order)
//!   visual <bright> <sat> <blur> -> `ok`
//!   sound <on|off> <volume> <duck> -> `ok`
//!   power <battery_fps> <freeze_on_saver> -> `ok`  (fps 0 = no separate rate)
//!   speed <rate>              -> `ok`   (0.25-2.0)
//!   fade <milliseconds>       -> `ok`   (0 = cut)
//!   span <on|off>             -> `ok`   (one wallpaper across every screen)
//!   hotkeys <on|off>          -> `ok`
//!   hibernate <seconds>       -> `ok`   (0 = keep the decoders open)
//!   optimize <path>           -> `ok` or `err busy`; progress arrives on
//!                                the `optimize` line of `status`
//!   motion <reactive> <parallax> -> `ok`   (0-1 each; 0 0 is off)
//!   memory <megabytes>        -> `ok`   (0 = no budget)
//!   apps <name>|<name>        -> `ok`   (freeze while one is in front)
//!   rules <rule>;<rule>       -> `ok`   (`t<minutes>|<path>...` by clock,
//!                                `d<0|1>|<path>...` by Windows theme;
//!                                an empty list clears them)
//!   idle <seconds>            -> `ok`   (stand still after this long with
//!                                no keyboard or mouse; 0 never)
//!   busy <fps>                -> `ok`   (the rate while the machine is busy
//!                                with something else; 0 keeps one rate)
//!   reducemotion <on|off>     -> `ok`   (honour Windows' animation setting)
//!   drift <0-1>               -> `ok`   (how far a photograph drifts)
//!   accent <on|off>           -> `ok`   (the Windows accent colour follows
//!                                the wallpaper; the user's own colours are
//!                                backed up and put back when this goes off)
//!   shader <path>|<name>=<v>  -> `ok`   (one shader file's own settings, as
//!                                declared by `// param` lines in the file;
//!                                they arrive on the `shader` status line)
//!   scene save <name>         -> `ok`   (what is on every screen, named)
//!   scene load <name>         -> `ok`
//!   scene delete <name>       -> `ok`
//!   freeze <on|off|toggle>    -> `ok`   (the last frame stays on screen)
//!   own <monitor> <fit|-> <fps> <bright|-> <sat> <blur> -> `ok`
//!                                ("follow the desktop" is `-` in a slot,
//!                                 0 for the frame rate, and either `-` or a
//!                                 negative number for the brightness —
//!                                 the status line and the session file both
//!                                 need a number in that slot)
//!   quit                      -> `ok`, then the engine shuts down
//!
//! Anything unrecognised gets `err unknown command`.
//!
//! Paths are separated by `|` rather than spaces, and never quoted: Windows
//! paths contain spaces constantly but never a pipe character.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::caps::GpuProfile;
use crate::compositor::{Fit, Motion, Overrides, Rule, Sound, Visual};
use crate::power::battery::PowerPolicy;

pub const PIPE_NAME: &str = r"\\.\pipe\muivly";

/// What the UI can ask the engine to do.
#[derive(Debug, Clone)]
pub enum Command {
    /// An empty list clears the monitor; one item is a fixed wallpaper; more
    /// is a playlist.
    SetPlaylist {
        monitor: String,
        items: Vec<PathBuf>,
    },
    SetEnabled {
        monitor: String,
        enabled: bool,
    },
    Next {
        monitor: String,
    },
    Fps(u32),
    SetFit(Fit),
    Interval(u64),
    /// Whether a playlist plays in a drawn order rather than as written.
    SetShuffle(bool),
    SetVisual(Visual),
    SetSound(Sound),
    /// What to do about running on a battery.
    SetPower(PowerPolicy),
    /// Playback rate, 1.0 being the speed the file was authored at.
    SetSpeed(f32),
    /// How long one wallpaper takes to replace another.
    SetFade(Duration),
    /// One wallpaper stretched across every screen.
    SetSpan(bool),
    SetHotkeys(bool),
    /// How long the desktop stays out of sight before the decoders are
    /// handed back. Zero keeps them open.
    SetHibernate(u64),
    /// How far the wallpaper answers to sound and to the cursor.
    SetMotion(Motion),
    /// A memory budget in megabytes; 0 leaves the tier's own cap alone.
    SetMemory(u32),
    /// Applications that freeze the wallpaper while they are in front.
    SetApps(Vec<String>),
    /// Wallpapers that change themselves, by clock or by theme.
    SetRules(Vec<Rule>),
    /// How long the machine may sit untouched before the wallpaper stands
    /// still. Zero never does.
    SetIdle(u64),
    /// The frame rate to fall to while the machine is busy with something
    /// else. Zero keeps one rate.
    SetBusyFps(u32),
    /// Whether to stand still when Windows asks for fewer animations.
    SetReduceMotion(bool),
    /// How far a photograph drifts on its own, 0-1.
    SetDrift(f32),
    /// Whether the Windows accent colour follows the wallpaper.
    SetAccent(bool),
    /// One shader file's own settings, by the names the file declared.
    SetShaderParams {
        path: PathBuf,
        values: HashMap<String, f32>,
    },
    /// Save what is on screen now under a name, recall it, or forget it.
    SaveScene(String),
    LoadScene(String),
    DeleteScene(String),
    /// Settings one monitor keeps for itself. The default value hands it
    /// back to the desktop's.
    SetOverrides {
        monitor: String,
        overrides: Overrides,
    },
    /// Stop everything moving without taking the wallpaper away.
    Freeze(bool),
    Quit,
}

#[derive(Debug, Clone)]
pub struct MonitorState {
    pub device_name: String,
    pub enabled: bool,
    pub index: usize,
    pub items: Vec<PathBuf>,
    /// What this screen has chosen not to share with the others.
    pub overrides: Overrides,
}

/// What the engine tells the UI about itself.
#[derive(Debug, Clone)]
pub struct Status {
    pub fps: u32,
    pub paused: bool,
    /// Stopped on purpose, rather than because nothing is visible.
    pub frozen: bool,
    pub fit: String,
    pub interval_secs: u64,
    /// Whether a playlist plays in a drawn order rather than as written.
    pub shuffle: bool,
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
    /// How long out of sight before the decoders are handed back; 0 never.
    pub hibernate_secs: u64,
    /// Whether they are handed back right now.
    pub hibernating: bool,
    /// How far the wallpaper answers to sound, and to the cursor.
    pub reactive: f32,
    pub parallax: f32,
    /// The memory budget in megabytes; 0 is none.
    pub memory_mb: u32,
    /// The application list and the rule list, in the same written form the
    /// commands take. Sent on their own lines: both contain spaces.
    pub apps: String,
    pub rules: String,
    /// One line per saved arrangement, in the form `scene` takes.
    pub scenes: Vec<String>,
    /// One line per shader on screen that declares settings of its own:
    /// `<path>|<name>,<min>,<max>,<default>,<value>,<label>|...`
    pub shaders: Vec<String>,
    /// How long the machine may sit untouched before the wallpaper stands
    /// still, and whether it is standing still right now.
    pub idle_secs: u64,
    pub away: bool,
    /// The frame rate while the machine is busy, whether it is, and how busy
    /// the last sample found it.
    pub busy_fps: u32,
    pub busy: bool,
    pub load: f32,
    /// Whether Windows' reduce-motion setting is honoured.
    pub reduce_motion: bool,
    /// How far a photograph drifts on its own, 0-1.
    pub drift: f32,
    /// Whether the Windows accent colour follows the wallpaper.
    pub accent: bool,
    /// How long this engine has been up, and how much of that it spent not
    /// drawing anything at all.
    pub uptime_secs: u64,
    pub resting_secs: u64,
    /// The frame rate cap while unplugged; 0 means the same as plugged in.
    pub battery_fps: u32,
    pub pause_on_saver: bool,
    /// What the machine is actually running on at the moment.
    pub on_battery: bool,
    pub saver: bool,
    pub battery_percent: u8,
    /// Share of one core, 0-100, measured by the engine about once a second.
    pub cpu: f32,
    pub ram_mb: u32,
    /// Frames actually presented per second, which is not the target fps:
    /// a 24 fps clip on a 60 fps setting presents 24 times.
    pub real_fps: f32,
    /// A clip being rewritten smaller, if one is. Sent on its own line for
    /// the same reason as `error`: it contains paths.
    pub optimize: Option<crate::optimize::Job>,
    /// The last thing that went wrong, in words meant for the user. Sent on
    /// its own line because a message contains spaces.
    pub error: Option<String>,
    pub monitors: Vec<MonitorState>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            fps: 0,
            paused: false,
            frozen: false,
            fit: Fit::default().name().to_string(),
            interval_secs: 0,
            shuffle: false,
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
            apps: String::new(),
            rules: String::new(),
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
            battery_fps: 24,
            pause_on_saver: true,
            on_battery: false,
            saver: false,
            battery_percent: 100,
            cpu: 0.0,
            ram_mb: 0,
            real_fps: 0.0,
            optimize: None,
            error: None,
            monitors: Vec::new(),
        }
    }
}

/// How many pipe instances listen at once.
///
/// One was a race the UI lost regularly. The client opens a fresh connection
/// per request and polls status on a timer, so a request the user triggers
/// lands on top of a poll often enough to notice — and with a single
/// instance the second one gets `ERROR_PIPE_BUSY`, which the UI can only
/// read as "the engine is not running". The same hole is open in the instant
/// between one conversation ending and the next instance being created.
///
/// Two spare listeners close both. They are created once and reused for the
/// life of the process — a thread per connection would be cheaper to write
/// and a thread created every 1.5 seconds forever to answer a poll.
const INSTANCES: usize = 3;

/// Start serving on background threads. Returns immediately.
pub fn serve(profile: GpuProfile, status: Arc<Mutex<Status>>, commands: Sender<Command>) {
    let profile = Arc::new(profile);

    for i in 0..INSTANCES {
        let profile = Arc::clone(&profile);
        let status = Arc::clone(&status);
        let commands = commands.clone();

        let _ = std::thread::Builder::new()
            .name(format!("muivly-ipc-{i}"))
            .spawn(move || {
                loop {
                    match accept_one(&profile, &status, &commands) {
                        // The client disconnected. Loop straight back into
                        // accepting; the other instances covered the gap.
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!("ipc: {e}");
                            std::thread::sleep(std::time::Duration::from_millis(200));
                        }
                    }
                }
            });
    }
}

fn accept_one(
    profile: &GpuProfile,
    status: &Arc<Mutex<Status>>,
    commands: &Sender<Command>,
) -> std::io::Result<()> {
    let name: Vec<u16> = PIPE_NAME.encode_utf16().chain(std::iter::once(0)).collect();

    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            // REJECT_REMOTE_CLIENTS is the important one. A named pipe is
            // reachable over SMB as `\\<machine>\pipe\muivly` unless it says
            // otherwise, which would put "change this desktop's wallpaper,
            // and tell me which files it points at" on the network. Nothing
            // about this protocol is meant to leave the machine.
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            8192,
            8192,
            0,
            None,
        )
    };

    if handle.is_invalid() {
        return Err(std::io::Error::last_os_error());
    }

    // Blocks until the UI connects. This thread does nothing else, so
    // blocking here costs nothing.
    let connected = unsafe { ConnectNamedPipe(handle, None) };
    if let Err(e) = connected {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Err(std::io::Error::other(e));
    }

    let pipe = Pipe(handle);
    let result = converse(pipe, profile, status, commands);

    unsafe {
        let _ = DisconnectNamedPipe(handle);
        let _ = CloseHandle(handle);
    }

    result
}

fn converse(
    pipe: Pipe,
    profile: &GpuProfile,
    status: &Arc<Mutex<Status>>,
    commands: &Sender<Command>,
) -> std::io::Result<()> {
    let mut writer = Pipe(pipe.0);
    let reader = BufReader::new(pipe);

    for line in reader.lines() {
        let line = line?;
        let response = handle(line.trim(), profile, status, commands);
        writer.write_all(response.as_bytes())?;
        writer.flush()?;
    }

    Ok(())
}

fn handle(
    line: &str,
    profile: &GpuProfile,
    status: &Arc<Mutex<Status>>,
    commands: &Sender<Command>,
) -> String {
    let (verb, rest) = match line.split_once(' ') {
        Some((v, r)) => (v, r.trim()),
        None => (line, ""),
    };

    match verb {
        "status" => {
            let status = status.lock().expect("status mutex poisoned");
            let mut out = format!(
                "ok fps={} paused={} frozen={} fit={} interval={} shuffle={} brightness={:.3} \
                 saturation={:.3} blur={:.3} sound={} volume={:.3} duck={} \
                 ducking={} speed={:.2} fade={} span={} hotkeys={} \
                 hibernate={} hibernating={} reactive={:.2} parallax={:.2} \
                 memory={} batfps={} \
                 batfreeze={} battery={} saver={} charge={} cpu={:.1} \
                 ram={} realfps={:.1} idle={} away={} busyfps={} busy={} \
                 load={:.1} reducemotion={} drift={:.2} accent={} \
                 uptime={} resting={}\n",
                status.fps,
                status.paused,
                status.frozen,
                status.fit,
                status.interval_secs,
                status.shuffle,
                status.brightness,
                status.saturation,
                status.blur,
                status.sound,
                status.volume,
                status.duck,
                status.ducking,
                status.speed,
                status.fade_ms,
                status.span,
                status.hotkeys,
                status.hibernate_secs,
                status.hibernating,
                status.reactive,
                status.parallax,
                status.memory_mb,
                status.battery_fps,
                status.pause_on_saver,
                status.on_battery,
                status.saver,
                status.battery_percent,
                status.cpu,
                status.ram_mb,
                status.real_fps,
                status.idle_secs,
                status.away,
                status.busy_fps,
                status.busy,
                status.load,
                status.reduce_motion,
                status.drift,
                status.accent,
                status.uptime_secs,
                status.resting_secs,
            );

            for monitor in &status.monitors {
                let items: Vec<String> = monitor
                    .items
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                out.push_str(&format!(
                    "monitor {} {} {} {}\n",
                    monitor.device_name,
                    monitor.enabled,
                    monitor.index,
                    items.join("|")
                ));
            }

            // Only the monitors that differ from the desktop get a line: on
            // the machines this project is for there is usually one screen
            // and none of this is ever sent.
            for monitor in &status.monitors {
                let own = monitor.overrides;
                if own == Overrides::default() {
                    continue;
                }
                let visual = own.visual.unwrap_or_default();
                out.push_str(&format!(
                    "own {} {} {} {:.3} {:.3} {:.3}\n",
                    monitor.device_name,
                    own.fit.map(|f| f.name()).unwrap_or("-"),
                    own.fps.unwrap_or(0),
                    if own.visual.is_some() {
                        visual.brightness
                    } else {
                        -1.0
                    },
                    visual.saturation,
                    visual.blur,
                ));
            }

            // Empty almost always, so they cost a comparison rather than a
            // line each on every poll.
            if !status.apps.is_empty() {
                out.push_str(&format!("apps {}\n", status.apps));
            }
            if !status.rules.is_empty() {
                out.push_str(&format!("rules {}\n", status.rules));
            }

            // One line each, and absent for everybody who has never saved an
            // arrangement or put a shader on a screen.
            for scene in &status.scenes {
                out.push_str(&format!("scene {scene}\n"));
            }
            for shader in &status.shaders {
                out.push_str(&format!("shader {shader}\n"));
            }

            // `optimize <state> <percent> <source>|<output-or-error>`. One
            // line rather than four keys: it is absent almost always, and
            // when it is there the UI wants all of it at once.
            if let Some(job) = &status.optimize {
                let (state, detail) = match (&job.output, &job.error) {
                    (Some(path), _) => ("done", path.display().to_string()),
                    (_, Some(message)) => ("failed", message.clone()),
                    _ => ("running", String::new()),
                };
                out.push_str(&format!(
                    "optimize {state} {:.0} {}|{}\n",
                    job.progress * 100.0,
                    job.source.display(),
                    detail
                ));
            }

            if let Some(message) = &status.error {
                out.push_str(&format!("error {message}\n"));
            }

            out.push_str("end\n");
            out
        }

        "monitors" => {
            let mut out = String::new();
            for adapter in &profile.adapters {
                for monitor in &adapter.outputs {
                    out.push_str(&format!(
                        "monitor {} {} {} {} {} {} {} {}\n",
                        monitor.device_name,
                        monitor.x,
                        monitor.y,
                        monitor.width,
                        monitor.height,
                        monitor.refresh_hz,
                        monitor.primary,
                        adapter.name,
                    ));
                }
            }
            out.push_str("end\n");
            out
        }

        "set" => {
            let Some((monitor, list)) = rest.split_once(' ') else {
                // No list at all means clear this monitor.
                return send(
                    commands,
                    Command::SetPlaylist {
                        monitor: rest.to_string(),
                        items: Vec::new(),
                    },
                );
            };

            let mut items = Vec::new();
            for part in list.split('|').map(str::trim).filter(|p| !p.is_empty()) {
                let path = PathBuf::from(part);
                if !path.is_file() {
                    return format!("err no such file: {part}\n");
                }
                items.push(path);
            }

            send(
                commands,
                Command::SetPlaylist {
                    monitor: monitor.to_string(),
                    items,
                },
            )
        }

        "next" if !rest.is_empty() => send(
            commands,
            Command::Next {
                monitor: rest.to_string(),
            },
        ),

        "enable" => match rest.split_once(' ') {
            Some((monitor, value)) => send(
                commands,
                Command::SetEnabled {
                    monitor: monitor.to_string(),
                    enabled: value == "true",
                },
            ),
            None => "err usage: enable <monitor> <true|false>\n".to_string(),
        },

        "fps" => match rest.parse::<u32>() {
            Ok(n) if (1..=240).contains(&n) => send(commands, Command::Fps(n)),
            _ => "err fps must be 1-240\n".to_string(),
        },

        "fit" => match Fit::parse(rest) {
            Some(fit) => send(commands, Command::SetFit(fit)),
            None => "err fit must be cover, contain or stretch\n".to_string(),
        },

        "shuffle" => send(
            commands,
            Command::SetShuffle(rest == "on" || rest == "true"),
        ),

        "interval" => match rest.parse::<u64>() {
            // A minute is the shortest interval that is not just flicker.
            Ok(n) if n == 0 || (60..=86400).contains(&n) => send(commands, Command::Interval(n)),
            _ => "err interval must be 0 or 60-86400 seconds\n".to_string(),
        },

        // `visual <brightness> <saturation> <blur>`, each 0-2 for the first
        // two and 0-1 for the last. One command rather than three: they are
        // adjusted together on one panel, and three round trips per drag of
        // a slider is three times the pipe traffic for no gain.
        "visual" => {
            let numbers: Vec<f32> = rest.split(' ').filter_map(|n| n.parse().ok()).collect();
            let [brightness, saturation, blur] = numbers[..] else {
                return "err usage: visual <brightness> <saturation> <blur>\n".to_string();
            };

            if !(0.0..=2.0).contains(&brightness)
                || !(0.0..=2.0).contains(&saturation)
                || !(0.0..=1.0).contains(&blur)
            {
                return "err brightness and saturation are 0-2, blur is 0-1\n".to_string();
            }

            send(
                commands,
                Command::SetVisual(Visual {
                    brightness,
                    saturation,
                    blur,
                }),
            )
        }

        // `sound <on|off> <volume> [duck]`. The third field is optional so an
        // older client — or a person testing by hand — still works.
        "sound" => {
            let mut parts = rest.split(' ');
            let (Some(state), Some(volume)) = (parts.next(), parts.next()) else {
                return "err usage: sound <on|off> <volume> [duck]\n".to_string();
            };
            let Ok(volume) = volume.parse::<f32>() else {
                return "err volume must be 0-1\n".to_string();
            };
            if !(0.0..=1.0).contains(&volume) {
                return "err volume must be 0-1\n".to_string();
            }

            send(
                commands,
                Command::SetSound(Sound {
                    enabled: state == "on",
                    volume,
                    duck: parts.next().map(|d| d == "true" || d == "on") != Some(false),
                }),
            )
        }

        // `power <battery_fps> <freeze_on_saver>`
        "power" => {
            let Some((fps, freeze)) = rest.split_once(' ') else {
                return "err usage: power <battery_fps> <freeze_on_saver>\n".to_string();
            };
            let Ok(battery_fps) = fps.parse::<u32>() else {
                return "err battery fps must be 0 or 1-240\n".to_string();
            };
            if battery_fps > 240 {
                return "err battery fps must be 0 or 1-240\n".to_string();
            }

            send(
                commands,
                Command::SetPower(PowerPolicy {
                    battery_fps,
                    pause_on_saver: freeze == "true" || freeze == "on",
                }),
            )
        }

        "speed" => match rest.parse::<f32>() {
            // Wider than this stops being a wallpaper: a tenth speed looks
            // frozen and four times looks like a fault.
            Ok(rate) if (0.25..=2.0).contains(&rate) => send(commands, Command::SetSpeed(rate)),
            _ => "err speed must be 0.25-2.0\n".to_string(),
        },

        "fade" => match rest.parse::<u64>() {
            Ok(ms) if ms <= 3000 => send(commands, Command::SetFade(Duration::from_millis(ms))),
            _ => "err fade must be 0-3000 milliseconds\n".to_string(),
        },

        "span" => send(commands, Command::SetSpan(rest == "on" || rest == "true")),

        "hotkeys" => send(
            commands,
            Command::SetHotkeys(rest == "on" || rest == "true"),
        ),

        // Below five seconds this would fire on an alt-tab and the user would
        // see the reopen; an hour is long enough to mean "never" to anyone
        // who does not want it at all.
        // `motion <reactive> <parallax>`, both 0-1. One command for the same
        // reason `visual` is one: they sit on one panel and are dragged
        // together.
        "motion" => {
            let numbers: Vec<f32> = rest.split(' ').filter_map(|n| n.parse().ok()).collect();
            let [reactive, parallax] = numbers[..] else {
                return "err usage: motion <reactive> <parallax>\n".to_string();
            };
            if !(0.0..=1.0).contains(&reactive) || !(0.0..=1.0).contains(&parallax) {
                return "err reactive and parallax are 0-1\n".to_string();
            }
            send(commands, Command::SetMotion(Motion { reactive, parallax }))
        }

        // Under 100 MB there is no frame size that fits and the cap would
        // just mean 720p for everybody; past 4 GB it stops being a budget.
        "memory" => match rest.parse::<u32>() {
            Ok(mb) if mb == 0 || (100..=4096).contains(&mb) => {
                send(commands, Command::SetMemory(mb))
            }
            _ => "err memory must be 0 or 100-4096 MB\n".to_string(),
        },

        // An empty list is how the feature is switched off, so this one
        // takes no arguments at all as a valid message.
        "apps" => send(
            commands,
            Command::SetApps(crate::power::apps::parse_list(rest)),
        ),

        "rules" => send(
            commands,
            Command::SetRules(crate::compositor::parse_rules(rest)),
        ),

        // A minute is the shortest that is not simply "while you read a
        // paragraph"; four hours is long enough to mean "never" for anybody
        // who does not want it.
        "idle" => match rest.parse::<u64>() {
            Ok(secs) if secs == 0 || (60..=14400).contains(&secs) => {
                send(commands, Command::SetIdle(secs))
            }
            _ => "err idle must be 0 or 60-14400 seconds\n".to_string(),
        },

        "busy" => match rest.parse::<u32>() {
            Ok(fps) if fps == 0 || (1..=60).contains(&fps) => {
                send(commands, Command::SetBusyFps(fps))
            }
            _ => "err busy must be 0 or 1-60 fps\n".to_string(),
        },

        "reducemotion" => send(
            commands,
            Command::SetReduceMotion(rest == "on" || rest == "true"),
        ),

        "drift" => match rest.parse::<f32>() {
            Ok(drift) if (0.0..=1.0).contains(&drift) => send(commands, Command::SetDrift(drift)),
            _ => "err drift must be 0-1\n".to_string(),
        },

        "accent" => send(commands, Command::SetAccent(rest == "on" || rest == "true")),

        // `shader <path>|<name>=<value>|<name>=<value>`. The path is first
        // because it is the only field that may contain spaces.
        "shader" => {
            let mut parts = rest.split('|');
            let Some(path) = parts.next().filter(|p| !p.is_empty()) else {
                return "err usage: shader <path>|<name>=<value>\n".to_string();
            };

            let values: HashMap<String, f32> = parts
                .filter_map(|field| field.split_once('='))
                .filter_map(|(name, value)| Some((name.to_string(), value.parse().ok()?)))
                .collect();
            if values.is_empty() {
                return "err no settings in that message\n".to_string();
            }

            send(
                commands,
                Command::SetShaderParams {
                    path: PathBuf::from(path),
                    values,
                },
            )
        }

        // `scene save|load|delete <name>`. One verb with three actions
        // rather than three verbs, because they are one feature and a name
        // is all any of them takes.
        "scene" => match rest.split_once(' ') {
            Some(("save", name)) if crate::compositor::valid_scene_name(name) => {
                send(commands, Command::SaveScene(name.to_string()))
            }
            Some(("load", name)) => send(commands, Command::LoadScene(name.to_string())),
            Some(("delete", name)) => send(commands, Command::DeleteScene(name.to_string())),
            _ => "err usage: scene <save|load|delete> <name>\n".to_string(),
        },

        "hibernate" => match rest.parse::<u64>() {
            Ok(secs) if secs == 0 || (5..=3600).contains(&secs) => {
                send(commands, Command::SetHibernate(secs))
            }
            _ => "err hibernate must be 0 or 5-3600 seconds\n".to_string(),
        },

        "freeze" => {
            let frozen = match rest {
                "on" | "true" => true,
                "off" | "false" => false,
                // Toggle needs to know the current state, and the status
                // mutex is right here.
                "toggle" | "" => !status.lock().expect("status mutex poisoned").frozen,
                _ => return "err usage: freeze <on|off|toggle>\n".to_string(),
            };
            send(commands, Command::Freeze(frozen))
        }

        // `own <monitor> <fit|-> <fps> <brightness|-> <saturation> <blur>`
        //
        // Six fields in one message rather than one command per setting: the
        // UI edits them on one panel and sends the panel, which means a
        // monitor can never end up half-overridden.
        "own" => match parse_overrides(rest) {
            Ok((monitor, overrides)) => {
                send(commands, Command::SetOverrides { monitor, overrides })
            }
            Err(message) => format!("err {message}\n"),
        },

        // Rewriting a clip is not a render-loop concern: it opens a device
        // of its own, runs for a minute, and reports through the status the
        // UI is already polling. So it starts here rather than travelling
        // through the command channel and blocking the wallpaper.
        "optimize" if !rest.is_empty() => {
            let source = PathBuf::from(rest);
            if !source.is_file() {
                return "err no such file\n".to_string();
            }

            // The largest screen decides the size: a clip rewritten for the
            // small monitor would be upscaled on the big one, which is the
            // one thing worse than decoding too many pixels.
            let size = profile
                .adapters
                .iter()
                .flat_map(|adapter| &adapter.outputs)
                .map(|monitor| (monitor.width, monitor.height))
                .max_by_key(|(width, height)| *width as u64 * *height as u64)
                .unwrap_or((1920, 1080));

            if crate::optimize::start(
                source,
                size,
                profile.rec.target_fps.max(1),
                Arc::clone(status),
            ) {
                "ok\n".to_string()
            } else {
                "err a clip is already being rewritten\n".to_string()
            }
        }

        "quit" => send(commands, Command::Quit),

        _ => "err unknown command\n".to_string(),
    }
}

/// `<monitor> <fit|-> <fps> <brightness|-> <saturation> <blur>`.
///
/// Its own function because it is the one message with six fields, three
/// spellings of "leave this one alone", and no hardware anywhere near it —
/// which makes it the one message worth testing directly.
fn parse_overrides(rest: &str) -> Result<(String, Overrides), &'static str> {
    let parts: Vec<&str> = rest.split(' ').collect();
    let [monitor, fit, fps, brightness, saturation, blur] = parts[..] else {
        return Err("usage: own <monitor> <fit|-> <fps> <bright|-> <sat> <blur>");
    };

    let fit = match fit {
        "-" => None,
        name => match Fit::parse(name) {
            Some(fit) => Some(fit),
            None => return Err("fit must be cover, contain, stretch or -"),
        },
    };

    let fps = match fps.parse::<u32>() {
        Ok(0) => None,
        Ok(n) if n <= 240 => Some(n),
        _ => return Err("fps must be 0 (follow the desktop) or 1-240"),
    };

    // Two spellings of "this screen has no grade of its own": `-`, which is
    // what a person types, and a negative brightness, which is what the
    // status line and the session file write because they need a number in
    // the slot. Both have to be accepted — the UI sends the second one, and
    // rejecting it meant the button that hands a monitor back to the
    // desktop's settings quietly did nothing.
    let clearing = brightness == "-"
        || brightness
            .parse::<f32>()
            .map(|value| value < 0.0)
            .unwrap_or(false);

    let visual = if clearing {
        None
    } else {
        let numbers: Vec<f32> = [brightness, saturation, blur]
            .iter()
            .filter_map(|n| n.parse().ok())
            .collect();
        let [brightness, saturation, blur] = numbers[..] else {
            return Err("brightness, saturation and blur must be numbers");
        };
        if !(0.0..=2.0).contains(&brightness)
            || !(0.0..=2.0).contains(&saturation)
            || !(0.0..=1.0).contains(&blur)
        {
            return Err("brightness and saturation are 0-2, blur is 0-1");
        }
        Some(Visual {
            brightness,
            saturation,
            blur,
        })
    };

    Ok((monitor.to_string(), Overrides { fit, visual, fps }))
}

fn send(commands: &Sender<Command>, command: Command) -> String {
    match commands.send(command) {
        Ok(()) => "ok\n".to_string(),
        // The render loop is gone; there is nothing left to command.
        Err(_) => "err engine stopped\n".to_string(),
    }
}

/// Minimal `Read`/`Write` over a pipe handle, so the standard library's line
/// handling can be used instead of a hand-rolled buffer.
struct Pipe(HANDLE);

/// The client closing its end. Windows reports this as an error; for a
/// reader it means end of input, and treating it as anything else turns
/// every normal disconnect into a logged failure.
const ERROR_BROKEN_PIPE: i32 = 109;

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut read = 0u32;
        match unsafe { ReadFile(self.0, Some(buf), Some(&mut read), None) } {
            Ok(()) => Ok(read as usize),
            Err(e) if e.code().0 & 0xFFFF == ERROR_BROKEN_PIPE => Ok(0),
            Err(e) => Err(std::io::Error::other(e)),
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut written = 0u32;
        unsafe { WriteFile(self.0, Some(buf), Some(&mut written), None) }
            .map_err(std::io::Error::other)?;
        Ok(written as usize)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_monitor_can_be_given_settings_of_its_own() {
        let (name, own) = parse_overrides(r"\.\DISPLAY1 contain 10 0.600 1.000 0.250").unwrap();

        assert_eq!(name, r"\.\DISPLAY1");
        assert_eq!(own.fit, Some(Fit::Contain));
        assert_eq!(own.fps, Some(10));
        assert_eq!(own.visual.unwrap().brightness, 0.6);
        assert_eq!(own.visual.unwrap().blur, 0.25);
    }

    /// The bug this guards: the panel writes a negative brightness to mean
    /// "no grade of its own" — the same spelling the status line and the
    /// session file use — and only `-` was accepted. The button that handed
    /// a monitor back to the desktop's settings answered `err` and did
    /// nothing, on a screen where nothing looked wrong.
    #[test]
    fn both_spellings_of_no_grade_are_accepted() {
        for line in [r"\.\DISPLAY1 - 0 - 1 0", r"\.\DISPLAY1 - 0 -1 1 0"] {
            let (_, own) = parse_overrides(line).unwrap_or_else(|e| panic!("{line:?}: {e}"));
            assert_eq!(own.visual, None, "{line:?}");
            assert_eq!(own.fit, None, "{line:?}");
            assert_eq!(own.fps, None, "{line:?}");
        }
    }

    #[test]
    fn a_screen_may_differ_in_one_thing_only() {
        let (_, own) = parse_overrides(r"\.\DISPLAY2 - 15 - 1 0").unwrap();
        assert_eq!(own.fps, Some(15));
        assert_eq!(own.fit, None);
        assert_eq!(own.visual, None);
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        assert!(parse_overrides(r"\.\DISPLAY1 sideways 10 1 1 0").is_err());
        assert!(parse_overrides(r"\.\DISPLAY1 cover 999 1 1 0").is_err());
        // Brightness above the range the shader was built for.
        assert!(parse_overrides(r"\.\DISPLAY1 cover 0 9 1 0").is_err());
        // Too few fields.
        assert!(parse_overrides(r"\.\DISPLAY1 cover 0").is_err());
    }

    /// Zero is how the frame rate says "follow the desktop"; it must never
    /// come back as a cap of zero, which would stop the monitor presenting
    /// at all.
    #[test]
    fn a_frame_rate_of_zero_means_follow_the_desktop() {
        let (_, own) = parse_overrides(r"\.\DISPLAY1 cover 0 1 1 0").unwrap();
        assert_eq!(own.fps, None);
    }
}
