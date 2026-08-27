//! Hardware decode into hardware encode, once, off the render path.
//!
//! The pipeline is a source reader and a sink writer sharing one D3D11
//! device, so a frame is decoded into GPU memory, scaled there, encoded
//! there, and never becomes a buffer in system memory on the way. The device
//! is created here and dropped when the job ends — the engine's own devices
//! belong to the render loop and are not lent out.
//!
//! Audio is re-encoded rather than dropped. A wallpaper with a soundtrack
//! that loses it after being "optimised" is a bug the user cannot undo.

use std::path::Path;

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaType, IMFSinkWriter, IMFSourceReader, MFAudioFormat_AAC,
    MFAudioFormat_Float, MFAudioFormat_PCM, MFCreateAttributes, MFCreateDXGIDeviceManager,
    MFCreateMediaType, MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL, MFMediaType_Audio,
    MFMediaType_Video, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_DISABLE_THROTTLING,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_ANY_STREAM, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READER_MEDIASOURCE,
};

use crate::decoder::clamp_size;

// Kept out of the list above: these three are the encoder-tuning path, and
// grouping them says so.
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoMaxNumRefFrame, ICodecAPI, IMFSinkWriterEx,
};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_UI4};

/// Sample rate and channel count every rewritten soundtrack lands on.
///
/// Not the source's: a wallpaper soundtrack is ambience, and matching an
/// unusual rate exactly would mean carrying whatever the file had — 96 kHz
/// eight-channel included — through an encoder that has no reason to be
/// asked for it.
const AUDIO_RATE: u32 = 48_000;
const AUDIO_CHANNELS: u32 = 2;
/// 96 kbit/s AAC, which for looping ambience is transparent enough and is a
/// tenth of what the video costs.
const AUDIO_BYTES_PER_SECOND: u32 = 12_000;

/// The bitrate a rewritten clip is given, in bits per second.
///
/// Bits per pixel per frame, rather than a fixed number: a 720p loop and a
/// 1440p one have nothing in common, and a single bitrate would either
/// starve one or waste space on the other. 0.07 is at the generous end for
/// H.264 — wallpapers are often gradients and slow pans, where banding is
/// the artefact people notice and it is exactly what a tight bitrate
/// produces.
///
/// The floor keeps a small clip from looking worse than it did; the ceiling
/// is there because past it the file grows and nothing on screen changes.
pub fn bitrate_for(size: (u32, u32), fps: u32) -> u32 {
    let pixels = size.0 as f64 * size.1 as f64;
    let raw = pixels * fps.max(1) as f64 * 0.07;
    (raw as u32).clamp(1_500_000, 12_000_000)
}

/// Decode `source`, scale it to fit `max`, and write it to `destination`.
///
/// `progress` is called with 0.0 to 1.0 as the clip is read. Errors come
/// back as a sentence rather than an `HRESULT`, because the only place they
/// go is a line in the settings window.
pub fn rewrite(
    source: &Path,
    destination: &Path,
    max: (u32, u32),
    fps: u32,
    mut progress: impl FnMut(f32),
) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create the output folder: {e}"))?;
    }

    unsafe { run(source, destination, max, fps, &mut progress) }.map_err(describe)
}

/// Media Foundation's failures are all the same `HRESULT` from a user's
/// point of view, and none of them is worth showing raw.
fn describe(error: windows::core::Error) -> String {
    let code = error.code().0 as u32;
    match code {
        // MF_E_TOPO_CODEC_NOT_FOUND / MF_E_INVALIDMEDIATYPE
        0xC00D5212 | 0xC00D36B4 => {
            "no hardware encoder for this file — Muivly does not convert on the CPU".to_string()
        }
        0xC00D36C4 => "the file's format is not one Windows can read".to_string(),
        _ => format!("could not rewrite the file: {}", error.message()),
    }
}

unsafe fn run(
    source: &Path,
    destination: &Path,
    max: (u32, u32),
    fps: u32,
    progress: &mut impl FnMut(f32),
) -> windows::core::Result<()> {
    unsafe {
        crate::decoder::start_media_foundation()?;

        let device = create_device()?;
        let manager = {
            let mut token = 0u32;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.expect("MFCreateDXGIDeviceManager succeeded without a manager");
            manager.ResetDevice(&device, token)?;
            manager
        };

        let video_stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let audio_stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

        // The reader is allowed a video processor here, unlike during
        // playback: scaling is the entire point of this job, and the pool it
        // costs lives for the length of the rewrite rather than for the
        // length of the wallpaper.
        let reader = {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 4)?;
            let attributes = attributes.expect("MFCreateAttributes succeeded without attributes");
            attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &manager)?;
            attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
            attributes.SetUINT32(&MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS, 1)?;
            MFCreateSourceReaderFromURL(PCWSTR(wide(source).as_ptr()), &attributes)?
        };

        let native = native_size(&reader, video_stream)?;
        let size = clamp_size(native, max);
        let source_fps = frame_rate(&reader, video_stream).unwrap_or((30, 1));
        let target_fps = fps
            .max(1)
            .min(source_fps.0.div_ceil(source_fps.1.max(1)).max(1));

        // NV12 at the size we want, which is where the scale happens.
        let nv12 = MFCreateMediaType()?;
        nv12.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        nv12.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        nv12.SetUINT64(&MF_MT_FRAME_SIZE, packed(size.0, size.1))?;
        reader.SetCurrentMediaType(video_stream, None, &nv12)?;
        // Read back rather than assumed: the processor is free to land near
        // what it was asked for rather than exactly on it, and the encoder
        // has to be told the truth.
        let size = current_size(&reader, video_stream).unwrap_or(size);

        // Whatever the audio is, decoded. `MFAudioFormat_PCM` is what every
        // decoder can produce; the encoder takes it from there.
        let has_audio = set_audio_output(&reader, audio_stream).is_ok();

        let writer = {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 2)?;
            let attributes = attributes.expect("MFCreateAttributes succeeded without attributes");
            attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
            // Nothing is being played, so there is no clock to keep to and
            // no reason to be paced by one.
            attributes.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
            MFCreateSinkWriterFromURL(PCWSTR(wide(destination).as_ptr()), None, &attributes)?
        };

        let out_video = MFCreateMediaType()?;
        out_video.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out_video.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        out_video.SetUINT32(&MF_MT_AVG_BITRATE, bitrate_for(size, target_fps))?;
        out_video.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        out_video.SetUINT64(&MF_MT_FRAME_SIZE, packed(size.0, size.1))?;
        out_video.SetUINT64(&MF_MT_FRAME_RATE, packed(target_fps, 1))?;
        out_video.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, packed(1, 1))?;
        let video_out = writer.AddStream(&out_video)?;

        let in_video = MFCreateMediaType()?;
        in_video.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        in_video.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        in_video.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        in_video.SetUINT64(&MF_MT_FRAME_SIZE, packed(size.0, size.1))?;
        in_video.SetUINT64(&MF_MT_FRAME_RATE, packed(source_fps.0, source_fps.1.max(1)))?;
        in_video.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, packed(1, 1))?;
        writer.SetInputMediaType(video_out, &in_video, None)?;

        // The whole point of the rewrite, other than the frame size: tell
        // the encoder to keep one reference frame instead of the four or
        // more it would choose on its own. See `tune_encoder`.
        let tuned = tune_encoder(&writer, video_out, target_fps);
        println!(
            "optimize: {}",
            if tuned {
                "encoder set to one reference frame"
            } else {
                "encoder kept its own reference frames (it would not be told)"
            }
        );

        let audio_out = if has_audio {
            match add_audio_stream(&writer, &reader, audio_stream) {
                Ok(index) => Some(index),
                // A soundtrack that will not re-encode is not a reason to
                // lose the wallpaper. The video is written silent and the
                // user keeps the original if they want the sound.
                Err(e) => {
                    eprintln!("optimize: dropping audio ({})", e.message());
                    None
                }
            }
        } else {
            None
        };

        // An audio stream nobody is writing is still a stream the reader
        // decodes and hands over, sample after sample, for the length of the
        // clip. Deselecting it is the difference between ignoring that work
        // and not doing it.
        if audio_out.is_none() {
            let _ = reader.SetStreamSelection(audio_stream, false);
        }

        writer.BeginWriting()?;

        let duration = duration_of(&reader).unwrap_or(0);
        // Frames closer together than this are the ones a lower frame rate
        // throws away. A tenth of a frame of slack, so a clip whose
        // timestamps wobble does not lose every other frame.
        let min_gap = 10_000_000i64 / target_fps.max(1) as i64 * 9 / 10;
        let mut last_written: Option<i64> = None;
        let mut reported = -1.0f32;

        loop {
            let mut actual_stream = 0u32;
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample = None;

            reader.ReadSample(
                MF_SOURCE_READER_ANY_STREAM.0 as u32,
                0,
                Some(&mut actual_stream),
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )?;

            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }

            let Some(sample) = sample else {
                // A stream tick — a gap in the audio, usually. Nothing to
                // write and not the end of anything.
                continue;
            };

            if actual_stream == video_stream {
                let keep = match last_written {
                    Some(previous) => timestamp - previous >= min_gap,
                    None => true,
                };
                if keep {
                    last_written = Some(timestamp);
                    writer.WriteSample(video_out, &sample)?;
                }

                if duration > 0 {
                    let done = (timestamp as f32 / duration as f32).clamp(0.0, 1.0);
                    // Reported in percent steps: the UI polls this and a
                    // float that changes every frame is a lock taken 60
                    // times a second for a number nobody can read.
                    if done - reported >= 0.01 {
                        reported = done;
                        progress(done);
                    }
                }
            } else if actual_stream == audio_stream {
                if let Some(index) = audio_out {
                    writer.WriteSample(index, &sample)?;
                }
            }
        }

        writer.Finalize()?;
        progress(1.0);
        Ok(())
    }
}

/// Ask the encoder for the smallest picture buffer a decoder will later need
/// to play the result.
///
/// This is the other half of what "Lighten" is for. A decoder's memory is its
/// reference-frame count times the frame size; the rewrite already fixes the
/// frame size, and this fixes the count. H.264 encoders default to four or
/// more references because they are built for films that get seeked through
/// and scrubbed — a wallpaper loop is neither, and every reference past the
/// first is a full frame of GPU memory held for as long as the wallpaper is
/// on screen.
///
/// A short GOP goes with it: a keyframe a second costs a little bitrate and
/// means a decoder never has to reach far back to build a picture.
///
/// Everything here is best effort. Some encoders refuse to be told, and a
/// clip that is merely no smaller in memory than it would have been is still
/// a clip that was successfully made smaller on screen — so nothing here can
/// fail the job.
unsafe fn tune_encoder(writer: &IMFSinkWriter, stream: u32, fps: u32) -> bool {
    unsafe {
        let Ok(extended) = writer.cast::<IMFSinkWriterEx>() else {
            return false;
        };

        // The encoder is not promised to be the first transform on the
        // stream: a converter can sit in front of it. Whichever one answers
        // to the reference-frame setting is the encoder, which is a more
        // reliable test than its position.
        for index in 0..4 {
            let mut transform = None;
            if extended
                .GetTransformForStream(stream, index, None, &mut transform)
                .is_err()
            {
                break;
            }
            let Some(transform) = transform else { continue };
            let Ok(codec) = transform.cast::<ICodecAPI>() else {
                continue;
            };

            if set_codec_u32(&codec, &CODECAPI_AVEncVideoMaxNumRefFrame, 1).is_err() {
                continue;
            }
            // One keyframe a second. Not fatal on its own — the references
            // are what the memory is made of.
            let _ = set_codec_u32(&codec, &CODECAPI_AVEncMPVGOPSize, fps.max(1));
            return true;
        }

        false
    }
}

/// One unsigned number into an `ICodecAPI` setting.
///
/// `VARIANT` is built by hand rather than through a helper: the windows crate
/// offers no conversion for `VT_UI4`, and every field of the union except the
/// one being written is zero.
unsafe fn set_codec_u32(
    codec: &ICodecAPI,
    key: &windows::core::GUID,
    value: u32,
) -> windows::core::Result<()> {
    unsafe {
        let variant = VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_UI4,
                    Anonymous: VARIANT_0_0_0 { ulVal: value },
                    ..Default::default()
                }),
            },
        };
        codec.SetValue(key, &variant)
    }
}
/// Ask the reader for decoded audio, in the shape the AAC encoder wants.
unsafe fn set_audio_output(reader: &IMFSourceReader, stream: u32) -> windows::core::Result<()> {
    unsafe {
        let pcm = MFCreateMediaType()?;
        pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        pcm.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)?;
        pcm.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)?;

        // Float first for the files whose decoder will not give 16-bit PCM
        // directly; the resampler handles the rest.
        if reader.SetCurrentMediaType(stream, None, &pcm).is_ok() {
            return Ok(());
        }

        let float = MFCreateMediaType()?;
        float.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        float.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
        float.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)?;
        float.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)?;
        reader.SetCurrentMediaType(stream, None, &float)
    }
}

unsafe fn add_audio_stream(
    writer: &IMFSinkWriter,
    reader: &IMFSourceReader,
    stream: u32,
) -> windows::core::Result<u32> {
    unsafe {
        let aac = MFCreateMediaType()?;
        aac.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        aac.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        aac.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        aac.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)?;
        aac.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)?;
        aac.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AUDIO_BYTES_PER_SECOND)?;
        let index = writer.AddStream(&aac)?;

        // Whatever the reader settled on is what the encoder is fed.
        let decoded = reader.GetCurrentMediaType(stream)?;
        writer.SetInputMediaType(index, &decoded, None)?;
        Ok(index)
    }
}

unsafe fn create_device() -> windows::core::Result<ID3D11Device> {
    unsafe {
        let mut device = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
        let device = device.expect("D3D11CreateDevice succeeded without a device");

        // Both the decoder and the encoder touch this device from their own
        // threads.
        if let Ok(multithread) = device.cast::<ID3D10Multithread>() {
            let _ = multithread.SetMultithreadProtected(true);
        }

        Ok(device)
    }
}

fn native_size(reader: &IMFSourceReader, stream: u32) -> windows::core::Result<(u32, u32)> {
    unsafe {
        let native = reader.GetNativeMediaType(stream, 0)?;
        let packed = native.GetUINT64(&MF_MT_FRAME_SIZE)?;
        Ok(unpack(packed))
    }
}

fn current_size(reader: &IMFSourceReader, stream: u32) -> windows::core::Result<(u32, u32)> {
    unsafe {
        let current: IMFMediaType = reader.GetCurrentMediaType(stream)?;
        Ok(unpack(current.GetUINT64(&MF_MT_FRAME_SIZE)?))
    }
}

fn frame_rate(reader: &IMFSourceReader, stream: u32) -> windows::core::Result<(u32, u32)> {
    unsafe {
        let native = reader.GetNativeMediaType(stream, 0)?;
        Ok(unpack(native.GetUINT64(&MF_MT_FRAME_RATE)?))
    }
}

/// The clip's length in 100ns units, for the progress number.
fn duration_of(reader: &IMFSourceReader) -> Option<i64> {
    unsafe {
        let value = reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .ok()?;
        let ticks: u64 = (&value).try_into().ok()?;
        Some(ticks as i64)
    }
}

/// Media Foundation packs a pair of 32-bit numbers into one 64-bit
/// attribute, high half first. Sizes, frame rates and aspect ratios all use
/// it.
fn packed(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn unpack(value: u64) -> (u32, u32) {
    ((value >> 32) as u32, (value & 0xFFFF_FFFF) as u32)
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bigger_frame_gets_a_bigger_bitrate() {
        assert!(bitrate_for((2560, 1440), 30) > bitrate_for((1280, 720), 30));
    }

    #[test]
    fn a_small_clip_still_gets_a_usable_bitrate() {
        // A 480p loop computes to well under a megabit, which looks worse
        // than the file it replaced. The floor is what stops "optimising"
        // from meaning "ruining".
        assert_eq!(bitrate_for((640, 480), 24), 1_500_000);
    }

    #[test]
    fn four_k_is_capped_rather_than_unbounded() {
        assert_eq!(bitrate_for((3840, 2160), 60), 12_000_000);
    }

    #[test]
    fn packing_round_trips() {
        assert_eq!(unpack(packed(1920, 1080)), (1920, 1080));
    }
}
