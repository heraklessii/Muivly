// wallpaper-core — the Muivly wallpaper engine. Runs as an independent
// background service; the settings UI (wallpaper-ui) is a separate process.

mod audio;
mod bench;
mod caps;
mod compositor;
mod decoder;
mod ipc;
mod optimize;
mod power;
mod session;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

const HELP: &str = "\
Muivly wallpaper engine

USAGE:
    muivly-core [OPTIONS] [VIDEO]

ARGS:
    <VIDEO>      Video file to play as the wallpaper. Without one, the
                 engine idles and the desktop keeps its Windows wallpaper
                 until the settings app assigns something.

OPTIONS:
    --caps       Print the detected hardware profile and exit.
                 Paste this into a bug report.
    --diag       Dump the desktop window tree (Progman/WorkerW layout).
    --benchmark <VIDEO> [SECONDS]
                 Play a wallpaper for a while and print what it cost on
                 this machine: CPU, memory and frames presented. Thirty
                 seconds unless another number is given.
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

    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = args.first().cloned();
    match arg.as_deref() {
        Some("--caps") => print!("{}", caps::probe().summary()),
        Some("--diag") => compositor::dump_window_tree(),

        // A benchmark is the engine, run for a while, measured. It claims
        // the same single-instance lock as an ordinary run: two engines
        // drawing to one desktop would measure each other.
        Some("--benchmark") => {
            let Some(video) = args.get(1) else {
                eprintln!("usage: muivly-core --benchmark <VIDEO> [SECONDS]");
                std::process::exit(2);
            };
            let seconds = args
                .get(2)
                .and_then(|n| n.parse().ok())
                .unwrap_or(bench::DEFAULT_SECONDS);

            if !claim_single_instance() {
                eprintln!("muivly-core is already running; stop it before measuring");
                std::process::exit(2);
            }

            unsafe {
                let _ = SetConsoleCtrlHandler(Some(ctrl_handler), true);
            }
            std::process::exit(bench::run(PathBuf::from(video), seconds));
        }
        Some("-V" | "--version") => println!("muivly-core {}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => print!("{HELP}"),
        Some(other) if other.starts_with('-') => {
            eprintln!("unknown option: {other}\n\n{HELP}");
            std::process::exit(2);
        }
        // Until there is a settings UI to drive it, a path on the command
        // line is how a wallpaper gets chosen.
        Some(path) => run(Some(PathBuf::from(path))),
        None => run(None),
    }
}

fn run(video: Option<PathBuf>) {
    if let Some(path) = &video {
        if !path.is_file() {
            eprintln!("no such file: {}", path.display());
            std::process::exit(2);
        }
    }

    // A second engine would render a second wallpaper into the same WorkerW,
    // decode the same video again, and answer to a name only one of them can
    // be reached at — so a later "quit" stops one and leaves the other
    // running. One per session, and the extra launch is a no-op.
    if !claim_single_instance() {
        println!("muivly-core is already running");
        return;
    }

    let profile = caps::probe();
    print!("{}", profile.summary());

    // The rule is no CPU decode, ever. Saying so out loud is better than
    // silently showing nothing, and far better than quietly burning a core.
    let playable = profile.rec.tier != caps::Tier::Unsupported;
    if video.is_some() && !playable {
        eprintln!(
            "\ncannot play video on this machine: {}",
            profile.rec.reason
        );
        eprintln!("showing the placeholder wallpaper instead");
    }

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), true);
    }

    // The settings UI talks to this pipe. It is served on its own thread so a
    // UI that stalls mid-message cannot stall the wallpaper.
    let (tx, rx) = std::sync::mpsc::channel();
    let status = Arc::new(Mutex::new(ipc::Status::default()));
    ipc::serve(profile.clone(), Arc::clone(&status), tx);
    println!("ipc: listening on {}", ipc::PIPE_NAME);

    let path = video.filter(|_| playable);
    if let Err(e) = compositor::run(&profile, path, rx, status) {
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

/// Take the one-engine-per-session claim, or report that someone else has it.
///
/// A named mutex is the cheapest lock Windows offers for this and it is
/// released by the kernel however the process ends, including a crash — a
/// lock file would need cleaning up after one. `Local\` scopes it to the
/// logon session, which is the right scope: two users logged in at once each
/// have their own desktop and each should get their own wallpaper.
fn claim_single_instance() -> bool {
    unsafe {
        let name = windows::core::w!(r"Local\muivly-core");
        let Ok(handle) = CreateMutexW(None, true, name) else {
            // Without an answer, running is the better guess: refusing to
            // start would be the worse failure of the two.
            return true;
        };

        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return false;
        }

        // Deliberately never closed. The handle is the claim, and it has to
        // outlive everything else in the process.
        true
    }
}
