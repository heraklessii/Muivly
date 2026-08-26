//! The wallpaper shader.
//!
//! Two pixel shaders over one vertex shader. `ps_video` samples a decoded
//! NV12 frame and converts it to RGB; `ps_image` samples a BGRA texture that
//! WIC already decoded for us. Both end in the same `finish` call, so a
//! brightness or blur setting behaves identically whichever kind of wallpaper
//! is on screen.
//!
//! NV12 keeps luma at full resolution and the two chroma channels
//! interleaved at half resolution. Both planes are views over the same
//! texture, so sampling them costs no extra memory — the conversion to RGB
//! happens here, on the GPU, at zero cost worth measuring.
//!
//! A monitor with no wallpaper does not get a shader at all — its surface is
//! hidden and the Windows wallpaper shows through.

pub const SOURCE: &str = r#"
cbuffer Params : register(b0)
{
    float  time;
    float  letterbox;
    float2 uvScale;
    float2 uvOffset;
    float  brightness;   // 1 = untouched
    float  saturation;   // 1 = untouched, 0 = grey
    float2 blurStep;     // one texel times the blur radius, 0 = off
    float  alpha;        // how much of the outgoing frame is left, 1 -> 0
    float  _pad;         // constant buffers are sized in 16-byte registers
};

Texture2D<float>  texLuma   : register(t0);
Texture2D<float2> texChroma : register(t1);
// t2, not t0: the two pixel shaders share one signature, and giving the
// image its own slot means the NV12 planes and the BGRA texture can never be
// read as each other, whichever shader ran last.
Texture2D<float4> texImage  : register(t2);
SamplerState      samp      : register(s0);

struct VSOut
{
    float4 pos : SV_POSITION;
    float2 uv  : TEXCOORD0;
};

// One oversized triangle covers the screen with no vertex buffer bound:
// vertex 0 -> (0,0), 1 -> (2,0), 2 -> (0,2) in UV, which maps to a triangle
// whose visible portion is exactly the viewport.
VSOut vs_main(uint id : SV_VertexID)
{
    VSOut o;
    o.uv  = float2((id << 1) & 2, id & 2);
    o.pos = float4(o.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}

// Brightness and saturation, then out. Kept in one place so the two shaders
// below cannot drift apart.
float4 finish(float3 rgb)
{
    rgb *= brightness;
    // Rec.709 luminance: the grey a viewer perceives, not the arithmetic
    // mean, which would turn saturation down unevenly across hues.
    float grey = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    rgb = lerp(grey.xxx, rgb, saturation);
    return float4(saturate(rgb), 1.0);
}

// The nine taps of a 3x3 gaussian. Not a separable two-pass blur: that needs
// an intermediate render target per monitor and a second draw, and a
// wallpaper blur is a mood setting, not an effect anyone inspects closely.
// The radius is carried in blurStep, so a wide blur is a wide sample spread
// rather than more taps.
static const float2 TAPS[9] = {
    float2(-1, -1), float2(0, -1), float2(1, -1),
    float2(-1,  0), float2(0,  0), float2(1,  0),
    float2(-1,  1), float2(0,  1), float2(1,  1)
};
static const float WEIGHTS[9] = {
    0.0625, 0.125, 0.0625,
    0.125,  0.25,  0.125,
    0.0625, 0.125, 0.0625
};

// True where the crop in contain mode has run off the texture and the bars
// belong. The sampler clamps rather than failing, so without this the edge
// pixel would be smeared across the bar instead of it being black.
bool in_bar(float2 uv)
{
    return letterbox > 0.5 && (any(uv < 0.0) || any(uv > 1.0));
}

float3 sample_video(float2 uv)
{
    float  y = texLuma.Sample(samp, uv);
    float2 c = texChroma.Sample(samp, uv);

    // BT.709 limited range, which is what essentially every H.264/HEVC file
    // carries. Luma runs 16-235 and chroma 16-240 in 8-bit terms, so the
    // range has to be expanded before the matrix is applied — skipping that
    // step is what makes video look washed out and grey.
    y = (y - 0.0627451) * 1.164384;
    float cb = c.r - 0.5;
    float cr = c.g - 0.5;

    return float3(
        y + 1.792741 * cr,
        y - 0.213249 * cb - 0.532909 * cr,
        y + 2.112402 * cb
    );
}

float4 ps_video(VSOut i) : SV_TARGET
{
    float2 uv = i.uv * uvScale + uvOffset;
    if (in_bar(uv))
    {
        return float4(0.0, 0.0, 0.0, 1.0);
    }

    if (blurStep.x <= 0.0 && blurStep.y <= 0.0)
    {
        return finish(sample_video(uv));
    }

    float3 sum = 0.0;
    for (int t = 0; t < 9; t++)
    {
        sum += WEIGHTS[t] * sample_video(uv + TAPS[t] * blurStep);
    }
    return finish(sum);
}

// The outgoing wallpaper during a crossfade.
//
// Its pixels were captured after the fit, the grade and the blur had already
// been applied, so there is nothing to do here but sample and hand over an
// alpha. Screen UV is used directly for the same reason: the capture is
// already in the shape of this screen.
float4 ps_fade(VSOut i) : SV_TARGET
{
    return float4(texImage.Sample(samp, i.uv).rgb, alpha);
}

float4 ps_image(VSOut i) : SV_TARGET
{
    float2 uv = i.uv * uvScale + uvOffset;
    if (in_bar(uv))
    {
        return float4(0.0, 0.0, 0.0, 1.0);
    }

    if (blurStep.x <= 0.0 && blurStep.y <= 0.0)
    {
        return finish(texImage.Sample(samp, uv).rgb);
    }

    float3 sum = 0.0;
    for (int t = 0; t < 9; t++)
    {
        sum += WEIGHTS[t] * texImage.Sample(samp, uv + TAPS[t] * blurStep).rgb;
    }
    return finish(sum);
}
"#;
