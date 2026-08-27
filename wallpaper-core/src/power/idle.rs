//! Two questions the desktop answers about whether anybody is watching.
//!
//! Coverage (`mod.rs`) asks whether the wallpaper can be *seen*. This asks
//! whether anyone is *there* — and whether Windows has been told they would
//! rather things did not move.
//!
//! The gap this closes is the expensive one. A visible desktop with nobody in
//! front of it costs the full frame rate: on the machine in the README that
//! is 13.7% of a core, for a picture nobody is looking at, for as long as the
//! user is away from their desk. Coverage never fires — nothing is covering
//! anything — and the screen saver and the display timeout are minutes away
//! or switched off entirely.
//!
//! Both readings are cheap and both are cached anyway: the wallpaper asks
//! several times a second and neither answer can change that fast.

use std::cell::Cell;
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

/// How long an animation reading is kept. This is a checkbox in Settings,
/// not something that changes while a frame is being drawn.
const ANIMATION_RECHECK: Duration = Duration::from_secs(2);

thread_local! {
    static ANIMATIONS: Cell<Option<(bool, Instant)>> = const { Cell::new(None) };
}

/// How long since the user last touched the keyboard or the mouse.
///
/// Windows counts this for the whole session whether or not anybody asks, so
/// reading it is one call and no hook, no thread and no input being watched
/// by us — which matters: a wallpaper that installed an input hook to work
/// this out would be a wallpaper that reads every keystroke on the machine.
pub fn since_input() -> Duration {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        // No answer means "somebody is there", which is the safe guess: the
        // failure mode of the other one is a desktop that freezes while its
        // owner is looking at it.
        return Duration::ZERO;
    }

    // `dwTime` is a 32-bit tick count and wraps every 49 days; the 64-bit
    // one does not. Comparing them in 32 bits is what makes the wrap
    // harmless, and the wrap is why this is not `GetTickCount64` twice.
    let now = unsafe { GetTickCount64() } as u32;
    Duration::from_millis(now.wrapping_sub(info.dwTime) as u64)
}

/// Whether Windows still wants things on screen to move.
///
/// This is the "Show animations in Windows" switch, which is what the
/// accessibility setting for reduced motion actually flips. Somebody who has
/// turned it off has said, system-wide, that moving pictures are a problem
/// for them — and a live wallpaper is the largest moving picture on their
/// desktop. Honouring it costs one cached call.
pub fn animations_wanted() -> bool {
    if let Some((value, at)) = ANIMATIONS.get() {
        if at.elapsed() < ANIMATION_RECHECK {
            return value;
        }
    }

    let mut enabled = BOOL::from(true);
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };

    // A machine that will not answer is treated as one that wants animation:
    // the default is what every Windows install ships with.
    let value = ok.is_err() || enabled.as_bool();
    ANIMATIONS.set(Some((value, Instant::now())));
    value
}

/// Whether the wallpaper should stand still because nobody is at the machine.
///
/// `after_secs` of zero switches the whole idea off.
pub fn away(after_secs: u64) -> bool {
    after_secs > 0 && since_input() >= Duration::from_secs(after_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one piece of this that is arithmetic rather than a system call.
    /// Everything else needs a desktop with a person in front of it.
    #[test]
    fn switched_off_is_never_away() {
        assert!(!away(0));
    }
}
