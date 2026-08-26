//! The wallpaper shaders.
//!
//! `ps_video` samples a decoded NV12 frame; `ps_gradient` is the placeholder
//! shown when there is no video, in the Mui palette (see
//! docs/design_system.md) so the desktop still looks like the product.
//!
//! NV12 keeps luma at full resolution and the two chroma channels
//! interleaved at half resolution. Both planes are views over the same
//! texture, so sampling them costs no extra memory — the conversion to RGB
//! happens here, on the GPU, at zero cost worth measuring.

pub const SOURCE: &str = r#"
cbuffer Params : register(b0)
{
    float  time;
    float  letterbox;
    float2 uvScale;
    float2 uvOffset;
    float2 _pad;
};

Texture2D<float>  texLuma   : register(t0);
Texture2D<float2> texChroma : register(t1);
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

float4 ps_video(VSOut i) : SV_TARGET
{
    float2 uv = i.uv * uvScale + uvOffset;

    // In contain mode the sample runs off the texture where the bars belong.
    // The sampler clamps rather than failing, so without this the edge pixel
    // would be smeared across the bar instead of it being black.
    if (letterbox > 0.5 && (any(uv < 0.0) || any(uv > 1.0)))
    {
        return float4(0.0, 0.0, 0.0, 1.0);
    }

    float  y = texLuma.Sample(samp, uv);
    float2 c = texChroma.Sample(samp, uv);

    // BT.709 limited range, which is what essentially every H.264/HEVC file
    // carries. Luma runs 16-235 and chroma 16-240 in 8-bit terms, so the
    // range has to be expanded before the matrix is applied — skipping that
    // step is what makes video look washed out and grey.
    y = (y - 0.0627451) * 1.164384;
    float cb = c.r - 0.5;
    float cr = c.g - 0.5;

    float3 rgb = float3(
        y + 1.792741 * cr,
        y - 0.213249 * cb - 0.532909 * cr,
        y + 2.112402 * cb
    );

    return float4(saturate(rgb), 1.0);
}

float4 ps_gradient(VSOut i) : SV_TARGET
{
    const float3 base   = float3(0.059, 0.067, 0.082); // #0f1115
    const float3 accent = float3(0.176, 0.831, 0.749); // #2dd4bf

    // Two slow waves crossing at different angles and speeds, so the pattern
    // never visibly repeats.
    float a = sin(i.uv.x * 3.1 + i.uv.y * 1.7 + time * 0.35);
    float b = sin(i.uv.y * 2.3 - i.uv.x * 1.1 + time * 0.21);
    float w = saturate(0.5 + 0.25 * (a + b));

    // Kept dark on purpose: a wallpaper that glows is a wallpaper you notice.
    float3 colour = lerp(base, accent, w * 0.35);

    // Vignette, so the corners do not compete with desktop icons.
    float2 d = i.uv - 0.5;
    colour *= 1.0 - 0.35 * dot(d, d);

    return float4(colour, 1.0);
}
"#;
