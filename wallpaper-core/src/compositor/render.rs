//! D3D11 device, swap chains and the draw call.
//!
//! One device per adapter. Monitors attached to the same GPU share it;
//! monitors on a different GPU get their own device, because sharing a
//! texture across adapters costs a trip through system memory and that is
//! exactly what this project refuses to do.
//!
//! Decoders are keyed by file path, so two monitors showing the same video
//! share one decode without anything having to arrange it. Two monitors
//! showing *different* videos genuinely need two decodes; there is no way
//! around that, which is why the low tiers do not offer it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{DXGI_STATUS_OCCLUDED, HMODULE};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11SamplerState, ID3D11Texture2D, ID3D11VertexShader,
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_SAMPLER_DESC,
    D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT,
    D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory2, IDXGIFactory6, IDXGISwapChain1, DXGI_PRESENT,
    DXGI_PRESENT_TEST, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use super::shader::SOURCE;
use super::window::Surface;
use crate::decoder::VideoDecoder;

/// How a video is mapped onto a screen of a different shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Fill the screen, crop whatever hangs over. No bars, some loss.
    #[default]
    Cover,
    /// Show the whole frame, leave bars where the shapes disagree.
    Contain,
    /// Fill the screen by distorting. Included because some wallpapers are
    /// abstract enough not to care.
    Stretch,
}

impl Fit {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "cover" => Some(Fit::Cover),
            "contain" => Some(Fit::Contain),
            "stretch" => Some(Fit::Stretch),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Fit::Cover => "cover",
            Fit::Contain => "contain",
            Fit::Stretch => "stretch",
        }
    }

    /// UV scale and offset that map screen space onto the video.
    fn uv(self, video: (u32, u32), screen: (u32, u32)) -> ([f32; 2], [f32; 2]) {
        if self == Fit::Stretch || video.1 == 0 || screen.1 == 0 {
            return ([1.0, 1.0], [0.0, 0.0]);
        }

        let video_aspect = video.0 as f32 / video.1 as f32;
        let screen_aspect = screen.0 as f32 / screen.1 as f32;
        let wider = video_aspect > screen_aspect;

        // Cover crops the long axis (a scale below 1 samples a slice of the
        // video); contain shrinks it instead, which needs a scale above 1 so
        // the sample runs off the texture and shows as a bar.
        let ratio = if wider {
            screen_aspect / video_aspect
        } else {
            video_aspect / screen_aspect
        };
        let scale = if self == Fit::Cover {
            ratio
        } else {
            1.0 / ratio
        };

        if wider == (self == Fit::Cover) {
            ([scale, 1.0], [(1.0 - scale) / 2.0, 0.0])
        } else {
            ([1.0, scale], [0.0, (1.0 - scale) / 2.0])
        }
    }
}

/// Matches the `cbuffer Params` in the shader. Constant buffers are sized in
/// 16-byte registers, hence the padding.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Params {
    time: f32,
    /// 1 in contain mode. Sampling past the edge is what produces the bars,
    /// and the shader needs to know to paint those black rather than smear
    /// the clamped edge pixel across them.
    letterbox: f32,
    uv_scale: [f32; 2],
    uv_offset: [f32; 2],
    _pad: [f32; 2],
}

/// One monitor: its window, its swap chain, its view.
struct Target {
    // Held for its Drop: dropping the Surface destroys the window, and that
    // must not happen while the swap chain still points at it.
    _surface: Surface,
    swap_chain: IDXGISwapChain1,
    rtv: ID3D11RenderTargetView,
    monitor: crate::caps::MonitorInfo,
    width: u32,
    height: u32,
    /// Set when DXGI reports the window is fully covered. While it is set the
    /// target is not drawn at all — only cheaply polled.
    occluded: bool,
    /// Which file this monitor shows. `None` means the placeholder gradient.
    video: Option<PathBuf>,
    /// A monitor the user switched off keeps whatever Windows draws there.
    enabled: bool,
}

pub struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    /// The placeholder gradient, used when a monitor has no video.
    ps_gradient: ID3D11PixelShader,
    ps_video: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    params: ID3D11Buffer,
    targets: Vec<Target>,
    /// Keyed by path: monitors showing the same file share one decode.
    decoders: HashMap<PathBuf, VideoDecoder>,
    fit: Fit,
}

impl Renderer {
    /// Build a renderer for every surface on one adapter.
    pub fn new(luid: i64, surfaces: Vec<Surface>) -> windows::core::Result<Self> {
        unsafe {
            let adapter = adapter_by_luid(luid)?;

            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                // VIDEO_SUPPORT is what lets Media Foundation put its decoder
                // on this device. Without it the decode silently lands on the
                // CPU, which this project does not allow.
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;

            let device = device.expect("D3D11CreateDevice succeeded without a device");
            let context = context.expect("D3D11CreateDevice succeeded without a context");

            // Media Foundation decodes on its own threads and touches this
            // device from them. Without multithread protection that is a data
            // race inside the driver, and it shows up as a hang or a corrupt
            // frame rather than an error.
            if let Ok(multithread) = device.cast::<ID3D10Multithread>() {
                let _ = multithread.SetMultithreadProtected(true);
            }

            let factory: IDXGIFactory2 = CreateDXGIFactory1()?;
            let mut targets = Vec::with_capacity(surfaces.len());
            for surface in surfaces {
                targets.push(make_target(&device, &factory, surface)?);
            }

            let (vs, ps_gradient, ps_video) = compile_shaders(&device)?;
            let params = make_params_buffer(&device)?;
            let sampler = make_sampler(&device)?;

            Ok(Self {
                device,
                context,
                vs,
                ps_gradient,
                ps_video,
                sampler,
                params,
                targets,
                decoders: HashMap::new(),
                fit: Fit::default(),
            })
        }
    }

    pub fn monitor_count(&self) -> usize {
        self.targets.len()
    }

    /// Whether this adapter drives the named display.
    pub fn has_monitor(&self, device_name: &str) -> bool {
        self.targets
            .iter()
            .any(|t| t.monitor.device_name == device_name)
    }

    pub fn set_fit(&mut self, fit: Fit) {
        self.fit = fit;
    }

    /// Point one monitor at a file, or at nothing.
    ///
    /// The device, its swap chains and its windows all stay up, so the
    /// desktop keeps showing the previous frame until the first new one
    /// arrives — no flicker, no black gap.
    pub fn set_video(
        &mut self,
        device_name: &str,
        video: Option<&Path>,
    ) -> windows::core::Result<()> {
        let Some(target) = self
            .targets
            .iter_mut()
            .find(|t| t.monitor.device_name == device_name)
        else {
            return Ok(());
        };

        target.video = video.map(|p| p.to_path_buf());
        self.sync_decoders()
    }

    pub fn set_enabled(&mut self, device_name: &str, enabled: bool) -> windows::core::Result<()> {
        if let Some(target) = self
            .targets
            .iter_mut()
            .find(|t| t.monitor.device_name == device_name)
        {
            target.enabled = enabled;
        }
        self.sync_decoders()
    }

    /// Open decoders that are now needed and drop the ones that are not.
    ///
    /// Dropping matters more than opening: a decoder nobody references still
    /// holds GPU buffers and a Media Foundation thread pool.
    fn sync_decoders(&mut self) -> windows::core::Result<()> {
        let wanted: Vec<PathBuf> = self
            .targets
            .iter()
            .filter(|t| t.enabled)
            .filter_map(|t| t.video.clone())
            .collect();

        self.decoders.retain(|path, _| wanted.contains(path));

        for path in wanted {
            if !self.decoders.contains_key(&path) {
                let decoder = VideoDecoder::open(&self.device, &path)?;
                self.decoders.insert(path, decoder);
            }
        }

        Ok(())
    }

    /// How many times each open video has looped. The compositor uses this to
    /// advance a playlist when a clip ends.
    pub fn loop_counts(&self) -> Vec<(PathBuf, u32)> {
        self.decoders
            .iter()
            .map(|(path, decoder)| (path.clone(), decoder.loops()))
            .collect()
    }

    /// Draw one frame to every visible monitor on this adapter.
    ///
    /// Returns how many were actually drawn. Zero means everything on this
    /// adapter is covered or switched off, and the caller can back off.
    pub fn draw(&mut self, elapsed: Duration) -> windows::core::Result<usize> {
        let mut drawn = 0;
        let time = elapsed.as_secs_f32();

        // Work out what is visible *before* decoding. Decoding a frame nobody
        // will see is the single most expensive thing this program could do
        // by accident, and it is exactly what the project promises not to.
        let visible: Vec<bool> = self
            .targets
            .iter_mut()
            .map(|target| {
                if !target.enabled || crate::power::is_covered(&target.monitor) {
                    return false;
                }

                if target.occluded {
                    // A test present asks DXGI whether the window would be
                    // visible, without rendering or flipping anything. This
                    // is the entire cost of a covered monitor.
                    let status = unsafe { target.swap_chain.Present(0, DXGI_PRESENT_TEST) };
                    if status == DXGI_STATUS_OCCLUDED {
                        return false;
                    }
                    target.occluded = false;
                }

                true
            })
            .collect();

        if !visible.iter().any(|v| *v) {
            return Ok(0);
        }

        // Only decode what a visible monitor is actually showing.
        let needed: Vec<PathBuf> = self
            .targets
            .iter()
            .zip(&visible)
            .filter(|(_, visible)| **visible)
            .filter_map(|(target, _)| target.video.clone())
            .collect();

        for (path, decoder) in self.decoders.iter_mut() {
            if needed.contains(path) {
                decoder.update(&self.context, elapsed)?;
            }
        }

        unsafe {
            for (target, visible) in self.targets.iter_mut().zip(visible) {
                if !visible {
                    continue;
                }

                let frame = target
                    .video
                    .as_ref()
                    .and_then(|path| self.decoders.get(path))
                    .map(|decoder| decoder.frame());

                // The fit is per monitor: the same video on a 16:9 and a
                // 16:10 screen needs a different crop.
                let (uv_scale, uv_offset) = match &frame {
                    Some(frame) => self
                        .fit
                        .uv((frame.width, frame.height), (target.width, target.height)),
                    None => ([1.0, 1.0], [0.0, 0.0]),
                };

                self.context.UpdateSubresource(
                    &self.params,
                    0,
                    None,
                    &Params {
                        time,
                        letterbox: if self.fit == Fit::Contain { 1.0 } else { 0.0 },
                        uv_scale,
                        uv_offset,
                        ..Default::default()
                    } as *const _ as *const _,
                    0,
                    0,
                );

                self.context
                    .OMSetRenderTargets(Some(&[Some(target.rtv.clone())]), None);
                self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: target.width as f32,
                    Height: target.height as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));

                self.context
                    .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                self.context.VSSetShader(&self.vs, None);
                self.context
                    .PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));

                match &frame {
                    Some(frame) => {
                        self.context.PSSetShader(&self.ps_video, None);
                        self.context
                            .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
                        self.context.PSSetShaderResources(
                            0,
                            Some(&[Some(frame.luma.clone()), Some(frame.chroma.clone())]),
                        );
                    }
                    None => self.context.PSSetShader(&self.ps_gradient, None),
                }

                // A single triangle large enough to cover the screen. No
                // vertex buffer: the vertex shader derives the corners from
                // SV_VertexID.
                self.context.Draw(3, 0);

                // Sync interval 0: pacing is done by the caller against the
                // tier's target fps. Letting each swap chain block on its own
                // vblank would serialise monitors with different refresh
                // rates and produce a visible hitch.
                let status = target.swap_chain.Present(0, DXGI_PRESENT::default());

                // DXGI_STATUS_OCCLUDED is a *success* code, so it has to be
                // compared for explicitly — treating the result as "did it
                // fail" would miss it and keep rendering behind a fullscreen
                // game forever.
                if status == DXGI_STATUS_OCCLUDED {
                    target.occluded = true;
                } else {
                    status.ok()?;
                    drawn += 1;
                }
            }
        }

        Ok(drawn)
    }
}

unsafe fn adapter_by_luid(luid: i64) -> windows::core::Result<IDXGIAdapter1> {
    unsafe {
        let factory: IDXGIFactory6 = CreateDXGIFactory1()?;

        for i in 0.. {
            let Ok(adapter) = factory.EnumAdapters1(i) else {
                break;
            };
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            let found = (desc.AdapterLuid.HighPart as i64) << 32 | desc.AdapterLuid.LowPart as i64;
            if found == luid {
                return Ok(adapter);
            }
        }

        Err(windows::core::Error::from_thread())
    }
}

unsafe fn make_target(
    device: &ID3D11Device,
    factory: &IDXGIFactory2,
    surface: Surface,
) -> windows::core::Result<Target> {
    unsafe {
        let (width, height) = (surface.monitor.width, surface.monitor.height);
        let monitor = surface.monitor.clone();

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };

        let swap_chain = factory.CreateSwapChainForHwnd(device, surface.hwnd, &desc, None, None)?;

        let back_buffer: ID3D11Texture2D = swap_chain.GetBuffer(0)?;
        let mut rtv = None;
        device.CreateRenderTargetView(&back_buffer, None, Some(&mut rtv))?;

        Ok(Target {
            _surface: surface,
            swap_chain,
            rtv: rtv.expect("CreateRenderTargetView succeeded without a view"),
            monitor,
            width,
            height,
            occluded: false,
            video: None,
            enabled: true,
        })
    }
}

unsafe fn make_params_buffer(device: &ID3D11Device) -> windows::core::Result<ID3D11Buffer> {
    unsafe {
        let initial = Params::default();

        let desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Params>() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };

        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: &initial as *const _ as *const _,
            ..Default::default()
        };

        let mut buffer = None;
        device.CreateBuffer(&desc, Some(&data), Some(&mut buffer))?;
        Ok(buffer.expect("CreateBuffer succeeded without a buffer"))
    }
}

unsafe fn make_sampler(device: &ID3D11Device) -> windows::core::Result<ID3D11SamplerState> {
    unsafe {
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            // Clamp, not wrap: the crop can land a sample fractionally past
            // the edge, and wrapping would show a strip of the opposite side.
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };

        let mut sampler = None;
        device.CreateSamplerState(&desc, Some(&mut sampler))?;
        Ok(sampler.expect("CreateSamplerState succeeded without a sampler"))
    }
}

unsafe fn compile_shaders(
    device: &ID3D11Device,
) -> windows::core::Result<(ID3D11VertexShader, ID3D11PixelShader, ID3D11PixelShader)> {
    unsafe {
        let vs_blob = compile(SOURCE, c"vs_main", c"vs_5_0")?;
        let vs_code = std::slice::from_raw_parts(
            vs_blob.GetBufferPointer() as *const u8,
            vs_blob.GetBufferSize(),
        );

        let mut vs = None;
        device.CreateVertexShader(vs_code, None, Some(&mut vs))?;

        Ok((
            vs.expect("CreateVertexShader succeeded without a shader"),
            pixel_shader(device, c"ps_gradient")?,
            pixel_shader(device, c"ps_video")?,
        ))
    }
}

unsafe fn pixel_shader(
    device: &ID3D11Device,
    entry: &std::ffi::CStr,
) -> windows::core::Result<ID3D11PixelShader> {
    unsafe {
        let blob = compile(SOURCE, entry, c"ps_5_0")?;
        let code =
            std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize());

        let mut ps = None;
        device.CreatePixelShader(code, None, Some(&mut ps))?;
        Ok(ps.expect("CreatePixelShader succeeded without a shader"))
    }
}

unsafe fn compile(
    source: &str,
    entry: &std::ffi::CStr,
    target: &std::ffi::CStr,
) -> windows::core::Result<windows::Win32::Graphics::Direct3D::ID3DBlob> {
    unsafe {
        let mut code = None;
        let mut errors = None;

        let result = D3DCompile(
            source.as_ptr() as *const _,
            source.len(),
            None,
            None,
            None,
            PCSTR(entry.as_ptr() as *const u8),
            PCSTR(target.as_ptr() as *const u8),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        );

        if let Err(e) = result {
            // The compiler's message says which line is wrong; the HRESULT
            // alone says nothing useful.
            if let Some(errors) = errors {
                let text = std::slice::from_raw_parts(
                    errors.GetBufferPointer() as *const u8,
                    errors.GetBufferSize(),
                );
                eprintln!("shader compile failed: {}", String::from_utf8_lossy(text));
            }
            return Err(e);
        }

        Ok(code.expect("D3DCompile succeeded without producing code"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A video and screen of the same shape need no adjustment at all.
    #[test]
    fn matching_aspect_needs_no_adjustment() {
        for fit in [Fit::Cover, Fit::Contain, Fit::Stretch] {
            let (scale, offset) = fit.uv((1920, 1080), (2560, 1440));
            assert!((scale[0] - 1.0).abs() < 0.001, "{fit:?}");
            assert!((scale[1] - 1.0).abs() < 0.001, "{fit:?}");
            assert!(offset[0].abs() < 0.001, "{fit:?}");
            assert!(offset[1].abs() < 0.001, "{fit:?}");
        }
    }

    #[test]
    fn cover_crops_a_wide_video_horizontally() {
        // 21:9 video on a 16:9 screen: the sides are lost.
        let (scale, offset) = Fit::Cover.uv((2560, 1080), (1920, 1080));
        assert!(scale[0] < 1.0);
        assert_eq!(scale[1], 1.0);
        assert!(offset[0] > 0.0, "the crop must be centred");
    }

    #[test]
    fn contain_shrinks_a_wide_video_vertically() {
        // Same video, but now the whole width is kept and bars appear above
        // and below — so the vertical axis samples past the texture.
        let (scale, offset) = Fit::Contain.uv((2560, 1080), (1920, 1080));
        assert_eq!(scale[0], 1.0);
        assert!(scale[1] > 1.0);
        assert!(offset[1] < 0.0);
    }

    #[test]
    fn cover_crops_a_tall_video_vertically() {
        // A 9:16 phone video on a 16:9 screen loses its top and bottom.
        let (scale, offset) = Fit::Cover.uv((1080, 1920), (1920, 1080));
        assert_eq!(scale[0], 1.0);
        assert!(scale[1] < 1.0);
        assert!(offset[1] > 0.0);
    }

    #[test]
    fn stretch_never_adjusts() {
        let (scale, offset) = Fit::Stretch.uv((2560, 1080), (1920, 1080));
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn a_zero_sized_video_does_not_divide_by_zero() {
        let (scale, _) = Fit::Cover.uv((0, 0), (1920, 1080));
        assert_eq!(scale, [1.0, 1.0]);
    }

    #[test]
    fn fit_names_round_trip() {
        for fit in [Fit::Cover, Fit::Contain, Fit::Stretch] {
            assert_eq!(Fit::parse(fit.name()), Some(fit));
        }
        assert_eq!(Fit::parse("nonsense"), None);
    }
}
