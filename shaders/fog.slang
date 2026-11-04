// Fog lit by the lamps (D-099): authored fog volumes, boxes with a density, a height falloff, a
// colour and an absorption, laid into a froxel volume in front of the camera (D-032's layout,
// dust.slang), each froxel lit by the frame's lamps whose reach meets its column of the lamps'
// grid (#386, `NearLamps`; unshadowed, as the source's raster mode lights its fog) and by the sun
// through a shadow ray; integrated front to back and applied to the HDR image by each pixel's
// depth. Each lamp lights the fog by its share (`Lamp::fog`: an authored scene keeps some out of it),
// and a scene's lamps of the fog alone (lights that light no surface) come as a list of their own.
// Driven by crates/forge-render/src/fog.rs.
//
// The patent rules D-099 keeps: one trilinear read of the volume a pixel (no finer grid built
// from it), the lamps' lists per world column filled by their reach (never by resampling), their
// weights analytic, one colour a froxel (no directional representation blended over frames).
#include "meshlet.slang"

// Mirrors `GpuFogBox` in fog.rs: a fog volume.
struct FogBox
{
    float4 row0;     // the scene frame to the box's own (a unit cube centred on 0): three rows of
    float4 row1;     // an affine transform
    float4 row2;
    float4 medium;   // x: density (extinction, 1/m), y: height falloff (1/m), z: the share absorbed, w: its priority (the list's order)
    float4 shape;    // x: the box's height (m), y: its edges' blend (m), z: the share of the probes' light it scatters, w: 0
    float4 tint;     // rgb: the colour its scattering takes, a: 0
};

// Mirrors `GpuFog` in fog.rs.
struct Fog
{
    float4x4 inv_view_proj; // clip → camera-relative (the drawing camera, jitter included; D-004)
    float4 camera;          // xyz: the camera in the scene frame; w: the volume's far end (m)
    float4 sun;             // xyz: towards the sun; w: the sun's pre-exposed illuminance
    float4 sun_color;       // rgb: the sunlight's colour; a: the phase's anisotropy g
    float4 global;          // the scene's own fog everywhere: x density (1/m), y the height it thins above (the scene frame), z its falloff (1/m), w the share absorbed
    float4 scales;          // x: the sun's light into the fog, y: the lamps', z: the probes' in the boxes (times each one's share), w: the probes' in the fog everywhere
    Frame* frame;           // the resolve's frame: its lamps, their grid, the TLAS
    FogBox* boxes;
    Lamp* fog_lamps;        // lamps that light the fog alone (an authored scene's), near the camera
    ProbeField* probes;     // the probes whose light the fog scatters, or null (with `sky`)
    float4* sky;            // the sky's light (SH), the probes' lookup's fallback
    uint box_count;
    uint light;             // storage index of the lit froxels: rgb scattered light, a extinction
    uint light_sampled;
    uint volume;            // storage index of the integrated volume: rgb in-scattered, a transmittance
    uint volume_sampled;
    uint depth;             // sampled index of the frame's depth
    uint color;             // storage index of the HDR colour (read and written)
    uint width;
    uint height;
    uint fog_lamp_count;
};

struct FogPush
{
    Fog* fog;
};

[[vk::push_constant]]
FogPush fog_push;

static const uint FOG_FROXELS_X = 160;
static const uint FOG_FROXELS_Y = 90;
static const uint FOG_SLICES = 64;
// The least share of the probes' light a froxel scatters, per metre (extinction times its share),
// for it to be looked up.
static const float FOG_LEAST_AMBIENT = 2e-4;
// A lamp's light scattered towards the camera at `world` (lux times the phase, its fog share
// taken), fading to nothing at `reach`; a lamp's light is taken from no nearer than `half_size`.
float3 fog_lamp_light(Lamp lamp, float reach, float3 world, float3 direction, float half_size, float anisotropy)
{
    float3 d = lamp.position_radius.xyz - world;
    float dist2 = dot(d, d);
    if (dist2 >= reach * reach)
        return float3(0.0);
    float dist = sqrt(dist2);
    float3 to_lamp = d / max(dist, 1e-6);
    float window = saturate(1.0 - pow(dist / reach, 4.0));
    float least = max(lamp.position_radius.w, half_size);
    float3 lux = lamp.colour_reach.rgb / max(dist2, least * least) * window * window * lamp_beam(lamp, to_lamp);
    return (1.0 - asfloat(lamp.flags.y)) * lux * fog_phase(dot(-direction, -to_lamp), anisotropy);
}

// Distance (m) of slice boundary `k` (0..FOG_SLICES): quadratic, fine near the camera.
float fog_slice_distance(Fog* g, float k)
{
    float t = k / float(FOG_SLICES);
    return g[0].camera.w * t * t;
}

// The direction through the screen position `uv` (0..1, y down).
float3 fog_view_direction(Fog* g, float2 uv)
{
    float4 far = mul(g[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0));
    return normalize(far.xyz / far.w);
}

// Henyey–Greenstein: the share of light scattered at `cos_theta` from its direction, per sr.
float fog_phase(float cos_theta, float g)
{
    float g2 = g * g;
    return (1.0 - g2) / (4.0 * 3.14159265 * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5));
}

// The medium at `world`: its extinction (1/m), the share of it that scatters, and the colour
// its scattering takes. The boxes come in priority order (`fog.rs`, ties by their order in the
// scene): each covering box lays its medium over those before it by its fade, so where they
// overlap the highest priority's rules (the source's rule) and its edges blend into what lies
// under them.
void fog_medium(Fog* g, float3 world, out float extinction, out float albedo, out float3 tint, out float ambient)
{
    // The scene's own fog under the boxes: even up to its height, thinning above it.
    extinction = g[0].global.x * exp(-g[0].global.z * max(world.y - g[0].global.y, 0.0));
    albedo = 1.0 - saturate(g[0].global.w);
    tint = float3(1.0);
    // `ambient` is the extinction that scatters the probes' light (the extinction times its
    // share), blended as one: the share and the extinction blended apart and multiplied gave a
    // box's edges a shell 3 to 5 times too bright where the fog everywhere's share is high (2 at
    // night in the shipped city) and the box's low.
    ambient = extinction * g[0].scales.w;
    for (uint k = 0u; k < g[0].box_count; k++)
    {
        FogBox b = g[0].boxes[k];
        float3 p = float3(dot(b.row0.xyz, world) + b.row0.w, dot(b.row1.xyz, world) + b.row1.w, dot(b.row2.xyz, world) + b.row2.w);
        float3 a = abs(p);
        if (any(a > 0.5))
            continue;
        // Its size along each of its axes (a row's length is one over it), so its height above
        // its floor and its nearest face in metres.
        float3 size = 1.0 / max(float3(length(b.row0.xyz), length(b.row1.xyz), length(b.row2.xyz)), 1e-6);
        float above = (p.y + 0.5) * size.y;
        float3 to_faces = (0.5 - a) * size;
        float inside = min(to_faces.x, min(to_faces.y, to_faces.z));
        float fade = b.shape.y > 0.0 ? saturate(inside / b.shape.y) : 1.0;
        float box = b.medium.x * exp(-b.medium.y * max(above, 0.0));
        extinction = lerp(extinction, box, fade);
        albedo = lerp(albedo, 1.0 - saturate(b.medium.z), fade);
        tint = lerp(tint, b.tint.rgb, fade);
        ambient = lerp(ambient, box * b.shape.z * g[0].scales.z, fade);
    }
}

// A thread per froxel: the medium there and the light it scatters towards the camera, the
// lamps' (their column's, unshadowed) and the sun's (through a shadow ray), at the froxel's
// middle. Jittered within the froxel each frame (as D-032's dust is), the volume shimmered with
// nothing to settle it (a still camera's moving pixels 1.50 → 2.81 % in the private district's
// bathroom), and a sample landing beside a bulb lit its whole froxel white.
[shader("compute")]
[numthreads(8, 8, 1)]
void fog_light_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= FOG_FROXELS_X || id.y >= FOG_FROXELS_Y || id.z >= FOG_SLICES)
        return;
    Fog* g = fog_push.fog;
    Frame* f = g[0].frame;
    float2 uv = (float2(id.xy) + 0.5) / float2(FOG_FROXELS_X, FOG_FROXELS_Y);
    float3 direction = fog_view_direction(g, uv);
    float near = fog_slice_distance(g, float(id.z));
    float far = fog_slice_distance(g, float(id.z) + 1.0);
    float t = 0.5 * (near + far);
    float3 world = g[0].camera.xyz + direction * t;
    // Half the froxel's size, across and in depth: a lamp's light is taken from no nearer, so a
    // bulb inside a froxel lights it as its whole volume's mean would, not as its middle beside
    // the bulb would.
    float across = length(fog_view_direction(g, uv + float2(1.0 / float(FOG_FROXELS_X), 0.0)) - direction) * t;
    float half_size = 0.5 * max(across, far - near);
    float extinction;
    float albedo;
    float3 tint;
    float ambient_extinction;
    fog_medium(g, world, extinction, albedo, tint, ambient_extinction);
    float3 scattered = float3(0.0);
    if (extinction > 0.0)
    {
        float anisotropy = g[0].sun_color.a;
        // The lamps whose reach meets the froxel's column (every lamp beyond the grid), their
        // light as the light lists weigh it at a surface facing each (lux).
        float3 lamps = float3(0.0);
        if (f[0].lamp_count > 0u && f[0].lamps != nullptr)
        {
            LampRuns runs = lamp_runs(f, REGION_NONE, REGION_NONE, world, true);
            for (uint r = 0u; r < 3u; r++)
            {
                for (uint e = runs.run[r].x; e < runs.run[r].y; e++)
                {
                    Lamp lamp = f[0].lamps[lamp_at(f, runs, e)];
                    lamps += fog_lamp_light(lamp, lamp.colour_reach.w, world, direction, half_size, anisotropy);
                }
            }
        }
        // The lamps of the fog alone (an authored scene's lights that light no surface), to their
        // own reach.
        for (uint k = 0u; k < g[0].fog_lamp_count; k++)
        {
            Lamp lamp = g[0].fog_lamps[k];
            lamps += fog_lamp_light(lamp, lamp.colour_reach.w, world, direction, half_size, anisotropy);
        }
        // The sun through a shadow ray.
        float3 sun = normalize(g[0].sun.xyz);
        float visible = 0.0;
        if (g[0].sun.w > 0.0 && sun.y > -0.1)
        {
            RayDesc ray;
            ray.Origin = world;
            ray.Direction = sun;
            ray.TMin = 0.0;
            ray.TMax = 1e5;
            visible = f[0].tlas != 0 && ray_blocked(f, ray) ? 0.0 : 1.0;
        }
        float3 sunlight = g[0].sun.w * g[0].sun_color.rgb * fog_phase(dot(-direction, -sun), anisotropy) * visible;
        // The probes' light (D-099's item 3): the sky's and the scene's bounce, glowing surfaces'
        // included, as a white surface facing up and one facing down take it, in the region the
        // froxel stands in, the mean radiance taken as (E_up + E_down) / 2π, scattered every way
        // alike: exact for light even over each half of the sky; light from straight above counts
        // twice, light from the side (a sunlit wall's) less than it should. One lookup in a rolled loop: each
        // inlined copy grows the pass (#348). Skipped where the froxel's extinction times its share is
        // under FOG_LEAST_AMBIENT a metre: over the volume's 64 m, under 1.3 % of that radiance (an
        // authored city's fog everywhere by day, a twentieth of 0.003 a metre, which cost 0.45 ms).
        float3 ambient = float3(0.0);
        // Without probes, the open sky's light.
        if (ambient_extinction > FOG_LEAST_AMBIENT && g[0].sky != nullptr)
        {
            [loop]
            for (uint k = 0u; k < 2u; k++)
            {
                float3 n = float3(0.0, k == 0u ? 1.0 : -1.0, 0.0);
                float3 open = sh_irradiance(g[0].sky, n);
                ambient += g[0].probes != nullptr ? probe_irradiance_hit(g[0].probes, world, n, -direction, open, false) : open;
            }
            ambient *= ambient_extinction * f[0].sun_illuminance * f[0].exposure / 6.28318531;
        }
        scattered = albedo * tint * (extinction * (lamps * (f[0].exposure * g[0].scales.y) + sunlight * g[0].scales.x) + ambient);
    }
    g_storage_images_rgba16f[NonUniformResourceIndex(g[0].light)][uint2(id.z * FOG_FROXELS_X + id.x, id.y)] = float4(scattered, extinction);
}

// A thread per froxel column: the light gathered and the transmittance from the camera to each
// slice's far end (dust.slang's).
[shader("compute")]
[numthreads(8, 8, 1)]
void fog_integrate_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= FOG_FROXELS_X || id.y >= FOG_FROXELS_Y)
        return;
    Fog* g = fog_push.fog;
    RWTexture2D<float4> light = g_storage_images_rgba16f[NonUniformResourceIndex(g[0].light)];
    RWTexture2D<float4> volume = g_storage_images_rgba16f[NonUniformResourceIndex(g[0].volume)];
    float3 gathered = 0.0;
    float transmittance = 1.0;
    for (uint k = 0; k < FOG_SLICES; k++)
    {
        uint2 texel = uint2(k * FOG_FROXELS_X + id.x, id.y);
        float4 l = light[texel];
        float dt = fog_slice_distance(g, float(k) + 1.0) - fog_slice_distance(g, float(k));
        float segment = exp(-l.a * dt);
        gathered += transmittance * (l.a > 1e-7 ? l.rgb * (1.0 - segment) / l.a : l.rgb * dt);
        transmittance *= segment;
        volume[texel] = float4(gathered, transmittance);
    }
}

// The volume at screen `uv` and `distance` m: bilinear within the two nearest slices, linear
// between them: one trilinear read.
float4 fog_at(Fog* g, float2 uv, float distance)
{
    float w = sqrt(saturate(distance / g[0].camera.w)) * float(FOG_SLICES) - 1.0;
    float2 inside = clamp(uv * float2(FOG_FROXELS_X, FOG_FROXELS_Y), 0.5, float2(FOG_FROXELS_X, FOG_FROXELS_Y) - 0.5);
    Texture2D<float4> atlas = g_sampled_images[NonUniformResourceIndex(g[0].volume_sampled)];
    SamplerState linear = g_samplers[SAMPLER_LINEAR_CLAMP];
    float2 atlas_size = float2(FOG_FROXELS_X * FOG_SLICES, FOG_FROXELS_Y);
    if (w <= 0.0)
    {
        float4 first = atlas.SampleLevel(linear, inside / atlas_size, 0.0);
        float s = saturate(w + 1.0);
        return float4(first.rgb * s, lerp(1.0, first.a, s));
    }
    float k = min(floor(w), float(FOG_SLICES - 2));
    float s = saturate(w - k);
    float4 a = atlas.SampleLevel(linear, (inside + float2(k * FOG_FROXELS_X, 0.0)) / atlas_size, 0.0);
    float4 b = atlas.SampleLevel(linear, (inside + float2((k + 1.0) * FOG_FROXELS_X, 0.0)) / atlas_size, 0.0);
    return lerp(a, b, s);
}

// A thread per pixel: the colour dimmed by the fog in front of it and brightened by what the fog
// scatters; empty pixels (the sky) see the whole volume.
[shader("compute")]
[numthreads(8, 8, 1)]
void fog_apply_main(uint3 id : SV_DispatchThreadID)
{
    Fog* g = fog_push.fog;
    if (id.x >= g[0].width || id.y >= g[0].height)
        return;
    float2 uv = (float2(id.xy) + 0.5) / float2(g[0].width, g[0].height);
    float depth = g_sampled_images_r[NonUniformResourceIndex(g[0].depth)].Load(int3(id.xy, 0));
    float distance = g[0].camera.w;
    if (depth > 0.0)
    {
        float4 relative = mul(g[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0));
        distance = length(relative.xyz / relative.w);
    }
    float4 fog = fog_at(g, uv, distance);
    RWTexture2D<float4> color = g_storage_images_rgba16f[NonUniformResourceIndex(g[0].color)];
    float4 c = color[id.xy];
    color[id.xy] = float4(c.rgb * fog.a + fog.rgb, c.a);
}
