// Spectrum — a wallpaper that listens.
//
// Eight bars, one per band of whatever the machine is playing: bass on the
// left, treble on the right. Nothing here is a decoder. The whole wallpaper
// is the program below plus one screen-sized texture, and the sound comes
// from a reading Muivly takes on a frame it was drawing anyway.
//
// Muivly supplies:
//
//   float2 iResolution     the frame size in pixels
//   float  iTime           seconds since this wallpaper started
//   float  iFrame          frames drawn since it started
//   float2 iMouse          the cursor, -1 to 1 across the desktop
//   float  iLevel          how loud the machine is, 0 to 1
//   float  iBandCount      how many bands there are
//   float  iBand(int i)    band i, bass first, 0 to 1
//
// A file that never mentions iBand costs nothing in the audio stack: the
// capture is opened only for a wallpaper that reads it, and closed when that
// wallpaper leaves the screen. This one reads it, so this one opens it.
//
// The bars are drawn from a distance field rather than a loop over pixels:
// one pass, no branching, no texture reads. On integrated graphics the whole
// thing is a handful of instructions per pixel.

// param bars 2.0 8.0 8.0 How many bars
// param glow 0.0 1.0 0.35 How far the light spreads
// param floorlight 0.0 0.6 0.12 How lit the background is when it is quiet

// The colour a bar takes at its position across the screen. Teal into violet,
// which is Muivly's own palette and reads as one picture rather than eight.
float3 tint(float across)
{
    float3 cool = float3(0.18, 0.83, 0.75);
    float3 warm = float3(0.54, 0.36, 0.92);
    return lerp(cool, warm, across);
}

float4 mainImage(float2 uv)
{
    // The bar this pixel belongs to, and how far it is from that bar's
    // centre. `count` is a setting, so a 2-bar wallpaper is the same file.
    float count = max(2.0, floor(bars));
    float slot = floor(uv.x * count);
    float across = (slot + 0.5) / count;

    // Bands run bass to treble; a bar past the last band reads the last one
    // rather than reading nothing.
    int index = (int)min(slot, iBandCount - 1.0);
    float level = iBand(index);

    // Bars grow from the bottom. A little floor so a silent desktop is not a
    // black rectangle with nothing in it.
    float height = 0.06 + level * 0.72;
    float column = abs(frac(uv.x * count) - 0.5);

    // Two distance fields: one across (the bar's width), one up (its top).
    // Multiplied rather than min-ed, which is what softens the corners.
    float width = smoothstep(0.42, 0.30, column);
    float top = smoothstep(height + 0.02, height - 0.02, 1.0 - uv.y);
    float bar = width * top;

    // The glow is the same fields, wider and dimmer. This is the only part
    // that costs anything and it is still two smoothsteps.
    float halo = smoothstep(0.75, 0.0, column) * smoothstep(height + 0.35, 0.0, 1.0 - uv.y);

    float3 colour = tint(across) * (bar + halo * glow * level);

    // A very dark ground, lifted slightly by how loud the room is. The
    // wallpaper is still a wallpaper when nothing is playing.
    float3 ground = float3(0.055, 0.062, 0.078) * (1.0 + iLevel * floorlight * 4.0);

    return float4(ground + colour, 1.0);
}
