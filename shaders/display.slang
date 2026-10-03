// The display pass, driven by crates/forge-render/src/display.rs: a pre-exposed HDR image in,
// the display image out through the chosen tone curve and the output's encoding
// (`tonemap.slang`). The ballad does the same inside its TAA resolve; this pass serves the
// paths without temporal filtering, and DLSS's output (D-045), with bloom as the resolve mixes it.
import bindless;
import tonemap;

struct DisplayPush
{
    uint image;
    uint curve;
    uint bloom;           // the bloom chain's top level (sampled index), 0xFFFFFFFF for none (bloom.slang)
    float bloom_strength; // how much of the shown image is bloom
    Output output;
    ToneTables tables;
};

// Levels the bloom chain's top level sums (crates/forge-render/src/bloom.rs, LEVELS).
static const float BLOOM_LEVELS = 6.0;

[[vk::push_constant]]
DisplayPush display_push;

struct Varyings
{
    float4 position : SV_Position;
};

[shader("vertex")]
Varyings vert_main(uint id : SV_VertexID)
{
    Varyings o;
    o.position = float4((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0, 0.0, 1.0);
    return o;
}

[shader("fragment")]
float4 frag_main(Varyings input) : SV_Target
{
    uint2 pixel = uint2(input.position.xy);
    float3 hdr = g_sampled_images[NonUniformResourceIndex(display_push.image)].Load(int3(int2(pixel), 0)).rgb;
    if (display_push.bloom != 0xFFFFFFFFu)
    {
        uint width;
        uint height;
        g_sampled_images[NonUniformResourceIndex(display_push.image)].GetDimensions(width, height);
        float2 uv = input.position.xy / float2(float(width), float(height));
        float3 bloom = sample_image_level(display_push.bloom, SAMPLER_LINEAR_CLAMP, uv, 0.0).rgb / BLOOM_LEVELS;
        hdr = lerp(hdr, bloom, display_push.bloom_strength);
    }
    return float4(display_output(hdr, display_push.curve, display_push.output, display_push.tables, pixel), 1.0);
}
