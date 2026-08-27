//! Sound for the wallpaper that is playing.
//!
//! Off by default, and deliberately so: a desktop background that makes noise
//! without being asked is a bug in everyone's book. The user turns it on.
//!
//! One stream, not one per monitor. The same clip on two screens is one
//! soundtrack, and two different clips would be two songs at once, which is
//! nobody's idea of a wallpaper — so the audio follows the primary monitor
//! and nothing else.
//!
//! Playback lives on its own thread and talks to WASAPI in shared mode, which
//! is what lets Muivly mix with everything else on the machine instead of
//! seizing the sound card. Media Foundation does the decode; audio decode on
//! the CPU is measured in single-digit percentages of one core and there is
//! no hardware path for it to take.

mod duck;
mod meter;
mod spectrum;

pub use meter::Meter;
pub use spectrum::{Spectrum, BANDS};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::Media::MediaFoundation::{
    IMFSourceReader, MFAudioFormat_Float, MFCreateMediaType, MFCreateSourceReaderFromURL,
    MFMediaType_Audio, MFStartup, MFSTARTUP_NOSOCKET, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

/// What the engine asks for and what the mixer is told to expect. Shared mode
/// resamples whatever the device actually runs at, so these are ours to pick.
/// `WAVE_FORMAT_IEEE_FLOAT`. The bindings expose it only under the multimedia
/// headers this crate does not otherwise need, and it has been 3 since 1996.
const FORMAT_FLOAT: u16 = 3;
const CHANNELS: u16 = 2;
const SAMPLE_RATE: u32 = 48_000;
/// How much sound is queued ahead. Long enough to survive a scheduling hiccup
/// on a busy machine, short enough that a mute is not heard a beat later.
const BUFFER: Duration = Duration::from_millis(200);
/// How often a stopped stream checks whether the desktop is visible again.
/// A fifth of a second is below what anyone notices coming out of a game,
/// and it is five wakeups a second rather than a decoded soundtrack.
const SILENT_POLL: Duration = Duration::from_millis(200);
/// How often the other applications on the machine are checked for sound.
/// Enumerating sessions is not free, and a third of a second plus the fade
/// below is still faster than a person reaches for their volume key.
const DUCK_POLL: Duration = Duration::from_millis(300);
/// How much of the chosen volume is left while something else is playing.
/// Not silence: a wallpaper that vanishes and reappears is more distracting
/// than one that steps back, and this is quiet enough to be under speech.
const DUCK_GAIN: f32 = 0.12;
/// How far the gain may move per buffer chunk. Chunks are a fraction of the
/// 200 ms buffer, so this lands the fade at roughly a fifth of a second —
/// fast enough not to talk over the first word, slow enough not to click.
const FADE_STEP: f32 = 0.05;

/// The controls the playback thread watches. Everything here is read every
/// few milliseconds by that thread and written by the render loop, which is
/// why it is atomics rather than a lock: the render loop must never wait on
/// the sound card.
struct Controls {
    /// Volume as f32 bits. Atomics have no float form.
    volume: AtomicU32,
    muted: AtomicBool,
    stop: AtomicBool,
    /// Whether to step aside while another application is making sound.
    duck: AtomicBool,
    /// Set by the playback thread while it is stepping aside, so the UI can
    /// say why the wallpaper went quiet instead of looking broken.
    ducking: AtomicBool,
}

/// A soundtrack, playing until dropped.
pub struct Audio {
    path: PathBuf,
    controls: Arc<Controls>,
}

impl Audio {
    /// Start playing the audio track of `path`, looping for as long as this
    /// value lives. A file with no audio stream simply plays silence.
    pub fn play(path: &Path, volume: f32, muted: bool, duck: bool) -> Self {
        let controls = Arc::new(Controls {
            volume: AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()),
            muted: AtomicBool::new(muted),
            stop: AtomicBool::new(false),
            duck: AtomicBool::new(duck),
            ducking: AtomicBool::new(false),
        });

        let owned = path.to_path_buf();
        let theirs = Arc::clone(&controls);
        let _ = std::thread::Builder::new()
            .name("muivly-audio".into())
            .spawn(move || {
                if let Err(e) = play_loop(&owned, &theirs) {
                    // Not fatal and not worth a dialog: the wallpaper is the
                    // point, the sound is a bonus, and the usual cause is a
                    // clip that has no audio track at all.
                    eprintln!("audio: {}", e.message());
                }
            });

        Self {
            path: path.to_path_buf(),
            controls,
        }
    }

    /// Which file this soundtrack belongs to, so the caller can tell whether
    /// it still matches what is on screen.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_volume(&self, volume: f32) {
        self.controls
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Go quiet without losing the track. Set the moment every monitor goes
    /// behind a fullscreen window — a wallpaper nobody can see should not be
    /// heard either, and now also should not be decoded: the playback thread
    /// stops its stream and sleeps rather than pushing silence.
    pub fn set_muted(&self, muted: bool) {
        self.controls.muted.store(muted, Ordering::Relaxed);
    }

    /// Whether to stand down while another application is making sound.
    pub fn set_duck(&self, duck: bool) {
        self.controls.duck.store(duck, Ordering::Relaxed);
    }

    /// Whether that is happening right now.
    pub fn is_ducking(&self) -> bool {
        self.controls.ducking.load(Ordering::Relaxed)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.controls.stop.store(true, Ordering::Relaxed);
    }
}

/// Decode and feed the mixer until asked to stop.
fn play_loop(path: &Path, controls: &Controls) -> windows::core::Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)?;

        let reader = open_audio(path)?;
        let (client, render, buffer_frames) = open_device()?;

        client.Start()?;

        let mut leftover: Vec<f32> = Vec::new();
        let mut running = true;

        // A device that will not hand back a session manager is not a reason
        // to play nothing; it is a reason not to duck.
        let watcher = duck::Duck::new()
            .inspect_err(|e| eprintln!("audio: no session watch, ducking off ({})", e.message()))
            .ok();
        let mut ducked = false;
        let mut checked = Instant::now() - DUCK_POLL;
        // Where the fade has actually reached, as a fraction of the chosen
        // volume. Held across iterations: this is the ramp.
        let mut level = 1.0f32;

        while !controls.stop.load(Ordering::Relaxed) {
            // Muted means every monitor is covered — a fullscreen game, a
            // locked screen. Feeding the mixer silence would still decode
            // every sample of a track nobody can hear, on the CPU, which is
            // the one thing this project promises not to do while nothing is
            // visible. So the stream is genuinely stopped and this thread
            // sleeps until the desktop comes back.
            if controls.muted.load(Ordering::Relaxed) {
                if running {
                    let _ = client.Stop();
                    // Whatever was queued belongs to a moment that has
                    // passed; without this it plays as a stutter on resume.
                    let _ = client.Reset();
                    leftover.clear();
                    running = false;
                }
                std::thread::sleep(SILENT_POLL);
                continue;
            }

            if !running {
                client.Start()?;
                running = true;
            }

            // Step aside while anything else on the machine is audible. The
            // check is on its own clock, so it costs the same whether the
            // buffer is being filled every few milliseconds or not at all.
            if let Some(watcher) = &watcher {
                if controls.duck.load(Ordering::Relaxed) {
                    if checked.elapsed() >= DUCK_POLL {
                        checked = Instant::now();
                        ducked = watcher.others_playing();
                        controls.ducking.store(ducked, Ordering::Relaxed);
                    }
                } else if ducked {
                    ducked = false;
                    controls.ducking.store(false, Ordering::Relaxed);
                }
            }

            // How much room the mixer has. Filling it and then waiting is
            // what keeps this thread asleep almost all of the time.
            let padding = client.GetCurrentPadding()?;
            let free = buffer_frames.saturating_sub(padding);

            if free == 0 {
                std::thread::sleep(BUFFER / 4);
                continue;
            }

            if leftover.is_empty() {
                match read_audio(&reader)? {
                    // A read can legitimately come back with nothing — a
                    // gap, a format change. Taking that as "try again
                    // immediately" spins this thread on a core until the
                    // stream recovers, so it waits like every other empty
                    // case here.
                    Some(samples) if samples.is_empty() => {
                        std::thread::sleep(BUFFER / 4);
                        continue;
                    }
                    Some(samples) => leftover = samples,
                    // End of the track: a wallpaper loops, so does its sound.
                    None => {
                        rewind(&reader)?;
                        continue;
                    }
                }
            }

            let wanted = (free as usize).min(leftover.len() / CHANNELS as usize);
            if wanted == 0 {
                leftover.clear();
                continue;
            }

            // The ramp, moved one step per chunk. Jumping straight to the
            // ducked level would be an audible click at the start of every
            // notification and every video.
            let target = if ducked { DUCK_GAIN } else { 1.0 };
            level += (target - level).clamp(-FADE_STEP, FADE_STEP);

            let gain = f32::from_bits(controls.volume.load(Ordering::Relaxed)) * level;

            let taken = wanted * CHANNELS as usize;
            let data = render.GetBuffer(wanted as u32)?;
            let out = std::slice::from_raw_parts_mut(data as *mut f32, taken);
            for (slot, sample) in out.iter_mut().zip(&leftover[..taken]) {
                *slot = sample * gain;
            }
            render.ReleaseBuffer(wanted as u32, 0)?;

            leftover.drain(..taken);
        }

        let _ = client.Stop();
        Ok(())
    }
}

/// Open the file and ask for the one format this module knows how to hand to
/// the mixer: interleaved 32-bit float.
fn open_audio(path: &Path) -> windows::core::Result<IMFSourceReader> {
    use std::os::windows::ffi::OsStrExt;

    unsafe {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None)?;

        let output = MFCreateMediaType()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
        output.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32)?;
        output.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, SAMPLE_RATE)?;
        output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 32)?;

        reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, None, &output)?;

        Ok(reader)
    }
}

/// The default playback device, in shared mode.
///
/// `AUTOCONVERTPCM` is what lets us name our own format instead of matching
/// whatever the device happens to run at — the audio engine resamples, which
/// is exactly what it is there for and cheaper than doing it ourselves.
fn open_device() -> windows::core::Result<(IAudioClient, IAudioRenderClient, u32)> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

        let format = WAVEFORMATEX {
            wFormatTag: FORMAT_FLOAT,
            nChannels: CHANNELS,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: SAMPLE_RATE * CHANNELS as u32 * 4,
            nBlockAlign: CHANNELS * 4,
            wBitsPerSample: 32,
            cbSize: 0,
        };

        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            // Buffer duration is in 100ns units.
            (BUFFER.as_nanos() / 100) as i64,
            0,
            &format,
            None,
        )?;

        let frames = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;

        Ok((client, render, frames))
    }
}

/// One decoded chunk of audio, or `None` at the end of the track.
fn read_audio(reader: &IMFSourceReader) -> windows::core::Result<Option<Vec<f32>>> {
    unsafe {
        let mut flags = 0u32;
        let mut sample = None;

        reader.ReadSample(
            MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
            0,
            None,
            Some(&mut flags),
            None,
            Some(&mut sample),
        )?;

        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(None);
        }

        let Some(sample) = sample else {
            return Ok(Some(Vec::new()));
        };

        let buffer = sample.ConvertToContiguousBuffer()?;
        let mut data = std::ptr::null_mut();
        let mut length = 0u32;
        buffer.Lock(&mut data, None, Some(&mut length))?;

        let samples = std::slice::from_raw_parts(data as *const f32, length as usize / 4).to_vec();

        let _ = buffer.Unlock();
        Ok(Some(samples))
    }
}

fn rewind(reader: &IMFSourceReader) -> windows::core::Result<()> {
    use windows::core::GUID;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Variant::VT_I8;

    let mut position = PROPVARIANT::default();

    unsafe {
        (*position.Anonymous.Anonymous).vt = VT_I8;
        (*position.Anonymous.Anonymous).Anonymous.hVal = 0;
        reader.SetCurrentPosition(&GUID::zeroed(), &position)
    }
}
