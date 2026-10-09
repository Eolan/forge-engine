// A distant body in the sky (#220; crates/forge-render/src/skybody.rs): a sphere far enough to be
// a disc, the Moon from the Earth or the Earth from the Moon. Its colour map is shaded by the sun,
// its air is a rim of scattered light at its limb, and the air in front of the camera dims it
// (the CPU's transmittance, in the sun's term). Over an air's sky it adds its light to the sky's,
// which lies in front of it; in space it replaces the stars behind it.
import bindless;

struct Push
{
    float4x4 body_from_clip; // clip space to the body's frame (+Y its north, +Z its prime meridian)
    float4 centre;           // xyz: from the camera towards its centre, its frame, unit; w: sin of its angular radius
    float4 sun;              // xyz: towards the sun, its frame; w: pre-exposed luminance of an albedo of 1 facing the sun
    float4 rim;              // rgb: its air's light at the limb, a share of the sun's term; a: the air's depth, a share of its radius (0: none)
    uint image;              // sampled index of its colour map
    uint color;              // storage index of the HDR image (read and written)
    uint depth;              // sampled index of the depth
    uint size;               // width | height << 16 | 1 << 31 when it is seen through an air (its light added)
};

[[vk::push_constant]]
Push push;

static const float MAX_PRE_EXPOSED = 16384.0;
static const float TAU = 6.28318530718;

float3 body_direction(float2 uv)
{
    float4 p = mul(push.body_from_clip, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0));
    return normalize(p.xyz / p.w);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void skybody_main(uint3 id : SV_DispatchThreadID)
{
    uint2 size = uint2(push.size & 0xFFFFu, (push.size >> 16) & 0x7FFFu);
    bool through_air = (push.size >> 31) != 0u;
    if (id.x >= size.x || id.y >= size.y)
        return;
    float depth = g_sampled_images_r[NonUniformResourceIndex(push.depth)].Load(int3(id.xy, 0));
    if (depth > 0.0)
        return;
    float2 uv = (float2(id.xy) + 0.5) / float2(size);
    float3 d = body_direction(uv);
    float3 c = push.centre.xyz;
    float sin_radius = push.centre.w;
    // Distances in units of the body's distance: its centre at `c`, its radius `sin_radius`.
    float along = dot(d, c);
    float miss2 = max(1.0 - along * along, 0.0);
    float air = push.rim.a;
    float outer = sin_radius * (1.0 + air);
    if (along <= 0.0 || miss2 >= outer * outer)
        return;
    float3 light = 0.0;
    float r2 = sin_radius * sin_radius;
    float sun_term = push.sun.w;
    if (miss2 < r2)
    {
        // On the disc: the point the ray meets, its normal, its latitude and longitude.
        float t = along - sqrt(r2 - miss2);
        float3 n = (d * t - c) / sin_radius;
        float lat = asin(clamp(n.y, -1.0, 1.0));
        float lon = atan2(n.x, n.z);
        float2 map_uv = float2(lon / TAU + 0.5, 0.5 - lat / (0.5 * TAU));
        // The level whose texels, on the disc, match the pixel's angle.
        uint2 map_size;
        g_sampled_images[NonUniformResourceIndex(push.image)].GetDimensions(map_size.x, map_size.y);
        float pixel = acos(clamp(dot(d, body_direction(uv + float2(1.0 / float(size.x), 0.0))), -1.0, 1.0));
        float level = max(log2(pixel / (sin_radius * TAU / float(map_size.x))), 0.0);
        float3 albedo = sample_image_level(push.image, SAMPLER_LINEAR_REPEAT, map_uv, level).rgb;
        float lit = saturate(dot(n, push.sun.xyz));
        light = albedo * lit * sun_term;
        // Its air over the disc: more towards the limb, where the view crosses more of it, lit
        // where the sun reaches it (a little past the terminator).
        if (air > 0.0)
        {
            float edge = 1.0 - saturate(dot(n, -d));
            float sunlit = saturate(dot(n, push.sun.xyz) + 0.2);
            light += push.rim.rgb * sun_term * sunlit * (0.15 + 0.85 * edge * edge * edge * edge);
        }
    }
    else if (air > 0.0)
    {
        // Beyond the limb, the air alone, fading over its depth.
        float above = (sqrt(miss2) - sin_radius) / (sin_radius * air);
        float3 limb = normalize(d * along - c);
        float sunlit = saturate(dot(limb, push.sun.xyz) + 0.2);
        light = push.rim.rgb * sun_term * sunlit * exp(-4.0 * above) * (1.0 - above);
    }
    else
        return;
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(push.color)];
    float4 old = color[id.xy];
    float3 seen = through_air || miss2 >= r2 ? old.rgb + light : light;
    color[id.xy] = float4(min(seen, MAX_PRE_EXPOSED), old.a);
}
