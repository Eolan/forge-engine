// Luminance histogram for automatic exposure, driven by crates/forge-render/src/exposure.rs
// (Narkowicz 2016). The scene is stored pre-exposed (luminance in cd/m² times the frame's
// exposure); every pixel's luminance lands in one of 256 bins: bin 0 holds everything at or
// below 2^MIN_LOG2 (black, NaN), bins 1..255 split [MIN_LOG2, MAX_LOG2] evenly in log2, the
// last one also holding everything brighter. The CPU reads the bins two frames later.
import bindless;

// Must match exposure.rs.
static const uint BINS = 256;
static const float MIN_LOG2 = -16.0;
static const float MAX_LOG2 = 8.0;

struct HistogramPush
{
    uint* bins;
    uint image;
    uint width;
    uint height;
    uint pad;
};

[[vk::push_constant]]
HistogramPush histogram_push;

groupshared uint g_bins[BINS];

uint luminance_bin(float luminance)
{
    if (!(luminance > exp2(MIN_LOG2)))
        return 0;
    float t = (log2(luminance) - MIN_LOG2) / (MAX_LOG2 - MIN_LOG2);
    return 1 + min(uint(t * float(BINS - 1)), BINS - 2);
}

// 16 × 16 pixels per group: counted in shared memory, then one global atomic per non-empty bin.
[shader("compute")]
[numthreads(16, 16, 1)]
void histogram_main(uint3 id : SV_DispatchThreadID, uint index : SV_GroupIndex)
{
    g_bins[index] = 0;
    GroupMemoryBarrierWithGroupSync();
    if (id.x < histogram_push.width && id.y < histogram_push.height)
    {
        float3 color = g_sampled_images[NonUniformResourceIndex(histogram_push.image)].Load(int3(id.xy, 0)).rgb;
        float luminance = dot(color, float3(0.2126, 0.7152, 0.0722));
        uint ignored;
        InterlockedAdd(g_bins[luminance_bin(luminance)], 1u, ignored);
    }
    GroupMemoryBarrierWithGroupSync();
    uint count = g_bins[index];
    if (count != 0)
    {
        uint ignored;
        InterlockedAdd(histogram_push.bins[index], count, ignored);
    }
}
