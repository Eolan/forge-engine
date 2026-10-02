// MaxCLL and MaxFALL of the HDR frame as shown (issue #125, crates/forge-app/src/content_light.rs).
// Every pixel's largest channel over Rec.2020 as a PQ signal: the HDR10 image's own (PQ is the
// same curve on every channel, so the largest signal is the largest channel's), scRGB's linear
// Rec.709 (1.0 = 80 nits) converted and encoded. The signals are counted into 256 bins, the
// largest one kept in 1/65535ths after them. The CPU reads the words two frames later.
import bindless;
import tonemap;

// Must match content_light.rs.
static const uint BINS = 256;

struct MetadataPush
{
    uint* bins; // BINS counts, then the largest signal
    uint image;
    uint width;
    uint height;
    uint encoding; // OUTPUT_PQ or OUTPUT_SCRGB
};

[[vk::push_constant]]
MetadataPush metadata_push;

groupshared uint g_bins[BINS];
groupshared uint g_largest;

// The largest channel's PQ signal of `pixel`.
float signal_at(uint2 pixel)
{
    float3 color = g_sampled_images[NonUniformResourceIndex(metadata_push.image)].Load(int3(pixel, 0)).rgb;
    if (metadata_push.encoding == OUTPUT_PQ)
        return saturate(max(color.r, max(color.g, color.b)));
    float3 nits = rec709_to_rec2020(color * 80.0);
    return saturate(pq_encode(float3(max(nits.r, max(nits.g, nits.b)))).x);
}

// 32 × 32 pixels per group, a 2 × 2 quad per thread, counted in shared memory, then one global
// atomic per non-empty bin and one for the group's largest signal. Flat regions (space, the
// sky) share a bin: a quad of one bin counts with one atomic, and a wave of them with one.
[shader("compute")]
[numthreads(16, 16, 1)]
void histogram_main(uint3 id : SV_DispatchThreadID, uint index : SV_GroupIndex)
{
    g_bins[index] = 0;
    if (index == 0)
        g_largest = 0;
    GroupMemoryBarrierWithGroupSync();
    uint bins[4];
    uint pixels = 0;
    uint largest = 0;
    for (uint i = 0; i < 4; i++)
    {
        uint2 pixel = id.xy * 2 + uint2(i & 1, i >> 1);
        if (pixel.x < metadata_push.width && pixel.y < metadata_push.height)
        {
            float signal = signal_at(pixel);
            bins[pixels] = min(uint(signal * float(BINS)), BINS - 1);
            largest = max(largest, uint(round(signal * 65535.0)));
            pixels++;
        }
    }
    bool one_bin = pixels > 0;
    for (uint i = 1; i < pixels; i++)
        one_bin = one_bin && bins[i] == bins[0];
    uint first = pixels > 0 ? bins[0] : 0;
    uint ignored;
    if (WaveActiveAllTrue(one_bin) && WaveActiveAllEqual(first))
    {
        uint total = WaveActiveSum(pixels);
        if (WaveIsFirstLane())
            InterlockedAdd(g_bins[first], total, ignored);
    }
    else if (one_bin)
        InterlockedAdd(g_bins[first], pixels, ignored);
    else
    {
        for (uint i = 0; i < pixels; i++)
            InterlockedAdd(g_bins[bins[i]], 1u, ignored);
    }
    uint wave_largest = WaveActiveMax(largest);
    if (WaveIsFirstLane() && wave_largest != 0)
        InterlockedMax(g_largest, wave_largest, ignored);
    GroupMemoryBarrierWithGroupSync();
    uint count = g_bins[index];
    if (count != 0)
        InterlockedAdd(metadata_push.bins[index], count, ignored);
    if (index == 0 && g_largest != 0)
        InterlockedMax(metadata_push.bins[BINS], g_largest, ignored);
}
