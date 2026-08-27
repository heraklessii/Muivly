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
use std::time::{Duration, Instant};

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{DXGI_STATUS_OCCLUDED, HMODULE};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11BlendState, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext,
    ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_BUFFER_DESC,
    D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SAMPLER_DESC, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
    D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
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
use crate::decoder::{Frame, Wallpaper};

/// A rectangle in desktop coordinates. Desktop coordinates can be negative —
/// a monitor placed left of the primary one starts below zero — which is why
/// this is not a pair of sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

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

    /// UV scale and offset for one monitor showing its slice of a wallpaper
    /// stretched across every screen.
    ///
    /// The fit is decided once, against the bounding box of the whole desktop
    /// — otherwise each monitor would crop the video its own way and the
    /// picture would not line up across the bezels, which is the entire point
    /// of spanning. The monitor's own rectangle then selects its part of that
    /// mapping.
    ///
    /// Monitors that do not tile the box exactly (a small screen beside a
    /// large one, a stack with a step in it) still line up: each takes the
    /// piece of the image that sits where it does. The desktop-shaped gap
    /// between them is simply never shown, which is what a projector aimed
    /// at that wall would do.
    fn uv_span(self, video: (u32, u32), desktop: Rect, monitor: Rect) -> ([f32; 2], [f32; 2]) {
        if desktop.width == 0 || desktop.height == 0 {
            return ([1.0, 1.0], [0.0, 0.0]);
        }

        let (scale, offset) = self.uv(video, (desktop.width, desktop.height));

        let fx = (monitor.x - desktop.x) as f32 / desktop.width as f32;
        let fy = (monitor.y - desktop.y) as f32 / desktop.height as f32;
        let fw = monitor.width as f32 / desktop.width as f32;
        let fh = monitor.height as f32 / desktop.height as f32;

        (
            [scale[0] * fw, scale[1] * fh],
            [offset[0] + scale[0] * fx, offset[1] + scale[1] * fy],
        )
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

/// How the user wants the wallpaper to look, beyond what the file itself
/// contains. All three are settings, not state: they survive a change of
/// wallpaper and apply to every monitor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visual {
    /// 1.0 leaves the image alone. Below dims, above brightens.
    pub brightness: f32,
    /// 1.0 leaves the image alone. 0.0 is greyscale.
    pub saturation: f32,
    /// 0.0 is sharp, 1.0 is the widest blur on offer.
    pub blur: f32,
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            saturation: 1.0,
            blur: 0.0,
        }
    }
}

/// How much the wallpaper answers to things outside the file.
///
/// Both are amounts, not switches, and both are zero by default — a
/// wallpaper that moves when nothing asked it to is a distraction, and the
/// machines this project is for are the ones where a distraction costs
/// something. At zero neither is measured at all: no meter is opened and
/// the cursor is never asked for.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Motion {
    /// How far the picture answers to the sound coming out of the machine.
    /// 0 is off, 1 is as far as it goes.
    pub reactive: f32,
    /// How far the picture shifts under the cursor. Same range.
    pub parallax: f32,
}

/// The measured half of the pair: what the meter and the cursor actually
/// said this frame, which the compositor reads and the renderer applies.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Drive {
    /// The smoothed output level, 0-1.
    pub level: f32,
    /// Where the cursor is on the desktop, -1 to 1 on each axis, with the
    /// centre at zero.
    pub cursor: (f32, f32),
    /// The same sound split into bands, bass first, each 0-1. Only a shader
    /// reads these, and only one that asked — everything here is zero unless
    /// something on screen declared it wanted them.
    pub bands: [f32; crate::audio::BANDS],
}

/// What one pass of `draw` actually did.
///
/// The two numbers answer different questions and must not be conflated.
/// `live` decides whether the loop may back off; `presented` is what the user
/// is told, and a monitor that showed the same frame again did not present
/// anything even though it is very much still live.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pass {
    pub live: usize,
    pub presented: usize,
}

/// How far the widest blur reaches, in texels of the source frame.
///
/// Nine taps spread this wide is an approximation of a real gaussian, not
/// the thing itself. Wider than this and the taps separate visibly into nine
/// ghosts; this is where it still reads as blur.
const BLUR_TEXELS: f32 = 8.0;

/// Matches the `cbuffer Params` in the shader. Constant buffers are sized in
/// 16-byte registers, hence the layout.
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
    brightness: f32,
    saturation: f32,
    /// One texel times the blur radius, in UV space. Zero switches the blur
    /// off entirely rather than sampling nine times for the same pixel.
    blur_step: [f32; 2],
    /// How much of the outgoing wallpaper is still on screen during a
    /// crossfade. Only `ps_fade` reads it.
    alpha: f32,
    /// D3D11 rejects a constant buffer whose size is not a multiple of 16
    /// bytes, and the fields above come to 44. The shader never reads this.
    _pad: f32,
}

/// Settings a monitor may keep for itself instead of following the desktop.
///
/// All optional, and all default to `None`: a machine with one screen never
/// meets any of this, and a user who never opens the per-monitor panel gets
/// exactly the behaviour they had before it existed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Overrides {
    /// A 32:9 screen and a portrait one rarely want the same crop.
    pub fit: Option<Fit>,
    /// A dim second monitor beside a bright main one, say.
    pub visual: Option<Visual>,
    /// A frame rate cap for this screen alone. The render loop still runs at
    /// the desktop rate; this monitor simply presents less often, which is
    /// where the saving is — a secondary screen at 10 fps costs a tenth of
    /// the flips and none of the extra decode, because the decode is shared.
    pub fps: Option<u32>,
}

/// The frame that was on screen when the wallpaper changed, held just long
/// enough to fade out from underneath the new one.
struct Fade {
    /// A copy of the last screen, already fitted and graded. Dropped the
    /// moment the fade ends — at 4K it is 32 MB, which is not memory this
    /// project leaves lying around for an effect that lasted 300 ms.
    view: ID3D11ShaderResourceView,
    started: Instant,
    length: Duration,
}

impl Fade {
    /// How much of the outgoing frame is left, 1.0 down to 0.0.
    fn alpha(&self) -> f32 {
        fade_alpha(self.started.elapsed(), self.length)
    }

    fn finished(&self) -> bool {
        self.started.elapsed() >= self.length
    }
}

/// The transition curve, as a free function so it can be tested without a
/// GPU texture to hang it off.
///
/// Linear rather than eased: a wallpaper crossfade is two still-ish images
/// swapping over a third of a second, and an ease on that reads as a stall
/// at one end rather than as smoothness.
fn fade_alpha(elapsed: Duration, length: Duration) -> f32 {
    if length.is_zero() {
        return 0.0;
    }
    (1.0 - elapsed.as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
}

/// One monitor: its window, its swap chain, its view.
struct Target {
    // Also held for its Drop: dropping the Surface destroys the window, and
    // that must not happen while the swap chain still points at it.
    surface: Surface,
    swap_chain: IDXGISwapChain1,
    rtv: ID3D11RenderTargetView,
    monitor: crate::caps::MonitorInfo,
    width: u32,
    height: u32,
    /// Set when DXGI reports the window is fully covered. While it is set the
    /// target is not drawn at all — only cheaply polled.
    occluded: bool,
    /// Which file this monitor shows. `None` means nothing of ours belongs
    /// on this screen.
    video: Option<PathBuf>,
    /// A monitor the user switched off shows the Windows wallpaper again.
    enabled: bool,
    /// Whether the surface window is currently in front of the desktop.
    shown: bool,
    /// Set when the screen no longer matches what the shader would draw for
    /// reasons other than a new video frame — the fit changed, the surface
    /// just appeared, a covered monitor came back. Without it the loop would
    /// have to present every tick just in case, which is most of the work a
    /// still frame costs.
    redraw: bool,
    /// What this screen wants for itself, where it differs from the desktop.
    overrides: Overrides,
    /// When this monitor last flipped, for its own frame rate cap.
    presented_at: Option<Instant>,
    /// Set while the previous wallpaper is fading out over this one.
    fade: Option<Fade>,
}

pub struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    ps_video: ID3D11PixelShader,
    ps_image: ID3D11PixelShader,
    /// Draws the outgoing frame over the incoming one during a crossfade.
    ps_fade: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    params: ID3D11Buffer,
    /// Straight alpha blending, bound only for the fade pass. Every other
    /// draw covers the screen completely and wants no blending at all.
    blend: ID3D11BlendState,
    targets: Vec<Target>,
    /// Keyed by path: monitors showing the same file share one decode.
    decoders: HashMap<PathBuf, Wallpaper>,
    fit: Fit,
    visual: Visual,
    /// Playback rate for every decoder here.
    speed: f32,
    /// How long a wallpaper takes to replace the one before it. Zero is a
    /// cut, which is what this did before there was a choice.
    fade: Duration,
    /// Set when one wallpaper is stretched across every screen; carries the
    /// bounding box of the whole desktop, which is what the slices are cut
    /// out of.
    span: Option<Rect>,
    /// What this adapter can decode in hardware, for the message shown when
    /// a file will not open.
    decode: crate::caps::DecodeCaps,
    /// The largest frame any decoder here is asked to produce.
    max_scale: (u32, u32),
    /// How much the wallpaper answers to sound and to the cursor.
    motion: Motion,
    /// What they measured this frame.
    drive: Drive,
    /// How far a photograph drifts on its own. Zero leaves it still.
    drift: f32,
    /// Where each shader file's own settings are set, keyed by path and then
    /// by the name the file declared. Held here rather than in the decoder
    /// because a decoder is dropped whenever the desktop is covered for long
    /// enough, and a slider that reset itself behind a game would be a bug.
    shader_params: HashMap<PathBuf, HashMap<String, f32>>,
    /// Set while every decoder has been handed back because nothing on this
    /// adapter has been visible for long enough to be worth the memory. See
    /// `set_idle`.
    idle: bool,
    /// Files that would not open, waiting to be shown to the user once.
    errors: Vec<String>,
}

/// Everything a draw needs that is not the target it draws into.
///
/// Passed as one borrow bundle so a draw can happen while `targets` is
/// borrowed mutably — the fields are disjoint, but only if they are named
/// individually rather than through `&self`.
struct Shaders<'a> {
    vs: &'a ID3D11VertexShader,
    ps_video: &'a ID3D11PixelShader,
    ps_image: &'a ID3D11PixelShader,
    ps_fade: &'a ID3D11PixelShader,
    sampler: &'a ID3D11SamplerState,
    params: &'a ID3D11Buffer,
    blend: &'a ID3D11BlendState,
}

impl Renderer {
    /// Build a renderer for every surface on one adapter.
    pub fn new(
        luid: i64,
        decode: crate::caps::DecodeCaps,
        surfaces: Vec<Surface>,
        max_scale: (u32, u32),
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

            let (vs, ps_video, ps_image, ps_fade) = compile_shaders(&device)?;
            let params = make_params_buffer(&device)?;
            let sampler = make_sampler(&device)?;
            let blend = make_blend_state(&device)?;

            Ok(Self {
                device,
                context,
                vs,
                ps_video,
                ps_image,
                ps_fade,
                sampler,
                params,
                blend,
                targets,
                decoders: HashMap::new(),
                fit: Fit::default(),
                visual: Visual::default(),
                speed: 1.0,
                fade: Duration::ZERO,
                span: None,
                max_scale,
                motion: Motion::default(),
                drive: Drive::default(),
                drift: 0.0,
                shader_params: HashMap::new(),
                idle: false,
                decode,
                errors: Vec::new(),
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
        if self.fit == fit {
            return;
        }
        self.fit = fit;
        // The crop is baked into what is already on screen, so a still frame
        // would keep the old one until the video happened to advance.
        for target in &mut self.targets {
            target.redraw = true;
        }
    }

    /// Brightness, saturation and blur. Every monitor shares them: they are
    /// a mood for the desktop, not a property of one screen.
    pub fn set_visual(&mut self, visual: Visual) {
        if self.visual == visual {
            return;
        }
        self.visual = visual;
        for target in &mut self.targets {
            target.redraw = true;
        }
    }

    /// Give one monitor settings of its own, or hand it back to the desktop's.
    pub fn set_overrides(&mut self, device_name: &str, overrides: Overrides) {
        if let Some(target) = self
            .targets
            .iter_mut()
            .find(|t| t.monitor.device_name == device_name)
        {
            if target.overrides == overrides {
                return;
            }
            target.overrides = overrides;
            target.redraw = true;
        }
    }

    /// How far the wallpaper is allowed to answer to sound and the cursor.
    pub fn set_motion(&mut self, motion: Motion) {
        if self.motion == motion {
            return;
        }
        self.motion = motion;
        for target in &mut self.targets {
            target.redraw = true;
        }
    }

    /// What the meter and the cursor said this frame.
    ///
    /// Redrawing is the whole cost of these effects on a still frame, so it
    /// is asked for only when the numbers moved enough to change a pixel.
    /// Without the threshold a meter reading its own noise floor would
    /// redraw every monitor, every frame, forever — which is exactly the
    /// bill this project refuses to send a user who has both effects off.
    pub fn set_drive(&mut self, drive: Drive) {
        let moved = (drive.level - self.drive.level).abs() > 0.004
            || (drive.cursor.0 - self.drive.cursor.0).abs() > 0.004
            || (drive.cursor.1 - self.drive.cursor.1).abs() > 0.004;

        self.drive = drive;

        // A shader reads these itself and redraws every frame regardless, so
        // it is handed the numbers whatever the threshold said. Everything
        // else ignores this call.
        for decoder in self.decoders.values_mut() {
            decoder.set_drive(drive);
        }

        if self.motion == Motion::default() || !moved {
            return;
        }
        for target in &mut self.targets {
            target.redraw = true;
        }
    }

    /// How far a photograph is allowed to drift on its own, 0-1.
    ///
    /// Only photographs: a video, a GIF and a shader are already moving, and
    /// a second motion on top of the first is two things fighting rather than
    /// one picture that breathes.
    pub fn set_drift(&mut self, drift: f32) {
        let drift = drift.clamp(0.0, 1.0);
        if (self.drift - drift).abs() < f32::EPSILON {
            return;
        }
        self.drift = drift;
        for target in &mut self.targets {
            target.redraw = true;
        }
    }

    /// Set one shader file's own settings, by the names it declared.
    ///
    /// Kept for files that are not open: a user adjusting a shader that is
    /// only on the second monitor, or one that is hibernating, still expects
    /// the value to be there when it comes back.
    pub fn set_shader_params(&mut self, path: &Path, values: HashMap<String, f32>) {
        let slot = self.shader_params.entry(path.to_path_buf()).or_default();
        for (name, value) in values {
            slot.insert(name.clone(), value);
        }

        let Some(decoder) = self.decoders.get_mut(path) else {
            return;
        };
        let wanted: Vec<(String, f32)> = self
            .shader_params
            .get(path)
            .map(|values| values.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
        for (name, value) in wanted {
            decoder.set_param(&name, value);
        }
    }

    /// What every open wallpaper on this adapter declares it wants sliders
    /// for, with where each one is set right now. Empty for everything that
    /// is not a shader, which is almost everything.
    pub fn declared_params(&self) -> Vec<(PathBuf, Vec<(crate::decoder::ShaderParam, f32)>)> {
        self.decoders
            .iter()
            .filter(|(_, decoder)| !decoder.params().is_empty())
            .map(|(path, decoder)| {
                let values = self.shader_params.get(path);
                let params = decoder
                    .params()
                    .iter()
                    .map(|param| {
                        let value = values
                            .and_then(|set| set.get(&param.name))
                            .copied()
                            .unwrap_or(param.default);
                        (param.clone(), value)
                    })
                    .collect();
                (path.clone(), params)
            })
            .collect()
    }

    /// Whether anything on screen here reads the sound split into bands.
    /// Nothing else opens the loopback capture.
    pub fn wants_bands(&self) -> bool {
        self.showing().any(|decoder| decoder.wants_bands())
    }

    /// Whether anything on screen here is a shader.
    ///
    /// A shader is handed the cursor directly and may use it whether or not
    /// the parallax effect is switched on, so this is what decides whether
    /// the cursor is read at all.
    pub fn has_shader(&self) -> bool {
        self.showing().any(|decoder| decoder.is_program())
    }

    /// The decoders behind what is actually on a screen here — which is not
    /// every open decoder: one belonging to a monitor the user switched off
    /// is open until the next sync and is not on screen.
    fn showing(&self) -> impl Iterator<Item = &Wallpaper> {
        self.targets
            .iter()
            .filter(|t| t.enabled)
            .filter_map(|t| t.video.as_ref())
            .filter_map(|path| self.decoders.get(path))
    }

    /// Change the frame size cap, reopening whatever is playing.
    ///
    /// The cap is baked into a decoder when it opens — it is the size the
    /// video processor was asked for — so a new one only takes effect on a
    /// reopen. That is a visible restart of every clip, which is why this
    /// does nothing at all unless the number actually changed.
    pub fn set_max_scale(&mut self, max_scale: (u32, u32)) {
        if self.max_scale == max_scale {
            return;
        }
        self.max_scale = max_scale;
        self.decoders.clear();
        self.sync_decoders();
        for target in &mut self.targets {
            target.redraw = true;
        }
    }

    /// Playback rate for everything on this adapter.
    pub fn set_speed(&mut self, speed: f32) {
        if (self.speed - speed).abs() < f32::EPSILON {
            return;
        }
        self.speed = speed;
        for decoder in self.decoders.values_mut() {
            decoder.set_speed(speed);
        }
    }

    /// How long the previous wallpaper takes to fade away. Zero cuts.
    pub fn set_fade(&mut self, fade: Duration) {
        self.fade = fade;
    }

    /// Stretch one wallpaper across every screen, or stop doing so.
    ///
    /// The argument is the bounding box of the whole desktop, which only the
    /// compositor can work out — this renderer sees one adapter's monitors
    /// and a spanned image has to be cut from a box that includes the others.
    pub fn set_span(&mut self, desktop: Option<Rect>) {
        if self.span == desktop {
            return;
        }
        self.span = desktop;
        for target in &mut self.targets {
            target.redraw = true;
        }
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
        // Named individually rather than through `self`, so the capture
        // below can read the decoders and the device while this target is
        // borrowed mutably.
        let shaders = Shaders {
            vs: &self.vs,
            ps_video: &self.ps_video,
            ps_image: &self.ps_image,
            ps_fade: &self.ps_fade,
            sampler: &self.sampler,
            params: &self.params,
            blend: &self.blend,
        };
        let (device, context, decoders) = (&self.device, &self.context, &self.decoders);
        let (fit, visual, span, fade) = (self.fit, self.visual, self.span, self.fade);

        let Some(target) = self
            .targets
            .iter_mut()
            .find(|t| t.monitor.device_name == device_name)
        else {
            return Ok(());
        };

        let next = video.map(|p| p.to_path_buf());
        if target.video == next {
            return Ok(());
        }

        // Take a copy of what is on screen so it can fade out from under the
        // new wallpaper. Done here rather than at draw time because the old
        // decoder is about to be dropped — this is the last moment its frame
        // exists. Nothing is captured when there is no fade to run, when the
        // screen is not showing ours anyway, or when it is covered: an
        // invisible fade is a full-screen texture allocated for nobody.
        target.fade = None;
        if !fade.is_zero() && target.shown && target.enabled && !target.occluded {
            if let Some(frame) = target
                .video
                .as_ref()
                .and_then(|path| decoders.get(path))
                .map(|decoder| decoder.frame())
            {
                // No motion on a captured frame: the capture is what was on
                // screen, and re-applying a pulse to it would fade out from
                // a picture the user never saw.
                let look = Look {
                    fit,
                    visual,
                    ..Look::default()
                };
                let params = params_for(target, frame.size(), look, span, 0.0);
                match unsafe { capture(device, context, &shaders, target, &frame, &params) } {
                    Ok(view) => {
                        target.fade = Some(Fade {
                            view,
                            started: Instant::now(),
                            length: fade,
                        })
                    }
                    // A fade is decoration. Losing it costs the user a
                    // transition; refusing the wallpaper would cost them the
                    // wallpaper.
                    Err(e) => eprintln!("fade: {}", e.message()),
                }
            }
        }

        target.video = next;
        target.redraw = true;
        self.sync_decoders();
        Ok(())
    }

    pub fn set_enabled(&mut self, device_name: &str, enabled: bool) -> windows::core::Result<()> {
        if let Some(target) = self
            .targets
            .iter_mut()
            .find(|t| t.monitor.device_name == device_name)
        {
            target.enabled = enabled;
            target.redraw = true;
        }
        self.sync_decoders();
        Ok(())
    }

    /// Hand every decoder back, or open them again.
    ///
    /// A covered desktop already costs no CPU — the render loop stops the
    /// moment nothing is visible. What it does keep is the expensive part:
    /// two decoders' picture buffers and Media Foundation's thread pool,
    /// which on a 4K clip is most of the memory this process uses. Behind a
    /// fullscreen game that is memory held for a picture nobody can see for
    /// as long as the game is open.
    ///
    /// Nothing is captured on the way out. The surfaces stay up and are not
    /// presented to, so the front buffer keeps showing the last frame — the
    /// screen looks exactly as it did, which is the whole trick.
    ///
    /// Waking is a reopen from the start of the file. A wallpaper loops, so
    /// there is no position worth restoring, and seeking back to one would
    /// cost more than the frames it saved.
    pub fn set_idle(&mut self, idle: bool) {
        if self.idle == idle {
            return;
        }
        self.idle = idle;

        if idle {
            self.decoders.clear();
        } else {
            self.sync_decoders();
            // Nothing has been drawn since the decoders went away, and the
            // pixels on screen came from a decoder that no longer exists.
            for target in &mut self.targets {
                target.redraw = true;
            }
        }
    }

    /// The average colour of what one monitor is showing, as BGR bytes.
    ///
    /// Drawn into a 16 by 9 texture and read back, which is 144 pixels — the
    /// GPU does the averaging as part of the scale, and the trip through
    /// system memory is half a kilobyte. This is the one place in the project
    /// that reads pixels back at all, and it happens when a wallpaper
    /// changes rather than when a frame is drawn: at most a few times an
    /// hour, and never at all unless the accent colour is switched on.
    ///
    /// The frame is drawn through the same fit, grade and blur the screen
    /// gets, so the colour is the one the user is actually looking at rather
    /// than the file's.
    pub fn dominant_colour(&self, device_name: &str) -> Option<[u8; 3]> {
        const WIDE: u32 = 16;
        const HIGH: u32 = 9;

        let target = self
            .targets
            .iter()
            .find(|t| t.monitor.device_name == device_name)
            .filter(|t| t.enabled)?;
        let decoder = self.decoders.get(target.video.as_ref()?)?;
        let frame = decoder.frame();

        let shaders = Shaders {
            vs: &self.vs,
            ps_video: &self.ps_video,
            ps_image: &self.ps_image,
            ps_fade: &self.ps_fade,
            sampler: &self.sampler,
            params: &self.params,
            blend: &self.blend,
        };

        let fit = target.overrides.fit.unwrap_or(self.fit);
        let visual = target.overrides.visual.unwrap_or(self.visual);
        let params = params_for(
            target,
            frame.size(),
            Look {
                fit,
                visual,
                ..Look::default()
            },
            self.span,
            0.0,
        );

        unsafe {
            average_colour(
                &self.device,
                &self.context,
                &shaders,
                &frame,
                &params,
                (WIDE, HIGH),
            )
        }
        .ok()
    }

    /// Anything that went wrong since the last call, in words a user can
    /// read. Draining rather than reading: a message is shown once.
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    /// Open decoders that are now needed and drop the ones that are not.
    ///
    /// Dropping matters more than opening: a decoder nobody references still
    /// holds GPU buffers and a Media Foundation thread pool.
    ///
    /// A file that will not open is not fatal. It is usually a codec with no
    /// hardware decoder, or a file that moved — neither is a reason to take
    /// the wallpaper down on every other monitor, so the monitor is cleared,
    /// the reason is recorded for the UI, and the engine carries on.
    fn sync_decoders(&mut self) {
        // Idle is stronger than any assignment: a wallpaper chosen while the
        // desktop is covered is remembered, not opened. It gets its decoder
        // when the desktop comes back.
        if self.idle {
            self.decoders.clear();
            return;
        }

        loop {
            let wanted: Vec<PathBuf> = self
                .targets
                .iter()
                .filter(|t| t.enabled)
                .filter_map(|t| t.video.clone())
                .collect();

            self.decoders.retain(|path, _| wanted.contains(path));

            let Some(path) = wanted
                .into_iter()
                .find(|path| !self.decoders.contains_key(path))
            else {
                return;
            };

            match Wallpaper::open(&self.device, &path, self.max_scale) {
                Ok(mut decoder) => {
                    decoder.set_speed(self.speed);
                    decoder.set_drive(self.drive);

                    // A shader's own settings, remembered from the last time
                    // it was open — or its own defaults, written down here so
                    // the settings window has something to show.
                    let slot = self.shader_params.entry(path.clone()).or_default();
                    let wanted: Vec<(String, f32)> = decoder
                        .params()
                        .iter()
                        .map(|param| {
                            let value = *slot.entry(param.name.clone()).or_insert(param.default);
                            (param.name.clone(), value)
                        })
                        .collect();
                    for (name, value) in wanted {
                        decoder.set_param(&name, value);
                    }

                    self.decoders.insert(path, decoder);
                }
                Err(e) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    // The codec is worth a second look at the file: the
                    // HRESULT alone cannot tell "install this" from "convert
                    // this", and those are the only two things the user can
                    // do about it.
                    let reason = crate::decoder::why_not(&path, self.decode)
                        .unwrap_or_else(|| e.message().to_string());
                    let message = format!("cannot play {name}: {reason}");
                    eprintln!("{message}");
                    self.errors.push(message);

                    // Cleared so the next pass does not try the same file
                    // again, once a frame, forever.
                    for target in &mut self.targets {
                        if target.video.as_deref() == Some(path.as_path()) {
                            target.video = None;
                            target.redraw = true;
                        }
                    }
                }
            }
        }
    }

    /// How many times each open video has looped. The compositor uses this to
    /// advance a playlist when a clip ends.
    pub fn loop_counts(&self) -> Vec<(PathBuf, u32)> {
        self.decoders
            .iter()
            .map(|(path, decoder)| (path.clone(), decoder.loops()))
            .collect()
    }

    /// How long until any monitor on this adapter has a new frame to show.
    ///
    /// `None` means nothing here is playing. Zero means a frame is due now.
    /// The render loop uses the smallest answer across adapters to decide
    /// when to wake up, which is what keeps playback on the cadence of the
    /// video rather than on a grid that only approximately matches it.
    pub fn time_to_next(&self) -> Option<Duration> {
        self.targets
            .iter()
            .filter(|t| t.enabled)
            .filter_map(|t| t.video.as_ref())
            .filter_map(|path| self.decoders.get(path))
            .map(|decoder| {
                // A photograph asks for an hour, because its pixels will
                // never change. While it is drifting the pixels on screen do
                // change — the sampling window is moving over them — so it is
                // due as often as anything else is.
                if self.drift > 0.0 && decoder.is_photograph() {
                    Duration::ZERO
                } else {
                    decoder.time_to_next()
                }
            })
            .min()
    }

    /// Bring every visible monitor on this adapter up to date.
    ///
    /// Zero live means everything on this adapter is covered or switched off,
    /// and the caller can back off. A monitor whose frame has not changed
    /// still counts as live — it is on screen, it just cost nothing this time
    /// round.
    pub fn draw(&mut self, elapsed: Duration) -> windows::core::Result<Pass> {
        let time = elapsed.as_secs_f32();

        // Work out what is visible *before* decoding. Decoding a frame nobody
        // will see is the single most expensive thing this program could do
        // by accident, and it is exactly what the project promises not to.
        let visible: Vec<bool> = self
            .targets
            .iter_mut()
            .map(|target| {
                // Nothing assigned, or switched off: take the surface away so
                // the desktop underneath is what the user sees. Leaving it up
                // would keep the last frame frozen on that monitor forever.
                let wanted = target.enabled && target.video.is_some();
                if wanted != target.shown {
                    target.surface.set_visible(wanted);
                    target.shown = wanted;
                    // A window that just appeared has not been judged yet,
                    // and its back buffer holds nothing worth keeping.
                    target.occluded = false;
                    target.redraw = true;
                }

                if !wanted || crate::power::is_covered(&target.monitor) {
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
                    // Whatever was on screen while it was covered is gone.
                    target.redraw = true;
                }

                true
            })
            .collect();

        if !visible.iter().any(|v| *v) {
            return Ok(Pass::default());
        }

        // Only decode what a visible monitor is actually showing, and note
        // which screens that gave a new frame to. A screen whose frame did
        // not change is still on screen and still correct; presenting it
        // again would be the same pixels at the cost of a full draw and a
        // flip.
        //
        // A flag per target rather than a list of paths: this runs every
        // frame for the life of the process, and cloning a `PathBuf` per
        // monitor per frame is a heap allocation for an answer already
        // sitting in `targets`.
        let mut fresh = vec![false; self.targets.len()];
        // Decoders that will never produce another frame; see below.
        let mut dead: Vec<PathBuf> = Vec::new();

        for (path, decoder) in self.decoders.iter_mut() {
            let wanted =
                self.targets.iter().zip(&visible).any(|(target, shown)| {
                    *shown && target.video.as_deref() == Some(path.as_path())
                });
            if !wanted {
                continue;
            }

            if decoder.update(&self.context, elapsed)? {
                for (index, target) in self.targets.iter().enumerate() {
                    if target.video.as_deref() == Some(path.as_path()) {
                        fresh[index] = true;
                    }
                }
            }

            if decoder.is_finished() {
                dead.push(path.clone());
            }
        }

        // A decoder whose reader thread has stopped for good. Left in place
        // it holds its monitor on one still frame for the rest of the
        // session, keeps counting as live so the loop never rests or
        // hibernates, and tells the user nothing at all. Treated as the file
        // failing to open is treated, because that is what it is — the
        // failure simply arrived later.
        for path in dead {
            self.decoders.remove(&path);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let message = format!("{name} stopped playing: the file or the GPU went away");
            eprintln!("{message}");
            self.errors.push(message);

            for (index, target) in self.targets.iter_mut().enumerate() {
                if target.video.as_deref() == Some(path.as_path()) {
                    target.video = None;
                    target.redraw = true;
                    fresh[index] = false;
                }
            }
        }

        let mut live = 0;
        let mut presented = 0;

        let shaders = Shaders {
            vs: &self.vs,
            ps_video: &self.ps_video,
            ps_image: &self.ps_image,
            ps_fade: &self.ps_fade,
            sampler: &self.sampler,
            params: &self.params,
            blend: &self.blend,
        };
        let (context, decoders) = (&self.context, &self.decoders);
        let (global_fit, global_visual, span) = (self.fit, self.visual, self.span);
        let (motion, drive) = (self.motion, self.drive);
        let drift = self.drift;
        let now = Instant::now();

        unsafe {
            for (index, (target, visible)) in self.targets.iter_mut().zip(visible).enumerate() {
                if !visible {
                    continue;
                }

                // A monitor with a rate of its own presents on that rate.
                // The decode is shared and has already happened, so this
                // saves the draw and the flip — which on an integrated GPU
                // pushing a 4K second screen is most of the cost.
                if let Some(cap) = target.overrides.fps.filter(|fps| *fps > 0) {
                    let due = Duration::from_secs_f64(1.0 / cap as f64);
                    if target
                        .presented_at
                        .is_some_and(|last| now.duration_since(last) < due)
                    {
                        // The frame this tick brought is not being drawn, and
                        // by the tick the cap allows it will no longer be
                        // new. Without this the screen keeps whatever it had
                        // until the *next* new frame happens to land on an
                        // allowed tick — which for a clip slower than the cap
                        // is a monitor stuck on a stale frame.
                        if fresh[index] {
                            target.redraw = true;
                        }
                        live += 1;
                        continue;
                    }
                }

                // The decoder can be a frame behind the assignment when a
                // file has just been picked; nothing to draw yet, and the
                // surface stays where it is rather than flashing.
                let Some(frame) = target
                    .video
                    .as_ref()
                    .and_then(|path| decoders.get(path))
                    .map(|decoder| decoder.frame())
                else {
                    live += 1;
                    continue;
                };

                // A fade has to be redrawn every tick for as long as it
                // lasts: the frame underneath may be perfectly still and the
                // transition is the thing that is moving.
                if target.fade.as_ref().is_some_and(Fade::finished) {
                    target.fade = None;
                    target.redraw = true;
                }
                let fading = target.fade.is_some();

                // A drifting photograph has no new frame — the frame is the
                // same one it will always be — but the window being sampled
                // out of it has moved, so the screen is out of date.
                let drifting = drift > 0.0
                    && target
                        .video
                        .as_ref()
                        .and_then(|path| decoders.get(path))
                        .is_some_and(|decoder| decoder.is_photograph());

                // Nothing new to show and nothing else has changed: the
                // pixels already on screen are the right ones.
                if !fresh[index] && !target.redraw && !fading && !drifting {
                    live += 1;
                    continue;
                }

                // The fit is per monitor twice over: the same wallpaper on a
                // 16:9 and a 16:10 screen needs a different crop, and a
                // screen with settings of its own needs its own everything.
                let fit = target.overrides.fit.unwrap_or(global_fit);
                let visual = target.overrides.visual.unwrap_or(global_visual);
                let params = params_for(
                    target,
                    frame.size(),
                    Look {
                        fit,
                        visual,
                        motion,
                        drive,
                        drift: if drifting { drift } else { 0.0 },
                    },
                    span,
                    time,
                );

                draw_frame(
                    context,
                    &shaders,
                    &target.rtv,
                    (target.width, target.height),
                    &frame,
                    &params,
                );

                // The outgoing wallpaper, painted over the top at whatever
                // the fade has reached. Drawn second because it is the one
                // being taken away — the new frame is the ground.
                if let Some(fade) = &target.fade {
                    let alpha = fade.alpha();
                    let view = fade.view.clone();
                    draw_fade(context, &shaders, &params, &view, alpha);
                }

                // Sync interval 0: pacing is done by the caller against the
                // frame times of the video, capped at the target fps of the
                // tier. Letting each swap chain block on its own vblank would
                // serialise monitors with different refresh rates and produce
                // a visible hitch.
                let status = target.swap_chain.Present(0, DXGI_PRESENT::default());

                // DXGI_STATUS_OCCLUDED is a *success* code, so it has to be
                // compared for explicitly — treating the result as "did it
                // fail" would miss it and keep rendering behind a fullscreen
                // game forever.
                if status == DXGI_STATUS_OCCLUDED {
                    target.occluded = true;
                    target.redraw = true;
                } else {
                    status.ok()?;
                    // What is on screen now matches what the settings ask
                    // for. Until something changes it again, a still frame
                    // costs nothing — which is the whole reason the flag
                    // exists, and it has to be put down for that to be true.
                    target.redraw = false;
                    target.presented_at = Some(now);
                    live += 1;
                    presented += 1;
                }
            }
        }

        Ok(Pass { live, presented })
    }
}

/// Everything that shapes one monitor's picture, as one value.
///
/// Four settings that are decided per monitor and read together on every
/// draw. Passed as a bundle because passing them one by one is how a caller
/// eventually hands `params_for` the desktop's visual and the monitor's fit.
#[derive(Debug, Clone, Copy, Default)]
struct Look {
    fit: Fit,
    visual: Visual,
    motion: Motion,
    drive: Drive,
    /// How far a photograph drifts on its own. Zero for everything that is
    /// already moving.
    drift: f32,
}

/// Everything the shader needs to know about one monitor showing one frame.
fn params_for(
    target: &Target,
    source: (u32, u32),
    look: Look,
    span: Option<Rect>,
    time: f32,
) -> Params {
    let Look {
        fit,
        visual,
        motion,
        drive,
        drift,
    } = look;
    let (source_width, source_height) = source;

    let (uv_scale, uv_offset) = match span {
        Some(desktop) => fit.uv_span(
            source,
            desktop,
            Rect {
                x: target.monitor.x,
                y: target.monitor.y,
                width: target.width,
                height: target.height,
            },
        ),
        None => fit.uv(source, (target.width, target.height)),
    };

    // The blur is expressed in texels of the source, so the same setting
    // looks the same on a 720p clip and a 4K one.
    let blur_step = if visual.blur > 0.0 && source_width > 0 && source_height > 0 {
        let spread = visual.blur * BLUR_TEXELS;
        [spread / source_width as f32, spread / source_height as f32]
    } else {
        [0.0, 0.0]
    };

    // Drift first, then the pulse and the parallax on top of it: the drift is
    // where the picture is looking and the other two are what it does from
    // there.
    let (uv_scale, uv_offset) = apply_drift(uv_scale, uv_offset, drift, time);
    let (uv_scale, uv_offset) = apply_motion(uv_scale, uv_offset, motion, drive);

    Params {
        time,
        letterbox: if fit == Fit::Contain { 1.0 } else { 0.0 },
        uv_scale,
        uv_offset,
        // The sound lifts the picture rather than pushing it: a wallpaper
        // that dims on every beat reads as a fault, and one that brightens
        // reads as the music.
        brightness: visual.brightness * (1.0 + drive.level * motion.reactive * 0.5),
        saturation: visual.saturation,
        blur_step,
        alpha: 1.0,
        _pad: 0.0,
    }
}

/// The slow zoom and pan that makes a photograph look alive.
///
/// Same trick as the pulse and the parallax below: nothing on screen moves,
/// the window being sampled out of the picture does. That is why this is free
/// — it is two more terms in arithmetic the shader was doing anyway, with no
/// second texture, no pass and no decoder. A photograph is the cheapest
/// wallpaper Muivly can show and it stays that way with this switched on; the
/// only cost is that the screen is redrawn on the frame rate rather than
/// once and never again.
///
/// Two periods that do not divide into each other, so the picture never quite
/// repeats a move. A minute and a half for the zoom, two and a half for the
/// pan: slower than anybody watches, which is the point — a photograph that
/// visibly slides is a screensaver.
fn apply_drift(scale: [f32; 2], offset: [f32; 2], drift: f32, time: f32) -> ([f32; 2], [f32; 2]) {
    if drift <= 0.0 {
        return (scale, offset);
    }

    const ZOOM_PERIOD: f32 = 90.0;
    const PAN_PERIOD: f32 = 150.0;
    /// How far in the picture is pushed at full strength. Eight per cent is
    /// enough to have somewhere to pan to and not enough to soften a 1080p
    /// photograph on a 1080p screen.
    const ZOOM: f32 = 0.08;

    let turn = |period: f32| (time / period) * std::f32::consts::TAU;

    // Between 1 and 1 - ZOOM*drift, so the window only ever gets smaller:
    // sampling more than the frame has is what produces a smeared edge.
    let breath = 0.5 - 0.5 * turn(ZOOM_PERIOD).cos();
    let zoom = 1.0 - ZOOM * drift * breath;
    let scaled = [scale[0] * zoom, scale[1] * zoom];

    // Zooming about the centre, then drifting from there.
    let centred = [
        offset[0] + (scale[0] - scaled[0]) / 2.0,
        offset[1] + (scale[1] - scaled[1]) / 2.0,
    ];

    // A circle rather than a line, so the picture comes back to where it
    // started rather than jumping when the period ends.
    let reach = ZOOM * 0.5 * drift;
    let drifted = [
        centred[0] + turn(PAN_PERIOD).sin() * reach,
        centred[1] + turn(PAN_PERIOD * 0.75).cos() * reach,
    ];

    // The same edge guard as the motion below, and for the same reason.
    let clamp = |offset: f32, scale: f32| {
        if scale >= 1.0 {
            offset
        } else {
            offset.clamp(0.0, 1.0 - scale)
        }
    };

    (
        scaled,
        [clamp(drifted[0], scaled[0]), clamp(drifted[1], scaled[1])],
    )
}

/// The sound and the cursor, as a change to where the frame is sampled.
///
/// Both work by moving the sampling window rather than by moving anything on
/// screen: the picture is already being sampled through a scale and an
/// offset for the fit, so a pulse and a parallax shift are two more terms in
/// arithmetic the shader was doing anyway. Neither costs a pass, a texture,
/// or a single extra pixel of memory.
///
/// Zoom, then shift, then keep the window inside the frame. That last step
/// is what stops a hard cursor movement from sampling past the edge, which
/// on a `cover` fit would show as a smeared band down one side.
fn apply_motion(
    scale: [f32; 2],
    offset: [f32; 2],
    motion: Motion,
    drive: Drive,
) -> ([f32; 2], [f32; 2]) {
    if motion == Motion::default() {
        return (scale, offset);
    }

    // A tenth of the frame at full strength and full volume. Past that the
    // wallpaper is bouncing rather than breathing.
    let pulse = 1.0 - drive.level * motion.reactive * 0.1;
    let scaled = [scale[0] * pulse, scale[1] * pulse];

    // Zooming about the centre of the window, not its corner.
    let centred = [
        offset[0] + (scale[0] - scaled[0]) / 2.0,
        offset[1] + (scale[1] - scaled[1]) / 2.0,
    ];

    // Three per cent of the frame at full strength. Small on purpose: this
    // is depth, and a wallpaper that swings under the mouse is a toy.
    let reach = motion.parallax * 0.03;
    let shifted = [
        centred[0] + drive.cursor.0 * reach,
        centred[1] + drive.cursor.1 * reach,
    ];

    // Only where the window is small enough to have somewhere to go. In
    // `contain` the scale is above 1 and the frame is already showing bars;
    // clamping there would fight the letterbox rather than the edge.
    let clamp = |offset: f32, scale: f32| {
        if scale >= 1.0 {
            offset
        } else {
            offset.clamp(0.0, 1.0 - scale)
        }
    };

    (
        scaled,
        [clamp(shifted[0], scaled[0]), clamp(shifted[1], scaled[1])],
    )
}

/// Draw one decoded frame across one target.
///
/// Takes its pieces individually rather than `&Renderer` so it can be called
/// while the target it draws into is borrowed mutably.
fn draw_frame(
    context: &ID3D11DeviceContext,
    shaders: &Shaders<'_>,
    rtv: &ID3D11RenderTargetView,
    size: (u32, u32),
    frame: &Frame,
    params: &Params,
) {
    unsafe {
        context.UpdateSubresource(
            shaders.params,
            0,
            None,
            params as *const _ as *const _,
            0,
            0,
        );

        context.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
        context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: size.0 as f32,
            Height: size.1 as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        // The wallpaper covers every pixel it is given, so nothing blends
        // here. The fade pass turns blending on for itself and this puts it
        // back — a leftover blend state would show as the wallpaper going
        // half-transparent for good.
        context.OMSetBlendState(None, None, u32::MAX);

        bind_common(context, shaders);

        // Which shader and which views depends on what the decoder produced:
        // NV12 planes from a video, one BGRA texture from an image.
        // Everything downstream of the sample is shared.
        match frame {
            Frame::Nv12 { luma, chroma, .. } => {
                context.PSSetShader(shaders.ps_video, None);
                context.PSSetShaderResources(0, Some(&[Some(luma.clone()), Some(chroma.clone())]));
            }
            Frame::Bgra { view, .. } => {
                context.PSSetShader(shaders.ps_image, None);
                context.PSSetShaderResources(2, Some(&[Some(view.clone())]));
            }
        }

        // A single triangle large enough to cover the screen. No vertex
        // buffer: the vertex shader derives the corners from SV_VertexID.
        context.Draw(3, 0);
    }
}

/// Paint the outgoing wallpaper over the incoming one, `alpha` of the way.
fn draw_fade(
    context: &ID3D11DeviceContext,
    shaders: &Shaders<'_>,
    params: &Params,
    view: &ID3D11ShaderResourceView,
    alpha: f32,
) {
    unsafe {
        // Same parameters as the frame underneath, except that the capture
        // is already fitted and graded — so the crop is neutral and only the
        // alpha carries the transition.
        let params = Params {
            uv_scale: [1.0, 1.0],
            uv_offset: [0.0, 0.0],
            letterbox: 0.0,
            blur_step: [0.0, 0.0],
            alpha,
            ..*params
        };

        context.UpdateSubresource(
            shaders.params,
            0,
            None,
            &params as *const _ as *const _,
            0,
            0,
        );

        context.OMSetBlendState(Some(shaders.blend), None, u32::MAX);
        bind_common(context, shaders);
        context.PSSetShader(shaders.ps_fade, None);
        context.PSSetShaderResources(2, Some(&[Some(view.clone())]));
        context.Draw(3, 0);
        context.OMSetBlendState(None, None, u32::MAX);
    }
}

/// The state every pass shares: one triangle, one vertex shader, one
/// constant buffer, one sampler.
fn bind_common(context: &ID3D11DeviceContext, shaders: &Shaders<'_>) {
    unsafe {
        context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context.VSSetShader(shaders.vs, None);
        context.PSSetConstantBuffers(0, Some(&[Some(shaders.params.clone())]));
        context.PSSetSamplers(0, Some(&[Some(shaders.sampler.clone())]));
    }
}

/// Render the frame that is on screen into a texture of its own, so it can
/// be faded out after the decoder behind it is gone.
///
/// The copy is screen-shaped rather than source-shaped: it is taken after
/// the fit, the grade and the blur, which means the fade needs no state
/// beyond the pixels and cannot disagree with what the user was looking at.
unsafe fn capture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    shaders: &Shaders<'_>,
    target: &Target,
    frame: &Frame,
    params: &Params,
) -> windows::core::Result<ID3D11ShaderResourceView> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: target.width,
            Height: target.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            ..Default::default()
        };

        let mut texture = None;
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.expect("CreateTexture2D succeeded without a texture");

        let mut rtv = None;
        device.CreateRenderTargetView(&texture, None, Some(&mut rtv))?;
        let rtv = rtv.expect("CreateRenderTargetView succeeded without a view");

        // The same draw the monitor was already doing, aimed at the capture
        // instead of at the screen.
        draw_frame(
            context,
            shaders,
            &rtv,
            (target.width, target.height),
            frame,
            params,
        );

        let mut view = None;
        device.CreateShaderResourceView(&texture, None, Some(&mut view))?;
        Ok(view.expect("CreateShaderResourceView succeeded without a view"))
    }
}

/// Draw one frame very small and read the result back, averaged.
///
/// The scale is the average: a bilinear downsample to 16 by 9 is the GPU
/// doing the arithmetic, and what comes back is small enough that summing it
/// on the CPU is 144 additions.
unsafe fn average_colour(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    shaders: &Shaders<'_>,
    frame: &Frame,
    params: &Params,
    size: (u32, u32),
) -> windows::core::Result<[u8; 3]> {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            ..Default::default()
        };

        let mut small = None;
        device.CreateTexture2D(&desc, None, Some(&mut small))?;
        let small = small.expect("CreateTexture2D succeeded without a texture");

        let mut rtv = None;
        device.CreateRenderTargetView(&small, None, Some(&mut rtv))?;
        let rtv = rtv.expect("CreateRenderTargetView succeeded without a view");

        draw_frame(context, shaders, &rtv, size, frame, params);

        // Nothing can be read from a texture the GPU owns; a staging copy is
        // the only way back to the CPU.
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        device.CreateTexture2D(&desc, None, Some(&mut staging))?;
        let staging = staging.expect("CreateTexture2D succeeded without a texture");
        context.CopyResource(&staging, &small);

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

        let mut totals = [0u64; 3];
        for row in 0..size.1 as usize {
            // The row pitch is the driver's, not the width: a 16-pixel row is
            // padded out to whatever alignment the hardware wants.
            let start = row * mapped.RowPitch as usize;
            let pixels = std::slice::from_raw_parts(
                (mapped.pData as *const u8).add(start),
                size.0 as usize * 4,
            );
            for pixel in pixels.as_chunks::<4>().0 {
                totals[0] += pixel[0] as u64;
                totals[1] += pixel[1] as u64;
                totals[2] += pixel[2] as u64;
            }
        }
        context.Unmap(&staging, 0);

        let count = (size.0 as u64 * size.1 as u64).max(1);
        Ok([
            (totals[0] / count) as u8,
            (totals[1] / count) as u8,
            (totals[2] / count) as u8,
        ])
    }
}

unsafe fn make_blend_state(device: &ID3D11Device) -> windows::core::Result<ID3D11BlendState> {
    unsafe {
        let mut desc = D3D11_BLEND_DESC::default();
        let rt = &mut desc.RenderTarget[0];
        rt.BlendEnable = true.into();
        // Straight alpha: the outgoing frame is opaque and its alpha is the
        // dial, so it mixes with what is underneath rather than adding to it.
        rt.SrcBlend = D3D11_BLEND_SRC_ALPHA;
        rt.DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
        rt.BlendOp = D3D11_BLEND_OP_ADD;
        rt.SrcBlendAlpha = D3D11_BLEND_ONE;
        rt.DestBlendAlpha = D3D11_BLEND_ZERO;
        rt.BlendOpAlpha = D3D11_BLEND_OP_ADD;
        rt.RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;

        let mut blend = None;
        device.CreateBlendState(&desc, Some(&mut blend))?;
        Ok(blend.expect("CreateBlendState succeeded without a state"))
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
            surface,
            swap_chain,
            rtv: rtv.expect("CreateRenderTargetView succeeded without a view"),
            monitor,
            width,
            height,
            occluded: false,
            video: None,
            enabled: true,
            shown: false,
            redraw: true,
            overrides: Overrides::default(),
            presented_at: None,
            fade: None,
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

type Shading = (
    ID3D11VertexShader,
    ID3D11PixelShader,
    ID3D11PixelShader,
    ID3D11PixelShader,
);

unsafe fn compile_shaders(device: &ID3D11Device) -> windows::core::Result<Shading> {
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
            pixel_shader(device, c"ps_video")?,
            pixel_shader(device, c"ps_image")?,
            pixel_shader(device, c"ps_fade")?,
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

    #[test]
    fn motion_switched_off_changes_nothing() {
        // The default path is the one every user is on. It has to be exactly
        // the arithmetic that was there before any of this existed.
        let (scale, offset) = apply_motion(
            [0.5, 1.0],
            [0.25, 0.0],
            Motion::default(),
            Drive {
                level: 1.0,
                cursor: (1.0, -1.0),
                ..Drive::default()
            },
        );
        assert_eq!(scale, [0.5, 1.0]);
        assert_eq!(offset, [0.25, 0.0]);
    }

    #[test]
    fn a_loud_moment_samples_a_smaller_window() {
        // Sampling less of the frame across the same screen is a zoom in,
        // which is what a pulse looks like.
        let (scale, _) = apply_motion(
            [1.0, 1.0],
            [0.0, 0.0],
            Motion {
                reactive: 1.0,
                parallax: 0.0,
            },
            Drive {
                level: 1.0,
                cursor: (0.0, 0.0),
                ..Drive::default()
            },
        );
        assert!(scale[0] < 1.0 && scale[0] > 0.85);
    }

    #[test]
    fn a_pulse_stays_centred() {
        let (scale, offset) = apply_motion(
            [1.0, 1.0],
            [0.0, 0.0],
            Motion {
                reactive: 1.0,
                parallax: 0.0,
            },
            Drive {
                level: 1.0,
                cursor: (0.0, 0.0),
                ..Drive::default()
            },
        );
        // Equal margins either side: the window shrank about the middle
        // rather than about the top-left corner.
        assert!((offset[0] - (1.0 - scale[0]) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn parallax_never_samples_past_the_edge() {
        // A `cover` fit on a wider clip samples a slice, and pushing that
        // slice off the end shows as a smeared band down one side.
        let motion = Motion {
            reactive: 0.0,
            parallax: 1.0,
        };
        for cursor in [-1.0f32, 1.0] {
            let (scale, offset) = apply_motion(
                [0.5, 1.0],
                [0.25, 0.0],
                motion,
                Drive {
                    level: 0.0,
                    cursor: (cursor, cursor),
                    ..Drive::default()
                },
            );
            assert!(offset[0] >= 0.0);
            assert!(offset[0] + scale[0] <= 1.0 + 1e-6);
        }
    }

    #[test]
    fn a_letterboxed_frame_is_not_clamped_into_its_bars() {
        // Contain sets a scale above 1 on the long axis. Clamping that to
        // the frame would pull the picture off centre and eat one bar.
        let (_, offset) = apply_motion(
            [1.0, 1.5],
            [0.0, -0.25],
            Motion {
                reactive: 0.0,
                parallax: 1.0,
            },
            Drive {
                level: 0.0,
                cursor: (0.0, -1.0),
                ..Drive::default()
            },
        );
        assert!(offset[1] < 0.0);
    }

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

    /// Two 16:9 screens side by side, showing a 32:9 video. Together they
    /// cover the whole picture: the left one takes the left half and the
    /// right one takes the right half, with nothing repeated and nothing
    /// missed.
    #[test]
    fn a_spanned_video_is_cut_in_two_across_two_screens() {
        let desktop = Rect {
            x: 0,
            y: 0,
            width: 3840,
            height: 1080,
        };
        let video = (3840, 1080);

        let left = Fit::Cover.uv_span(
            video,
            desktop,
            Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
        );
        let right = Fit::Cover.uv_span(
            video,
            desktop,
            Rect {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
        );

        assert!((left.0[0] - 0.5).abs() < 0.001, "half the width each");
        assert!((left.1[0] - 0.0).abs() < 0.001, "the left starts at 0");
        assert!((right.1[0] - 0.5).abs() < 0.001, "the right starts halfway");
        assert!((left.0[1] - 1.0).abs() < 0.001, "full height on both");
    }

    /// A screen left of the primary one has a negative x. The slice it gets
    /// must still start at zero — the desktop's own origin is what the
    /// fractions are measured from, not the primary monitor's.
    #[test]
    fn spanning_starts_from_the_desktop_origin_not_the_primary() {
        let desktop = Rect {
            x: -1920,
            y: 0,
            width: 3840,
            height: 1080,
        };

        let (_, offset) = Fit::Cover.uv_span(
            (3840, 1080),
            desktop,
            Rect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
        );
        assert!(offset[0].abs() < 0.001);
    }

    /// A spanned wallpaper crops against the whole desktop, not against one
    /// screen. A 16:9 video on a 32:9 desktop loses its top and bottom; each
    /// monitor's slice inherits that same vertical crop rather than deciding
    /// its own.
    #[test]
    fn spanning_crops_against_the_desktop() {
        let desktop = Rect {
            x: 0,
            y: 0,
            width: 3840,
            height: 1080,
        };
        let monitor = Rect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };

        let (scale, _) = Fit::Cover.uv_span((1920, 1080), desktop, monitor);
        // Half the width (this screen is half the desktop) and less than the
        // full height (the video is taller in shape than the desktop).
        assert!((scale[0] - 0.5).abs() < 0.001);
        assert!(scale[1] < 1.0);
    }

    #[test]
    fn an_empty_desktop_does_not_divide_by_zero() {
        let empty = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        let (scale, offset) = Fit::Cover.uv_span((1920, 1080), empty, empty);
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn a_fade_runs_from_one_to_zero() {
        let length = Duration::from_millis(200);
        assert_eq!(fade_alpha(Duration::ZERO, length), 1.0);
        assert!((fade_alpha(Duration::from_millis(100), length) - 0.5).abs() < 0.001);
        assert_eq!(fade_alpha(length, length), 0.0);
        // Past the end, which happens whenever a frame lands late.
        assert_eq!(fade_alpha(Duration::from_secs(5), length), 0.0);
    }

    #[test]
    fn a_fade_of_no_length_is_a_cut_rather_than_a_division_by_zero() {
        assert_eq!(fade_alpha(Duration::ZERO, Duration::ZERO), 0.0);
    }

    #[test]
    fn fit_names_round_trip() {
        for fit in [Fit::Cover, Fit::Contain, Fit::Stretch] {
            assert_eq!(Fit::parse(fit.name()), Some(fit));
        }
        assert_eq!(Fit::parse("nonsense"), None);
    }
}
