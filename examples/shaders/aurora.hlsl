// Aurora — an example Muivly shader wallpaper.
//
// Drop this file on the Muivly window, or add it from the library, and it
// plays like any other wallpaper. There is no decoder behind it: this whole
// wallpaper is the program below plus one screen-sized texture, which is why
// it costs a few megabytes where a video costs a few hundred.
//
// The contract is one function. Muivly supplies everything else:
//
//   float2 iResolution   the frame size in pixels
//   float  iTime         seconds since this wallpaper started
//   float  iFrame        frames drawn since it started
//   float2 iMouse        the cursor, -1 to 1 across the desktop
//   float  iLevel        how loud the machine is, 0 to 1
//   float  iBand(int i)  that sound split into bands, bass first, 0 to 1
//
// `uv` runs 0 to 1 across the frame. Return a colour in 0-1 range.
//
// A file can also declare its own sliders, which show up in the settings
// window next to the wallpaper:
//
//   // param name min max default Label shown to the user
//
// See spectrum.hlsl in this folder for both.
//
// Written to be cheap: three sine layers and no loops. A shader runs once per
// pixel per frame on whatever GPU the machine has, so on the integrated
// graphics Muivly is built for, restraint here is the difference between a
// wallpaper and a fan.

float band(float2 uv, float offset, float speed, float scale)
{
    float wave = sin(uv.x * scale + iTime * speed + offset) * 0.12
               + sin(uv.x * scale * 0.5 - iTime * speed * 0.7) * 0.08;

    // Distance from this band's centre line, turned into a soft glow.
    float distance = abs(uv.y - (0.5 + wave + offset * 0.1));
    return 0.035 / (distance + 0.035);
}

float4 mainImage(float2 uv)
{
    // A dark ground the bands sit on, slightly lighter towards the bottom so
    // the frame does not read as flat black on an OLED panel.
    float3 colour = lerp(float3(0.02, 0.03, 0.06), float3(0.05, 0.06, 0.11), uv.y);

    colour += float3(0.05, 0.85, 0.70) * band(uv, 0.00, 0.35, 6.0) * 0.30;
    colour += float3(0.20, 0.55, 0.95) * band(uv, 0.25, 0.22, 4.0) * 0.22;
    colour += float3(0.65, 0.35, 0.90) * band(uv, -0.20, 0.15, 8.0) * 0.16;

    return float4(colour, 1.0);
}
