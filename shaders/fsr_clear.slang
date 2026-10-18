// FSR 3.1's clears (D-070): what AMD's SDK clears between and before its passes (the previous
// depth it reconstructs by atomic max, the downsampler's counter and pyramid, a reset's history),
// to the values it clears them to (its frame info starts at -1, 1). One mip of one image a
// dispatch. Driven by crates/forge-render/src/fsr.rs.

import bindless;

[[vk::binding(1, 0)]] [[vk::image_format("r16f")]] RWTexture2D<float> clear_r16f[];
[[vk::binding(1, 0)]] [[vk::image_format("r8")]] RWTexture2D<float> clear_r8[];
[[vk::binding(1, 0)]] [[vk::image_format("rgba32f")]] RWTexture2D<float4> clear_rgba32f[];
[[vk::binding(1, 0)]] [[vk::image_format("r32ui")]] RWTexture2D<uint> clear_r32ui[];

// The image's format, as crates/forge-render/src/fsr.rs names it.
static const uint CLEAR_R8 = 0;
static const uint CLEAR_R16F = 1;
static const uint CLEAR_RG16F = 2;
static const uint CLEAR_RGBA32F = 3;
static const uint CLEAR_R32UI = 4;
static const uint CLEAR_RGBA16F = 5;

struct ClearPush
{
    float4 value;
    uint image;
    uint format;
    uint width;
    uint height;
};

[[vk::push_constant]]
ConstantBuffer<ClearPush> clear_push;

[shader("compute")]
[numthreads(8, 8, 1)]
void clear_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= clear_push.width || id.y >= clear_push.height)
        return;
    uint image = NonUniformResourceIndex(clear_push.image);
    float4 value = clear_push.value;
    switch (clear_push.format)
    {
    case CLEAR_R8:
        clear_r8[image][id.xy] = value.x;
        break;
    case CLEAR_R16F:
        clear_r16f[image][id.xy] = value.x;
        break;
    case CLEAR_RG16F:
        g_storage_images_rg16f[image][id.xy] = value.xy;
        break;
    case CLEAR_RGBA32F:
        clear_rgba32f[image][id.xy] = value;
        break;
    case CLEAR_RGBA16F:
        g_storage_images_rgba16f[image][id.xy] = value;
        break;
    default:
        clear_r32ui[image][id.xy] = asuint(value.x);
        break;
    }
}
