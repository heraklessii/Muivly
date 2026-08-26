//! Frame pacing that is accurate enough to look like video.
//!
//! `thread::sleep` on Windows rounds the wait up to the system timer tick,
//! which is 15.6 ms unless some process has asked for better. A 30 fps
//! wallpaper wants to wait 33.3 ms and is given 46.8 ms instead, so every
//! other frame lands a whole tick late. On screen that is not "slightly
//! uneven" — it is the stutter this module exists to remove.
//!
//! A high-resolution waitable timer waits for as long as it was asked to,
//! and unlike `timeBeginPeriod` it does not raise the timer resolution for
//! every other process on the machine — which would cost battery everywhere
//! to make one wallpaper smooth.
//!
//! The high-resolution flag needs Windows 10 1803. Older builds fall back to
//! an ordinary waitable timer, which is no worse than the sleep it replaces.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
};

/// Waits shorter than this are not worth a syscall: the wakeup itself costs
/// more than the time being waited for.
const MIN_WAIT: Duration = Duration::from_micros(200);

pub struct Pacer {
    timer: Option<HANDLE>,
}

impl Pacer {
    pub fn new() -> Self {
        let timer = unsafe {
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
            .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))
            .ok()
        };

        Self { timer }
    }

    /// Block until `deadline`. Returns immediately if it has already passed.
    pub fn wait_until(&self, deadline: Instant) {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return;
        };
        if remaining < MIN_WAIT {
            return;
        }

        if let Some(timer) = self.timer {
            // A negative due time is relative, in 100ns units. Positive would
            // be an absolute date in 1601 terms, which is not what we mean.
            let due = -((remaining.as_nanos() / 100) as i64);
            unsafe {
                if SetWaitableTimer(timer, &due, 0, None, None, false).is_ok() {
                    WaitForSingleObject(timer, INFINITE);
                    return;
                }
            }
        }

        std::thread::sleep(remaining);
    }
}

impl Drop for Pacer {
    fn drop(&mut self) {
        if let Some(timer) = self.timer {
            unsafe {
                let _ = CloseHandle(timer);
            }
        }
    }
}
