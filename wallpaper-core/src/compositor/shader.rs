//! The placeholder wallpaper.
//!
//! This is scaffolding, not a feature: it proves the WorkerW window, the swap
//! chain and the frame loop all work before there is a video decoder to feed
//! them. The pixel shader gets replaced by a texture sample once the decoder
//! lands; the fullscreen-triangle vertex shader stays as it is.
//!
//! Colours are the Mui palette (see docs/design_system.md), so what shows up
//! on the desktop looks like it belongs to the product.

pub const SOURCE: &str = r#"
cbuffer Params : register(b0)
{
    float time;
    float3 _pad;
};

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

float4 ps_main(VSOut i) : SV_TARGET
{
    const float3 base  = float3(0.059, 0.067, 0.082);  // #0f1115
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
