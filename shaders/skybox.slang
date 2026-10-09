// The sky box (#220; crates/forge-render/src/skybox.rs): an equirectangular map of the sky added
// into the resolved HDR image wherever the depth is clear. The map's right ascension 0 is at its
// middle and increases to the left; its first row is declination +90°.
import bindless;

struct Push
{
    float4x4 sky_from_clip; // clip space to the sky's axes (x: RA 0, y: RA 90°, z: the north pole)
    float4 sun;             // xyz: towards the sun in the sky's axes; w: cos of its disc's radius
    float4 planet;          // xyz: from the camera to the body's centre, sky axes, km; w: its radius (0: none)
    float top;              // its atmosphere's top, km (0: none: the pass draws the sun's disc)
    float luminance;        // pre-exposed luminance of a map value of 1
    float sun_luminance;    // pre-exposed luminance of the sun's disc
    uint image;             // sampled index of the map
    uint color;             // storage index of the HDR image (read and written)
    uint depth;             // sampled index of the depth
    uint size;              // width | height << 16
    uint pad;
};

[[vk::push_constant]]
Push push;

// Pre-exposed values above this are white on any curve; fp16 overflows at 65504.
static const float MAX_PRE_EXPOSED = 16384.0;
// The air's lowest kilometres, through which the stars fade out.
static const float FADE_KM = 40.0;
static const float TAU = 6.28318530718;

// The direction, in the sky's axes, through the pixel at `uv`.
float3 sky_direction(float2 uv)
{
    float4 p = mul(push.sky_from_clip, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0));
    return normalize(p.xyz / p.w);
}

// The map's coordinates of `d` (sky axes).
float2 map_uv(float3 d)
{
    float ra = atan2(d.y, d.x);
    float dec = asin(clamp(d.z, -1.0, 1.0));
    return float2(0.5 - ra / TAU, 0.5 - dec / (0.5 * TAU));
}

[shader("compute")]
[numthreads(8, 8, 1)]
void skybox_main(uint3 id : SV_DispatchThreadID)
{
    uint2 size = uint2(push.size & 0xFFFFu, push.size >> 16);
    if (id.x >= size.x || id.y >= size.y)
        return;
    float depth = g_sampled_images_r[NonUniformResourceIndex(push.depth)].Load(int3(id.xy, 0));
    if (depth > 0.0)
        return;
    float2 uv = (float2(id.xy) + 0.5) / float2(size);
    float3 d = sky_direction(uv);
    // Through the body's air: the stars fade with the lowest altitude the ray reaches; a ray that
    // meets the ground takes none.
    float through = 1.0;
    if (push.planet.w > 0.0)
    {
        float3 centre = push.planet.xyz;
        float along = dot(centre, d);
        float closest = along > 0.0 ? length(centre - d * along) : length(centre);
        float altitude = closest - push.planet.w;
        if (altitude < 0.0)
            return;
        if (push.top > 0.0)
            through = smoothstep(0.0, FADE_KM, altitude);
    }
    // The level whose texels match the pixel's angle, from the next pixel's direction.
    uint2 map_size;
    g_sampled_images[NonUniformResourceIndex(push.image)].GetDimensions(map_size.x, map_size.y);
    float pixel = acos(clamp(dot(d, sky_direction(uv + float2(1.0 / float(size.x), 0.0))), -1.0, 1.0));
    float level = max(log2(pixel / (TAU / float(map_size.x))), 0.0);
    float3 light = sample_image_level(push.image, SAMPLER_LINEAR_REPEAT, map_uv(d), level).rgb * push.luminance;
    // With no air, the sun's disc is this pass's (the air's sky draws its own).
    if (push.top <= 0.0 && dot(d, push.sun.xyz) > push.sun.w)
        light += push.sun_luminance;
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(push.color)];
    float4 c = color[id.xy];
    color[id.xy] = float4(min(c.rgb + light * through, MAX_PRE_EXPOSED), c.a);
}
