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

use std::path::Path;
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
    IMFDXGIBuffer, IMFDXGIDeviceManager, IMFSourceReader, MFCreateAttributes,
    MFCreateDXGIDeviceManager, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Video,
    MFStartup, MFVideoFormat_NV12, MFSTARTUP_NOSOCKET, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READER_D3D_MANAGER, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::System::Variant::VT_I8;

/// A decoded frame, ready to sample: NV12 split into its two planes.
pub struct Frame {
    pub luma: ID3D11ShaderResourceView,
    pub chroma: ID3D11ShaderResourceView,
    pub width: u32,
    pub height: u32,
}

pub struct VideoDecoder {
    reader: IMFSourceReader,
    // Kept alive for the reader: it holds the D3D device the decoder writes to.
    _manager: IMFDXGIDeviceManager,
    texture: ID3D11Texture2D,
    luma: ID3D11ShaderResourceView,
    chroma: ID3D11ShaderResourceView,
    width: u32,
    height: u32,

    /// The frame read but not yet due. Media Foundation decodes ahead of
    /// playback; holding one frame back is what turns that into correct
    /// timing rather than a fast-forward.
    pending: Option<(i64, ID3D11Texture2D, u32)>,
    /// Playback position of the current loop, in 100ns units.
    origin: i64,
    /// How many times the clip has restarted. A playlist advances on this.
    loops: u32,
    finished: bool,
}

impl VideoDecoder {
    /// Open a file and prepare to decode it on `device`'s adapter.
    pub fn open(device: &ID3D11Device, path: &Path) -> windows::core::Result<Self> {
        unsafe {
            // NOSOCKET: this never plays from the network, and asking for the
            // network stack pulls in work we would only pay for.
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)?;

            let mut token = 0u32;
            let mut manager: Option<IMFDXGIDeviceManager> = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.expect("MFCreateDXGIDeviceManager succeeded without a manager");
            manager.ResetDevice(device, token)?;

            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 2)?;
            let attributes = attributes.expect("MFCreateAttributes succeeded without attributes");

            // Handing the reader our device is what makes the decode land on
            // the GPU instead of the CPU. Without it Media Foundation happily
            // falls back to a software decoder, which is exactly the outcome
            // this project refuses.
            attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &manager)?;
            attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;

            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attributes)?;

            // Ask for NV12 — the format hardware decoders produce natively.
            // Requesting anything else invites a conversion step.
            let output = MFCreateMediaType()?;
            output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            reader.SetCurrentMediaType(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                None,
                &output,
            )?;

            let actual =
                reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)?;
            let packed = actual.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let width = (packed >> 32) as u32;
            let height = (packed & 0xFFFF_FFFF) as u32;

            let (texture, luma, chroma) = make_sampleable(device, width, height)?;

            Ok(Self {
                reader,
                _manager: manager,
                texture,
                luma,
                chroma,
                width,
                height,
                pending: None,
                origin: 0,
                loops: 0,
                finished: false,
            })
        }
    }

    /// How many times the clip has played through.
    pub fn loops(&self) -> u32 {
        self.loops
    }

    pub fn frame(&self) -> Frame {
        Frame {
            luma: self.luma.clone(),
            chroma: self.chroma.clone(),
            width: self.width,
            height: self.height,
        }
    }

    /// Advance playback to `elapsed`. Returns true when a new frame was
    /// copied in, false when the current one is still the right one to show.
    pub fn update(
        &mut self,
        context: &ID3D11DeviceContext,
        elapsed: Duration,
    ) -> windows::core::Result<bool> {
        // Media Foundation timestamps are in 100-nanosecond units.
        let now = (elapsed.as_nanos() / 100) as i64 - self.origin;
        let mut advanced = false;

        loop {
            if self.pending.is_none() && !self.read_next(elapsed)? {
                break;
            }

            let Some((pts, _, _)) = &self.pending else {
                break;
            };

            if *pts > now {
                break;
            }

            // Late frames are dropped rather than shown: catching up matters
            // more than showing every frame of a wallpaper.
            let (_, source, subresource) = self.pending.take().expect("checked above");
            unsafe {
                context.CopySubresourceRegion(
                    &self.texture,
                    0,
                    0,
                    0,
                    0,
                    &source,
                    subresource,
                    None,
                );
            }
            advanced = true;
        }

        Ok(advanced)
    }

    /// Pull one sample. Returns false when there is nothing more to read.
    fn read_next(&mut self, elapsed: Duration) -> windows::core::Result<bool> {
        if self.finished {
            return Ok(false);
        }

        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample = None;

        unsafe {
            self.reader.ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )?;
        }

        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            // A wallpaper loops. Rewinding is cheaper than reopening, and it
            // keeps the decoder and its GPU allocations warm.
            self.restart(elapsed)?;
            return Ok(false);
        }

        let Some(sample) = sample else {
            // A sample-less read with no end-of-stream flag means the reader
            // needs another turn (a format change, or a gap).
            return Ok(false);
        };

        unsafe {
            let buffer = sample.GetBufferByIndex(0)?;
            let dxgi: IMFDXGIBuffer = buffer.cast()?;

            let mut resource: Option<ID3D11Texture2D> = None;
            dxgi.GetResource(
                &ID3D11Texture2D::IID,
                &mut resource as *mut _ as *mut *mut std::ffi::c_void,
            )?;
            let resource = resource.expect("IMFDXGIBuffer returned no resource");
            let subresource = dxgi.GetSubresourceIndex()?;

            self.pending = Some((timestamp, resource, subresource));
        }

        Ok(true)
    }

    fn restart(&mut self, elapsed: Duration) -> windows::core::Result<()> {
        use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

        let mut position = PROPVARIANT::default();

        unsafe {
            // A seek target is a PROPVARIANT holding a 100ns timestamp.
            // Zero rewinds to the start.
            (*position.Anonymous.Anonymous).vt = VT_I8;
            (*position.Anonymous.Anonymous).Anonymous.hVal = 0;

            self.reader.SetCurrentPosition(&GUID::zeroed(), &position)?;
        }

        // Restart the clock, so the next frame is due immediately rather than
        // the decoder trying to catch up to wall time it can never reach.
        self.origin = (elapsed.as_nanos() / 100) as i64;
        self.loops = self.loops.saturating_add(1);
        Ok(())
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        // MFShutdown is deliberately not called: another decoder on another
        // adapter may still be running, and Media Foundation is refcounted
        // per MFStartup. The process exiting is what ends it.
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

use std::os::windows::ffi::OsStrExt;
