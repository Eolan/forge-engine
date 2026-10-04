// AMD's FidelityFX shadow denoiser, ported to Slang for the sun's soft shadows where NVIDIA's NRD
// is absent (#173, D-049). Driven by crates/forge-render/src/ffx_shadows.rs.
//
// From `ffx-shadows-dnsr/` of https://github.com/GPUOpen-Effects/FidelityFX-Denoiser (commit
// d7dfecb, 2021), under the MIT licence below. Forge's changes, for its cross-vendor rules:
// - The shadow mask is packed by one thread per 8×4 tile from the trace's image, not by
//   `WaveActiveBitOr` over a wave assumed to hold the tile.
// - `WaveActiveAllTrue` and the quad reads (`QuadReadAcrossX/Y`) became a groupshared flag and
//   plain reads of the 2×2 quad's pixels: no assumption on the subgroup size.
// - Every image read is clamped and every write guarded to the image (NRD's lesson, #172:
//   Vulkan leaves out-of-bounds accesses undefined where D3D12 drops them).
// - Depths are the view depth the trace wrote (metres), not the depth buffer's; the
//   disocclusion test reprojects through the camera's step instead of a reprojection matrix.
// - The filter's groupshared tiles hold 32-bit floats, not fp16: the same on every vendor.
// - The last pass writes the visibility's square root, as SIGMA's output, so the resolve reads
//   both the same way.
//
// Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
import bindless;

// The camera of the frame (a per-frame block, `FfxShadowFrame` in ffx_shadows.rs).
struct FfxFrame
{
    float4 view_x;      // the view's rotation, its rows: camera-relative metres → view
    float4 view_y;
    float4 view_z;
    float4 previous_z;  // the previous view's z row, from this frame's camera-relative metres
    float4 unproject;   // 1 / projection[0][0], 1 / projection[1][1], the jitter in NDC (x, y)
    uint width;
    uint height;
    uint reset;         // 1: no history (the first frame, a cut)
    uint pad;
};

// The trace's penumbra where the ray reached the sun (`NRD_MISS` in meshlet.slang).
static const float LIT = 65504.0;
// A view depth past this is no surface (`NOTHING_DRAWN_Z` in meshlet.slang is 1e7).
static const float NO_SURFACE_Z = 1.0e6;
// The filter's depth similarity, metres (the sample's `DepthSimilaritySigma`).
static const float DEPTH_SIGMA = 1.0;

static const uint TILE_META_DATA_CLEAR_MASK = 1u;
static const uint TILE_META_DATA_LIGHT_MASK = 2u;

uint rounded_divide(uint value, uint divisor)
{
    return (value + divisor - 1) / divisor;
}

uint2 mask_tile(uint2 pixel)
{
    return uint2(pixel.x / 8, pixel.y / 4);
}

uint linear_mask_tile(uint2 tile, uint width)
{
    return tile.y * rounded_divide(width, 8) + tile.x;
}

uint bit_of(uint2 pixel)
{
    return 1u << ((pixel.y % 4) * 8 + (pixel.x % 8));
}

int2 clamp_pixel(int2 pixel, uint width, uint height)
{
    return clamp(pixel, int2(0, 0), int2(int(width) - 1, int(height) - 1));
}

float view_depth_at(uint image, int2 pixel)
{
    return abs(g_sampled_images_r[NonUniformResourceIndex(image)].Load(int3(pixel, 0)));
}

// ------------------------------------------------------------------------- the mask ---

struct PackPush
{
    uint* mask;        // one word per 8×4 tile, bit (y × 8 + x) set where the ray reached the sun
    uint penumbra;     // sampled (r32f): the trace's penumbra
    uint width;
    uint height;
    uint pad;
};

[[vk::push_constant]]
PackPush pack_push;

[shader("compute")]
[numthreads(8, 8, 1)]
void pack_main(uint3 id : SV_DispatchThreadID)
{
    PackPush p = pack_push;
    uint2 tiles = uint2(rounded_divide(p.width, 8), rounded_divide(p.height, 4));
    if (id.x >= tiles.x || id.y >= tiles.y)
        return;
    uint word = 0;
    for (uint y = 0; y < 4; y++)
    {
        for (uint x = 0; x < 8; x++)
        {
            // Past the image's edge a tile repeats its last pixel.
            int2 pixel = clamp_pixel(int2(id.xy * uint2(8, 4) + uint2(x, y)), p.width, p.height);
            float penumbra = g_sampled_images_r[NonUniformResourceIndex(p.penumbra)].Load(int3(pixel, 0));
            if (penumbra >= LIT)
                word |= 1u << (y * 8 + x);
        }
    }
    p.mask[id.y * tiles.x + id.x] = word;
}

// ------------------------------------------------- tile classification, the history ---

struct ClassifyPush
{
    FfxFrame* frame;
    uint* mask;
    uint* meta;              // one word per 8×8 tile: TILE_META_DATA_*
    uint view_z;             // sampled (r32f): this frame's view depth
    uint normal;             // sampled (rgba16f): the world normal
    uint motion;             // sampled (rg16f): the offset in UV to where the pixel was
    uint previous_view_z;    // sampled (r32f)
    uint previous_moments;   // sampled (rgba16f)
    uint history;            // sampled (rg16f): the previous frame's first filter pass
    uint reprojection;       // storage (rg16f): the shadow reprojected, its variance
    uint moments;            // storage (rgba16f): mean, M2, sample count
    uint view_z_out;         // storage (r32f): this frame's view depth, for the next
    uint pad0;
    uint pad1;
    uint pad2;
};

[[vk::push_constant]]
ClassifyPush classify_push;

groupshared uint g_false_count;

// True when `value` holds for every thread of the 8×8 group (FFX's ThreadGroupAllTrue, without
// the wave intrinsic). Called in uniform control flow.
bool group_all_true(bool value, uint group_index)
{
    GroupMemoryBarrierWithGroupSync();
    if (group_index == 0)
        g_false_count = 0;
    GroupMemoryBarrierWithGroupSync();
    if (!value)
        InterlockedOr(g_false_count, 1u);
    GroupMemoryBarrierWithGroupSync();
    return g_false_count == 0;
}

void search_spatial_region(uint2 gid, uint width, uint height, out bool all_in_light, out bool all_in_shadow)
{
    // The filters reach 1 + 2 + 4 = 7 pixels around the 8×8 tile: the 8×4 masks around it.
    int2 base_tile = int2(mask_tile(gid * 8));
    int2 last = int2(int(rounded_divide(width, 8)) - 1, int(rounded_divide(height, 4)) - 1);
    uint combined_or = 0;
    uint combined_and = 0xFFFFFFFFu;
    for (int j = -2; j <= 3; ++j)
    {
        for (int i = -1; i <= 1; ++i)
        {
            int2 tile = clamp(base_tile + int2(i, j), int2(0, 0), last);
            uint word = classify_push.mask[linear_mask_tile(uint2(tile), width)];
            combined_or |= word;
            combined_and &= word;
        }
    }
    all_in_light = combined_and == 0xFFFFFFFFu;
    all_in_shadow = combined_or == 0u;
}

// The pixel's position relative to the camera, metres, from its view depth.
float3 camera_relative(FfxFrame* f, uint2 pixel, float view_z)
{
    float2 size = float2(float(f[0].width), float(f[0].height));
    float2 ndc = float2((float(pixel.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (float(pixel.y) + 0.5) / size.y * 2.0);
    ndc -= f[0].unproject.zw;
    float depth = abs(view_z);
    float3 v = float3(ndc.x * depth * f[0].unproject.x, ndc.y * depth * f[0].unproject.y, view_z);
    return v.x * f[0].view_x.xyz + v.y * f[0].view_y.xyz + v.z * f[0].view_z.xyz;
}

bool is_disoccluded(FfxFrame* f, uint2 pixel, float view_z, float2 previous_uv)
{
    ClassifyPush p = classify_push;
    if (!(all(previous_uv > 0.0) && all(previous_uv < 1.0)))
        return true;
    float3 n = g_sampled_images[NonUniformResourceIndex(p.normal)].Load(int3(pixel, 0)).xyz;
    float3 relative = camera_relative(f, pixel, view_z);
    // The more the surface faces away from the eye, the larger the depth's errors.
    float3 view_direction = normalize(-relative);
    float z_alignment = pow(1.0 - dot(view_direction, normalize(n)), 8.0);
    float linear_depth = abs(dot(f[0].previous_z.xyz, relative) + f[0].previous_z.w);
    int2 previous = clamp_pixel(int2(previous_uv * float2(float(f[0].width), float(f[0].height))), f[0].width, f[0].height);
    float previous_depth = view_depth_at(p.previous_view_z, previous);
    float difference = abs(previous_depth - linear_depth) / linear_depth;
    return difference >= lerp(1e-2, 1e-1, z_alignment);
}

// The motion of the nearest of the pixel's 2×2 quad (FFX's quad reads).
float2 closest_motion(FfxFrame* f, uint2 pixel)
{
    ClassifyPush p = classify_push;
    uint2 quad = pixel & ~1u;
    int2 best = clamp_pixel(int2(pixel), f[0].width, f[0].height);
    float best_depth = view_depth_at(p.view_z, best);
    for (uint k = 0; k < 4; k++)
    {
        int2 other = clamp_pixel(int2(quad + uint2(k & 1u, k >> 1)), f[0].width, f[0].height);
        float depth = view_depth_at(p.view_z, other);
        if (depth < best_depth)
        {
            best_depth = depth;
            best = other;
        }
    }
    return g_sampled_images[NonUniformResourceIndex(p.motion)].Load(int3(best, 0)).xy;
}

// FFX's 17-tap Gaussian, exp(−3 i² / 81) normalised over i = −8…8, computed once here.
static const int KERNEL_RADIUS = 8;
static const float KERNEL[9] = { 0.11082929, 0.10679958, 0.09556847, 0.07941266, 0.06127663,
                                 0.04390662, 0.02921428, 0.01805055, 0.01035657 };
float kernel_weight(int i)
{
    return KERNEL[abs(i)];
}

// The horizontal half of a 17×17 neighbourhood of the binary mask: its weighted mean.
float horizontal_neighborhood(int2 pixel, uint width, uint height)
{
    if (pixel.y < 0 || pixel.y >= int(height))
        return 0.0;
    uint2 tile = mask_tile(uint2(pixel));
    uint index = linear_mask_tile(tile, width);
    bool first = tile.x == 0;
    bool last = tile.x == rounded_divide(width, 8) - 1;
    uint left_tile = first ? 0u : classify_push.mask[index - 1];
    uint center_tile = classify_push.mask[index];
    uint right_tile = last ? 0u : classify_push.mask[index + 1];
    uint row = (uint(pixel.y) % 4) * 8;
    uint left = (left_tile >> row) & 0xFFu;
    uint center = (center_tile >> row) & 0xFFu;
    uint right = (right_tile >> row) & 0xFFu;
    // [left, center, right] from the lowest bit, shifted so the pixel lands on bit 8.
    uint neighborhood = (left | (center << 8) | (right << 16)) >> (uint(pixel.x) % 8);
    float moment = 0.0;
    for (int i = 0; i < 8; ++i)
        moment += ((1u << uint(i)) & neighborhood) != 0 ? kernel_weight(8 - i) : 0.0;
    moment += ((1u << 8) & neighborhood) != 0 ? kernel_weight(0) : 0.0;
    for (int i = 1; i <= 8; ++i)
        moment += ((1u << uint(8 + i)) & neighborhood) != 0 ? kernel_weight(i) : 0.0;
    return moment;
}

groupshared float g_neighborhood[8][24];

float local_neighborhood(int2 pixel, uint2 gtid, uint width, uint height)
{
    float upper = horizontal_neighborhood(int2(pixel.x, pixel.y - 8), width, height);
    float center = horizontal_neighborhood(pixel, width, height);
    float lower = horizontal_neighborhood(int2(pixel.x, pixel.y + 8), width, height);
    g_neighborhood[gtid.x][gtid.y] = upper;
    g_neighborhood[gtid.x][gtid.y + 8] = center;
    g_neighborhood[gtid.x][gtid.y + 16] = lower;
    GroupMemoryBarrierWithGroupSync();
    float sum = center * kernel_weight(0) + (upper + lower) * kernel_weight(KERNEL_RADIUS);
    for (int i = 1; i < KERNEL_RADIUS; ++i)
    {
        float up = g_neighborhood[gtid.x][8 + gtid.y - i];
        float down = g_neighborhood[gtid.x][8 + gtid.y + i];
        sum += (up + down) * kernel_weight(i);
    }
    return sum;
}

void write_tile_meta(uint2 gid, uint2 gtid, uint width, bool cleared, bool all_in_light)
{
    if (all(gtid == 0))
    {
        uint word = (all_in_light ? TILE_META_DATA_LIGHT_MASK : 0u) | (cleared ? TILE_META_DATA_CLEAR_MASK : 0u);
        classify_push.meta[gid.y * rounded_divide(width, 8) + gid.x] = word;
    }
}

[shader("compute")]
[numthreads(8, 8, 1)]
void classify_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID, uint group_index : SV_GroupIndex)
{
    ClassifyPush p = classify_push;
    FfxFrame* f = p.frame;
    uint width = f[0].width;
    uint height = f[0].height;
    uint2 gid = group.xy;
    uint2 gtid = thread.xy;
    uint2 pixel = gid * 8 + gtid;
    bool inside = pixel.x < width && pixel.y < height;
    int2 at = clamp_pixel(int2(pixel), width, height);
    float view_z = g_sampled_images_r[NonUniformResourceIndex(p.view_z)].Load(int3(at, 0));
    if (inside)
        g_storage_images_r32f[NonUniformResourceIndex(p.view_z_out)][pixel] = view_z;
    bool receiver = inside && abs(view_z) < NO_SURFACE_Z;

    // Tiles of sky, then tiles whose 7-pixel surroundings are all lit or all shadowed, are only
    // cleared: neighbouring tiles' filters may read them.
    bool skip_sky = group_all_true(!receiver, group_index);
    bool all_in_light = false;
    bool all_in_shadow = false;
    search_spatial_region(gid, width, height, all_in_light, all_in_shadow);
    bool skip_tile = skip_sky || group_all_true(all_in_light || all_in_shadow, group_index);
    if (skip_tile)
    {
        float value = (!skip_sky && all_in_light) ? 1.0 : 0.0;
        write_tile_meta(gid, gtid, width, true, !skip_sky && all_in_light);
        if (inside)
        {
            g_storage_images_rg16f[NonUniformResourceIndex(p.reprojection)][pixel] = float2(value, 0.0);
            g_storage_images_rgba16f[NonUniformResourceIndex(p.moments)][pixel] = float4(value, 0.0, receiver ? 1.0 : 0.0, 0.0);
        }
        return;
    }
    write_tile_meta(gid, gtid, width, false, false);

    float2 motion = closest_motion(f, uint2(at));
    float neighborhood = local_neighborhood(int2(pixel), gtid, width, height);

    float2 size = float2(float(width), float(height));
    float2 uv = (float2(pixel) + 0.5) / size;
    float2 history_uv = uv + motion;
    int2 history_pixel = clamp_pixel(int2(history_uv * size), width, height);

    float4 moments_current = 0.0;
    float variance = 0.0;
    float shadow_clamped = 0.0;
    if (receiver)
    {
        uint word = p.mask[linear_mask_tile(mask_tile(pixel), width)];
        float shadow_current = (word & bit_of(pixel)) != 0 ? 1.0 : 0.0;

        // The temporal mean and variance (Welford's).
        {
            bool disoccluded = f[0].reset != 0 || is_disoccluded(f, pixel, view_z, history_uv);
            float3 previous = disoccluded ? float3(0.0, 0.0, 0.0)
                : g_sampled_images[NonUniformResourceIndex(p.previous_moments)].Load(int3(history_pixel, 0)).xyz;
            float old_m = previous.x;
            float old_s = previous.y;
            float count = previous.z + 1.0;
            float new_m = old_m + (shadow_current - old_m) / count;
            float new_s = old_s + (shadow_current - old_m) * (shadow_current - new_m);
            variance = count > 1.0 ? new_s / (count - 1.0) : 1.0;
            moments_current = float4(new_m, new_s, count, 0.0);
        }

        // The history, clamped to the neighbourhood's mean ± half its deviation.
        {
            float mean = neighborhood;
            float spatial_variance = max(neighborhood - mean * mean, 0.0);
            float deviation = sqrt(spatial_variance);
            float lo = mean - 0.5 * deviation;
            float hi = mean + 0.5 * deviation;
            float shadow_previous = shadow_current;
            if (f[0].reset == 0)
                shadow_previous = g_sampled_images[NonUniformResourceIndex(p.history)]
                    .SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], history_uv, 0.0).x;
            shadow_clamped = clamp(shadow_previous, lo, hi);

            // Fewer frames of history where it left the neighbourhood.
            float discontinuity = (shadow_previous - mean) / max(0.5 * deviation, 0.001);
            moments_current.z *= exp(-discontinuity * discontinuity / 20.0);

            // A larger variance over the first frames.
            if (moments_current.z < 16.0)
            {
                variance = max(variance, spatial_variance);
                variance *= max(16.0 - moments_current.z, 1.0);
            }
        }

        float history_weight = sqrt(max(8.0 - moments_current.z, 0.0) / 8.0);
        shadow_clamped = lerp(shadow_clamped, shadow_current, lerp(0.05, 1.0, history_weight));
    }

    if (inside)
    {
        g_storage_images_rg16f[NonUniformResourceIndex(p.reprojection)][pixel] = float2(shadow_clamped, variance);
        g_storage_images_rgba16f[NonUniformResourceIndex(p.moments)][pixel] = moments_current;
    }
}

// ----------------------------------------------------------------------- the filters ---

struct FilterPush
{
    FfxFrame* frame;
    uint* meta;
    uint input;      // sampled (rg16f): the shadow and its variance
    uint view_z;     // sampled (r32f)
    uint normal;     // sampled (rgba16f)
    uint output;     // storage: rg16f (the shadow, its variance), or r32f on the last pass
    uint step;       // the à-trous step: 1, 2, 4
    uint last;       // 1: write the shadow's square root (r32f), as SIGMA's output
    uint pad0;
    uint pad1;
};

[[vk::push_constant]]
FilterPush filter_push;

groupshared float2 g_input[16][16];
groupshared float g_depth[16][16];
groupshared float3 g_normal[16][16];

void load_shared(int2 pixel, int2 slot, uint width, uint height)
{
    FilterPush p = filter_push;
    int2 at = clamp_pixel(pixel, width, height);
    g_input[slot.y][slot.x] = g_sampled_images[NonUniformResourceIndex(p.input)].Load(int3(at, 0)).xy;
    g_depth[slot.y][slot.x] = g_sampled_images_r[NonUniformResourceIndex(p.view_z)].Load(int3(at, 0));
    g_normal[slot.y][slot.x] = g_sampled_images[NonUniformResourceIndex(p.normal)].Load(int3(at, 0)).xyz;
}

float filtered_variance(int2 slot)
{
    const float kernel[2][2] = { { 1.0 / 4.0, 1.0 / 8.0 }, { 1.0 / 8.0, 1.0 / 16.0 } };
    float variance = 0.0;
    for (int y = -1; y <= 1; ++y)
        for (int x = -1; x <= 1; ++x)
            variance += kernel[abs(x)][abs(y)] * g_input[slot.y + y][slot.x + x].y;
    return variance;
}

float2 denoise(int2 slot, uint step)
{
    float2 center = g_input[slot.y][slot.x];
    float3 normal_center = g_normal[slot.y][slot.x];
    float depth_center = abs(g_depth[slot.y][slot.x]);
    float deviation = sqrt(max(filtered_variance(slot) + 1e-9, 0.0));
    float weight_sum = 1.0;
    float2 shadow_sum = center;
    const float kernel[3] = { 1.0, 2.0 / 3.0, 1.0 / 6.0 };
    for (int y = -1; y <= 1; ++y)
    {
        for (int x = -1; x <= 1; ++x)
        {
            if (x == 0 && y == 0)
                continue;
            int2 at = slot + int2(x, y) * int(step);
            float depth = abs(g_depth[at.y][at.x]);
            if (depth >= NO_SURFACE_Z)
                continue;
            float2 neighbour = g_input[at.y][at.x];
            float w = kernel[abs(x)] * kernel[abs(y)];
            w *= exp(-abs(center.x - neighbour.x) / deviation);
            w *= exp(-abs(depth_center - depth) / DEPTH_SIGMA);
            w *= pow(saturate(dot(normal_center, g_normal[at.y][at.x])), 32.0);
            shadow_sum += float2(w, w * w) * neighbour;
            weight_sum += w;
        }
    }
    return float2(shadow_sum.x / weight_sum, shadow_sum.y / (weight_sum * weight_sum));
}

[shader("compute")]
[numthreads(8, 8, 1)]
void filter_main(uint3 group : SV_GroupID, uint3 thread : SV_GroupThreadID)
{
    FilterPush p = filter_push;
    FfxFrame* f = p.frame;
    uint width = f[0].width;
    uint height = f[0].height;
    uint2 gid = group.xy;
    int2 gtid = int2(thread.xy);
    int2 pixel = int2(gid * 8) + gtid;
    bool inside = uint(pixel.x) < width && uint(pixel.y) < height;
    uint meta = p.meta[gid.y * rounded_divide(width, 8) + gid.x];

    float2 result = 0.0;
    if ((meta & TILE_META_DATA_CLEAR_MASK) != 0)
    {
        // Uniform over the group: no barrier is skipped by some threads only.
        result.x = (meta & TILE_META_DATA_LIGHT_MASK) != 0 ? 1.0 : 0.0;
    }
    else
    {
        // The tile and 4 pixels around it: 16×16.
        int2 origin = pixel - 4;
        load_shared(origin, gtid, width, height);
        load_shared(origin + int2(8, 0), gtid + int2(8, 0), width, height);
        load_shared(origin + int2(0, 8), gtid + int2(0, 8), width, height);
        load_shared(origin + int2(8, 8), gtid + int2(8, 8), width, height);
        GroupMemoryBarrierWithGroupSync();
        int2 slot = gtid + 4;
        if (inside && abs(g_depth[slot.y][slot.x]) < NO_SURFACE_Z)
            result = denoise(slot, p.step);
    }
    if (!inside)
        return;
    if (p.last != 0)
        g_storage_images_r32f[NonUniformResourceIndex(p.output)][pixel] = sqrt(saturate(result.x));
    else
        g_storage_images_rg16f[NonUniformResourceIndex(p.output)][pixel] = result;
}
