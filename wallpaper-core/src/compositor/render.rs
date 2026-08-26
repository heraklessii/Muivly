//! D3D11 device, swap chains and the draw call.
//!
//! One device per adapter. Monitors attached to the same GPU share it (and
//! will share a decoded video texture); monitors on a different GPU get their
//! own device, because sharing a texture across adapters costs a trip through
//! system memory and that is exactly what this project refuses to do.

use std::path::Path;
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

/// Matches the `cbuffer Params` in the shader. Constant buffers are sized in
/// 16-byte registers, hence the padding.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Params {
    time: f32,
    _pad0: f32,
    /// Maps screen UV onto the video, cropping the overhanging axis so the
    /// frame fills the monitor without stretching.
    uv_scale: [f32; 2],
    uv_offset: [f32; 2],
    _pad1: [f32; 2],
}

impl Params {
    /// "Cover" fit: scale until both axes are filled, crop the excess. A
    /// wallpaper with letterbox bars would look broken, so the choice is
    /// always to lose some edge rather than show background.
    fn fit(video: (u32, u32), screen: (u32, u32)) -> ([f32; 2], [f32; 2]) {
        let video_aspect = video.0 as f32 / video.1 as f32;
        let screen_aspect = screen.0 as f32 / screen.1 as f32;

        if video_aspect > screen_aspect {
            // Video is wider: show a horizontal slice of it.
            let scale = screen_aspect / video_aspect;
            ([scale, 1.0], [(1.0 - scale) / 2.0, 0.0])
        } else {
            let scale = video_aspect / screen_aspect;
            ([1.0, scale], [0.0, (1.0 - scale) / 2.0])
        }
    }
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
}

pub struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    /// The placeholder gradient, used when there is no video to play.
    ps_gradient: ID3D11PixelShader,
    ps_video: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    params: ID3D11Buffer,
    targets: Vec<Target>,
    /// One decoder for this adapter, feeding every monitor attached to it.
    decoder: Option<VideoDecoder>,
}

impl Renderer {
    /// Build a renderer for every surface on one adapter.
    pub fn new(
        luid: i64,
        surfaces: Vec<Surface>,
        video: Option<&Path>,
    ) -> windows::core::Result<Self> {
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

            let decoder = match video {
                Some(path) => Some(VideoDecoder::open(&device, path)?),
                None => None,
            };

            Ok(Self {
                device,
                context,
                vs,
                ps_gradient,
                ps_video,
                sampler,
                params,
                targets,
                decoder,
            })
        }
    }

    /// Swap the wallpaper without tearing down the device, its swap chains
    /// or its windows — the desktop keeps showing the old frame until the
    /// first new one arrives.
    pub fn set_video(&mut self, video: Option<&Path>) -> windows::core::Result<()> {
        self.decoder = match video {
            Some(path) => Some(VideoDecoder::open(&self.device, path)?),
            None => None,
        };
        Ok(())
    }

    /// The video's pixel dimensions, if a video is playing.
    pub fn video_size(&self) -> Option<(u32, u32)> {
        self.decoder.as_ref().map(|d| {
            let frame = d.frame();
            (frame.width, frame.height)
        })
    }

    pub fn monitor_count(&self) -> usize {
        self.targets.len()
    }

    /// Draw one frame to every visible monitor on this adapter.
    ///
    /// Returns how many were actually drawn. Zero means everything on this
    /// adapter is covered, and the caller can stop spending time here.
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
                if crate::power::is_covered(&target.monitor) {
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

        // One decode for the whole adapter — every monitor attached to it
        // samples the same texture. This is the "one decode, shared texture"
        // rule in practice.
        let video = match &mut self.decoder {
            Some(decoder) => {
                decoder.update(&self.context, elapsed)?;
                Some(decoder.frame())
            }
            None => None,
        };

        unsafe {
            for (target, visible) in self.targets.iter_mut().zip(visible) {
                if !visible {
                    continue;
                }

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

                // The fit differs per monitor: the same video on a 16:9 and a
                // 16:10 screen needs a different crop.
                let (uv_scale, uv_offset) = match &video {
                    Some(frame) => {
                        Params::fit((frame.width, frame.height), (target.width, target.height))
                    }
                    None => ([1.0, 1.0], [0.0, 0.0]),
                };
                self.context.UpdateSubresource(
                    &self.params,
                    0,
                    None,
                    &Params {
                        time,
                        uv_scale,
                        uv_offset,
                        ..Default::default()
                    } as *const _ as *const _,
                    0,
                    0,
                );

                self.context
                    .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                self.context.VSSetShader(&self.vs, None);
                self.context
                    .PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));

                match &video {
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
