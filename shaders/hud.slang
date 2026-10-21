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

// ------------------------------------------------------------------- the lens ---
// A magnifier (#328): inside a circle, the frame round its centre `factor` times larger, read
// from a copy of the frame made just before, tinted and darkened towards its rim; its edge is a
// pixel wide. Drawn before the quads, so a reticle stands over it.

struct LensPush
{
    float2 centre;          // the window's pixels
    float radius;           // pixels
    float factor;           // 2: a 2× scope
    float4 tint;            // rgb: times the lens's picture
    float vignette;         // how dark the rim gets, 0 to 1
    float vignette_start;   // the share of the radius where it starts darkening
    uint frame;             // sampled image: the frame's copy
    uint encoding;          // as `Push::encoding`
    float2 size;            // the window's pixels
    float2 pad;
};

[[vk::push_constant]]
LensPush lens_push;

struct LensVaryings
{
    float4 position : SV_Position;
};

[shader("vertex")]
LensVaryings lens_vert_main(uint vertex : SV_VertexID)
{
    uint2 k = CORNERS[vertex % 6];
    float2 corner = lens_push.centre + (float2(k) * 2.0 - 1.0) * (lens_push.radius + 1.0);
    LensVaryings o;
    o.position = float4(corner / lens_push.size * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

[shader("fragment")]
float4 lens_main(LensVaryings input) : SV_Target0
{
    float2 pixel = input.position.xy;
    float2 offset = pixel - lens_push.centre;
    float d = length(offset);
    float edge = saturate(lens_push.radius - d + 0.5);
    if (edge <= 0.0)
        discard;
    float2 source = lens_push.centre + offset / max(lens_push.factor, 1e-3);
    float3 color = sample_image_level(lens_push.frame, SAMPLER_LINEAR_CLAMP, source / lens_push.size, 0.0).rgb;
    color *= lens_push.tint.rgb;
    float rim = smoothstep(lens_push.vignette_start, 1.0, d / max(lens_push.radius, 1.0));
    color *= 1.0 - saturate(lens_push.vignette) * rim;
    return float4(color, edge);
}
