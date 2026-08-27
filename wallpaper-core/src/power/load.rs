//! Standing down while the machine has real work to do.
//!
//! The rest of `power/` asks whether the wallpaper can be seen and what it is
//! running on. This asks a different question: whether the machine can afford
//! it right now. On the four-core laptops this project targets, a compile, an
//! update or a game loading is the moment a wallpaper stops being free — not
//! because it got more expensive, but because everything else did.
//!
//! The measurement is the whole system's, from `GetSystemTimes`, which
//! Windows keeps whether or not anybody asks. It costs one call a second and
//! no counters, no WMI and no performance-data thread.
//!
//! Hysteresis, deliberately: a threshold with no gap under it would have the
//! wallpaper stepping between two frame rates several times a minute, which
//! reads as stutter rather than as thrift.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::GetSystemTimes;

/// Busy enough to stand down, and quiet enough to come back. The gap is what
/// stops the wallpaper oscillating between the two rates.
const BUSY_ABOVE: f32 = 80.0;
const QUIET_BELOW: f32 = 60.0;

/// How often the counters are read. Any faster and the sample is mostly
/// noise; any slower and the wallpaper takes too long to get out of the way.
const SAMPLE: Duration = Duration::from_secs(1);

/// How many consecutive busy samples before standing down.
///
/// Two, so a single spike — opening a folder, a browser tab painting — does
/// not drop the wallpaper's frame rate for a second. Coming back is
/// immediate: the machine being free again is not something to be cautious
/// about.
const BUSY_SAMPLES: u32 = 2;

pub struct LoadWatch {
    last: Option<(u64, u64)>,
    checked: Option<Instant>,
    busy_runs: u32,
    busy: bool,
    /// The last reading, for the settings window. Share of the whole machine
    /// rather than of one core.
    percent: f32,
}

impl Default for LoadWatch {
    fn default() -> Self {
        Self {
            last: None,
            checked: None,
            busy_runs: 0,
            busy: false,
            percent: 0.0,
        }
    }
}

impl LoadWatch {
    /// Read the counters if it is time to, and report whether the machine is
    /// busy enough that the wallpaper should get out of the way.
    ///
    /// Returns `(busy, changed)` in the same shape as `PowerWatch::poll`.
    pub fn poll(&mut self) -> (bool, bool) {
        if self.checked.is_some_and(|at| at.elapsed() < SAMPLE) {
            return (self.busy, false);
        }
        self.checked = Some(Instant::now());

        let Some((idle, total)) = read_times() else {
            return (self.busy, false);
        };

        let Some((last_idle, last_total)) = self.last.replace((idle, total)) else {
            // The first reading has nothing to be a difference from.
            return (self.busy, false);
        };

        let spent = total.saturating_sub(last_total);
        if spent == 0 {
            return (self.busy, false);
        }
        let idled = idle.saturating_sub(last_idle);
        self.percent = (100.0 * (1.0 - idled as f32 / spent as f32)).clamp(0.0, 100.0);

        let was = self.busy;
        if self.busy {
            if self.percent < QUIET_BELOW {
                self.busy = false;
                self.busy_runs = 0;
            }
        } else if self.percent >= BUSY_ABOVE {
            self.busy_runs += 1;
            if self.busy_runs >= BUSY_SAMPLES {
                self.busy = true;
            }
        } else {
            self.busy_runs = 0;
        }

        (self.busy, self.busy != was)
    }

    /// What the last sample said, 0-100 across the whole machine.
    pub fn percent(&self) -> f32 {
        self.percent
    }
}

/// Idle ticks and total ticks since the machine started.
///
/// Kernel time already includes idle time, which is the trap in this API: a
/// naive "idle / (kernel + user)" reads far too low.
fn read_times() -> Option<(u64, u64)> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();

    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;

    let ticks = |time: FILETIME| ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
    Some((ticks(idle), ticks(kernel) + ticks(user)))
}

/// The frame rate to use, given what the machine is doing.
///
/// A free function so the arithmetic can be tested without a CPU under load.
/// Zero for `busy_fps` is how the feature is switched off, and a busy rate
/// above the normal one is nonsense that would speed the wallpaper up under
/// load — so it is ignored rather than obeyed.
pub fn effective_fps(normal: u32, busy_fps: u32, busy: bool) -> u32 {
    if !busy || busy_fps == 0 || busy_fps >= normal {
        return normal;
    }
    busy_fps.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_machine_keeps_its_frame_rate() {
        assert_eq!(effective_fps(30, 10, false), 30);
    }

    #[test]
    fn a_busy_machine_drops_to_the_lower_rate() {
        assert_eq!(effective_fps(30, 10, true), 10);
    }

    #[test]
    fn switched_off_changes_nothing() {
        assert_eq!(effective_fps(30, 0, true), 30);
    }

    /// Somebody who has set the busy rate above their normal one has said
    /// something they cannot have meant, and a wallpaper that speeds up when
    /// the machine is struggling is the worst possible reading of it.
    #[test]
    fn a_busy_rate_above_the_normal_one_is_ignored() {
        assert_eq!(effective_fps(24, 60, true), 24);
    }

    /// The counters are cumulative and the arithmetic is a difference, so a
    /// machine that has been up for weeks must not overflow or go negative.
    #[test]
    fn the_reading_stays_in_range() {
        let mut watch = LoadWatch::default();
        // Two polls back to back: the second is inside the sample window and
        // must answer from the cache rather than dividing by a zero gap.
        let (first, _) = watch.poll();
        let (second, changed) = watch.poll();
        assert_eq!(first, second);
        assert!(!changed);
        assert!((0.0..=100.0).contains(&watch.percent()));
    }
}
