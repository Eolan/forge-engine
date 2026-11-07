// Light with no surface, added over the scene. Driven by crates/forge-render/src/holograms.rs.
//
// Each picture's colour times its alpha, its hue, saturation and value turned as its row says, times
// its row's glow and the frame's exposure, added to the HDR scene where nothing nearer stands. It
// tests the frame's depth itself and writes none.

import bindless;
import exact;

struct HologramVertex
{
    float3 position;
    uint row;
    float2 uv;
    float2 pad;
};

struct HologramRow
{
    float3 glow;
    float hue;
    uint picture;
    float saturation;
    float value;
    float pad;
};

struct HologramFrame
{
    float4x4 view_proj;
    float3 camera;
    float exposure;
    HologramVertex* vertices;
    HologramRow* rows;
    uint depth;
};

[[vk::push_constant]]
HologramFrame* f;

struct Varyings
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
    nointerpolation uint row : TEXCOORD1;
};

[shader("vertex")]
Varyings vert_main(uint vertex : SV_VertexID)
{
    HologramVertex v = f[0].vertices[vertex];
    Varyings out;
    out.position = mul(f[0].view_proj, float4(v.position - f[0].camera, 1.0));
    out.uv = v.uv;
    out.row = v.row;
    return out;
}

// A colour's hue turned about the grey axis (Rodrigues' rotation), so a picture's greys stay grey.
float3 turn_hue(float3 c, float turns)
{
    if (turns == 0.0)
        return c;
    const float angle = turns * 6.2831853;
    const float3 k = float3(0.57735027, 0.57735027, 0.57735027);
    const float s = sin(angle);
    const float d = cos(angle);
    return c * d + cross(k, c) * s + k * dot(k, c) * (1.0 - d);
}

[shader("fragment")]
float4 frag_main(Varyings input) : SV_Target0
{
    // Reversed-Z: what stands nearer has the greater depth.
    const float scene = g_sampled_images_r[NonUniformResourceIndex(f[0].depth)].Load(int3(int2(input.position.xy), 0));
    if (scene > input.position.z)
        discard;
    const HologramRow row = f[0].rows[input.row];
    const float4 picture = g_sampled_images[NonUniformResourceIndex(row.picture)].Sample(g_samplers[SAMPLER_ANISOTROPIC_REPEAT], input.uv);
    float3 c = turn_hue(picture.rgb, row.hue);
    const float grey = dot(c, float3(0.2126, 0.7152, 0.0722));
    c = max(lerp_exact(float3(grey), c, row.saturation), 0.0) * row.value;
    // Alpha 0: light added, over the premultiplied blend. Held within what the HDR target stores, as the
    // glass's and the water's are: an infinity would reach the upscaler's history and the bloom.
    return float4(min(c * picture.a * row.glow * f[0].exposure, 32768.0), 0.0);
}
