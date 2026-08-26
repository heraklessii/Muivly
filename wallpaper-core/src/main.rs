// wallpaper-core — the Muivly wallpaper engine. Runs as an independent
// background service; the settings UI (wallpaper-ui) is a separate process.

mod caps;
mod compositor;
mod decoder;
mod ipc;
mod power;

use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

const HELP: &str = "\
Muivly wallpaper engine

USAGE:
    muivly-core [OPTIONS]

OPTIONS:
    --caps       Print the detected hardware profile and exit.
                 Paste this into a bug report.
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
        Some("-V" | "--version") => println!("muivly-core {}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => print!("{HELP}"),
        Some(arg) => {
            eprintln!("unknown option: {arg}\n\n{HELP}");
            std::process::exit(2);
        }
        // The engine itself does not exist yet. Until the compositor lands,
        // running with no arguments only reports what the machine can do.
        None => print!("{}", caps::probe().summary()),
    }
}
