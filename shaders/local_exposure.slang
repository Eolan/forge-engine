// Local exposure (D-067, #275): what the display passes read of the bilateral grid of log
// luminance that `local_exposure_grid.slang` builds from this frame's resolved HDR image. Each
// pixel reads its base log luminance B at its own position and luminance, 40 % from the grid and
// 60 % from a wide Gaussian of the cells' means (Patry 2021), and takes the gain
// 2^((c − 1)(B − M)): c the shadow contrast under middle grey M, the highlight contrast over it,
// clamped to +3 EV and −2 EV, none in near-black. The grid is built and read within the frame,
// never kept for the next one (D-067: clear of US 11,756,172's later-frame claims). CPU mirror
// with tests: crates/forge-render/src/local_exposure.rs (keep the two in step).
module local_exposure;

import bindless;
import exact;

// The grid: 64 × 36 cells over the image, 64 slices of log2 pre-exposed luminance over
// [-16, 8] (the histogram's range, 0.375 EV a slice). Its slices lie 8 × 8 in a 512 × 288 atlas,
// slice s at tile (s % 8, s / 8), so a trilinear read is two bilinear taps.
public static const uint LE_CELLS_X = 64;
public static const uint LE_CELLS_Y = 36;
public static const uint LE_SLICES = 64;
public static const uint LE_TILES_X = 8;
public static const float LE_MIN_LOG2 = -16.0;
public static const float LE_MAX_LOG2 = 8.0;
public static const float LE_SLICE = (LE_MAX_LOG2 - LE_MIN_LOG2) / float(LE_SLICES);
// The gain's clamp, in EV.
public static const float LE_MAX_LIFT = 3.0;
public static const float LE_MAX_DARKEN = 2.0;
// No gain below M − 12 EV, all of it above M − 10.
public static const float LE_DARK_FLOOR = 12.0;
public static const float LE_DARK_FADE = 2.0;
// A grid read whose weight (the share of a cell's pixels in that luminance, blurred) is under
// this fades to the Gaussian's base: too few pixels there to say.
public static const float LE_MIN_WEIGHT = 0.01;

// What the display pass reads (local_exposure.rs, `LocalExposurePush`).
public struct LocalExposure
{
    public uint grid;        // the blurred grid's atlas (sampled index), 0xFFFFFFFF: off
    public uint means;       // the cells' blurred means (sampled index)
    public float middle;     // M, log2 of middle grey in pre-exposed units
    public float shadow;     // the contrast under M
    public float highlight;  // the contrast over M
    public float blend;      // the Gaussian's share of the base
    public uint view;        // 1: the false-colour view of the gain
    public uint pad;
};

static const float3 LUMA = float3(0.2126, 0.7152, 0.0722);

// A pixel's log2 luminance, clamped to the grid's range.
public float local_log_luminance(float3 color)
{
    float y = dot(max(color, 0.0), LUMA);
    return clamp(log2(max(y, exp2(LE_MIN_LOG2))), LE_MIN_LOG2, LE_MAX_LOG2);
}

// The slice coordinate of a log luminance: slice i's centre at i, in [0, LE_SLICES − 1].
public float local_slice(float l)
{
    return clamp((l - LE_MIN_LOG2) / LE_SLICE - 0.5, 0.0, float(LE_SLICES - 1));
}

// The atlas texel of cell `cell` in slice `slice`.
public uint2 local_atlas_texel(uint2 cell, uint slice)
{
    return uint2(slice % LE_TILES_X, slice / LE_TILES_X) * uint2(LE_CELLS_X, LE_CELLS_Y) + cell;
}

// One slice of the grid at `uv`, bilinear and clamped to the slice's tile: (Σ w l, Σ w) per pixel.
float2 grid_tap(uint grid, float2 uv, uint slice)
{
    float2 cells = float2(LE_CELLS_X, LE_CELLS_Y);
    float2 at = clamp(uv * cells, 0.5, cells - 0.5);
    float2 tile = float2(slice % LE_TILES_X, slice / LE_TILES_X) * cells;
    float2 atlas = cells * float2(LE_TILES_X, LE_SLICES / LE_TILES_X);
    return sample_image_level(grid, SAMPLER_LINEAR_CLAMP, (tile + at) / atlas, 0.0).xy;
}

// The cells' blurred means at `uv` through a cubic B-spline, from four bilinear taps (Sigg &
// Hadwiger 2005), as Patry upsampled his: bilinear's creases would show in the gain.
float blurred_mean(uint means, float2 uv)
{
    float2 cells = float2(LE_CELLS_X, LE_CELLS_Y);
    float2 t = uv * cells - 0.5;
    float2 i = floor(t);
    float2 f = t - i;
    float2 f2 = f * f;
    float2 f3 = f2 * f;
    float2 w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    float2 w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    float2 w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    float2 w3 = f3 / 6.0;
    float2 g0 = w0 + w1;
    float2 g1 = w2 + w3;
    float2 h0 = (i - 0.5 + w1 / g0) / cells;
    float2 h1 = (i + 1.5 + w3 / g1) / cells;
    float a = sample_image_level(means, SAMPLER_LINEAR_CLAMP, float2(h0.x, h0.y), 0.0).x;
    float b = sample_image_level(means, SAMPLER_LINEAR_CLAMP, float2(h1.x, h0.y), 0.0).x;
    float c = sample_image_level(means, SAMPLER_LINEAR_CLAMP, float2(h0.x, h1.y), 0.0).x;
    float d = sample_image_level(means, SAMPLER_LINEAR_CLAMP, float2(h1.x, h1.y), 0.0).x;
    return g0.y * (g0.x * a + g1.x * b) + g1.y * (g0.x * c + g1.x * d);
}

// The base log luminance of a pixel of log luminance `l` at `uv`, given the Gaussian's base there.
public float local_base(LocalExposure le, float2 uv, float l, float blurred)
{
    float z = local_slice(l);
    uint z0 = uint(z);
    uint z1 = min(z0 + 1, LE_SLICES - 1);
    // `lerp_exact`: TAA's display pass reads this and compiles precise (#281, exact.slang).
    float2 g = lerp_exact(grid_tap(le.grid, uv, z0), grid_tap(le.grid, uv, z1), z - float(z0));
    float bilateral = lerp_exact(blurred, g.x / max(g.y, 1e-6), saturate(g.y / LE_MIN_WEIGHT));
    return lerp_exact(bilateral, blurred, le.blend);
}

// The Gaussian's base at `uv` (it varies over cells, so a pass reading neighbours takes it once).
public float local_blurred(LocalExposure le, float2 uv)
{
    return blurred_mean(le.means, uv);
}

// The gain in EV of a pixel of log luminance `l` whose base is `base`.
public float local_gain_ev(LocalExposure le, float l, float base)
{
    float c = base < le.middle ? le.shadow : le.highlight;
    float ev = clamp((c - 1.0) * (base - le.middle), -LE_MAX_DARKEN, LE_MAX_LIFT);
    return ev * saturate((l - (le.middle - LE_DARK_FLOOR)) / LE_DARK_FADE);
}

// The gain in EV of a pre-exposed colour at `uv` (0 when off), the Gaussian's base given.
public float local_exposure_ev(LocalExposure le, float3 color, float2 uv, float blurred)
{
    if (le.grid == 0xFFFFFFFFu)
        return 0.0;
    float l = local_log_luminance(color);
    return local_gain_ev(le, l, local_base(le, uv, l, blurred));
}

// The false-colour view: the shown image in grey, blue where the gain lifts, red where it
// darkens, full at 2 EV either way. `display` is the shown colour, linear in [0, 1].
public float3 local_exposure_view(float3 display, float ev)
{
    float grey = 0.08 + 0.6 * dot(saturate(display), LUMA);
    float amount = saturate(abs(ev) / 2.0);
    float3 tint = ev > 0.0 ? float3(0.15, 0.4, 1.0) : float3(1.0, 0.18, 0.1);
    return lerp_exact(float3(grey, grey, grey), tint, amount);
}
