// The light the city leaves a surface (D-108 step 6), at half resolution before the resolve: with
// no probes, a surface no room covers took the whole open sky, as if it stood alone in a field. Per
// pixel, a few directions round its normal, cosine-weighted, are marched against the heights seen
// from above (`heights.slang`). An open direction keeps the sky's light, which the sky's irradiance
// already holds; a blocked one brings what it met, lit once, in the sky's place: its sun from the
// rings, the open sky's light on its face, a guessed reflectance. Written as the share of the open
// sky's irradiance the surface takes (rgb, 1: all of it), and in alpha the share of the sky its
// mirror direction sees. One spatial pass and one temporal one, as the near light's.
import bindless;
import heights;
import shadow_rings;
import sh;
import skyview;

// Mirrors `GpuGather` in crates/forge-render/src/height_light.rs.
struct Gather
{
    float4x4 world_from_clip;       // this view's clip space to metres relative to its camera
    float4x4 previous_from_current; // this frame's clip space to the last's, unjittered
    float4 clip_w;                  // a relative point's view depth: dot(xyz, p) + w
    float4 offset;                  // xyz: from this view's camera to the one the heights and the rings were drawn for
    float4 sun_dir;                 // xyz: towards the key light; w: how far a direction is marched, metres
    float4 sun_color;               // rgb: the key light's; w: what a blocked direction's face reflects
    HeightSet* heights;
    RingSet* rings;                 // or null: the sun unshadowed
    float4* sky;                    // the sky's irradiance and its table (`sh.slang`, `skyview.slang`)
    uint directions;                // per pixel
    uint steps;                     // per direction
    uint frame;                     // turns the noise
    uint reset;                     // 1: no history to read
    uint spacing;                   // half-resolution pixels between two marched ones, each way: 1 all, 2 one in four
    uint2 first;                    // the first marched pixel: it goes round with the frames
    uint pad;
};

struct Push
{
    Gather* gather;
    uint depth_image;  // sampled: the view's depth
    uint z_image;      // sampled: this frame's view depth at half resolution
    uint last_z_image; // sampled: the last frame's
    uint input_image;  // sampled: the filter's input, or the temporal pass's
    uint last_image;   // sampled: the last frame's light
    uint output_image; // storage
    uint width;        // of the half-resolution images
    uint height;
    uint last_count_image; // sampled: how many frames the last frame's light holds (r)
    uint count_image;      // storage (rg16f): this frame's
};

[[vk::push_constant]]
Push push;

static const float PI = 3.14159265;
static const float SKY_Z = 1e6;
// The most frames a pixel's light is the average of: what stands still changes slowly, and a
// short average flickered (frames three apart differed on 12 % of a still view's pixels at ten).
static const float HISTORY_FRAMES = 64.0;
// The most a surface takes, in open skies: what a sunlit wall sends a shaded one.
static const float MOST_SHARE = 8.0;

float noise_at(uint2 p, uint frame, float2 magic)
{
    float2 q = float2(p) + 5.588238 * float(frame % 64u);
    return frac(52.9829189 * frac(dot(q, magic)));
}

// The point of view depth `z` the half-resolution pixel centre `uv` shows, relative to the camera.
float3 position_at(Gather* g, float2 uv, float z)
{
    float4 q = mul(g[0].world_from_clip, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0));
    float3 on_ray = q.xyz / q.w;
    return on_ray * (z / (dot(g[0].clip_w.xyz, on_ray) + g[0].clip_w.w));
}

// Half resolution: the nearest of the four pixels, as its view depth.
[shader("compute")]
[numthreads(8, 8, 1)]
void z_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    Gather* g = push.gather;
    Texture2D<float> depth = g_sampled_images_r[NonUniformResourceIndex(push.depth_image)];
    float nearest = 0.0;
    for (uint y = 0u; y < 2u; y++)
    {
        for (uint x = 0u; x < 2u; x++)
            nearest = max(nearest, depth.Load(int3(int2(id.xy * 2u) + int2(x, y), 0)));
    }
    float z = SKY_Z;
    if (nearest > 0.0)
    {
        float2 uv = (float2(id.xy) + float2(0.5)) / float2(push.width, push.height);
        float4 q = mul(g[0].world_from_clip, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, nearest, 1.0));
        z = dot(g[0].clip_w.xyz, q.xyz / q.w) + g[0].clip_w.w;
    }
    g_storage_images_r32f[NonUniformResourceIndex(push.output_image)][id.xy] = z;
}

// The sky's light towards `d` as a surface takes it, in the resolve's units: what its irradiance
// holds of that direction.
float3 sky_towards(float4* sky, float3 d)
{
    float4 up_table = sky[SH_COEFFICIENTS];
    return sky_light_on_surfaces(sky, sky_view_luminance(uint(up_table.w + 0.5), up_table.xyz, sky[SH_COEFFICIENTS + 1].xyz, d) * PI);
}

// What a blocked direction met sends back, lit once: its sun from the rings at that point, the
// open sky's light on its face, the reflectance a guess.
float3 hit_light(Gather* g, HeightHit hit, float turn)
{
    float ndl = saturate(dot(hit.n, g[0].sun_dir.xyz));
    float lit = ndl > 0.0 ? ring_sun_taps(g[0].rings, hit.at, turn, 1u) : 0.0;
    return g[0].sun_color.w * (g[0].sun_color.rgb * (ndl * lit) + sh_irradiance(g[0].sky, hit.n));
}

float luminance(float3 c)
{
    return dot(c, float3(0.2126, 0.7152, 0.0722));
}

// One thread a marched pixel: every half-resolution pixel, or with a spacing of 2 one in four,
// which goes round with the frames (`Gather::first`); the filter spreads them and the history
// holds the rest.
[shader("compute")]
[numthreads(8, 8, 1)]
void trace_main(uint3 id : SV_DispatchThreadID)
{
    Gather* g = push.gather;
    uint2 pixel = id.xy * g[0].spacing + g[0].first;
    if (pixel.x >= push.width || pixel.y >= push.height)
        return;
    RWTexture2D<float4> result = g_storage_images_rgba16f[NonUniformResourceIndex(push.output_image)];
    Texture2D<float> zs = g_sampled_images_r[NonUniformResourceIndex(push.z_image)];
    int2 here = int2(pixel);
    int2 last = int2(push.width, push.height) - int2(1);
    float z = zs.Load(int3(here, 0));
    // The sky, and what no level holds: the open sky's light, as before.
    result[id.xy] = float4(1.0);
    if (z >= SKY_Z)
        return;
    float2 size = float2(push.width, push.height);
    float2 uv = (float2(pixel) + float2(0.5)) / size;
    float3 p = position_at(g, uv, z);
    // The normal from the depth, across the nearer neighbour each way.
    float3 left = position_at(g, uv - float2(1.0 / size.x, 0.0), zs.Load(int3(clamp(here - int2(1, 0), int2(0), last), 0))) - p;
    float3 right = position_at(g, uv + float2(1.0 / size.x, 0.0), zs.Load(int3(clamp(here + int2(1, 0), int2(0), last), 0))) - p;
    float3 up = position_at(g, uv - float2(0.0, 1.0 / size.y), zs.Load(int3(clamp(here - int2(0, 1), int2(0), last), 0))) - p;
    float3 down = position_at(g, uv + float2(0.0, 1.0 / size.y), zs.Load(int3(clamp(here + int2(0, 1), int2(0), last), 0))) - p;
    float3 across = dot(left, left) < dot(right, right) ? -left : right;
    float3 along = dot(up, up) < dot(down, down) ? -up : down;
    float3 n = cross(across, along);
    if (dot(n, n) < 1e-20)
        return;
    n = normalize(n);
    float3 v = normalize(-p);
    if (dot(n, v) < 0.0)
        n = -n;
    HeightSet* set = g[0].heights;
    float3 origin = p + g[0].offset.xyz;
    float top;
    float texel;
    if (!height_over(set, origin, 0u, top, texel))
        return;
    // Clear of its own column's texels.
    origin += n * (1.5 * texel);
    float reach = g[0].sun_dir.w;
    float3 open = sh_irradiance(g[0].sky, n);
    float3 tangent = normalize(abs(n.y) < 0.99 ? cross(float3(0.0, 1.0, 0.0), n) : cross(float3(1.0, 0.0, 0.0), n));
    float3 bitangent = cross(n, tangent);
    // With one pixel in four marched a frame, the noise changes when the round is over and
    // repeats after four rounds: sixteen frames, which the history's average spans four times,
    // so a still view's light stands still.
    uint cycle = g[0].spacing > 1u ? (g[0].frame >> 2u) & 3u : g[0].frame;
    float turn = noise_at(pixel, cycle, float2(0.06711056, 0.00583715));
    float ring = noise_at(pixel, cycle, float2(0.00583715, 0.06711056));
    float start = noise_at(pixel, cycle, float2(0.0331, 0.0719));
    uint count = max(g[0].directions, 1u);
    float3 change = float3(0.0);
    for (uint i = 0u; i < count; i++)
    {
        // Cosine-weighted round the normal: equal shares of the disc under the hemisphere.
        float u = (float(i) + ring) / float(count);
        float phi = 2.0 * PI * frac(turn + float(i) * 0.61803399);
        float r = sqrt(u);
        float3 d = normalize(tangent * (r * cos(phi)) + bitangent * (r * sin(phi)) + n * sqrt(max(1.0 - u, 0.0)));
        HeightHit hit = height_march(set, origin, d, reach, g[0].steps, frac(start + float(i) * 0.75487767));
        if (hit.blocked)
            change += hit_light(g, hit, turn) - sky_towards(g[0].sky, d);
    }
    float3 share = (open + change / float(count)) / max(open, float3(1e-5));
    // The mirror direction, once: the share of the sky there that what it meets sends, at most 1.
    float mirror = 1.0;
    float3 mirrored = reflect(-v, n);
    if (dot(mirrored, n) > 1e-3)
    {
        HeightHit hit = height_march(set, origin, mirrored, reach, g[0].steps, start);
        if (hit.blocked)
            mirror = saturate(luminance(hit_light(g, hit, turn)) / max(luminance(sky_towards(g[0].sky, mirrored)), 1e-5));
    }
    float4 written = float4(clamp(share, float3(-MOST_SHARE), float3(MOST_SHARE)), mirror);
    if (any(isnan(written)) || any(isinf(written)))
        written = float4(1.0);
    result[id.xy] = written;
}

// One spatial pass of a fixed radius: the three by three marched pixels round each half-resolution
// pixel, by how near each is in depth and in the picture. Where none of them is of its surface,
// a negative alpha: the history alone then stands, or the open sky.
[shader("compute")]
[numthreads(8, 8, 1)]
void filter_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    Gather* g = push.gather;
    Texture2D<float4> input_image = g_sampled_images[NonUniformResourceIndex(push.input_image)];
    Texture2D<float> zs = g_sampled_images_r[NonUniformResourceIndex(push.z_image)];
    int spacing = int(g[0].spacing);
    int2 first = int2(g[0].first);
    int2 size = int2(push.width, push.height);
    float z = zs.Load(int3(id.xy, 0));
    // Where the pixel falls among the marched ones, and the marched pixel nearest to it.
    float2 among = float2(int2(id.xy) - first) / float(spacing);
    int2 nearest = int2(round(among));
    float4 sum = float4(0.0);
    float weight = 0.0;
    for (int y = -1; y <= 1; y++)
    {
        for (int x = -1; x <= 1; x++)
        {
            int2 marched = nearest + int2(x, y);
            int2 at = marched * spacing + first;
            if (any(marched < int2(0)) || any(at >= size))
                continue;
            if (abs(zs.Load(int3(at, 0)) - z) <= 0.02 * z)
            {
                float2 apart = float2(marched) - among;
                float w = exp(-0.5 * dot(apart, apart));
                sum += w * input_image.Load(int3(marched, 0));
                weight += w;
            }
        }
    }
    g_storage_images_rgba16f[NonUniformResourceIndex(push.output_image)][id.xy] = weight > 0.0 ? sum / weight : float4(1.0, 1.0, 1.0, -1.0);
}

// The one temporal pass: each pixel's share averaged with what its place held over the frames
// before, where its depth says it is the same surface; beside it, how many frames that is.
[shader("compute")]
[numthreads(8, 8, 1)]
void temporal_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= push.width || id.y >= push.height)
        return;
    Gather* g = push.gather;
    float4 now = g_sampled_images[NonUniformResourceIndex(push.input_image)].Load(int3(id.xy, 0));
    float z = g_sampled_images_r[NonUniformResourceIndex(push.z_image)].Load(int3(id.xy, 0));
    RWTexture2D<float4> result = g_storage_images_rgba16f[NonUniformResourceIndex(push.output_image)];
    RWTexture2D<float2> counts = g_storage_images_rg16f[NonUniformResourceIndex(push.count_image)];
    // No marched pixel of its surface round it this frame: what its place held, or the open sky.
    bool none = now.a < 0.0;
    if (none)
        now = float4(1.0);
    result[id.xy] = now;
    counts[id.xy] = float2(none ? 0.0 : 1.0, 0.0);
    if (g[0].reset != 0u || z >= SKY_Z)
        return;
    float2 uv = (float2(id.xy) + float2(0.5)) / float2(push.width, push.height);
    // Where the same point stood in the last frame's picture: through this frame's clip space,
    // whose depth is the near plane's view depth over the point's.
    float4 q = mul(g[0].world_from_clip, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0));
    float at_near = dot(g[0].clip_w.xyz, q.xyz / q.w) + g[0].clip_w.w;
    float4 before = mul(g[0].previous_from_current, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, at_near / z, 1.0));
    float2 last_uv = before.w > 0.0 ? float2(0.5, -0.5) * before.xy / before.w + float2(0.5) : float2(-1.0);
    if (any(last_uv < float2(0.0)) || any(last_uv >= float2(1.0)))
        return;
    float kept_z = g_sampled_images_r[NonUniformResourceIndex(push.last_z_image)].SampleLevel(g_samplers[SAMPLER_NEAREST_CLAMP], last_uv, 0.0);
    float4 kept = g_sampled_images[NonUniformResourceIndex(push.last_image)].SampleLevel(g_samplers[SAMPLER_LINEAR_CLAMP], last_uv, 0.0);
    float frames = g_sampled_images[NonUniformResourceIndex(push.last_count_image)].SampleLevel(g_samplers[SAMPLER_NEAREST_CLAMP], last_uv, 0.0).r;
    // Another surface, nothing kept, or a value that is no number (kept, it would stay for good).
    if (!(abs(kept_z - z) <= 0.05 * z) || !(frames >= 1.0) || any(isnan(kept)) || any(isinf(kept)))
        return;
    if (none)
    {
        result[id.xy] = kept;
        counts[id.xy] = float2(frames, 0.0);
        return;
    }
    float held = min(frames + 1.0, HISTORY_FRAMES);
    result[id.xy] = lerp(kept, now, 1.0 / held);
    counts[id.xy] = float2(held, 0.0);
}
