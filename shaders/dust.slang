// Sunlit dust between the rocks (issue #58): a froxel volume in front of the camera, lit by the
// sun through a shadow ray per froxel against the scene's TLAS (the shafts), integrated front
// to back and applied to the HDR image by each pixel's depth. The layout follows the sky's
// aerial perspective (sky.slang): SLICES depth slices of FROXELS_X × FROXELS_Y froxels, side by
// side in 2-D atlases. Driven by crates/forge-render/src/dust.rs.
import bindless;
import noise;

// Must match GpuDust in dust.rs.
struct Dust
{
    float4x4 inv_view_proj; // clip → world (the drawing camera, jitter included)
    float4 camera;          // xyz: the camera, world metres
    float4 sun;             // xyz: towards the sun; w: the sun's pre-exposed illuminance
    float4 sun_color;       // rgb: the sunlight's colour
    float4 params;          // x: extinction (1/m), y: the volume's far end (m), z: anisotropy g, w: noise scale (1/m)
    float4 fill;            // rgb: light scattered from the surroundings, pre-exposed units per unit extinction
    uint64_t tlas;          // 0: no shafts
    uint light;             // storage index of the lit froxels: rgb scattered light, a extinction
    uint light_sampled;
    uint volume;            // storage index of the integrated volume: rgb in-scattered, a transmittance
    uint volume_sampled;
    uint depth;             // sampled index of the frame's depth
    uint color;             // storage index of the HDR colour (read and written)
    uint width;
    uint height;
    uint frame;             // the noise's frame: TAA's, modulo its jitter period
    uint pad;
};

struct Push
{
    Dust* dust;
};

[[vk::push_constant]]
Push push;

static const uint FROXELS_X = 160;
static const uint FROXELS_Y = 90;
static const uint SLICES = 64;
static const float PI = 3.14159265;

// Distance (m) of slice boundary `k` (0..SLICES): quadratic, fine near the camera.
float slice_distance(Dust* d, float k)
{
    float t = k / float(SLICES);
    return d[0].params.y * t * t;
}

// The world-space direction through the screen position `uv` (0..1, y down).
float3 view_direction(Dust* d, float2 uv)
{
    float4 far = mul(d[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0));
    return normalize(far.xyz / far.w - d[0].camera.xyz);
}

// Smooth value noise over integer lattice cells, in [0, 1).
float hash3(int3 c)
{
    uint h = uint(c.x) * 73856093u ^ uint(c.y) * 19349663u ^ uint(c.z) * 83492791u;
    h = (h ^ (h >> 13)) * 1274126177u;
    return float(h & 0xFFFFFFu) / 16777216.0;
}

float value_noise(float3 p)
{
    float3 i = floor(p);
    float3 t = p - i;
    t = t * t * (3.0 - 2.0 * t);
    int3 c = int3(i);
    float n000 = hash3(c), n100 = hash3(c + int3(1, 0, 0)), n010 = hash3(c + int3(0, 1, 0)), n110 = hash3(c + int3(1, 1, 0));
    float n001 = hash3(c + int3(0, 0, 1)), n101 = hash3(c + int3(1, 0, 1)), n011 = hash3(c + int3(0, 1, 1)), n111 = hash3(c + int3(1, 1, 1));
    float x00 = lerp(n000, n100, t.x), x10 = lerp(n010, n110, t.x), x01 = lerp(n001, n101, t.x), x11 = lerp(n011, n111, t.x);
    return lerp(lerp(x00, x10, t.y), lerp(x01, x11, t.y), t.z);
}

// Henyey–Greenstein: the share of light scattered at `cos_theta` from its direction, per sr.
float henyey_greenstein(float cos_theta, float g)
{
    float g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5));
}

// A thread per froxel: the dust's extinction there (varying in world space, so it stays put as
// the camera moves) and the sunlight it scatters towards the camera, through a shadow ray.
// The sample point jitters within the froxel over TAA's cycle.
[shader("compute")]
[numthreads(8, 8, 1)]
void light_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= FROXELS_X || id.y >= FROXELS_Y || id.z >= SLICES)
        return;
    Dust* d = push.dust;
    float2 jitter = spatio_temporal_noise(id.xy + uint2(id.z * 37u, id.z * 11u), d[0].frame);
    float2 uv = (float2(id.xy) + 0.5) / float2(FROXELS_X, FROXELS_Y);
    float3 direction = view_direction(d, uv);
    float t = lerp(slice_distance(d, float(id.z)), slice_distance(d, float(id.z) + 1.0), jitter.x);
    float3 world = d[0].camera.xyz + direction * t;
    float shape = 0.6 * value_noise(world * d[0].params.w) + 0.4 * value_noise(world * d[0].params.w * 3.1 + 17.0);
    float extinction = d[0].params.x * saturate(shape * 1.6 - 0.25);
    float3 sun = d[0].sun.xyz;
    float visible = 1.0;
    if (d[0].tlas != 0 && extinction > 0.0)
    {
        RayDesc ray;
        ray.Origin = world;
        ray.Direction = sun;
        ray.TMin = 0.0;
        ray.TMax = 1e5;
        RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER | RAY_FLAG_FORCE_OPAQUE> q;
        q.TraceRayInline(RaytracingAccelerationStructure(d[0].tlas), RAY_FLAG_NONE, 0xFF, ray);
        q.Proceed();
        visible = q.CommittedStatus() == COMMITTED_TRIANGLE_HIT ? 0.0 : 1.0;
    }
    float phase = henyey_greenstein(dot(-direction, -sun), d[0].params.z);
    float3 scattered = extinction * (d[0].sun.w * d[0].sun_color.rgb * phase * visible + d[0].fill.rgb);
    g_storage_images_rgba16f[NonUniformResourceIndex(d[0].light)][uint2(id.z * FROXELS_X + id.x, id.y)] = float4(scattered, extinction);
}

// A thread per froxel column: the light gathered and the transmittance from the camera to each
// slice's far end.
[shader("compute")]
[numthreads(8, 8, 1)]
void integrate_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= FROXELS_X || id.y >= FROXELS_Y)
        return;
    Dust* d = push.dust;
    RWTexture2D<float4> light = g_storage_images_rgba16f[NonUniformResourceIndex(d[0].light)];
    RWTexture2D<float4> volume = g_storage_images_rgba16f[NonUniformResourceIndex(d[0].volume)];
    float3 gathered = 0.0;
    float transmittance = 1.0;
    for (uint k = 0; k < SLICES; k++)
    {
        uint2 texel = uint2(k * FROXELS_X + id.x, id.y);
        float4 l = light[texel];
        float dt = slice_distance(d, float(k) + 1.0) - slice_distance(d, float(k));
        float segment = exp(-l.a * dt);
        // Exact over the segment for constant light and extinction; the light itself when clear.
        gathered += transmittance * (l.a > 1e-7 ? l.rgb * (1.0 - segment) / l.a : l.rgb * dt);
        transmittance *= segment;
        volume[texel] = float4(gathered, transmittance);
    }
}

// The volume at screen `uv` and `distance` m: bilinear within the two nearest slices, linear
// between them (`aerial_at` in sky.slang).
float4 dust_at(Dust* d, float2 uv, float distance)
{
    float w = sqrt(saturate(distance / d[0].params.y)) * float(SLICES) - 1.0;
    float2 inside = clamp(uv * float2(FROXELS_X, FROXELS_Y), 0.5, float2(FROXELS_X, FROXELS_Y) - 0.5);
    Texture2D<float4> atlas = g_sampled_images[NonUniformResourceIndex(d[0].volume_sampled)];
    SamplerState linear = g_samplers[SAMPLER_LINEAR_CLAMP];
    float2 atlas_size = float2(FROXELS_X * SLICES, FROXELS_Y);
    if (w <= 0.0)
    {
        float4 first = atlas.SampleLevel(linear, inside / atlas_size, 0.0);
        float f = saturate(w + 1.0);
        return float4(first.rgb * f, lerp(1.0, first.a, f));
    }
    float k = min(floor(w), float(SLICES - 2));
    float f = saturate(w - k);
    float4 a = atlas.SampleLevel(linear, (inside + float2(k * FROXELS_X, 0.0)) / atlas_size, 0.0);
    float4 b = atlas.SampleLevel(linear, (inside + float2((k + 1.0) * FROXELS_X, 0.0)) / atlas_size, 0.0);
    return lerp(a, b, f);
}

// A thread per pixel: the colour dimmed by the dust in front of it and brightened by what the
// dust scatters; empty pixels (the sky) see the whole volume.
[shader("compute")]
[numthreads(8, 8, 1)]
void apply_main(uint3 id : SV_DispatchThreadID)
{
    Dust* d = push.dust;
    if (id.x >= d[0].width || id.y >= d[0].height)
        return;
    float2 uv = (float2(id.xy) + 0.5) / float2(d[0].width, d[0].height);
    float depth = g_sampled_images_r[NonUniformResourceIndex(d[0].depth)].Load(int3(id.xy, 0));
    float distance = d[0].params.y;
    if (depth > 0.0)
    {
        float4 world = mul(d[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
        distance = length(world.xyz / world.w - d[0].camera.xyz);
    }
    float4 dust = dust_at(d, uv, distance);
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(d[0].color)];
    float4 c = color[id.xy];
    color[id.xy] = float4(c.rgb * dust.a + dust.rgb, c.a);
}
