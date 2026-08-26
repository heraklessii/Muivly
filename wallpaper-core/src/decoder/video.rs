//! Hardware video decode through Media Foundation.
//!
//! The whole pipeline stays on the GPU. Media Foundation is handed our D3D11
//! device, so its decoder writes into GPU memory and hands back a texture we
//! already own; the only movement is a `CopySubresourceRegion` from the
//! decoder's output into a texture we can sample from, which the GPU performs
//! without the frame ever reaching system memory.
//!
//! That copy is needed because decoder output textures are created with
//! `D3D11_BIND_DECODER` and usually without `D3D11_BIND_SHADER_RESOURCE`, so
//! they cannot be sampled directly. It is a GPU-local blit, not a round trip.
//!
//! One decoder per adapter, shared by every monitor attached to it. See
//! docs/decisions.md.
//!
//! ## Why a thread
//!
//! `ReadSample` is synchronous and its cost is not constant: a keyframe, a
//! cold file, a fragmented moov box each cost far more than the average
//! frame. Paying that on the render thread means the frame it lands in
//! misses its deadline, which is a visible hitch however good the pacing is.
//! So the reader lives on its own thread and hands finished frames over a
//! two-deep channel. Two is deliberate: enough to absorb a spike, few enough
//! that a covered monitor stops the decoder within a frame or two, because
//! the thread blocks on a full channel rather than running ahead.

use std::path::Path;
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::time::Duration;

use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Graphics::Direct3D::D3D11_SRV_DIMENSION_TEXTURE2D;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0,
    D3D11_TEX2D_SRV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIBuffer, IMFSample, IMFSourceReader, MFCreateAttributes, MFCreateDXGIDeviceManager,
    MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Video, MFStartup,
    MFVideoFormat_AV1, MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_HEVC_ES,
    MFVideoFormat_NV12, MFVideoFormat_VP90, MFSTARTUP_NOSOCKET, MF_LOW_LATENCY, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_I8;

use super::{clamp_size, Frame};

/// The most playback time one update may advance, in 100ns units.
///
/// The engine's clock runs whether or not the decoder is asked for frames,
/// and it is not asked while a monitor is covered or while the loop is
/// throttled. Treating that gap as playback time to make up means decoding
/// and discarding every frame inside it, and a gap longer than the clip means
/// running off the end and starting over — which on screen looks like the
/// video snapping back to the beginning every second or so. A wallpaper has
/// nothing to stay in sync with, so a gap is not made up: playback simply
/// resumes where it stopped.
const MAX_STEP: i64 = 2_000_000; // 200 ms

/// How many decoded frames may wait ahead of playback.
const QUEUE: usize = 2;

/// A COM interface being moved to, or shared with, the reader thread.
///
/// windows-rs does not implement `Send` for COM interfaces, because whether
/// one may cross threads is a per-interface question the bindings cannot
/// answer. Here the answer is yes for all three that travel: the source
/// reader is used from exactly one thread at a time (the reader thread owns
/// it outright), `IMFSample` is an ordinary free-threaded COM object, and the
/// D3D11 device has multithread protection turned on before any of this
/// starts — which is the same guarantee Media Foundation itself relies on to
/// decode into it.
struct Sent<T>(T);

// SAFETY: see the type comment above.
unsafe impl<T> Send for Sent<T> {}

/// One frame, decoded and waiting.
struct Decoded {
    /// Presentation timestamp, in the 100ns clock of the clip.
    pts: i64,
    /// Which pass through the clip this frame belongs to. Timestamps restart
    /// from zero at every loop, so without this the consumer could not tell
    /// "the first frame again" from "a frame that is very late".
    pass: u32,
    /// Held until the copy has been issued. The texture below is a slot in
    /// the pool of the decoder, and releasing the sample is what hands that
    /// slot back — after which the decoder is free to write the next frame
    /// over the pixels we were about to read. Keeping only the texture alive
    /// is not enough; the pool tracks the sample, not the resource.
    _sample: Sent<IMFSample>,
    texture: Sent<ID3D11Texture2D>,
    subresource: u32,
}

pub struct VideoDecoder {
    frames: Receiver<Decoded>,
    /// The texture the shader samples, and a view per NV12 plane.
    texture: ID3D11Texture2D,
    luma: ID3D11ShaderResourceView,
    chroma: ID3D11ShaderResourceView,
    width: u32,
    height: u32,

    /// The frame received but not yet due. Media Foundation decodes ahead of
    /// playback; holding one frame back is what turns that into correct
    /// timing rather than a fast-forward.
    pending: Option<Decoded>,
    /// Where playback has reached inside the clip, in 100ns units. This is
    /// the clock of the clip, not that of the engine.
    position: i64,
    /// The engine clock at the previous update, to measure how far to move.
    last_clock: Option<i64>,
    /// Which pass through the clip is on screen.
    pass: u32,
    /// How many times the clip has restarted. A playlist advances on this.
    loops: u32,
    /// Playback rate. 1.0 is the speed the clip was authored at; the decode
    /// itself is untouched, only how fast the clip clock is advanced against
    /// the engine's.
    speed: f32,
    /// Set when the reader thread has stopped for good, so a file that turned
    /// out to have no readable video stream is not polled forever.
    finished: bool,
}

impl VideoDecoder {
    /// Open a file and start decoding it on the adapter of `device`.
    pub fn open(
        device: &ID3D11Device,
        path: &Path,
        max_scale: (u32, u32),
    ) -> windows::core::Result<Self> {
        // The reader is built here, on the calling thread, so a file that
        // cannot be opened is an error the caller sees rather than a thread
        // that quietly dies and a monitor that stays black.
        let (reader, width, height) = open_reader(device, path, max_scale)?;
        let (texture, luma, chroma) = make_sampleable(device, width, height)?;

        let (tx, frames) = sync_channel(QUEUE);
        let reader = Sent(reader);
        std::thread::Builder::new()
            .name("muivly-decode".into())
            .spawn(move || read_loop(reader, tx))
            .map_err(|e| {
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
            })?;

        Ok(Self {
            frames,
            texture,
            luma,
            chroma,
            width,
            height,
            pending: None,
            position: 0,
            last_clock: None,
            pass: 0,
            loops: 0,
            speed: 1.0,
            finished: false,
        })
    }

    /// How many times the clip has played through.
    pub fn loops(&self) -> u32 {
        self.loops
    }

    /// Play faster or slower than the clip was authored at.
    ///
    /// Nothing is asked of the decoder: it produces the same frames at the
    /// same cost either way, and only the clock they are shown against
    /// moves. Slowing a clip down therefore *saves* work — fewer frames are
    /// due per second — and speeding one up never asks the hardware for more
    /// than the file already contains.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.05);
    }

    pub fn frame(&self) -> Frame {
        Frame::Nv12 {
            luma: self.luma.clone(),
            chroma: self.chroma.clone(),
            width: self.width,
            height: self.height,
        }
    }

    /// How long until the next decoded frame is due to be shown.
    ///
    /// This is what lets the render loop wake on the cadence of the video
    /// instead of a fixed grid of its own. A 24 fps clip on a 60 fps grid
    /// would otherwise have some frames held for two ticks and some for
    /// three — even, correct pacing that still reads as stutter, because
    /// what the eye sees is the unevenness, not the frame rate.
    ///
    /// Zero means "as soon as possible": either the frame is already late, or
    /// nothing has arrived yet and the answer is not known until it does.
    pub fn time_to_next(&self) -> Duration {
        let Some(pending) = &self.pending else {
            return Duration::ZERO;
        };

        // A frame from the next pass through the clip starts that pass now.
        if pending.pass != self.pass {
            return Duration::ZERO;
        }

        let remaining = pending.pts - self.position;
        if remaining <= 0 {
            return Duration::ZERO;
        }

        // Clip time divided by the rate gives wall-clock time: at double
        // speed a frame 40 ms away in the clip is 20 ms away on the wall.
        Duration::from_nanos((remaining as f64 * 100.0 / self.speed as f64) as u64)
    }

    /// Advance playback to `elapsed`. Returns true when a new frame was
    /// copied in, false when the current one is still the right one to show.
    pub fn update(
        &mut self,
        context: &ID3D11DeviceContext,
        elapsed: Duration,
    ) -> windows::core::Result<bool> {
        // Media Foundation timestamps are in 100-nanosecond units.
        let clock = (elapsed.as_nanos() / 100) as i64;
        let step = match self.last_clock {
            Some(previous) => (clock - previous).clamp(0, MAX_STEP),
            // The first update of a clip is due immediately, whatever the
            // clock of the engine happens to read: a wallpaper assigned after
            // an hour of uptime starts at its first frame, not an hour in.
            None => 0,
        };
        self.last_clock = Some(clock);
        // Engine time turned into clip time. The cap above is on real
        // elapsed time, so a machine coming back from a suspend is still
        // limited to one step whatever rate is set.
        self.position += (step as f64 * self.speed as f64) as i64;

        let mut advanced = false;

        loop {
            if self.pending.is_none() {
                match self.frames.try_recv() {
                    Ok(frame) => self.pending = Some(frame),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.finished = true;
                        break;
                    }
                }
            }

            let Some(pending) = &self.pending else {
                break;
            };

            if pending.pass != self.pass {
                // The clip has come round again. Timestamps count from zero
                // after a seek, so the clip clock has to as well.
                self.pass = pending.pass;
                self.position = 0;
                self.loops = self.loops.saturating_add(1);
            } else if pending.pts > self.position {
                break;
            }

            // Late frames are dropped rather than shown: catching up matters
            // more than showing every frame of a wallpaper.
            let pending = self.pending.take().expect("checked above");
            unsafe {
                context.CopySubresourceRegion(
                    &self.texture,
                    0,
                    0,
                    0,
                    0,
                    &pending.texture.0,
                    pending.subresource,
                    None,
                );
            }
            // Only now is the copy queued and the slot of the sample free to
            // be written again.
            drop(pending);
            advanced = true;
        }

        Ok(advanced)
    }
}

/// The reader thread: pull samples for as long as anyone is listening.
///
/// Sending blocks when the channel is full, and that is the throttle. Nobody
/// has to tell this thread that a monitor was covered or that the frame rate
/// was turned down — the consumer simply stops taking frames and the decoder
/// stops being asked for them, which is the behaviour the whole project is
/// built around.
fn read_loop(reader: Sent<IMFSourceReader>, tx: std::sync::mpsc::SyncSender<Decoded>) {
    unsafe {
        // Every thread that touches COM needs its own apartment. Multithreaded
        // is what Media Foundation wants; a single-threaded apartment here
        // would serialise its worker threads through this one.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let reader = reader.0;
    let mut pass = 0u32;
    // The last timestamp handed on, to notice frames arriving in the wrong
    // order. A decoder is supposed to reorder B-frames back into
    // presentation order before we ever see them; when something stops it
    // doing that, playback goes subtly jerky and nothing reports an error.
    // Said once, because if it happens it happens for every frame.
    let mut previous_pts = i64::MIN;
    let mut warned = false;

    loop {
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample = None;

        let read = unsafe {
            reader.ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )
        };

        if read.is_err() {
            // A read that fails does not recover on the next try; the stream
            // is gone or the device was lost. Leaving is what tells the
            // consumer, through the channel closing.
            return;
        }

        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            // A wallpaper loops. Rewinding is cheaper than reopening, and it
            // keeps the decoder and its GPU allocations warm.
            if rewind(&reader).is_err() {
                return;
            }
            pass = pass.wrapping_add(1);
            previous_pts = i64::MIN;
            continue;
        }

        let Some(sample) = sample else {
            // A sample-less read with no end-of-stream flag means the reader
            // needs another turn (a format change, or a gap).
            continue;
        };

        if timestamp < previous_pts && !warned {
            warned = true;
            eprintln!(
                "decode: frames are arriving out of order ({} after {}); \
                 playback will look uneven",
                timestamp, previous_pts
            );
        }
        previous_pts = timestamp;

        let decoded = unsafe {
            let Ok(buffer) = sample.GetBufferByIndex(0) else {
                continue;
            };
            let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() else {
                continue;
            };

            let mut resource: Option<ID3D11Texture2D> = None;
            if dxgi
                .GetResource(
                    &ID3D11Texture2D::IID,
                    &mut resource as *mut _ as *mut *mut std::ffi::c_void,
                )
                .is_err()
            {
                continue;
            }
            let Some(resource) = resource else {
                continue;
            };
            let Ok(subresource) = dxgi.GetSubresourceIndex() else {
                continue;
            };

            Decoded {
                pts: timestamp,
                pass,
                _sample: Sent(sample),
                texture: Sent(resource),
                subresource,
            }
        };

        // The consumer went away: nothing left to decode for.
        if tx.send(decoded).is_err() {
            return;
        }
    }
}

/// Which codec a file's video stream is in, in words for a user.
///
/// Read straight from the container, without a decoder — so it works for
/// exactly the file that would not play, which is the only time anybody
/// asks. `None` means the container itself could not be read, in which case
/// the codec is not the interesting part of the answer.
///
/// This exists for the error message. "cannot play clip.webm" tells a user
/// nothing they can act on; "AV1, and this GPU has no AV1 decoder" tells
/// them the file needs converting, and "AV1, and Windows needs the free
/// extension" tells them it does not.
pub fn codec_of(path: &Path) -> Option<&'static str> {
    use std::os::windows::ffi::OsStrExt;

    start_media_foundation().ok()?;

    unsafe {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // No D3D manager and no attributes: nothing is being decoded here,
        // only the stream header read. That also means this cannot fail for
        // the reason the real open failed.
        let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None).ok()?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let media_type = reader.GetNativeMediaType(stream, 0).ok()?;
        let subtype = media_type.GetGUID(&MF_MT_SUBTYPE).ok()?;

        Some(match subtype {
            s if s == MFVideoFormat_H264 => "H.264",
            s if s == MFVideoFormat_HEVC || s == MFVideoFormat_HEVC_ES => "HEVC",
            s if s == MFVideoFormat_VP90 => "VP9",
            s if s == MFVideoFormat_AV1 => "AV1",
            _ => "an unrecognised codec",
        })
    }
}

/// Seek back to the start.
fn rewind(reader: &IMFSourceReader) -> windows::core::Result<()> {
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

    let mut position = PROPVARIANT::default();

    unsafe {
        // A seek target is a PROPVARIANT holding a 100ns timestamp. Zero
        // rewinds to the start.
        (*position.Anonymous.Anonymous).vt = VT_I8;
        (*position.Anonymous.Anonymous).Anonymous.hVal = 0;

        reader.SetCurrentPosition(&GUID::zeroed(), &position)
    }
}

/// Open the file and negotiate an output format we can sample.
///
/// Returns the reader and the frame size it settled on, which is not always
/// the size asked for: the source reader is free to refuse a scale it has no
/// transform for, and a clip at its native size is a better outcome than no
/// clip at all.
fn open_reader(
    device: &ID3D11Device,
    path: &Path,
    max_scale: (u32, u32),
) -> windows::core::Result<(IMFSourceReader, u32, u32)> {
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    // First try without the video processor. Most clips are 8-bit H.264 or
    // HEVC, whose hardware decoders hand back NV12 already — there is
    // nothing for a processor to do, and every one that gets built brings a
    // buffer pool of its own.
    let reader = create_reader(device, path, false)?;
    let native = match set_nv12(&reader, stream) {
        Ok(()) => frame_size(&reader, stream)?,
        // Refused: the decoder produces something that is not NV12 — 10-bit
        // HEVC hands back P010. Converting it needs the processor, so the
        // reader is built again with it. Dropped first so the file is not
        // open twice and the decoder's buffers are not held twice.
        Err(_) => {
            drop(reader);
            let reader = create_reader(device, path, true)?;
            set_nv12(&reader, stream)?;
            let size = frame_size(&reader, stream)?;
            return Ok((reader, size.0, size.1));
        }
    };

    // Scaling is not free and it is not what it looks like. The codec still
    // decodes at the native size — it has to — so asking for a smaller frame
    // inserts a video processor and a second buffer pool on top of the
    // decoder's. Measured on a 4K clip against a 1440p cap: CPU unchanged,
    // memory up. What it saves is the per-frame blit and the shader's
    // sampling, which only becomes worth a whole extra pool when the source
    // is absurdly larger than the screen.
    //
    // Four times the pixels is that line. An 8K clip on a 1440p desktop is
    // scaled; the 4K clip everyone actually has is left alone.
    let source_pixels = native.0 as u64 * native.1 as u64;
    let cap_pixels = max_scale.0 as u64 * max_scale.1 as u64;
    if source_pixels <= cap_pixels.saturating_mul(4) {
        return Ok((reader, native.0, native.1));
    }

    let wanted = clamp_size(native, max_scale);
    if wanted == native {
        return Ok((reader, native.0, native.1));
    }

    // Scaling is the one case that is worth a processor, so the reader is
    // rebuilt with one. Falling back to the reader we already have is the
    // right answer whenever that does not work out: a clip at its native
    // size is a better outcome than no clip at all.
    drop(reader);
    match scaled_reader(device, path, stream, wanted) {
        Ok(scaled) => Ok(scaled),
        Err(_) => {
            let reader = create_reader(device, path, false)?;
            set_nv12(&reader, stream)?;
            Ok((reader, native.0, native.1))
        }
    }
}

/// A reader that has been asked to produce `wanted`, and the size it agreed
/// to. The size is read back rather than assumed: the processor is free to
/// land somewhere near what it was asked for.
fn scaled_reader(
    device: &ID3D11Device,
    path: &Path,
    stream: u32,
    wanted: (u32, u32),
) -> windows::core::Result<(IMFSourceReader, u32, u32)> {
    unsafe {
        let reader = create_reader(device, path, true)?;

        let scaled = MFCreateMediaType()?;
        scaled.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        scaled.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        scaled.SetUINT64(
            &MF_MT_FRAME_SIZE,
            ((wanted.0 as u64) << 32) | wanted.1 as u64,
        )?;
        reader.SetCurrentMediaType(stream, None, &scaled)?;

        let size = frame_size(&reader, stream)?;
        Ok((reader, size.0, size.1))
    }
}

/// Ask the reader for NV12 — the format hardware decoders produce natively.
/// Requesting anything else invites a conversion step.
fn set_nv12(reader: &IMFSourceReader, stream: u32) -> windows::core::Result<()> {
    unsafe {
        let output = MFCreateMediaType()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        // Asking the decoder for a smaller output pool was tried here and
        // does nothing: `MF_SA_MINIMUM_OUTPUT_SAMPLE_COUNT` and
        // `MF_SA_REQUIRED_SAMPLE_COUNT` on the decoder transform are both
        // accepted and leave private bytes exactly where they were. They set
        // a floor, not a ceiling, and the floor that matters is the codec's
        // own reference-frame requirement. See docs/decisions.md.
        reader.SetCurrentMediaType(stream, None, &output)
    }
}

/// Build a source reader bound to our D3D11 device.
///
/// `processing` turns on `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING`,
/// which is the reader's licence to insert a video processor — to scale, or
/// to convert a format we cannot sample. It is off unless something actually
/// needs it, because a processor is a second buffer pool held for the life of
/// the wallpaper.
fn create_reader(
    device: &ID3D11Device,
    path: &Path,
    processing: bool,
) -> windows::core::Result<IMFSourceReader> {
    use std::os::windows::ffi::OsStrExt;

    start_media_foundation()?;

    unsafe {
        let mut token = 0u32;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.expect("MFCreateDXGIDeviceManager succeeded without a manager");
        manager.ResetDevice(device, token)?;

        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 4)?;
        let attributes = attributes.expect("MFCreateAttributes succeeded without attributes");

        // Handing the reader our device is what makes the decode land on the
        // GPU instead of the CPU. Without it Media Foundation happily falls
        // back to a software decoder, which is exactly the outcome this
        // project refuses.
        attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &manager)?;
        attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;

        // Nothing here is a camera. Left on, the reader loads the camera
        // plugin chain for every file opened — DLLs and threads bought for a
        // capture pipeline this program does not have.
        attributes.SetUINT32(&MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS, 1)?;

        // Low latency, which for a wallpaper is not about latency at all: it
        // tells the decoder not to build up the queue of decoded frames it
        // would keep for smooth seeking and playback of a film. Measured on a
        // 4K clip across two adapters, that queue is worth about 70 MB of
        // working set and 85 MB of private bytes — see docs/decisions.md.
        //
        // The risk this carries is real and is the reason it was left alone
        // for so long: some decoders stop reordering B-frames back into
        // presentation order in this mode, and the result is playback that
        // is subtly, unfixably jerky with nothing reporting an error. So the
        // reader thread now watches the timestamps it is handed, and says so
        // if they ever arrive out of order. Nothing has tripped it here; a
        // report that it has is a reason to take this line back out.
        attributes.SetUINT32(&MF_LOW_LATENCY, 1)?;

        if processing {
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        }

        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attributes)
    }
}

/// Start Media Foundation, once for the process.
///
/// `MFStartup` is reference counted and there is no matching `MFShutdown`
/// here — the platform stays up for as long as the engine does — so calling
/// it per file only ran the count up.
///
/// NOSOCKET: this never plays from the network, and asking for the network
/// stack pulls in work we would only pay for.
fn start_media_foundation() -> windows::core::Result<()> {
    use std::sync::OnceLock;
    static STARTED: OnceLock<windows::core::HRESULT> = OnceLock::new();

    let result = *STARTED.get_or_init(|| unsafe {
        match MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) {
            Ok(()) => windows::Win32::Foundation::S_OK,
            Err(e) => e.code(),
        }
    });

    result.ok()
}

/// The frame size the reader is currently producing.
fn frame_size(reader: &IMFSourceReader, stream: u32) -> windows::core::Result<(u32, u32)> {
    unsafe {
        let actual = reader.GetCurrentMediaType(stream)?;
        let packed = actual.GetUINT64(&MF_MT_FRAME_SIZE)?;
        Ok(((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32))
    }
}

/// A texture the shader can read, plus a view per NV12 plane.
///
/// NV12 stores luma at full resolution and the two chroma channels
/// interleaved at half resolution. D3D exposes those as two views over the
/// same texture: R8 for luma, R8G8 for chroma.
fn make_sampleable(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> windows::core::Result<(
    ID3D11Texture2D,
    ID3D11ShaderResourceView,
    ID3D11ShaderResourceView,
)> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };

        let mut texture = None;
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.expect("CreateTexture2D succeeded without a texture");

        let luma = plane_view(device, &texture, DXGI_FORMAT_R8_UNORM)?;
        let chroma = plane_view(device, &texture, DXGI_FORMAT_R8G8_UNORM)?;

        Ok((texture, luma, chroma))
    }
}

fn plane_view(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
) -> windows::core::Result<ID3D11ShaderResourceView> {
    unsafe {
        let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: format,
            ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_SRV {
                    MostDetailedMip: 0,
                    MipLevels: 1,
                },
            },
        };

        let mut view = None;
        device.CreateShaderResourceView(texture, Some(&desc), Some(&mut view))?;
        Ok(view.expect("CreateShaderResourceView succeeded without a view"))
    }
}
