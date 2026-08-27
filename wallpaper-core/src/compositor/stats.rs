//! What the engine is costing, measured by the engine itself.
//!
//! The README promises Muivly is lighter than the alternative. A promise the
//! user cannot check is marketing; this is the same number Task Manager
//! shows, put where they are already looking.
//!
//! Sampling is cheap — two syscalls — but it is not free, and nothing here
//! changes between one frame and the next, so it is taken about once a
//! second rather than every pass.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

const INTERVAL: Duration = Duration::from_millis(1000);

pub struct Stats {
    /// Share of one core, 0-100. Not of the whole machine: on an eight-core
    /// laptop "1%" would be true and useless.
    pub cpu: f32,
    pub ram_mb: u32,
    /// Frames actually presented in the last second, across every monitor.
    pub fps: f32,

    /// How long this engine has been up, and how much of that it spent not
    /// drawing at all — covered, hibernating, frozen, or waiting for somebody
    /// to come back to their desk.
    ///
    /// This is the number the whole project is about, and until now it was
    /// the one thing a user could not see. "Muivly has been running for six
    /// hours and drew nothing for four of them" is the claim in the README,
    /// measured on their machine rather than on ours.
    pub uptime: Duration,
    pub resting: Duration,

    taken: Instant,
    cpu_time: Duration,
    frames: u32,
    /// Loop passes that put something on screen, which is the frame rate one
    /// monitor sees.
    passes: u32,
    /// When the last pass was accounted for, so a pass that took a long time
    /// is counted as the time it actually took.
    last_pass: Instant,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            cpu: 0.0,
            ram_mb: 0,
            fps: 0.0,
            uptime: Duration::ZERO,
            resting: Duration::ZERO,
            taken: Instant::now(),
            cpu_time: process_cpu_time(),
            frames: 0,
            passes: 0,
            last_pass: Instant::now(),
        }
    }
}

impl Stats {
    /// Record what one pass of the render loop presented.
    ///
    /// Two counters, because two monitors showing the same clip flip twice
    /// per frame. Reporting the flips as a frame rate would tell a user with
    /// two screens that a 30 fps clip is running at 60, so the rate comes
    /// from the passes and the flips are left as what they are: the amount
    /// of work done.
    pub fn drew(&mut self, presented: usize) {
        self.frames += presented as u32;
        if presented > 0 {
            self.passes += 1;
        }
    }

    /// Account for the wall-clock time since the last pass, and whether the
    /// engine spent it drawing.
    ///
    /// Measured in time rather than in passes because the passes are not the
    /// same length: a resting engine wakes twice a second and a busy one
    /// thirty times, so counting passes would make resting look like a
    /// rounding error rather than like most of the day.
    pub fn rested(&mut self, resting: bool) {
        let since = self.last_pass.elapsed();
        self.last_pass = Instant::now();
        self.uptime += since;
        if resting {
            self.resting += since;
        }
    }
    /// Refresh the numbers if enough time has passed. Returns true when they
    /// changed, so the caller knows whether the UI needs telling.
    pub fn sample(&mut self) -> bool {
        let since = self.taken.elapsed();
        if since < INTERVAL {
            return false;
        }

        let now = process_cpu_time();
        let used = now.saturating_sub(self.cpu_time);

        self.cpu = (used.as_secs_f32() / since.as_secs_f32() * 100.0).clamp(0.0, 100.0);
        self.fps = self.passes as f32 / since.as_secs_f32();
        self.ram_mb = working_set_mb();

        self.taken = Instant::now();
        self.cpu_time = now;
        self.frames = 0;
        self.passes = 0;
        true
    }
}

/// Kernel plus user time this process has burned since it started.
fn process_cpu_time() -> Duration {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();

    unsafe {
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .is_err()
        {
            return Duration::ZERO;
        }
    }

    // FILETIME is a 64-bit count of 100ns ticks split across two 32-bit
    // halves, which is why it cannot simply be read as a number.
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

fn ticks(time: FILETIME) -> u64 {
    ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64
}

fn working_set_mb() -> u32 {
    let mut counters = PROCESS_MEMORY_COUNTERS::default();

    unsafe {
        if GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .is_err()
        {
            return 0;
        }
    }

    (counters.WorkingSetSize / (1024 * 1024)) as u32
}
