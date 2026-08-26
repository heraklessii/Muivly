//! Puts pixels on the desktop background.

mod diag;
mod render;
mod shader;
mod window;
mod workerw;

use std::path::Path;
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
use crate::ipc::{Command, Status};
use render::Renderer;
use window::Surface;

static RUNNING: AtomicBool = AtomicBool::new(true);

pub use diag::dump as dump_window_tree;

/// Ask the render loop to finish the current frame and return.
pub fn stop() {
    RUNNING.store(false, Ordering::Relaxed);
}

/// Create a surface for every monitor and render until stopped.
pub fn run(
    profile: &GpuProfile,
    video: Option<&Path>,
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

        let renderer = Renderer::new(adapter.luid, surfaces, video)?;
        match renderer.video_size() {
            Some((w, h)) => println!(
                "surface: {} monitor(s) on {} — decoding {w}x{h}",
                renderer.monitor_count(),
                adapter.name
            ),
            None => println!(
                "surface: {} monitor(s) on {}",
                renderer.monitor_count(),
                adapter.name
            ),
        }
        renderers.push(renderer);
    }

    let mut fps = profile.rec.target_fps.max(1);
    let mut frame_time = Duration::from_secs_f64(1.0 / fps as f64);
    let mut current_video = video.map(|p| p.to_path_buf());
    println!("rendering at {fps} fps — Ctrl+C to stop");

    {
        let mut status = status.lock().expect("status mutex poisoned");
        status.fps = fps;
        status.video = current_video.clone();
    }

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
        for command in commands.try_iter() {
            match command {
                Command::SetVideo(path) => {
                    for renderer in &mut renderers {
                        renderer.set_video(Some(&path))?;
                    }
                    println!("wallpaper: {}", path.display());
                    current_video = Some(path);
                }
                Command::Clear => {
                    for renderer in &mut renderers {
                        renderer.set_video(None)?;
                    }
                    println!("wallpaper: cleared");
                    current_video = None;
                }
                Command::Fps(n) => {
                    fps = n;
                    frame_time = Duration::from_secs_f64(1.0 / n as f64);
                    println!("fps: {n}");
                }
                Command::Quit => stop(),
            }

            let mut status = status.lock().expect("status mutex poisoned");
            status.fps = fps;
            status.video = current_video.clone();
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
            status.lock().expect("status mutex poisoned").paused = paused;
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
