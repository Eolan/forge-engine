// A game's HUD (#328), drawn by the shell after the demo's frame (its tone mapping) and before the
// debug overlay: rectangles in the window's pixels, each a solid colour or a texture of the
// bindless set times its colour, blended over the picture by their alpha, in their order.
// Driven by crates/forge-app/src/hud.rs.
import bindless;
import tonemap;

// Must match GpuHudQuad in hud.rs.
struct Quad
{
    float4 rect;   // left, top, right, bottom, the window's pixels
    float4 uv;     // the texture's region: u0, v0, u1, v1
    float4 color;  // linear, straight alpha; times the texture's
    uint texture;  // a sampled image of the bindless set; ~0: none
    uint3 pad;
};

struct Push
{
    Quad* quads;
    float2 size;     // the window's pixels
    uint encoding;   // the overlay's: 0 the display's own, OUTPUT_PQ, OUTPUT_SCRGB
    float ui_white;  // nits of the interface's white in HDR
};

[[vk::push_constant]]
Push push;

struct Varyings
{
    float4 position : SV_Position;
    float2 uv : UV;
    nointerpolation uint quad : QUAD;
};

// Two triangles a quad: its corners as (right?, bottom?).
static const uint2 CORNERS[6] = { uint2(0, 0), uint2(1, 0), uint2(0, 1), uint2(0, 1), uint2(1, 0), uint2(1, 1) };

[shader("vertex")]
Varyings vert_main(uint vertex : SV_VertexID)
{
    uint q = vertex / 6;
    uint2 k = CORNERS[vertex % 6];
    Quad quad = push.quads[q];
    float2 pixel = float2(k.x != 0 ? quad.rect.z : quad.rect.x, k.y != 0 ? quad.rect.w : quad.rect.y);
    Varyings o;
    // Vulkan's clip space has y down, as the window's pixels.
    o.position = float4(pixel / push.size * 2.0 - 1.0, 0.0, 1.0);
    o.uv = float2(k.x != 0 ? quad.uv.z : quad.uv.x, k.y != 0 ? quad.uv.w : quad.uv.y);
    o.quad = q;
    return o;
}

[shader("fragment")]
float4 frag_main(Varyings input) : SV_Target0
{
    Quad quad = push.quads[input.quad];
    float4 color = quad.color;
    if (quad.texture != 0xFFFFFFFFu)
        color *= sample_image(quad.texture, SAMPLER_LINEAR_CLAMP, input.uv);
    if (color.a <= 0.0)
        discard;
    if (push.encoding == OUTPUT_PQ)
        color.rgb = pq_encode(rec709_to_rec2020(color.rgb) * push.ui_white);
    else if (push.encoding == OUTPUT_SCRGB)
        color.rgb *= push.ui_white / 80.0;
    return color;
}
