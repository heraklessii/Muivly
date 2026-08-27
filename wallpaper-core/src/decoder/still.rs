//! Images and GIFs, decoded on the CPU through WIC.
//!
//! This is not a hole in the "decode always happens on the GPU" rule. That
//! rule exists because a video decoded on the CPU burns a core for as long as
//! it is on screen, and no GPU decodes a PNG in the first place. A still
//! image is decoded once, uploaded once, and then costs nothing at all — no
//! decode, no draw, no flip, until something else changes. It is the cheapest
//! wallpaper Muivly can show.
//!
//! A GIF is the same machinery with a clock attached. Frames are composited
//! on a canvas as they are needed rather than all decoded up front: a GIF
//! stores each frame as only the rectangle that changed, and holding every
//! frame expanded to full size would cost more memory than the video path it
//! is meant to be lighter than.

use std::path::Path;
use std::time::Duration;

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder, IWICBitmapSource,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

use super::{clamp_size, Frame};

/// How long a still image counts as "one play" for a playlist set to advance
/// when the item ends. A photo has no end of its own, and a playlist that
/// never moves is not a playlist.
const STILL_LENGTH: Duration = Duration::from_secs(30);

/// What a GIF frame with no delay recorded should be given. Browsers settled
/// on this decades ago and GIFs in the wild are authored against it.
const DEFAULT_DELAY: Duration = Duration::from_millis(100);

/// The most playback time one update may advance. Matches `video.rs`, and is
/// there for the same reason: see the comment in `update`.
const MAX_STEP: Duration = Duration::from_millis(200);

/// Whether this file is one WIC should open rather than Media Foundation.
pub fn is_still(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "jpe" | "bmp" | "gif" | "webp" | "tif" | "tiff" | "jxr" | "dds"
    )
}

/// One frame of an animated image: where it sits and how long it lasts.
struct AnimFrame {
    delay: Duration,
    /// GIF disposal: 2 means clear this rectangle before the next frame.
    /// 0 and 1 mean leave it, and 3 ("restore previous") is treated as 1 —
    /// it is vanishingly rare and the difference is one frame of one pixel
    /// region on files that misuse it.
    disposal: u8,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

pub struct StillDecoder {
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    width: u32,
    height: u32,

    /// Empty for a plain image. One entry per frame for an animation.
    frames: Vec<AnimFrame>,
    /// Kept open so frames can be decoded as they come up.
    decoder: Option<IWICBitmapDecoder>,
    factory: Option<IWICImagingFactory>,
    /// The composited image, BGRA, only used while animating.
    canvas: Vec<u8>,
    index: usize,
    /// How far into the current frame playback has reached.
    within: Duration,
    /// Total time on screen, which is how a still earns a loop count.
    total: Duration,
    last_clock: Option<Duration>,
    loops: u32,
    /// Playback rate. 1.0 is the speed the file was authored at.
    speed: f32,
    /// Set until the first upload: a still is drawn once and then never
    /// again, but it does have to be drawn that once.
    dirty: bool,
}

impl StillDecoder {
    pub fn open(
        device: &ID3D11Device,
        path: &Path,
        max_scale: (u32, u32),
    ) -> windows::core::Result<Self> {
        use std::os::windows::ffi::OsStrExt;

        unsafe {
            // WIC is COM, and COM wants the calling thread in an apartment.
            // Media Foundation may already have done this; asking twice is
            // harmless and cheaper than tracking who got there first.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let decoder = factory.CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )?;

            let count = decoder.GetFrameCount()?;
            let first = decoder.GetFrame(0)?;

            let mut native_width = 0u32;
            let mut native_height = 0u32;
            first.GetSize(&mut native_width, &mut native_height)?;

            if count <= 1 {
                // The simple case, and the one worth optimising for: decode,
                // scale, upload, and never think about it again.
                let (width, height) = clamp_size((native_width, native_height), max_scale);
                let pixels = convert(&factory, &first.cast()?, width, height)?;
                let (texture, view) = make_texture(device, width, height, Some(&pixels))?;

                return Ok(Self {
                    texture,
                    view,
                    width,
                    height,
                    frames: Vec::new(),
                    decoder: None,
                    factory: None,
                    canvas: Vec::new(),
                    index: 0,
                    within: Duration::ZERO,
                    total: Duration::ZERO,
                    last_clock: None,
                    loops: 0,
                    speed: 1.0,
                    dirty: true,
                });
            }

            // An animation is composited at its own size. Scaling every frame
            // would mean a resample per frame on the CPU, which is exactly the
            // per-frame cost this path exists to avoid; GIFs are small enough
            // that the saving would not pay for it.
            let (width, height) = (native_width, native_height);
            let mut frames = Vec::with_capacity(count as usize);
            for i in 0..count {
                frames.push(read_frame_meta(&decoder, i));
            }

            let (texture, view) = make_texture(device, width, height, None)?;

            let mut decoder = Self {
                texture,
                view,
                width,
                height,
                frames,
                decoder: Some(decoder),
                factory: Some(factory),
                canvas: vec![0; (width as usize) * (height as usize) * 4],
                index: 0,
                within: Duration::ZERO,
                total: Duration::ZERO,
                last_clock: None,
                loops: 0,
                speed: 1.0,
                dirty: true,
            };

            decoder.compose(0)?;
            Ok(decoder)
        }
    }

    pub fn frame(&self) -> Frame {
        Frame::Bgra {
            view: self.view.clone(),
            width: self.width,
            height: self.height,
        }
    }

    pub fn loops(&self) -> u32 {
        self.loops
    }

    /// Whether this is a photograph rather than an animation: one frame,
    /// uploaded once, that will never change again. What asks is the slow
    /// drift in `render.rs` — see `Wallpaper::is_photograph`.
    pub fn is_photograph(&self) -> bool {
        self.frames.is_empty()
    }

    /// Play faster or slower than the file was authored at. A still image
    /// ignores this; an animation runs at the rate asked for.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.05);
    }

    pub fn time_to_next(&self) -> Duration {
        if self.dirty {
            return Duration::ZERO;
        }

        match self.frames.get(self.index) {
            // Divided rather than multiplied: at double speed the next frame
            // is due in half the wall-clock time the file asks for.
            Some(frame) => frame.delay.saturating_sub(self.within).div_f32(self.speed),
            // A still never changes again. The render loop caps how long it
            // will actually wait, so this is simply "do not wake for me".
            None => Duration::from_secs(3600),
        }
    }

    pub fn update(
        &mut self,
        context: &ID3D11DeviceContext,
        elapsed: Duration,
    ) -> windows::core::Result<bool> {
        let step = match self.last_clock {
            // Capped for the same reason the video path caps it: the engine
            // clock runs while a monitor is covered and this one is not
            // asked for frames, so treating that gap as playback time means
            // an animation hidden behind a game for an hour resumes an hour
            // further on — thousands of frames stepped through at once, and
            // a loop count that jumps a playlist along with it. A wallpaper
            // has nothing to stay in sync with; it simply resumes.
            Some(previous) => elapsed.saturating_sub(previous).min(MAX_STEP),
            None => Duration::ZERO,
        };
        self.last_clock = Some(elapsed);
        // The clock the file is played against, which is the engine's clock
        // stretched by the speed setting.
        let step = step.mul_f32(self.speed);
        self.total += step;

        if self.frames.is_empty() {
            // A still counts time only so a playlist can move on from it.
            self.loops = (self.total.as_secs() / STILL_LENGTH.as_secs()) as u32;
        } else {
            self.within += step;
            let mut moved = false;

            // A loop rather than a single step: a long stall must not leave
            // the animation stuck one frame behind for the rest of its life.
            while self.within >= self.frames[self.index].delay {
                self.within -= self.frames[self.index].delay;
                self.index += 1;
                if self.index >= self.frames.len() {
                    self.index = 0;
                    self.loops = self.loops.saturating_add(1);
                }
                moved = true;
            }

            if moved {
                self.compose(self.index)?;
            }
        }

        if !self.dirty {
            return Ok(false);
        }
        self.dirty = false;

        if !self.canvas.is_empty() {
            unsafe {
                context.UpdateSubresource(
                    &self.texture,
                    0,
                    None,
                    self.canvas.as_ptr() as *const _,
                    self.width * 4,
                    0,
                );
            }
        }

        Ok(true)
    }

    /// Bring the canvas up to frame `index`.
    fn compose(&mut self, index: usize) -> windows::core::Result<()> {
        let (Some(decoder), Some(factory)) = (self.decoder.clone(), self.factory.clone()) else {
            return Ok(());
        };

        // Coming back round to the first frame starts the canvas clean;
        // otherwise every pass would paint on top of the last one.
        if index == 0 {
            self.canvas.fill(0);
        } else if let Some(previous) = self.frames.get(index - 1) {
            if previous.disposal == 2 {
                self.clear(previous.left, previous.top, previous.width, previous.height);
            }
        }

        let frame = &self.frames[index];
        let (left, top, width, height) = (frame.left, frame.top, frame.width, frame.height);

        unsafe {
            let wic = decoder.GetFrame(index as u32)?;
            let pixels = convert(&factory, &wic.cast()?, width, height)?;
            self.blend(&pixels, left, top, width, height);
        }

        self.dirty = true;
        Ok(())
    }

    /// Wipe a rectangle back to transparent, which on a wallpaper is black.
    fn clear(&mut self, left: u32, top: u32, width: u32, height: u32) {
        for row in 0..height {
            let Some(start) = self.offset(left, top + row) else {
                continue;
            };
            let span = (width as usize * 4).min(self.canvas.len() - start);
            self.canvas[start..start + span].fill(0);
        }
    }

    /// Paint a frame onto the canvas, source-over.
    ///
    /// The pixels are premultiplied, so the blend is `src + dst * (1 - a)`
    /// rather than a lerp. The common case by far is a fully opaque pixel,
    /// which is why that is checked first and copied outright.
    fn blend(&mut self, pixels: &[u8], left: u32, top: u32, width: u32, height: u32) {
        for row in 0..height {
            let Some(start) = self.offset(left, top + row) else {
                continue;
            };
            let source = row as usize * width as usize * 4;

            for column in 0..width as usize {
                let s = source + column * 4;
                let d = start + column * 4;
                if s + 4 > pixels.len() || d + 4 > self.canvas.len() {
                    break;
                }

                let alpha = pixels[s + 3];
                if alpha == 255 {
                    self.canvas[d..d + 4].copy_from_slice(&pixels[s..s + 4]);
                    continue;
                }
                if alpha == 0 {
                    continue;
                }

                let inverse = 255 - alpha as u32;
                for channel in 0..4 {
                    let kept = self.canvas[d + channel] as u32 * inverse / 255;
                    self.canvas[d + channel] = (pixels[s + channel] as u32 + kept).min(255) as u8;
                }
            }
        }
    }

    /// Byte offset of a pixel, or `None` when it falls outside the canvas.
    fn offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some((y as usize * self.width as usize + x as usize) * 4)
    }
}

/// Read what the container says about one frame without decoding its pixels.
///
/// Every field has a fallback: a GIF with no graphic control extension is
/// still a GIF, and a frame with no recorded position starts at the origin.
fn read_frame_meta(decoder: &IWICBitmapDecoder, index: u32) -> AnimFrame {
    let mut meta = AnimFrame {
        delay: DEFAULT_DELAY,
        disposal: 0,
        left: 0,
        top: 0,
        width: 0,
        height: 0,
    };

    unsafe {
        let Ok(frame) = decoder.GetFrame(index) else {
            return meta;
        };

        let mut width = 0u32;
        let mut height = 0u32;
        if frame.GetSize(&mut width, &mut height).is_ok() {
            meta.width = width;
            meta.height = height;
        }

        let Ok(reader) = frame.GetMetadataQueryReader() else {
            return meta;
        };

        // Delay is in hundredths of a second. Zero means "as fast as
        // possible", which every renderer since Netscape has read as the
        // default rather than a busy loop.
        if let Some(hundredths) = query_u16(&reader, "/grctlext/Delay") {
            if hundredths > 0 {
                meta.delay = Duration::from_millis(hundredths as u64 * 10);
            }
        }
        if let Some(disposal) = query_u8(&reader, "/grctlext/Disposal") {
            meta.disposal = disposal;
        }
        if let Some(left) = query_u16(&reader, "/imgdesc/Left") {
            meta.left = left as u32;
        }
        if let Some(top) = query_u16(&reader, "/imgdesc/Top") {
            meta.top = top as u32;
        }
    }

    meta
}

/// Read one metadata value, or `None` if it is absent or the wrong shape.
fn query(
    reader: &windows::Win32::Graphics::Imaging::IWICMetadataQueryReader,
    name: &str,
) -> Option<windows::Win32::System::Com::StructuredStorage::PROPVARIANT> {
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut value = PROPVARIANT::default();

    unsafe {
        reader
            .GetMetadataByName(PCWSTR(wide.as_ptr()), &mut value)
            .ok()?;
    }

    Some(value)
}

fn query_u16(
    reader: &windows::Win32::Graphics::Imaging::IWICMetadataQueryReader,
    name: &str,
) -> Option<u16> {
    let value = query(reader, name)?;
    unsafe { Some((*value.Anonymous.Anonymous).Anonymous.uiVal) }
}

fn query_u8(
    reader: &windows::Win32::Graphics::Imaging::IWICMetadataQueryReader,
    name: &str,
) -> Option<u8> {
    let value = query(reader, name)?;
    unsafe { Some((*value.Anonymous.Anonymous).Anonymous.bVal) }
}

/// Decode a WIC source into premultiplied BGRA at the size asked for.
///
/// Two steps, and both are skipped when they would be no-ops: a scaler only
/// when the size differs, a converter only to reach the one pixel format the
/// shader reads.
unsafe fn convert(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    width: u32,
    height: u32,
) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let mut native_width = 0u32;
        let mut native_height = 0u32;
        source.GetSize(&mut native_width, &mut native_height)?;

        let scaled: IWICBitmapSource = if (native_width, native_height) != (width, height) {
            let scaler = factory.CreateBitmapScaler()?;
            // Fant is the slow, good one. This runs once per image, or once
            // per GIF frame at ten frames a second — the quality is worth
            // more here than the microseconds.
            scaler.Initialize(source, width, height, WICBitmapInterpolationModeFant)?;
            scaler.cast()?
        } else {
            source.clone()
        };

        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaled,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let stride = width as usize * 4;
        let mut pixels = vec![0u8; stride * height as usize];
        converter.CopyPixels(std::ptr::null(), stride as u32, &mut pixels)?;

        Ok(pixels)
    }
}

/// A BGRA texture the shader can sample, optionally filled straight away.
fn make_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    pixels: Option<&[u8]>,
) -> windows::core::Result<(ID3D11Texture2D, ID3D11ShaderResourceView)> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };

        let initial = pixels.map(|data| D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr() as *const _,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        });

        let mut texture = None;
        device.CreateTexture2D(
            &desc,
            initial.as_ref().map(|d| d as *const _),
            Some(&mut texture),
        )?;
        let texture = texture.expect("CreateTexture2D succeeded without a texture");

        let mut view = None;
        device.CreateShaderResourceView(&texture, None, Some(&mut view))?;

        Ok((
            texture,
            view.expect("CreateShaderResourceView succeeded without a view"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_image_extensions_go_to_wic() {
        for name in ["a.png", "b.JPG", "c.gif", "d.WebP", "e.bmp"] {
            assert!(is_still(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn video_extensions_do_not() {
        for name in ["a.mp4", "b.WEBM", "c.mkv", "d.mov", "noextension"] {
            assert!(!is_still(Path::new(name)), "{name}");
        }
    }
}
