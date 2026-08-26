//! Getting out of the way when something else is making noise.
//!
//! Wallpaper sound is background by definition. The moment a video, a game or
//! a call starts, it stops being pleasant and becomes something playing over
//! the thing the user actually wants to hear — and the usual outcome is that
//! they turn wallpaper sound off for good.
//!
//! So the engine watches the other audio sessions on the default output
//! device and goes quiet while any of them is producing sound. Windows has a
//! per-session peak meter, which is the honest signal: a session can sit in
//! the Active state for minutes while playing nothing at all (a browser tab
//! with an idle player is the common case), and reacting to that would mute
//! the wallpaper for the whole time a tab is open.
//!
//! Everything here runs on the audio thread, which already has an MTA
//! apartment and is already awake — so this costs no thread of its own.

use windows::core::Interface;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Threading::GetCurrentProcessId;

/// Peak level, 0.0-1.0, above which another session counts as "playing".
///
/// Not zero: a stream that is running but silent still meters a hair above
/// nothing, and codecs leave dither behind. This is roughly -60 dBFS, which
/// is inaudible but far above that floor.
const AUDIBLE: f32 = 0.001;

/// Watches every other audio session on the default output.
pub struct Duck {
    manager: IAudioSessionManager2,
    own_pid: u32,
}

impl Duck {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;

            Ok(Self {
                manager,
                own_pid: GetCurrentProcessId(),
            })
        }
    }

    /// Whether anything other than Muivly is audible right now.
    ///
    /// The session list is re-read every time rather than cached: sessions
    /// appear and disappear as applications open and close, and a cached
    /// enumerator would keep answering for a media player that has quit.
    pub fn others_playing(&self) -> bool {
        unsafe {
            let Ok(sessions) = self.manager.GetSessionEnumerator() else {
                return false;
            };
            let Ok(count) = sessions.GetCount() else {
                return false;
            };

            for i in 0..count {
                let Ok(control) = sessions.GetSession(i) else {
                    continue;
                };
                let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };

                // Our own soundtrack is not something to duck under.
                if control.GetProcessId().unwrap_or(0) == self.own_pid {
                    continue;
                }

                // The system sounds session is exempt: a notification chime
                // is over before a fade would finish, and ducking for it
                // would leave the wallpaper dipping at random. This one
                // hands back a bare HRESULT in which S_FALSE means "no" —
                // and S_FALSE is a success code, so `is_ok()` here would
                // answer "yes" for every session on the machine.
                if control.IsSystemSoundsSession() == S_OK {
                    continue;
                }

                if control.GetState() != Ok(AudioSessionStateActive) {
                    continue;
                }

                // The meter hangs off the same session object. Where it is
                // not available, an active session is taken at its word.
                let audible = match control.cast::<IAudioMeterInformation>() {
                    Ok(meter) => meter.GetPeakValue().unwrap_or(1.0) > AUDIBLE,
                    Err(_) => true,
                };

                if audible {
                    return true;
                }
            }

            false
        }
    }
}
