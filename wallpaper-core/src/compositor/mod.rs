//! Puts pixels on the desktop background.

mod diag;
mod render;
mod shader;
mod window;
mod workerw;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, SystemParametersInfoW, TranslateMessage, MSG, PM_REMOVE,
    SPIF_SENDCHANGE, SPI_SETDESKWALLPAPER,
};

use crate::caps::GpuProfile;
use render::Renderer;
use window::Surface;

static RUNNING: AtomicBool = AtomicBool::new(true);

pub use diag::dump as dump_window_tree;

/// Ask the render loop to finish the current frame and return.
pub fn stop() {
    RUNNING.store(false, Ordering::Relaxed);
}

/// Create a surface for every monitor and render until stopped.
pub fn run(profile: &GpuProfile) -> windows::core::Result<()> {
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

    let fps = profile.rec.target_fps.max(1);
    let frame_time = Duration::from_secs_f64(1.0 / fps as f64);
    println!("rendering at {fps} fps — Ctrl+C to stop");

    // How often to check whether a covered desktop has become visible again.
    // Half a second is imperceptible to a user alt-tabbing out of a game, and
    // it is 15 wakeups saved for every one spent at 30 fps.
    const OCCLUDED_POLL: Duration = Duration::from_millis(500);

    let start = Instant::now();
    let mut was_paused = false;

    while RUNNING.load(Ordering::Relaxed) {
        let frame_start = Instant::now();

        pump_messages();

        let time = start.elapsed().as_secs_f32();
        let mut drawn = 0;
        for renderer in &mut renderers {
            drawn += renderer.draw(time)?;
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
