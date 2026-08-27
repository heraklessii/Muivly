//! What the machine is playing, split into bands, for shaders that answer to
//! music rather than to loudness.
//!
//! `meter.rs` reads the endpoint's own peak, which is one number and costs
//! one call. That is the right answer for a wallpaper that breathes with the
//! volume, and the wrong one for a wallpaper that draws a spectrum: a
//! spectrum needs the samples themselves.
//!
//! So this is a loopback capture — but not the one `meter.rs` declined. What
//! that comment refused was the thread and the wake-up: a capture client with
//! a timer of its own, waking every few milliseconds forever for an effect.
//! There is none of that here. The buffer is drained from the render loop, on
//! a pass the engine was making anyway, and it is sized long enough (a fifth
//! of a second) that a wallpaper drawing at 10 fps still never overflows it.
//! Nothing here wakes anything up.
//!
//! It is also opened lazily and dropped the moment nothing wants it: a
//! wallpaper with no bands in it never touches the audio stack at all.
//!
//! The bands come from a Goertzel filter per band rather than an FFT. Eight
//! filters over a thousand samples is about eight thousand multiplies a
//! frame, which is less arithmetic than one row of the blur — and it needs no
//! FFT library, no power-of-two window and no dependency.

use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

/// How many bands a shader gets. Eight is what fits in two constant-buffer
/// registers and is more than anyone can tell apart on a desktop background.
pub const BANDS: usize = 8;

/// The centre of each band, in hertz. Spaced by roughly an octave, because
/// that is how the ear spaces them — a linear split puts six of the eight
/// bands above 10 kHz, where music has almost nothing.
const CENTRES: [f32; BANDS] = [60.0, 130.0, 260.0, 520.0, 1040.0, 2100.0, 4200.0, 8400.0];

/// How many of the most recent samples each reading is taken over.
///
/// At 48 kHz this is 21 ms, which resolves the lowest band (60 Hz has a
/// 17 ms period) and is short enough that a beat lands on the frame it
/// happened in.
const WINDOW: usize = 1024;

/// How much sound the capture buffer holds. Five times a slow frame, so a
/// wallpaper drawing at 10 fps still drains it before Windows drops anything
/// on the floor.
const BUFFER: Duration = Duration::from_millis(200);

/// Attack and release, matching `meter.rs` for the same reasons: a band that
/// arrives late is not a beat, and one that falls away instantly strobes.
const ATTACK: f32 = 0.6;
const RELEASE: f32 = 0.12;

/// How long a capture may go without producing a single packet before it is
/// treated as dead. The usual cause is the user changing output device,
/// which leaves the old endpoint captured and silent forever.
const STALE: Duration = Duration::from_secs(5);

pub struct Spectrum {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    channels: usize,
    rate: f32,
    /// The most recent samples, mono, oldest first. A ring buffer would save
    /// the rotate; the rotate is a 4 KB memmove a frame and the ring would be
    /// a second index to get wrong.
    window: Vec<f32>,
    levels: [f32; BANDS],
    last_packet: Instant,
}

impl Spectrum {
    /// Open a loopback capture on the default output.
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

            // Loopback capture has to take the mixer's own format; unlike the
            // render path there is no resampler to ask for something else.
            let format = client.GetMixFormat()?;
            let channels = (*format).nChannels.max(1) as usize;
            let rate = (*format).nSamplesPerSec.max(1) as f32;
            let bits = (*format).wBitsPerSample;
            let tag = (*format).wFormatTag;

            let started = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                BUFFER.as_nanos() as i64 / 100,
                0,
                format,
                None,
            );
            // GetMixFormat allocates, whatever Initialize made of it.
            CoTaskMemFree(Some(format as *const _));
            started?;

            // Every Windows mixer since Vista runs in 32-bit float. Refusing
            // the rest is better than reading integers as floats, which is a
            // wallpaper reacting to noise. 0xFFFE is WAVE_FORMAT_EXTENSIBLE
            // and 3 is IEEE float.
            if bits != 32 || !(tag == 3 || tag == 0xFFFE) {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "the audio mixer is not running in 32-bit float",
                ));
            }

            let capture: IAudioCaptureClient = client.GetService()?;
            client.Start()?;

            Ok(Self {
                client,
                capture,
                channels,
                rate,
                window: vec![0.0; WINDOW],
                levels: [0.0; BANDS],
                last_packet: Instant::now(),
            })
        }
    }

    /// Drain whatever has been played since the last call and report the
    /// bands, each 0-1. Call once a frame.
    pub fn read(&mut self) -> [f32; BANDS] {
        let mut got_any = false;

        // Everything Windows has for us, however many packets that is. A
        // frame that took longer than usual leaves more than one.
        loop {
            match unsafe { self.capture.GetNextPacketSize() } {
                Ok(frames) if frames > 0 => {}
                _ => break,
            }

            let mut data = std::ptr::null_mut();
            let mut count = 0u32;
            let mut flags = 0u32;
            if unsafe {
                self.capture
                    .GetBuffer(&mut data, &mut count, &mut flags, None, None)
            }
            .is_err()
            {
                break;
            }

            // A silent packet carries no data worth reading — the pointer is
            // allowed to be meaningless — but it is still a packet arriving.
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 == 0 && !data.is_null() {
                let samples = unsafe {
                    std::slice::from_raw_parts(data as *const f32, count as usize * self.channels)
                };
                self.push(samples);
            } else {
                self.push_silence(count as usize);
            }

            got_any = true;
            let _ = unsafe { self.capture.ReleaseBuffer(count) };
        }

        if got_any {
            self.last_packet = Instant::now();
        }

        let raw = analyse(&self.window, self.rate);
        for (level, target) in self.levels.iter_mut().zip(raw) {
            let rate = if target > *level { ATTACK } else { RELEASE };
            *level += (target - *level) * rate;
        }
        self.levels
    }

    /// Whether this capture has stopped producing anything at all — the
    /// output device changed underneath it, usually.
    pub fn stale(&self) -> bool {
        self.last_packet.elapsed() > STALE
    }

    /// Downmix to mono and keep only the most recent `WINDOW` samples.
    fn push(&mut self, interleaved: &[f32]) {
        let mono: Vec<f32> = interleaved
            .chunks_exact(self.channels)
            .map(|frame| frame.iter().sum::<f32>() / self.channels as f32)
            .collect();
        self.append(&mono);
    }

    fn push_silence(&mut self, frames: usize) {
        let zeros = vec![0.0; frames.min(WINDOW)];
        self.append(&zeros);
    }

    fn append(&mut self, mono: &[f32]) {
        if mono.len() >= WINDOW {
            self.window.copy_from_slice(&mono[mono.len() - WINDOW..]);
            return;
        }
        self.window.rotate_left(mono.len());
        let start = WINDOW - mono.len();
        self.window[start..].copy_from_slice(mono);
    }
}

impl Drop for Spectrum {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}

/// One Goertzel pass per band over the window, normalised into 0-1.
///
/// A free function taking a slice so the maths can be tested against a
/// generated tone without an audio device anywhere near it.
fn analyse(window: &[f32], rate: f32) -> [f32; BANDS] {
    let mut out = [0.0f32; BANDS];
    if window.is_empty() || rate <= 0.0 {
        return out;
    }

    for (slot, centre) in out.iter_mut().zip(CENTRES) {
        // Above half the sample rate there is nothing to find, and asking for
        // it produces an alias rather than a zero.
        if centre * 2.0 >= rate {
            continue;
        }

        let omega = 2.0 * std::f32::consts::PI * centre / rate;
        let coefficient = 2.0 * omega.cos();
        let (mut previous, mut older) = (0.0f32, 0.0f32);

        for sample in window {
            let current = sample + coefficient * previous - older;
            older = previous;
            previous = current;
        }

        let power = previous * previous + older * older - coefficient * previous * older;
        let magnitude = power.max(0.0).sqrt() / window.len() as f32;

        // Music is logarithmic and a linear magnitude spends most of its
        // range doing nothing. This lands a quiet passage around a third and
        // a loud one near the top, which is where a wallpaper wants them.
        *slot = (1.0 + magnitude * 40.0).log10().clamp(0.0, 1.0);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure tone at one band's centre must light that band and leave its
    /// neighbours alone. This is the whole contract.
    #[test]
    fn a_tone_lands_in_its_own_band() {
        let rate = 48_000.0;
        let centre = CENTRES[4];
        let window: Vec<f32> = (0..WINDOW)
            .map(|i| (2.0 * std::f32::consts::PI * centre * i as f32 / rate).sin())
            .collect();

        let bands = analyse(&window, rate);
        let loudest = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i);
        assert_eq!(loudest, Some(4), "got {bands:?}");
    }

    #[test]
    fn silence_lights_nothing() {
        let bands = analyse(&[0.0; WINDOW], 48_000.0);
        assert!(bands.iter().all(|b| *b < 0.001), "got {bands:?}");
    }

    /// A clipped stream is the case that would otherwise push a band past 1
    /// and brighten a wallpaper past what its shader expects.
    #[test]
    fn every_band_stays_in_range() {
        let window: Vec<f32> = (0..WINDOW)
            .map(|i| if i % 2 == 0 { 8.0 } else { -8.0 })
            .collect();
        let bands = analyse(&window, 48_000.0);
        assert!(
            bands.iter().all(|b| (0.0..=1.0).contains(b)),
            "got {bands:?}"
        );
    }

    /// A device running at 8 kHz cannot carry the top bands, and asking for
    /// them would alias a low tone into a high one.
    #[test]
    fn bands_above_half_the_sample_rate_stay_dark() {
        let bands = analyse(&[0.5; WINDOW], 8_000.0);
        assert_eq!(bands[BANDS - 1], 0.0);
    }
}
