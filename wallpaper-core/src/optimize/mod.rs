//! Rewriting a clip once so it costs less every time it plays.
//!
//! The memory a video wallpaper uses is almost entirely the decoder's
//! picture buffers, and the size of those is decided by the codec's
//! reference-frame count times the frame size. Nothing the engine sets at
//! playback time moves that ceiling — the knobs Media Foundation offers set
//! a floor. See docs/decisions.md.
//!
//! The frame size, though, is not the engine's to decide. It belongs to the
//! file. A 4K clip on a 1080p laptop is decoding four times the pixels that
//! screen can show, for every frame, forever. Rewriting it once at the size
//! the desktop actually is cuts that for good — and unlike a scale applied
//! during playback, it costs nothing at run time because there is no
//! processor in the pipeline and no second buffer pool.
//!
//! Everything here is one-shot and off the render path: a worker thread,
//! hardware decode into hardware encode, and a file at the end. It is the
//! only place in the project that writes a video.

mod encode;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::ipc::Status;

/// Where a rewritten clip goes.
///
/// Not next to the original: the library is often a read-only folder, a
/// network share, or somewhere the user would not welcome a second file
/// appearing. This is the same directory the session file lives in, so an
/// uninstall takes it with everything else.
pub fn output_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(Path::new(&appdata).join("Muivly").join("light"))
}

/// The name a rewritten clip gets, which carries the size so two desktops
/// with different screens do not fight over one file.
fn output_path(source: &Path, size: (u32, u32)) -> Option<PathBuf> {
    let stem = source.file_stem()?.to_string_lossy().into_owned();
    Some(output_dir()?.join(format!("{stem} {}x{}.mp4", size.0, size.1)))
}

/// What the UI is told about a rewrite in progress.
#[derive(Debug, Clone)]
pub struct Job {
    pub source: PathBuf,
    /// 0.0 to 1.0. Measured against the clip's duration, so it is honest
    /// rather than a spinner.
    pub progress: f32,
    /// Set when the file is finished and ready to be added to the library.
    pub output: Option<PathBuf>,
    /// Set when it will not be finished, in words for the user.
    pub error: Option<String>,
}

/// One rewrite at a time, process-wide.
///
/// A hardware encoder is a single scarce resource on most integrated GPUs,
/// and two rewrites at once on the machines this project targets is how a
/// desktop stops responding. A second request is refused rather than queued:
/// the UI shows the one in progress, and "wait" is a better answer than a
/// queue nobody can see.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Start rewriting `source` at `size`, reporting into `status`.
///
/// Returns whether the job was started. False means one is already running.
pub fn start(source: PathBuf, size: (u32, u32), fps: u32, status: Arc<Mutex<Status>>) -> bool {
    if BUSY.swap(true, Ordering::SeqCst) {
        return false;
    }

    let publish = |status: &Arc<Mutex<Status>>, job: Job| {
        status.lock().expect("status mutex poisoned").optimize = Some(job);
    };

    publish(
        &status,
        Job {
            source: source.clone(),
            progress: 0.0,
            output: None,
            error: None,
        },
    );

    let spawned = std::thread::Builder::new()
        .name("muivly-optimize".into())
        .spawn(move || {
            let outcome = match output_path(&source, size) {
                Some(destination) => {
                    let reporter = {
                        let status = Arc::clone(&status);
                        let source = source.clone();
                        move |progress: f32| {
                            status.lock().expect("status mutex poisoned").optimize = Some(Job {
                                source: source.clone(),
                                progress,
                                output: None,
                                error: None,
                            });
                        }
                    };

                    match encode::rewrite(&source, &destination, size, fps, reporter) {
                        Ok(()) => Job {
                            source: source.clone(),
                            progress: 1.0,
                            output: Some(destination),
                            error: None,
                        },
                        Err(e) => {
                            // A half-written mp4 is worse than none: it is in
                            // the library, it looks like a wallpaper, and it
                            // will not play.
                            let _ = std::fs::remove_file(&destination);
                            Job {
                                source: source.clone(),
                                progress: 0.0,
                                output: None,
                                error: Some(e),
                            }
                        }
                    }
                }
                None => Job {
                    source: source.clone(),
                    progress: 0.0,
                    output: None,
                    error: Some("no APPDATA to write into".to_string()),
                },
            };

            match (&outcome.output, &outcome.error) {
                (Some(path), _) => println!("optimize: wrote {}", path.display()),
                (_, Some(e)) => eprintln!("optimize: {e}"),
                _ => {}
            }

            publish(&status, outcome);
            BUSY.store(false, Ordering::SeqCst);
        })
        .is_ok();

    if !spawned {
        BUSY.store(false, Ordering::SeqCst);
    }
    spawned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_name_carries_the_size() {
        // Without the size in the name, rewriting the same clip for a 1080p
        // laptop and a 1440p desktop would produce one file that is wrong
        // for one of them.
        let name = output_path(Path::new(r"C:\clips\rain.mp4"), (1920, 1080))
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(name.as_deref(), Some("rain 1920x1080.mp4"));
    }

    #[test]
    fn the_output_goes_under_appdata_rather_than_next_to_the_source() {
        // Libraries live on read-only shares and in folders the user tidies.
        // Writing there is how "optimise" turns into "why is there a second
        // copy of everything".
        let out = output_path(Path::new(r"C:\clips\rain.mp4"), (1920, 1080));
        assert!(out.is_none_or(|p| !p.starts_with(r"C:\clips")));
    }
}
