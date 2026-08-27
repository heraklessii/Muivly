//! Wallpapers that are a program rather than a file of pixels.
//!
//! This is the lightest wallpaper Muivly can show, by a wide margin. There is
//! no decoder, no picture buffer, no Media Foundation thread pool and no
//! reference frames — the whole wallpaper is one pixel shader and one
//! screen-sized texture, which on a 1080p desktop is 8 MB and nothing else.
//! Everything the memory work in `docs/decisions.md` could not move about a
//! video simply does not exist here.
//!
//! The contract is deliberately small. A `.hlsl` file defines one function:
//!
//! ```hlsl
//! float4 mainImage(float2 uv)
//! {
//!     return float4(uv.x, uv.y, 0.5 + 0.5 * sin(iTime), 1.0);
//! }
//! ```
//!
//! `uv` runs 0-1 across the frame, `iTime` is seconds since the wallpaper
//! started, and `iResolution` is the size in pixels. Everything else — the
//! vertex shader, the entry point, the constant buffer — is supplied here, so
//! the file the user writes is the part that is actually theirs.
//!
//! A file may also declare its own settings, which appear as sliders in the
//! settings window:
//!
//! ```hlsl
//! // param speed 0.1 3.0 1.0 How fast it moves
//! ```
//!
//! `name min max default`, and the rest of the line is a label. Up to eight
//! per file. See `parse_params`.
//!
//! `.glsl` and `.frag` files are Shadertoy shaders and are translated on the
//! way in; see `glsl.rs`.
//!
//! The shader draws into an offscreen texture rather than straight to the
//! screen. That costs one full-screen pass, and buys the fit, the grade, the
//! blur, the crossfade and the span for free: to everything downstream this
//! is a `Frame::Bgra` like any other, indistinguishable from a photo.

use std::path::Path;
use std::time::Duration;

use windows::core::PCSTR;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RenderTargetView,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use super::{clamp_size, Frame};
use crate::audio::BANDS;
use crate::compositor::Drive;

/// How long a shader counts as "one play" for a playlist set to advance when
/// the item ends. A generated wallpaper has no end of its own; this matches
/// the length a still image is given, for the same reason.
const CYCLE: Duration = Duration::from_secs(30);

/// The largest a generated frame is ever made.
///
/// A procedural wallpaper has no native size, so it would otherwise be drawn
/// at whatever the biggest screen happens to be — and a shader with real work
/// in it at 4K on an integrated GPU is exactly the kind of load this project
/// exists to avoid. 1440p upscales to a 4K screen without anyone noticing on
/// the gradients and noise fields these are made of.
const CEILING: (u32, u32) = (2560, 1440);

/// How many settings one shader may declare.
///
/// Eight, which is two constant-buffer registers and more sliders than a
/// wallpaper panel can show without becoming a synthesiser.
pub const MAX_PARAMS: usize = 8;

/// Whether this file is a shader rather than something to decode.
pub fn is_shader(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "hlsl" | "fx" | "glsl" | "frag"
    )
}

/// One setting a shader file declares for itself.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderParam {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// What to call it in the settings window. The name, when the file did
    /// not say.
    pub label: String,
}

/// Matches `cbuffer Muivly` in `PRELUDE`. Constant buffers are sized in
/// 16-byte registers, and this comes to six of them.
#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    frame: f32,
    mouse: [f32; 2],
    level: f32,
    band_count: f32,
    bands: [f32; BANDS],
    params: [f32; MAX_PARAMS],
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            resolution: [0.0; 2],
            time: 0.0,
            frame: 0.0,
            mouse: [0.0; 2],
            level: 0.0,
            band_count: BANDS as f32,
            bands: [0.0; BANDS],
            params: [0.0; MAX_PARAMS],
        }
    }
}

/// Everything the user's file may rely on, prepended to it.
const PRELUDE: &str = r#"
cbuffer Muivly : register(b0)
{
    float2 iResolution;  // the frame size in pixels
    float  iTime;        // seconds since this wallpaper started
    float  iFrame;       // frames drawn since it started
    float2 iMouse;       // the cursor, -1 to 1 across the desktop
    float  iLevel;       // how loud the machine is, 0 to 1
    float  iBandCount;   // how many entries iBands carries
    float4 iBands[2];    // the sound split into bands, bass first, 0 to 1
    float4 muivlyParams[2];
};

#define iResolutionXYZ float3(iResolution, 1.0)

float iBand(int index)
{
    return iBands[index / 4][index % 4];
}

struct MuivlyVSOut
{
    float4 pos : SV_POSITION;
    float2 uv  : TEXCOORD0;
};

MuivlyVSOut vs_main(uint id : SV_VertexID)
{
    MuivlyVSOut o;
    o.uv  = float2((id << 1) & 2, id & 2);
    o.pos = float4(o.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
"#;

/// Appended after it, so `mainImage` is already declared by the time the
/// entry point calls it and the user never writes a signature.
const EPILOGUE: &str = r#"
float4 ps_main(MuivlyVSOut input) : SV_Target
{
    return mainImage(input.uv);
}
"#;

pub struct ShaderDecoder {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    uniforms: ID3D11Buffer,
    /// Also held for its lifetime: the view and the render target below are
    /// both views onto it.
    _texture: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    view: ID3D11ShaderResourceView,
    width: u32,
    height: u32,

    /// What this file said it wanted sliders for, and where they are set.
    params: Vec<ShaderParam>,
    values: [f32; MAX_PARAMS],

    /// Where the wallpaper's own clock has reached. Not the engine's: this
    /// one stops while nothing is visible, so a shader does not jump forward
    /// by the length of a game when the desktop comes back.
    position: Duration,
    last_clock: Option<Duration>,
    frames: f32,
    loops: u32,
    speed: f32,
    /// Whether the file reads the sound split into bands.
    wants_bands: bool,
    /// What the cursor and the sound said this frame.
    drive: Drive,
}

impl ShaderDecoder {
    /// Compile a shader file and prepare the texture it draws into.
    ///
    /// A compile error comes back as the compiler's own message, with the
    /// line numbers of the user's file: this is the one place in Muivly
    /// where the person seeing the error is the person who wrote the input.
    pub fn open(
        device: &ID3D11Device,
        path: &Path,
        max_scale: (u32, u32),
    ) -> windows::core::Result<Self> {
        let body = std::fs::read_to_string(path).map_err(|e| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                format!("cannot read the shader: {e}"),
            )
        })?;

        // Settings are declared in comments, so they are read from the file
        // as written rather than from the translation.
        let params = parse_params(&body);

        let body = if super::glsl::is_glsl(path) {
            if let Some(reason) = super::glsl::unsupported(&body) {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    reason,
                ));
            }
            super::glsl::translate(&body)
        } else {
            if !body.contains("mainImage") {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "the shader has no mainImage(float2 uv) function",
                ));
            }
            body
        };

        // Asked of the file rather than of a setting: the loopback capture
        // is opened only for a wallpaper that actually reads the bands, so a
        // shader that does not never touches the audio stack.
        let wants_bands = body.contains("iBand");

        let source = format!("{PRELUDE}{}\n{body}\n{EPILOGUE}", defines(&params));
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());

        let (vs, ps) = unsafe { compile(device, &source, name.as_deref(), params.len())? };
        let size = clamp_size(CEILING, max_scale);
        let (texture, rtv, view) = unsafe { make_surface(device, size)? };
        let uniforms = unsafe { make_uniforms(device)? };

        let mut values = [0.0; MAX_PARAMS];
        for (slot, param) in values.iter_mut().zip(&params) {
            *slot = param.default;
        }

        Ok(Self {
            vs,
            ps,
            uniforms,
            _texture: texture,
            rtv,
            view,
            width: size.0,
            height: size.1,
            params,
            values,
            position: Duration::ZERO,
            last_clock: None,
            frames: 0.0,
            loops: 0,
            speed: 1.0,
            wants_bands,
            drive: Drive::default(),
        })
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

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.05);
    }

    /// What the file declared it wants sliders for.
    pub fn params(&self) -> &[ShaderParam] {
        &self.params
    }

    /// Whether this shader reads the sound split into bands, which is what
    /// decides whether the loopback capture is opened at all. A file that
    /// never mentions `iBand` costs nothing in the audio stack.
    pub fn wants_bands(&self) -> bool {
        self.wants_bands
    }

    /// Move one of the file's own settings. An unknown name is ignored: the
    /// settings window and the file can disagree after an edit, and the file
    /// is the one that is right.
    pub fn set_param(&mut self, name: &str, value: f32) {
        let Some(index) = self.params.iter().position(|p| p.name == name) else {
            return;
        };
        let param = &self.params[index];
        self.values[index] = value.clamp(param.min.min(param.max), param.max.max(param.min));
    }

    /// What the cursor and the meter said this frame.
    pub fn set_drive(&mut self, drive: Drive) {
        self.drive = drive;
    }

    /// Always due. A shader has no frame times of its own, so the engine's
    /// frame rate — the tier's, the per-monitor cap, the one battery leaves
    /// it — is the only thing deciding how often this runs.
    pub fn time_to_next(&self) -> Duration {
        Duration::ZERO
    }

    /// Draw the next frame into the offscreen texture.
    ///
    /// Returns true every time it is called, because a shader is animation
    /// by definition — there is no "the same frame again" to detect. A
    /// wallpaper that happens to be static costs a full-screen pass per
    /// frame for nothing, which is the one case where a video would be
    /// cheaper and is worth knowing before writing one.
    pub fn update(
        &mut self,
        context: &ID3D11DeviceContext,
        elapsed: Duration,
    ) -> windows::core::Result<bool> {
        // The same guard as the video path: a gap where nothing was drawn is
        // not playback time to make up. See `video.rs`.
        let step = match self.last_clock {
            Some(previous) => elapsed
                .saturating_sub(previous)
                .min(Duration::from_millis(200)),
            None => Duration::ZERO,
        };
        self.last_clock = Some(elapsed);
        self.position += step.mul_f32(self.speed);
        self.frames += 1.0;
        self.loops = (self.position.as_secs_f32() / CYCLE.as_secs_f32()) as u32;

        let uniforms = Uniforms {
            resolution: [self.width as f32, self.height as f32],
            time: self.position.as_secs_f32(),
            frame: self.frames,
            mouse: [self.drive.cursor.0, self.drive.cursor.1],
            level: self.drive.level,
            band_count: BANDS as f32,
            bands: self.drive.bands,
            params: self.values,
        };

        unsafe {
            context.UpdateSubresource(
                &self.uniforms,
                0,
                None,
                &uniforms as *const _ as *const _,
                0,
                0,
            );

            context.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            context.OMSetBlendState(None, None, u32::MAX);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(&self.vs, None);
            context.PSSetShader(&self.ps, None);
            context.PSSetConstantBuffers(0, Some(&[Some(self.uniforms.clone())]));
            context.Draw(3, 0);

            // The render target is unbound before returning: the compositor
            // is about to bind this texture as a shader resource, and D3D11
            // silently drops the bind — leaving the wallpaper black — if the
            // same resource is still attached as an output.
            context.OMSetRenderTargets(None, None);
        }

        Ok(true)
    }
}

/// The settings a shader file declares, in the order they appear.
///
/// `// param <name> <min> <max> <default> [label]`. A malformed line is
/// skipped rather than refused: the alternative is a wallpaper that will not
/// open because of a typo in a comment.
pub fn parse_params(source: &str) -> Vec<ShaderParam> {
    let mut out: Vec<ShaderParam> = Vec::new();

    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix("//") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix("param ") else {
            continue;
        };

        let mut fields = rest.split_whitespace();
        let (Some(name), Some(min), Some(max), Some(default)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };

        if !is_identifier(name) || out.iter().any(|p| p.name == name) || out.len() >= MAX_PARAMS {
            continue;
        }

        let (Ok(min), Ok(max), Ok(default)) = (
            min.parse::<f32>(),
            max.parse::<f32>(),
            default.parse::<f32>(),
        ) else {
            continue;
        };
        if !min.is_finite() || !max.is_finite() || !default.is_finite() || min >= max {
            continue;
        }

        let label = rest
            .split_whitespace()
            .skip(4)
            .collect::<Vec<_>>()
            .join(" ");

        out.push(ShaderParam {
            name: name.to_string(),
            min,
            max,
            default: default.clamp(min, max),
            label: if label.is_empty() {
                name.to_string()
            } else {
                label
            },
        });
    }

    out
}

/// The `#define` for each declared setting, so the file can use its own name.
///
/// A define rather than a variable: a `static float` initialised from a
/// constant buffer is legal HLSL but subtle, and this is a wallpaper file
/// somebody wrote in a text editor. A define cannot be optimised away, cannot
/// be shadowed by accident, and reads as itself in the compiler's errors.
fn defines(params: &[ShaderParam]) -> String {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            format!(
                "#define {} (muivlyParams[{}][{}])\n",
                param.name,
                index / 4,
                index % 4
            )
        })
        .collect()
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Compile the wrapped source into the two shaders it needs.
unsafe fn compile(
    device: &ID3D11Device,
    source: &str,
    name: Option<&str>,
    extra: usize,
) -> windows::core::Result<(ID3D11VertexShader, ID3D11PixelShader)> {
    unsafe {
        let vs_blob = compile_one(source, name, extra, c"vs_main", c"vs_5_0")?;
        let ps_blob = compile_one(source, name, extra, c"ps_main", c"ps_5_0")?;

        let vs_code = std::slice::from_raw_parts(
            vs_blob.GetBufferPointer() as *const u8,
            vs_blob.GetBufferSize(),
        );
        let ps_code = std::slice::from_raw_parts(
            ps_blob.GetBufferPointer() as *const u8,
            ps_blob.GetBufferSize(),
        );

        let mut vs = None;
        device.CreateVertexShader(vs_code, None, Some(&mut vs))?;
        let mut ps = None;
        device.CreatePixelShader(ps_code, None, Some(&mut ps))?;

        Ok((
            vs.expect("CreateVertexShader succeeded without a shader"),
            ps.expect("CreatePixelShader succeeded without a shader"),
        ))
    }
}

unsafe fn compile_one(
    source: &str,
    name: Option<&str>,
    extra: usize,
    entry: &std::ffi::CStr,
    profile: &std::ffi::CStr,
) -> windows::core::Result<windows::Win32::Graphics::Direct3D::ID3DBlob> {
    unsafe {
        // The file name is what the compiler puts in front of its line
        // numbers. Without it every error reads "unknown(43)", and the user
        // is holding a file whose line 43 is not the one being complained
        // about — the prelude is prepended, so the numbers are shifted.
        let file = name.map(|n| format!("{n}\0"));

        let mut blob = None;
        let mut errors = None;
        let result = D3DCompile(
            source.as_ptr() as *const _,
            source.len(),
            file.as_ref()
                .map(|f| PCSTR(f.as_ptr()))
                .unwrap_or(PCSTR::null()),
            None,
            None,
            PCSTR(entry.as_ptr() as *const u8),
            PCSTR(profile.as_ptr() as *const u8),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut blob,
            Some(&mut errors),
        );

        if let Err(e) = result {
            let message = errors
                .as_ref()
                .map(|blob| {
                    let text = std::slice::from_raw_parts(
                        blob.GetBufferPointer() as *const u8,
                        blob.GetBufferSize(),
                    );
                    String::from_utf8_lossy(text).trim().to_string()
                })
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| e.message().to_string());

            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                shift_line_numbers(&message, extra),
            ));
        }

        Ok(blob.expect("D3DCompile succeeded without a blob"))
    }
}

/// How many lines the prelude adds in front of the user's file.
fn prelude_lines() -> usize {
    // The format in `open` is "{PRELUDE}{defines}\n{body}", so the body
    // starts one line after the prelude and whatever defines came with it.
    PRELUDE.lines().count() + 1
}

/// Put the compiler's line numbers back into the user's file.
///
/// D3DCompile numbers the source it was handed, which begins with a prelude
/// the user never wrote. Reporting those numbers means telling somebody
/// their mistake is on line 43 of a 12-line file.
///
/// `extra` is the number of lines the declared settings added, which sits
/// between the prelude and the body.
fn shift_line_numbers(message: &str, extra: usize) -> String {
    let offset = prelude_lines() + extra;

    message
        .lines()
        .map(|line| match split_location(line) {
            Some(found) if found.line > offset => format!(
                "{}({}{}){}",
                found.name,
                found.line - offset,
                found.column,
                found.rest
            ),
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The pieces of `some.hlsl(43,7-9): error X3004: ...`.
struct Location<'a> {
    name: &'a str,
    line: usize,
    /// Everything after the line number and inside the brackets, comma
    /// included. Carried through untouched: the column is still the user's
    /// column, and only the line moved.
    column: &'a str,
    rest: &'a str,
}

fn split_location(line: &str) -> Option<Location<'_>> {
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    let inside = &line[open + 1..close];

    let (number, column) = match inside.find(',') {
        Some(comma) => (&inside[..comma], &inside[comma..]),
        None => (inside, ""),
    };

    Some(Location {
        name: &line[..open],
        line: number.trim().parse::<usize>().ok().filter(|n| *n > 0)?,
        column,
        rest: &line[close + 1..],
    })
}

unsafe fn make_surface(
    device: &ID3D11Device,
    size: (u32, u32),
) -> windows::core::Result<(
    ID3D11Texture2D,
    ID3D11RenderTargetView,
    ID3D11ShaderResourceView,
)> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
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
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            ..Default::default()
        };

        let mut texture = None;
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.expect("CreateTexture2D succeeded without a texture");

        let mut rtv = None;
        device.CreateRenderTargetView(&texture, None, Some(&mut rtv))?;
        let mut view = None;
        device.CreateShaderResourceView(&texture, None, Some(&mut view))?;

        Ok((
            texture,
            rtv.expect("CreateRenderTargetView succeeded without a view"),
            view.expect("CreateShaderResourceView succeeded without a view"),
        ))
    }
}

unsafe fn make_uniforms(device: &ID3D11Device) -> windows::core::Result<ID3D11Buffer> {
    unsafe {
        let initial = Uniforms::default();
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Uniforms>() as u32,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn hlsl_is_a_shader_and_mp4_is_not() {
        assert!(is_shader(&PathBuf::from("waves.hlsl")));
        assert!(is_shader(&PathBuf::from(r"C:\x\WAVES.FX")));
        assert!(is_shader(&PathBuf::from("toy.glsl")));
        assert!(!is_shader(&PathBuf::from("clip.mp4")));
        assert!(!is_shader(&PathBuf::from("no-extension")));
    }

    #[test]
    fn an_error_in_the_users_file_keeps_its_own_line_number() {
        // The prelude sits in front of the file, so the compiler's line 3 of
        // the user's body is reported as line 3 + the prelude's length.
        let reported = prelude_lines() + 3;
        let message = format!("waves.hlsl({reported},5-9): error X3004: undeclared 'foo'");
        assert!(shift_line_numbers(&message, 0).starts_with("waves.hlsl(3,"));
    }

    /// A file that declares settings puts a define per setting between the
    /// prelude and the body, and the numbers have to move by those too.
    #[test]
    fn declared_settings_shift_the_line_numbers_further() {
        let reported = prelude_lines() + 2 + 3;
        let message = format!("waves.hlsl({reported},1): error X3004: undeclared 'foo'");
        assert!(shift_line_numbers(&message, 2).starts_with("waves.hlsl(3,"));
    }

    #[test]
    fn an_error_inside_the_prelude_is_left_alone() {
        // Nothing in the prelude is the user's fault, and a negative line
        // number helps nobody. These are left where they are.
        let message = "waves.hlsl(4,1): error X0000: internal";
        assert_eq!(shift_line_numbers(message, 0), message);
    }

    #[test]
    fn a_message_without_a_location_survives_untouched() {
        let message = "error X3501: entrypoint not found";
        assert_eq!(shift_line_numbers(message, 0), message);
    }

    #[test]
    fn the_generated_size_never_exceeds_the_tier_cap() {
        // A low-tier machine caps at 1080p; a shader must not draw 1440p
        // there just because it has no native size to be capped against.
        assert_eq!(clamp_size(CEILING, (1920, 1080)), (1920, 1080));
    }

    #[test]
    fn a_declared_setting_is_read_with_its_label() {
        let params = parse_params("// param speed 0.1 3.0 1.0 How fast it moves\n");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "speed");
        assert_eq!(params[0].min, 0.1);
        assert_eq!(params[0].max, 3.0);
        assert_eq!(params[0].default, 1.0);
        assert_eq!(params[0].label, "How fast it moves");
    }

    #[test]
    fn a_setting_without_a_label_is_called_by_its_name() {
        let params = parse_params("//param tint 0 1 0.5");
        assert_eq!(params[0].label, "tint");
    }

    /// Every one of these is a typo somebody will make in a comment, and not
    /// one of them is a reason to refuse the wallpaper.
    #[test]
    fn malformed_declarations_are_skipped_rather_than_fatal() {
        let params = parse_params(
            "// param\n\
             // param 9lives 0 1 0.5\n\
             // param good 0 1 0.5\n\
             // param bad 1 0 0.5\n\
             // param broken x y z\n\
             // param good 0 2 1\n",
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "good");
    }

    #[test]
    fn a_file_may_not_declare_more_than_the_buffer_holds() {
        let source: String = (0..20)
            .map(|i| format!("// param p{i} 0 1 0.5\n"))
            .collect();
        assert_eq!(parse_params(&source).len(), MAX_PARAMS);
    }

    #[test]
    fn a_default_outside_its_own_range_is_pulled_back_in() {
        let params = parse_params("// param x 0 1 9");
        assert_eq!(params[0].default, 1.0);
    }

    /// The defines are what let the file use its own names, and the index
    /// arithmetic is where a ninth setting would silently read the wrong
    /// slot.
    #[test]
    fn each_setting_gets_its_own_slot() {
        let params = parse_params(
            "// param a 0 1 0\n// param b 0 1 0\n// param c 0 1 0\n\
             // param d 0 1 0\n// param e 0 1 0\n",
        );
        let text = defines(&params);
        assert!(text.contains("#define a (muivlyParams[0][0])"));
        assert!(text.contains("#define d (muivlyParams[0][3])"));
        assert!(text.contains("#define e (muivlyParams[1][0])"));
    }

    /// The constant buffer is declared in HLSL by hand and filled from Rust
    /// by memory layout. If these two disagree the wallpaper reads garbage.
    #[test]
    fn the_uniform_buffer_is_a_whole_number_of_registers() {
        assert_eq!(std::mem::size_of::<Uniforms>() % 16, 0);
        assert_eq!(std::mem::size_of::<Uniforms>(), 96);
    }
}
