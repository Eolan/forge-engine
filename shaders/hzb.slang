// Hierarchical-Z pyramid build: one dispatch per level, each texel takes the minimum
// (farthest, reversed-Z) of a 2×2 footprint in the source through a min-reduction sampler.
// Level 0 is sized to the previous power of two of the depth buffer: with a source-to-
// destination scale below 2, the 2×2 footprints of neighbouring texels overlap, so no depth
// texel is skipped and the pyramid stays conservative.
import bindless;

struct Push
{
    uint src_image;   // sampled image index (depth buffer or the pyramid itself)
    uint src_level;   // mip level to read
    uint dst_image;   // storage image index of the level being written
    uint dst_width;
    uint dst_height;
    uint pad0;
    uint pad1;
    uint pad2;
};

[[vk::push_constant]]
Push push;

[shader("compute")]
[numthreads(8, 8, 1)]
void hzb_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.dst_width || id.y >= push.dst_height)
        return;
    float2 uv = (float2(id.xy) + 0.5) / float2(push.dst_width, push.dst_height);
    float depth = g_sampled_images_r[NonUniformResourceIndex(push.src_image)].SampleLevel(
        g_samplers[SAMPLER_MIN_REDUCTION_CLAMP], uv, push.src_level);
    g_storage_images_r32f[NonUniformResourceIndex(push.dst_image)][id.xy] = depth;
}
