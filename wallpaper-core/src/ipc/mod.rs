//! Named pipe server. The settings UI is the only client.
//!
//! The protocol is one UTF-8 line per message, because the message set is
//! small enough that a serialisation library would cost more (binary size, a
//! dependency, a schema to keep in sync) than it saves. If this grows much
//! further, that trade flips — see docs/decisions.md.
//!
//! Requests:
//!   status                    -> `ok fps=<n> paused=<b> fit=<name> interval=<s>`,
//!                                then one `monitor <name> <enabled> <index> <path>|...`
//!                                line per display, then `end`
//!   monitors                  -> one `monitor <name> <x> <y> <w> <h> <hz> <primary> <adapter>`
//!                                line per display, then `end`
//!   set <monitor> <path>|...  -> `ok`   (empty list clears; `|` separates a playlist)
//!   next <monitor>            -> `ok`
//!   enable <monitor> <bool>   -> `ok`
//!   fps <n>                   -> `ok`
//!   fit <cover|contain|stretch> -> `ok`
//!   interval <seconds>        -> `ok`   (0 = advance when the clip ends)
//!   quit                      -> `ok`, then the engine shuts down
//!
//! Anything unrecognised gets `err unknown command`.
//!
//! Paths are separated by `|` rather than spaces, and never quoted: Windows
//! paths contain spaces constantly but never a pipe character.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::caps::GpuProfile;
use crate::compositor::Fit;

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
    Quit,
}

#[derive(Debug, Clone)]
pub struct MonitorState {
    pub device_name: String,
    pub enabled: bool,
    pub index: usize,
    pub items: Vec<PathBuf>,
}

/// What the engine tells the UI about itself.
#[derive(Debug, Clone)]
pub struct Status {
    pub fps: u32,
    pub paused: bool,
    pub fit: String,
    pub interval_secs: u64,
    pub monitors: Vec<MonitorState>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            fps: 0,
            paused: false,
            fit: Fit::default().name().to_string(),
            interval_secs: 0,
            monitors: Vec::new(),
        }
    }
}

/// Start serving on a background thread. Returns immediately.
pub fn serve(profile: GpuProfile, status: Arc<Mutex<Status>>, commands: Sender<Command>) {
    std::thread::spawn(move || {
        loop {
            match accept_one(&profile, &status, &commands) {
                // The client disconnected. Loop straight back into accepting:
                // the UI opens a fresh connection per request, and any pause
                // here is a window in which it finds no pipe at all.
                Ok(()) => {}
                Err(e) => {
                    eprintln!("ipc: {e}");
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    });
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
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
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
                "ok fps={} paused={} fit={} interval={}\n",
                status.fps, status.paused, status.fit, status.interval_secs
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

        "interval" => match rest.parse::<u64>() {
            // A minute is the shortest interval that is not just flicker.
            Ok(n) if n == 0 || (60..=86400).contains(&n) => send(commands, Command::Interval(n)),
            _ => "err interval must be 0 or 60-86400 seconds\n".to_string(),
        },

        "quit" => send(commands, Command::Quit),

        _ => "err unknown command\n".to_string(),
    }
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
