//! How loud the machine is, for wallpapers that answer to it.
//!
//! This is the endpoint's own peak meter, not a capture stream. Windows
//! keeps that number for the whole output device whether or not anybody
//! asks, so reading it is one call and no thread, no buffer and no audio
//! being copied anywhere. A loopback capture would give a spectrum instead
//! of a single level — and would cost a thread, a ring buffer and a wake-up
//! every few milliseconds, which is not a trade this project makes for an
//! effect.
//!
//! It measures everything the machine is playing, Muivly's own soundtrack
//! included. That is the right answer for a wallpaper that pulses with the
//! music: the music is usually somebody else's player.

use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

/// How fast the level climbs towards a louder reading, per frame.
///
/// Attack is nearly immediate: a beat that arrives late is not a beat. The
/// release is slow enough that the wallpaper falls away between them rather
/// than flickering at the frame rate, which is what an unsmoothed meter
/// looks like on screen.
const ATTACK: f32 = 0.6;
const RELEASE: f32 = 0.08;

pub struct Meter {
    meter: IAudioMeterInformation,
    level: f32,
}

impl Meter {
    /// Open the default output's meter.
    ///
    /// The device is resolved once. A user who changes output device mid
    /// session gets a meter that reads zero until the next start — worth a
    /// fix if anybody notices, and not worth re-resolving the endpoint every
    /// frame to prevent.
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let meter: IAudioMeterInformation = device.Activate(CLSCTX_ALL, None)?;

            Ok(Self { meter, level: 0.0 })
        }
    }

    /// The smoothed level, 0.0 to 1.0. Call once a frame.
    pub fn read(&mut self) -> f32 {
        let peak = unsafe { self.meter.GetPeakValue().unwrap_or(0.0) };
        self.level = smooth(self.level, peak);
        self.level
    }
}

/// One step of the envelope follower, as a free function so the shape can be
/// tested without an audio device.
fn smooth(current: f32, peak: f32) -> f32 {
    let target = peak.clamp(0.0, 1.0);
    let rate = if target > current { ATTACK } else { RELEASE };
    current + (target - current) * rate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_is_followed_quickly() {
        // Three frames is 50 ms at 60 fps. A beat has to be most of the way
        // there by then or the wallpaper visibly lags the music.
        let mut level = 0.0;
        for _ in 0..3 {
            level = smooth(level, 1.0);
        }
        assert!(level > 0.9, "reached only {level}");
    }

    #[test]
    fn silence_is_left_behind_slowly() {
        // The same three frames coming down should barely move: this is what
        // stops the wallpaper strobing between the beats.
        let mut level = 1.0;
        for _ in 0..3 {
            level = smooth(level, 0.0);
        }
        assert!(level > 0.7, "fell to {level}");
    }

    #[test]
    fn the_level_never_leaves_its_range() {
        // A meter that reports above 1.0 (a driver rounding, a clipped
        // stream) must not brighten the wallpaper past what the setting
        // allows.
        assert!(smooth(0.0, 5.0) <= 1.0);
        assert!(smooth(0.0, -1.0) >= 0.0);
    }
}
