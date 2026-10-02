// The HDR preview (issue #94, crates/forge-app/src/hdr.rs): the frame drawn into an off-screen
// HDR10 image, shown on the SDR swapchain, so everything but the present runs and can be
// looked at on any monitor.
// - Mode 0: what an SDR display at paper white shows of it: the PQ signal to nits, Rec.2020 to
//   Rec.709, divided by the UI's white, clamped. Below paper white this is the HDR image.
// - Mode 1: the same in grey, with false colours where HDR goes beyond SDR: yellow above paper
//   white (brighter towards the peak), red at the peak, blue outside Rec.709's gamut.
import bindless;
import tonemap;

struct PreviewPush
{
    uint image;
    uint mode;
    float paper_white; // nits shown as 1.0
    float peak;        // the preset's peak, in nits
};

[[vk::push_constant]]
PreviewPush preview_push;

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
    int2 pixel = int2(input.position.xy);
    float3 signal = g_sampled_images[NonUniformResourceIndex(preview_push.image)].Load(int3(pixel, 0)).rgb;
    float3 nits = rec2020_to_rec709(pq_decode(signal));
    float3 shown = nits / preview_push.paper_white;
    if (preview_push.mode == 1)
    {
        float y = dot(nits, float3(0.2126, 0.7152, 0.0722));
        float grey = saturate(y / preview_push.paper_white);
        shown = float3(grey, grey, grey);
        if (y >= 0.95 * preview_push.peak)
            shown = float3(1.0, 0.0, 0.0);
        else if (y > preview_push.paper_white)
        {
            float above = saturate(log2(y / preview_push.paper_white) / log2(preview_push.peak / preview_push.paper_white));
            shown = lerp(float3(0.35, 0.3, 0.0), float3(1.0, 0.9, 0.0), above);
        }
        else if (min(nits.r, min(nits.g, nits.b)) < -0.01 * max(y, 1.0))
            shown = float3(0.1, 0.3, 1.0) * max(grey, 0.3);
    }
    return float4(saturate(shown), 1.0);
}
