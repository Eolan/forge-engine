// The global bindless set (set 0), created by `forge_gpu::bindless`.
// Binding numbers and sampler slots must match `crates/forge-gpu/src/bindless.rs`.
module bindless;

[[vk::binding(0, 0)]]
public Texture2D<float4> g_sampled_images[];
[[vk::binding(0, 0)]]
public Texture2D<float> g_sampled_images_r[];
[[vk::binding(0, 0)]]
public Texture2D<uint> g_sampled_images_u32[];

[[vk::binding(1, 0)]]
[[vk::image_format("r32f")]]
public RWTexture2D<float> g_storage_images_r32f[];
[[vk::binding(1, 0)]]
[[vk::image_format("rgba16f")]]
public RWTexture2D<float4> g_storage_images_rgba16f[];

[[vk::binding(2, 0)]]
public SamplerState g_samplers[];

public static const uint SAMPLER_LINEAR_CLAMP = 0;
public static const uint SAMPLER_NEAREST_CLAMP = 1;
public static const uint SAMPLER_LINEAR_REPEAT = 2;
public static const uint SAMPLER_ANISOTROPIC_REPEAT = 3;
public static const uint SAMPLER_MIN_REDUCTION_CLAMP = 4;

public float4 sample_image(uint image, uint sampler, float2 uv)
{
    return g_sampled_images[NonUniformResourceIndex(image)].Sample(g_samplers[NonUniformResourceIndex(sampler)], uv);
}

public float4 sample_image_level(uint image, uint sampler, float2 uv, float level)
{
    return g_sampled_images[NonUniformResourceIndex(image)].SampleLevel(g_samplers[NonUniformResourceIndex(sampler)], uv, level);
}

public float sample_depth_level(uint image, uint sampler, float2 uv, float level)
{
    return g_sampled_images_r[NonUniformResourceIndex(image)].SampleLevel(g_samplers[NonUniformResourceIndex(sampler)], uv, level);
}
