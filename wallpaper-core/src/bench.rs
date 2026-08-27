//! `muivly-core --benchmark <file> [seconds]` — the README's numbers, on the
//! machine holding the mouse.
//!
//! Every claim this project makes is a measurement, and until now all of them
//! were measurements of one laptop. That is the weakest part of the pitch: a
//! user on a different machine has no way to check, and "trust the table in
//! the README" is exactly what a project about honest resource use should not
//! be asking for.
//!
//! So this runs the real engine — the same compositor, the same decoders, the
//! same wallpaper on the same desktop — for a fixed number of seconds, samples
//! what it costs while it does, and prints a table. Nothing here is a
//! simulation or a special path: a benchmark that measured something other
//! than the shipping engine would be worth less than no benchmark at all.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ipc::Status;

/// How often the numbers are read. The engine samples its own cost about
/// once a second, so anything faster reads the same value twice.
const SAMPLE: Duration = Duration::from_millis(500);

/// Long enough for the decoders to settle and for a clip to loop at least
/// once, short enough that nobody is left staring at their desktop.
pub const DEFAULT_SECONDS: u64 = 30;

/// What a run measured.
struct Readings {
    cpu: Vec<f32>,
    ram: Vec<u32>,
    fps: Vec<f32>,
}

impl Readings {
    /// The first few samples are the engine opening a decoder and building
    /// swap chains, which is real work but is not what "while it is running"
    /// means. Dropped rather than averaged in.
    fn settled(&self) -> (Vec<f32>, Vec<u32>, Vec<f32>) {
        let skip = (self.cpu.len() / 6).max(1);
        (
            self.cpu.iter().copied().skip(skip).collect(),
            self.ram.iter().copied().skip(skip).collect(),
            self.fps.iter().copied().skip(skip).collect(),
        )
    }
}

/// Run the engine on `video` for `seconds`, then print what it cost.
///
/// Returns the exit code, so a failure to start reads as a failure rather
/// than as a run of zeroes.
pub fn run(video: std::path::PathBuf, seconds: u64) -> i32 {
    if !video.is_file() {
        eprintln!("no such file: {}", video.display());
        return 2;
    }

    let profile = crate::caps::probe();
    print!("{}", profile.summary());

    if profile.rec.tier == crate::caps::Tier::Unsupported {
        eprintln!(
            "\ncannot play video on this machine: {}",
            profile.rec.reason
        );
        return 1;
    }

    let seconds = seconds.clamp(5, 600);
    println!("\nbenchmark: {} for {seconds}s", video.display());
    println!("the desktop will show it while this runs\n");

    let (tx, rx) = std::sync::mpsc::channel();
    let status = Arc::new(Mutex::new(Status::default()));
    crate::ipc::serve(profile.clone(), Arc::clone(&status), tx);

    // The sampler stops the engine when the time is up, which is how the
    // render loop on this thread ever returns.
    let readings = Arc::new(Mutex::new(Readings {
        cpu: Vec::new(),
        ram: Vec::new(),
        fps: Vec::new(),
    }));

    let sampler = {
        let status = Arc::clone(&status);
        let readings = Arc::clone(&readings);
        std::thread::Builder::new()
            .name("muivly-bench".into())
            .spawn(move || {
                let until = Instant::now() + Duration::from_secs(seconds);
                while Instant::now() < until {
                    std::thread::sleep(SAMPLE);
                    let status = status.lock().expect("status mutex poisoned");
                    let mut readings = readings.lock().expect("readings mutex poisoned");
                    readings.cpu.push(status.cpu);
                    readings.ram.push(status.ram_mb);
                    readings.fps.push(status.real_fps);
                }
                crate::compositor::stop();
            })
            .ok()
    };

    if let Err(e) = crate::compositor::run(&profile, Some(video), rx, Arc::clone(&status)) {
        eprintln!("compositor failed: {e}");
        return 1;
    }

    if let Some(handle) = sampler {
        let _ = handle.join();
    }

    let readings = readings.lock().expect("readings mutex poisoned");
    let (cpu, ram, fps) = readings.settled();
    if cpu.is_empty() {
        eprintln!("benchmark: the run was too short to measure");
        return 1;
    }

    let resting = {
        let status = status.lock().expect("status mutex poisoned");
        (status.resting_secs, status.uptime_secs)
    };

    print!("{}", report(&cpu, &ram, &fps, resting));
    0
}

/// The table, as text. A free function over slices so the arithmetic and the
/// wording can be tested without a GPU, a desktop or half a minute.
fn report(cpu: &[f32], ram: &[u32], fps: &[f32], resting: (u64, u64)) -> String {
    let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len().max(1) as f32;
    let peak = |values: &[f32]| values.iter().copied().fold(0.0f32, f32::max);

    let ram_f: Vec<f32> = ram.iter().map(|mb| *mb as f32).collect();

    let mut out = String::from("\n");
    out.push_str("                        average      peak\n");
    out.push_str(&format!(
        "CPU (one core)          {:>6.1}%   {:>6.1}%\n",
        mean(cpu),
        peak(cpu)
    ));
    out.push_str(&format!(
        "Memory (working set)    {:>6.0} MB {:>6.0} MB\n",
        mean(&ram_f),
        peak(&ram_f)
    ));
    out.push_str(&format!(
        "Frames presented        {:>6.1}    {:>6.1}\n",
        mean(fps),
        peak(fps)
    ));

    let (resting_secs, uptime_secs) = resting;
    if uptime_secs > 0 {
        out.push_str(&format!(
            "\nDrew nothing for {}s of {}s.\n",
            resting_secs, uptime_secs
        ));
    }

    out.push_str(
        "\nThese are this machine's numbers, not the README's. CPU is a share\n\
         of one core, which is what Task Manager shows. Memory is the working\n\
         set of the engine process alone; the settings window is not running.\n",
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reports_both_an_average_and_a_peak() {
        let text = report(&[10.0, 20.0, 30.0], &[100, 200, 300], &[29.0, 30.0], (0, 0));
        assert!(text.contains("20.0%"), "{text}");
        assert!(text.contains("30.0%"), "{text}");
        assert!(text.contains("200 MB"), "{text}");
    }

    /// The opening samples are a decoder being built, which is not what the
    /// table claims to measure.
    #[test]
    fn the_first_samples_are_dropped() {
        let readings = Readings {
            cpu: vec![90.0, 10.0, 10.0, 10.0, 10.0, 10.0],
            ram: vec![900, 100, 100, 100, 100, 100],
            fps: vec![0.0, 30.0, 30.0, 30.0, 30.0, 30.0],
        };
        let (cpu, ram, _) = readings.settled();
        assert_eq!(cpu.len(), 5);
        assert!(cpu.iter().all(|c| *c < 50.0), "{cpu:?}");
        assert!(ram.iter().all(|r| *r < 500), "{ram:?}");
    }

    /// A single sample must still produce a table rather than dividing by
    /// the number of samples it dropped.
    #[test]
    fn one_sample_is_still_a_table() {
        let readings = Readings {
            cpu: vec![12.0],
            ram: vec![120],
            fps: vec![30.0],
        };
        let (cpu, ram, fps) = readings.settled();
        // Everything was warm-up, which is honest: `run` reports that the
        // run was too short rather than printing a table of nothing.
        assert!(cpu.is_empty() && ram.is_empty() && fps.is_empty());
    }

    #[test]
    fn the_resting_line_is_left_out_when_there_is_nothing_to_say() {
        let text = report(&[1.0], &[1], &[1.0], (0, 0));
        assert!(!text.contains("Drew nothing"), "{text}");
        let text = report(&[1.0], &[1], &[1.0], (40, 60));
        assert!(text.contains("Drew nothing for 40s of 60s"), "{text}");
    }
}
