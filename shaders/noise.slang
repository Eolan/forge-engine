// Spatio-temporal noise for effects that TAA averages (issues #48, #54), after Intel's XeGTAO
// (MIT; shaders/third-party/XeGTAO-LICENSE.txt): the R2 sequence (Roberts 2018) along a
// Hilbert curve over 64 × 64 pixels, shifted every frame; and Jimenez's interleaved gradient
// noise, for an effect a 3 × 3 blur denoises (GTAO). Callers pass TAA's frame modulo its
// jitter period, so the pattern repeats with the jitter and TAA settles on it.
module noise;

// The index of (x, y) along a Hilbert curve over 64 × 64 pixels: neighbouring pixels get
// well-spread noise values.
public uint hilbert_index(uint x, uint y)
{
    uint index = 0;
    for (uint cur = 32; cur > 0; cur /= 2)
    {
        uint rx = (x & cur) > 0 ? 1u : 0u;
        uint ry = (y & cur) > 0 ? 1u : 0u;
        index += cur * cur * ((3u * rx) ^ ry);
        if (ry == 0)
        {
            if (rx == 1)
            {
                x = 63u - x;
                y = 63u - y;
            }
            uint t = x;
            x = y;
            y = t;
        }
    }
    return index;
}

// Two noise values in [0, 1) for a pixel and frame.
public float2 spatio_temporal_noise(uint2 pixel, uint frame)
{
    uint index = hilbert_index(pixel.x % 64u, pixel.y % 64u) + 288u * (frame % 64u);
    return frac(0.5 + float(index) * float2(0.75487766624669276, 0.56984029099805327));
}

// Interleaved gradient noise (Jorge Jimenez, "Next Generation Post Processing in Call of Duty:
// Advanced Warfare", SIGGRAPH 2014): a value in [0, 1) for a pixel and frame, made so that each
// 3 × 3 block of pixels spreads its values over the whole range, so a 3 × 3 blur averages it
// out. GTAO's samples use it (#48): the Hilbert curve's R2 above left rows of stripes through
// its 3 × 3 denoise wherever TAA had no history to average them (a moving camera; the owner's
// report of 2026-10-03). `offset`, in pixels, gives a second value at the same pixel.
public float interleaved_gradient_noise(uint2 pixel, uint frame, float2 offset)
{
    float2 p = float2(pixel) + offset + 5.588238 * float(frame % 64u);
    return frac(52.9829189 * frac(dot(p, float2(0.06711056, 0.00583715))));
}
