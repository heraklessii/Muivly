//! Puts pixels on the desktop background.

mod diag;
mod render;
mod shader;
mod window;
mod workerw;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, SystemParametersInfoW, TranslateMessage, MSG, PM_REMOVE,
    SPIF_SENDCHANGE, SPI_SETDESKWALLPAPER,
};

use crate::caps::GpuProfile;
use crate::ipc::{Command, MonitorState, Status};
use render::Renderer;
use window::Surface;

pub use diag::dump as dump_window_tree;
pub use render::Fit;

static RUNNING: AtomicBool = AtomicBool::new(true);

/// Ask the render loop to finish the current frame and return.
pub fn stop() {
    RUNNING.store(false, Ordering::Relaxed);
}

/// What one monitor is playing, and where it is in a playlist.
struct Playback {
    /// Empty means the placeholder gradient. One entry is a fixed wallpaper.
    /// More than one is a playlist.
    items: Vec<PathBuf>,
    index: usize,
    enabled: bool,
    /// When the current item started, for interval-based advancing.
    started: Instant,
    /// The decoder's loop count when the current item started, for
    /// advance-when-the-clip-ends.
    loops_at_start: u32,
}

impl Playback {
    fn current(&self) -> Option<&PathBuf> {
        self.items.get(self.index)
    }
}

/// Create a surface for every monitor and render until stopped.
pub fn run(
    profile: &GpuProfile,
    initial: Option<PathBuf>,
    commands: Receiver<Command>,
    status: Arc<Mutex<Status>>,
) -> windows::core::Result<()> {
    let target = workerw::find().ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "no WorkerW and no Progman: is Explorer running?",
        )
    })?;
    println!("parent: {}", target.how);
    let parent = target.hwnd;

    // One renderer per adapter. On a hybrid laptop with monitors on both
    // GPUs this is two devices — the alternative, one device feeding a
    // cross-adapter shared texture, copies every frame through system
    // memory. See docs/decisions.md.
    let mut renderers = Vec::new();
    for adapter in &profile.adapters {
        let mut surfaces = Vec::new();
        for monitor in &adapter.outputs {
            surfaces.push(Surface::create(parent, monitor)?);
        }

        let renderer = Renderer::new(adapter.luid, surfaces)?;
        println!(
            "surface: {} monitor(s) on {}",
            renderer.monitor_count(),
            adapter.name
        );
        renderers.push(renderer);
    }

    let mut playback: HashMap<String, Playback> = HashMap::new();
    for adapter in &profile.adapters {
        for monitor in &adapter.outputs {
            playback.insert(
                monitor.device_name.clone(),
                Playback {
                    items: initial.iter().cloned().collect(),
                    index: 0,
                    enabled: true,
                    started: Instant::now(),
                    loops_at_start: 0,
                },
            );
        }
    }

    // Apply whatever the command line asked for.
    for (name, state) in &playback {
        apply(&mut renderers, name, state.current())?;
    }

    let mut fps = profile.rec.target_fps.max(1);
    let mut frame_time = Duration::from_secs_f64(1.0 / fps as f64);
    let mut fit = Fit::default();
    // Zero means "advance when the clip ends" rather than on a clock.
    let mut interval_secs = 0u64;

    println!("rendering at {fps} fps — Ctrl+C to stop");
    publish(&status, fps, fit, interval_secs, false, &playback);

    // How often to check whether a covered desktop has become visible again.
    // Half a second is imperceptible to a user alt-tabbing out of a game, and
    // it is 15 wakeups saved for every one spent at 30 fps.
    const OCCLUDED_POLL: Duration = Duration::from_millis(500);

    let start = Instant::now();
    let mut was_paused = false;

    while RUNNING.load(Ordering::Relaxed) {
        let frame_start = Instant::now();

        pump_messages();

        // Apply whatever the UI asked for since the last frame. Draining
        // rather than handling one keeps a burst of clicks from taking a
        // frame each.
        let mut changed = false;
        for command in commands.try_iter() {
            changed = true;
            match command {
                Command::SetPlaylist { monitor, items } => {
                    let entry = playback.entry(monitor.clone()).or_insert_with(|| Playback {
                        items: Vec::new(),
                        index: 0,
                        enabled: true,
                        started: Instant::now(),
                        loops_at_start: 0,
                    });
                    entry.items = items;
                    entry.index = 0;
                    entry.started = Instant::now();
                    entry.loops_at_start = 0;

                    let current = entry.current().cloned();
                    match &current {
                        Some(path) => println!("{monitor}: {}", path.display()),
                        None => println!("{monitor}: cleared"),
                    }
                    apply(&mut renderers, &monitor, current.as_ref())?;
                }

                Command::SetEnabled { monitor, enabled } => {
                    if let Some(entry) = playback.get_mut(&monitor) {
                        entry.enabled = enabled;
                    }
                    for renderer in &mut renderers {
                        if renderer.has_monitor(&monitor) {
                            renderer.set_enabled(&monitor, enabled)?;
                        }
                    }
                    println!("{monitor}: {}", if enabled { "on" } else { "off" });
                }

                Command::Next { monitor } => {
                    advance(&mut renderers, &mut playback, &monitor)?;
                }

                Command::Fps(n) => {
                    fps = n;
                    frame_time = Duration::from_secs_f64(1.0 / n as f64);
                    println!("fps: {n}");
                }

                Command::SetFit(next) => {
                    fit = next;
                    for renderer in &mut renderers {
                        renderer.set_fit(fit);
                    }
                    println!("fit: {}", fit.name());
                }

                Command::Interval(secs) => {
                    interval_secs = secs;
                    println!("interval: {secs}s");
                }

                Command::Quit => stop(),
            }
        }

        // Playlists advance either on a clock or when the clip ends. Both are
        // checked here rather than in the renderer, which has no idea what a
        // playlist is.
        let loops: HashMap<PathBuf, u32> = renderers
            .iter()
            .flat_map(|r| r.loop_counts())
            .collect::<HashMap<_, _>>();

        let due: Vec<String> = playback
            .iter()
            .filter(|(_, state)| state.enabled && state.items.len() > 1)
            .filter(|(_, state)| {
                if interval_secs > 0 {
                    state.started.elapsed().as_secs() >= interval_secs
                } else {
                    state
                        .current()
                        .and_then(|path| loops.get(path))
                        .is_some_and(|count| *count > state.loops_at_start)
                }
            })
            .map(|(name, _)| name.clone())
            .collect();

        for monitor in due {
            advance(&mut renderers, &mut playback, &monitor)?;
            changed = true;
        }

        let elapsed = start.elapsed();
        let mut drawn = 0;
        for renderer in &mut renderers {
            drawn += renderer.draw(elapsed)?;
        }

        // Every monitor is covered — by a fullscreen game, a maximised
        // window, or a locked screen. Rendering into it would be work no one
        // can see, which is precisely what this project refuses to spend.
        let paused = drawn == 0;
        if paused != was_paused {
            println!(
                "{}",
                if paused {
                    "paused: every monitor is covered"
                } else {
                    "resumed: desktop visible again"
                }
            );
            was_paused = paused;
            changed = true;
        }

        if changed {
            publish(&status, fps, fit, interval_secs, paused, &playback);
        }

        // Sleeping the remainder is what keeps this cheap. A wallpaper that
        // spins at the display's refresh rate is the exact problem this
        // project exists to avoid.
        let budget = if paused { OCCLUDED_POLL } else { frame_time };
        if let Some(remaining) = budget.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    // Dropping the renderers destroys the windows. Windows does not repaint
    // the wallpaper underneath on its own, so ask it to.
    drop(renderers);
    restore_desktop();

    Ok(())
}

/// Move one monitor to the next item in its playlist.
fn advance(
    renderers: &mut [Renderer],
    playback: &mut HashMap<String, Playback>,
    monitor: &str,
) -> windows::core::Result<()> {
    let Some(state) = playback.get_mut(monitor) else {
        return Ok(());
    };
    if state.items.is_empty() {
        return Ok(());
    }

    state.index = (state.index + 1) % state.items.len();
    state.started = Instant::now();

    let current = state.current().cloned();
    // The next clip may already be open on another monitor, in which case it
    // has looped a few times already; its count is the baseline, not zero.
    state.loops_at_start = current
        .as_ref()
        .and_then(|path| {
            renderers
                .iter()
                .flat_map(|r| r.loop_counts())
                .find(|(open, _)| open == path)
                .map(|(_, count)| count)
        })
        .unwrap_or(0);

    apply(renderers, monitor, current.as_ref())
}

/// Route a monitor's wallpaper to whichever adapter drives it.
fn apply(
    renderers: &mut [Renderer],
    monitor: &str,
    video: Option<&PathBuf>,
) -> windows::core::Result<()> {
    for renderer in renderers.iter_mut() {
        if renderer.has_monitor(monitor) {
            renderer.set_video(monitor, video.map(|p| p.as_path()))?;
        }
    }
    Ok(())
}

fn publish(
    status: &Arc<Mutex<Status>>,
    fps: u32,
    fit: Fit,
    interval_secs: u64,
    paused: bool,
    playback: &HashMap<String, Playback>,
) {
    let mut status = status.lock().expect("status mutex poisoned");
    status.fps = fps;
    status.fit = fit.name().to_string();
    status.interval_secs = interval_secs;
    status.paused = paused;
    status.monitors = playback
        .iter()
        .map(|(name, state)| MonitorState {
            device_name: name.clone(),
            enabled: state.enabled,
            index: state.index,
            items: state.items.clone(),
        })
        .collect();
    // A map has no order; the UI shows these as a list and a list that
    // reshuffles itself between polls is unusable.
    status
        .monitors
        .sort_by(|a, b| a.device_name.cmp(&b.device_name));
}

fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, Some(HWND::default()), 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Re-apply the user's own wallpaper, so quitting leaves the desktop as it
/// was found.
fn restore_desktop() {
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            None,
            // No UPDATEINIFILE: we are not changing the user's setting, only
            // asking the shell to redraw what it already has.
            SPIF_SENDCHANGE,
        );
    }
}
