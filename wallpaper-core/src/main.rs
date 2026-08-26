// wallpaper-core — the Muivly wallpaper engine. Runs as an independent
// background service; the settings UI (wallpaper-ui) is a separate process.

mod caps;
mod compositor;
mod decoder;
mod ipc;
mod power;

use windows::core::BOOL;
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

const HELP: &str = "\
Muivly wallpaper engine

USAGE:
    muivly-core [OPTIONS]

OPTIONS:
    --run        Render a wallpaper on every monitor until stopped.
    --caps       Print the detected hardware profile and exit.
                 Paste this into a bug report.
    --diag       Dump the desktop window tree (Progman/WorkerW layout).
    -V, --version
    -h, --help
";

fn main() {
    // Must run before anything reads a display size. Without it Windows hands
    // back DPI-virtualized coordinates and every pixel budget is off by the
    // scaling factor. A wallpaper compositor needs real pixels anyway.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    match std::env::args().nth(1).as_deref() {
        Some("--caps") => print!("{}", caps::probe().summary()),
        Some("--diag") => compositor::dump_window_tree(),
        Some("-V" | "--version") => println!("muivly-core {}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => print!("{HELP}"),
        // Until there is a settings UI to drive it, running with no arguments
        // does what a user would expect from a wallpaper engine.
        Some("--run") | None => run(),
        Some(arg) => {
            eprintln!("unknown option: {arg}\n\n{HELP}");
            std::process::exit(2);
        }
    }
}

fn run() {
    let profile = caps::probe();
    print!("{}", profile.summary());

    if profile.rec.tier == caps::Tier::Unsupported {
        eprintln!(
            "\nthis machine cannot play video wallpapers: {}",
            profile.rec.reason
        );
        eprintln!("rendering anyway — the placeholder wallpaper needs no decoder");
    }

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), true);
    }

    if let Err(e) = compositor::run(&profile) {
        eprintln!("compositor failed: {e}");
        std::process::exit(1);
    }

    println!("stopped");
}

/// Ctrl+C must not kill the process outright: the render loop needs one more
/// pass to tear its windows down and let the shell repaint the real wallpaper.
unsafe extern "system" fn ctrl_handler(_: u32) -> BOOL {
    compositor::stop();
    true.into()
}
