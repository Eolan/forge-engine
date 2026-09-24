// The display pass, driven by crates/forge-render/src/display.rs: a pre-exposed HDR image in,
// the display image out through the chosen tone curve (`tonemap.slang`). The ballad does the
// same inside its TAA resolve; this pass serves the paths without temporal filtering.
import bindless;
import tonemap;

struct DisplayPush
{
    uint image;
    uint curve;
    uint encode_srgb;
    uint pad;
};

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
    float3 hdr = g_sampled_images[NonUniformResourceIndex(display_push.image)].Load(int3(int2(input.position.xy), 0)).rgb;
    return float4(display_output(hdr, display_push.curve, display_push.encode_srgb != 0), 1.0);
}
