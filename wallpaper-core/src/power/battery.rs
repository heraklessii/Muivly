//! Whether the machine is running off its battery, and what to do about it.
//!
//! A wallpaper is the least important thing on a laptop that is running out
//! of charge, and it is one of the few things on screen that costs power
//! while the user is not looking at it. So it gets its own budget: a lower
//! frame rate on battery, and nothing at all while Windows itself has
//! decided the machine is in trouble.
//!
//! `GetSystemPowerStatus` is a cheap call, but it is a syscall and the
//! answer changes on the timescale of a cable being unplugged — so it is
//! read a few times a minute rather than a few times a second.

use std::time::{Duration, Instant};

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// How often the power source is re-read. Nobody notices a wallpaper taking
/// two seconds to react to a charger, and this is 60 syscalls saved for
/// every one spent at 30 fps.
const RECHECK_AFTER: Duration = Duration::from_secs(2);

/// `ACLineStatus` when the machine is not plugged in. 1 is online and 255 is
/// "the firmware does not know", which is treated as plugged in: a desktop
/// with no battery must not be throttled.
const AC_OFFLINE: u8 = 0;

/// Bit 0 of `SystemStatusFlag`: Windows' own battery saver is on. Present
/// since Windows 10; older builds report zero, which reads as "off".
const SAVER_ON: u8 = 1;

/// What the machine is running on right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerState {
    pub on_battery: bool,
    /// Windows' battery saver, which the user switched on (or which switched
    /// itself on at 20%). Treated as an explicit instruction to stop
    /// spending power on decoration.
    pub saver: bool,
    /// Charge left, 0-100. 255 from the API means unknown, reported as 100
    /// so nothing downstream reads "unknown" as "nearly flat".
    pub percent: u8,
}

/// What the user wants to happen on battery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerPolicy {
    /// Frame rate cap while unplugged. Zero means the plugged-in rate is
    /// used on battery too.
    pub battery_fps: u32,
    /// Freeze the wallpaper entirely while battery saver is on.
    pub pause_on_saver: bool,
}

impl Default for PowerPolicy {
    fn default() -> Self {
        // Both on by default, because the machines this project is for are
        // the ones where it matters. 24 fps is the rate film has used for a
        // century and it is barely distinguishable on a looping wallpaper —
        // it is not a degraded mode, it is the sensible one.
        Self {
            battery_fps: 24,
            pause_on_saver: true,
        }
    }
}

impl PowerPolicy {
    /// The frame rate to actually run at, given the plugged-in setting.
    pub fn effective_fps(&self, wanted: u32, state: PowerState) -> u32 {
        if state.on_battery && self.battery_fps > 0 {
            wanted.min(self.battery_fps).max(1)
        } else {
            wanted
        }
    }

    /// Whether drawing should stop altogether. The desktop keeps whatever
    /// frame was last presented — the alternative, taking the surface away,
    /// would flash the Windows wallpaper every time the charger is pulled.
    pub fn should_freeze(&self, state: PowerState) -> bool {
        self.pause_on_saver && state.saver
    }
}

/// Reads the power source, no more often than it can usefully change.
pub struct PowerWatch {
    state: PowerState,
    checked: Instant,
}

impl Default for PowerWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerWatch {
    pub fn new() -> Self {
        Self {
            state: read(),
            // Dated so the first poll after startup does the read again
            // rather than trusting a value from before the render loop began.
            checked: Instant::now() - RECHECK_AFTER,
        }
    }

    /// The current state, and whether it just changed.
    pub fn poll(&mut self) -> (PowerState, bool) {
        if self.checked.elapsed() < RECHECK_AFTER {
            return (self.state, false);
        }

        self.checked = Instant::now();
        let next = read();
        let changed = next != self.state;
        self.state = next;
        (self.state, changed)
    }

    pub fn state(&self) -> PowerState {
        self.state
    }
}

fn read() -> PowerState {
    let mut status = SYSTEM_POWER_STATUS::default();

    // A machine whose firmware will not answer is treated as plugged in.
    // Throttling a desktop that reported nothing would be the worse mistake
    // of the two.
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return PowerState {
            on_battery: false,
            saver: false,
            percent: 100,
        };
    }

    PowerState {
        on_battery: status.ACLineStatus == AC_OFFLINE,
        saver: status.SystemStatusFlag & SAVER_ON != 0,
        percent: if status.BatteryLifePercent > 100 {
            100
        } else {
            status.BatteryLifePercent
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(saver: bool) -> PowerState {
        PowerState {
            on_battery: true,
            saver,
            percent: 50,
        }
    }

    const PLUGGED: PowerState = PowerState {
        on_battery: false,
        saver: false,
        percent: 100,
    };

    #[test]
    fn plugged_in_keeps_the_rate_the_user_chose() {
        assert_eq!(PowerPolicy::default().effective_fps(60, PLUGGED), 60);
    }

    #[test]
    fn battery_caps_but_never_raises() {
        let policy = PowerPolicy::default();
        assert_eq!(policy.effective_fps(60, battery(false)), 24);
        // A user who already chose 15 fps asked for less than the cap, and
        // the cap is not a target to climb to.
        assert_eq!(policy.effective_fps(15, battery(false)), 15);
    }

    #[test]
    fn a_zero_cap_means_no_separate_battery_rate() {
        let policy = PowerPolicy {
            battery_fps: 0,
            pause_on_saver: false,
        };
        assert_eq!(policy.effective_fps(60, battery(false)), 60);
    }

    #[test]
    fn the_rate_never_reaches_zero() {
        let policy = PowerPolicy {
            battery_fps: 0,
            pause_on_saver: false,
        };
        assert_eq!(policy.effective_fps(1, battery(false)), 1);
    }

    #[test]
    fn saver_freezes_only_when_asked() {
        assert!(PowerPolicy::default().should_freeze(battery(true)));
        assert!(!PowerPolicy::default().should_freeze(battery(false)));

        let policy = PowerPolicy {
            battery_fps: 24,
            pause_on_saver: false,
        };
        assert!(!policy.should_freeze(battery(true)));
    }

    #[test]
    fn saver_on_a_plugged_in_machine_still_counts() {
        // Battery saver can be switched on manually with the charger in.
        // It is an instruction, not a reading of the cable.
        let state = PowerState {
            on_battery: false,
            saver: true,
            percent: 100,
        };
        assert!(PowerPolicy::default().should_freeze(state));
    }
}
