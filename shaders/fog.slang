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
    float4 sun_color;       // rgb: the sunlight's colour; a: the sun's phase anisotropy g
    float4 global;          // the scene's own fog everywhere: x density (1/m), y the height it thins above (the scene frame), z its falloff (1/m), w the share absorbed
    float4 scales;          // x: the sun's light into the fog, y: the lamps', z: the probes' in the boxes (times each one's share), w: the probes' in the fog everywhere
    float4 phase;           // x: the lamps' phase anisotropy g; y: the lamps tested by a shadow ray a froxel (`FogParams::lamp_shadows`); zw 0
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
    float4x4 prev_view_proj; // last frame's clip from camera-relative (its jitter included)
    float4 prev_camera;      // xyz: last frame's camera in the scene frame; w: the history's weight (0: none)
    float4 jitter;           // xyz: this frame's offset within the froxels (−0.5 to 0.5), 0 without a history
    float last_exposure;     // the exposure last frame's history was lit at (0: none)
    float exposure;          // this frame's (both the CPU's: their ratio stays exact)
    uint history_read;       // sampled index of last frame's history (0xFFFFFFFF: none)
    uint history_write;      // storage index of this frame's history
    uint parity;             // this frame's turn: its history's slot
    uint tiles;              // storage index of the froxel columns' farthest surfaces (fog_tiles_main)
    uint tiles_sampled;      // its sampled index (fog_bound_main)
    uint cull;               // 1: froxels behind every surface in view are not lit (fog_culled)
    uint bound;              // storage index of each column's last slice lit (fog_bound_main)
    uint bound_sampled;      // its sampled index (fog_culled)
    uint pad0;
    uint pad1;
    // The distance fog (D-101, `DistanceFog` in fog.rs; distance_phase.y 0: none).
    float4 distance_range;   // x: its start (m), y: where its colour is the far one (m), z: the height its layers are given at (the scene frame), w: the elevation its cover of the sky ends at (radians; 0: the layers' own)
    float4 distance_layers;  // x, y: the first layer's extinction (1/m) at that height and its falloff (1/m); z, w: the second's
    float4 distance_near;    // rgb: its colour at its start; a: the sun's light into it
    float4 distance_far;     // rgb: its colour from `far` on; a: the sky's light into it, times the sun's pre-exposed illuminance over 2π
    float4 distance_phase;   // x: the sun's phase anisotropy g; y: 1 when on; zw 0
};

// A froxel's own offset added to the frame's (pcg3d, Jarzynski and Olano 2020), so that
// neighbouring froxels sample different places in the same frame: 0 to 1 each.
float3 fog_froxel_offset(uint3 v)
{
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> 16u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return float3(v & 0xFFFFFFu) / 16777216.0;
}

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
// lamps' (their column's, unshadowed) and the sun's (through a shadow ray). With a history
// (`FogParams::history`, D-099's amendment of 2026-11-05) at a point jittered within the froxel
// each frame, blended into last frame's volume where the froxel's middle stood then; where last
// frame's volume did not reach (a first frame, the screen's edge as the camera turns), at its
// middle. Jittered without a history (as D-032's dust is), the volume shimmered with nothing to
// settle it (a still camera's moving pixels 1.50 → 2.81 % in the private district's bathroom).
// The lamps' shadows (D-099's option, `FogParams::lamp_shadows`): a froxel's strongest lamps, up
// to FOG_SHADOW_RAYS, that give FOG_SHADOW_SHARE or more of its lamps' light are each tested by a
// shadow ray, the rest left unshadowed (after Ward 1991's adaptive shadow testing, the strongest
// lights tested: deterministic, no random draw; the weights the lamps' unshadowed light, never
// learned from rays, D-099's patent rules). Off (0), the lamps' sum is the unshadowed one.
static const uint FOG_SHADOW_RAYS = 4u;
static const float FOG_SHADOW_SHARE = 0.1;
// How far before a lamp's sphere its shadow ray from the fog ends, metres: its own fitting (the
// glass under a pendant's bulb, a lantern's frame) stands within it. Ended 5 cm short, the
// fitting met every ray from under a pendant lamp and put out its cone; within this distance of a
// lamp, a wall lets its light into the fog behind. A flat piece's ray ends 2 cm short of its
// middle, as `lamp_ray`'s: what covers the piece (a curtain, a window's recess) is in its way
// (#370).
static const float FOG_SHADOW_CLEAR = 0.3;

// Last frame's volume (D-099's amendment, the history) at the point that stands at screen `uv`
// and `t` m from the camera now: reprojected through last frame's camera, its slice found by
// inverting the light's sample distances (slice k's at the middle of its near and far ends:
// `far / (2 S²) · (2k² + 2k + 1)`), bilinear within two slices and linear between them, brought
// to this frame's exposure. False where last frame's volume did not reach the point, or there is
// none, or its value is not finite (a fault kept in a history would never clear).
bool fog_history(Fog* g, Frame* f, float2 uv, float t, out float4 previous)
{
    previous = float4(0.0);
    if (g[0].prev_camera.w <= 0.0 || g[0].history_read == 0xFFFFFFFFu)
        return false;
    float3 relative = g[0].camera.xyz + fog_view_direction(g, uv) * t - g[0].prev_camera.xyz;
    float distance = length(relative);
    if (distance > g[0].camera.w)
        return false;
    float4 clip = mul(g[0].prev_view_proj, float4(relative, 1.0));
    if (clip.w <= 1e-4)
        return false;
    float2 prev_uv = float2(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    if (any(prev_uv < 0.0) || any(prev_uv > 1.0))
        return false;
    float slices = float(FOG_SLICES);
    float w = 0.5 * (sqrt(max(4.0 * distance * slices * slices / g[0].camera.w - 1.0, 0.0)) - 1.0);
    Texture2D<float4> history = g_sampled_images[NonUniformResourceIndex(g[0].history_read)];
    SamplerState linear = g_samplers[SAMPLER_LINEAR_CLAMP];
    float2 atlas_size = float2(FOG_FROXELS_X * FOG_SLICES, FOG_FROXELS_Y);
    float2 inside = clamp(prev_uv * float2(FOG_FROXELS_X, FOG_FROXELS_Y), 0.5, float2(FOG_FROXELS_X, FOG_FROXELS_Y) - 0.5);
    float k = clamp(floor(w), 0.0, slices - 2.0);
    float s = saturate(w - k);
    float4 a = history.SampleLevel(linear, (inside + float2(k * FOG_FROXELS_X, 0.0)) / atlas_size, 0.0);
    float4 b = history.SampleLevel(linear, (inside + float2((k + 1.0) * FOG_FROXELS_X, 0.0)) / atlas_size, 0.0);
    previous = lerp(a, b, s);
    // Not finite (a fault kept in a history would never clear), or reaching a culled froxel.
    if (!all(isfinite(previous)) || previous.a < 0.0)
        return false;
    float last_exposure = g[0].last_exposure;
    previous.rgb = min(previous.rgb * (last_exposure > 0.0 ? g[0].exposure / last_exposure : 1.0), 65000.0);
    return true;
}

// The farthest surface each froxel column shows: the largest distance from the camera of the
// depth's pixels in its screen tile, the sky's (no depth) infinite. Read by fog_light_main's cull.
[shader("compute")]
[numthreads(8, 8, 1)]
void fog_tiles_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= FOG_FROXELS_X || id.y >= FOG_FROXELS_Y)
        return;
    Fog* g = fog_push.fog;
    float2 size = float2(g[0].width, g[0].height);
    uint2 lo = uint2(floor(float2(id.xy) * size / float2(FOG_FROXELS_X, FOG_FROXELS_Y)));
    uint2 hi = min(uint2(ceil(float2(id.xy + 1u) * size / float2(FOG_FROXELS_X, FOG_FROXELS_Y))), uint2(size));
    Texture2D<float4> depth = g_sampled_images[NonUniformResourceIndex(g[0].depth)];
    float farthest = 0.0;
    for (uint y = lo.y; y < hi.y; y++)
    {
        for (uint x = lo.x; x < hi.x; x++)
        {
            float z = depth.Load(int3(int2(x, y), 0)).r;
            if (z <= 0.0)
            {
                farthest = 1e30;
                break;
            }
            float2 uv = (float2(x, y) + 0.5) / size;
            float4 p = mul(g[0].inv_view_proj, float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, z, 1.0));
            farthest = max(farthest, length(p.xyz / p.w));
        }
        if (farthest >= 1e30)
            break;
    }
    g_storage_images_r32f[NonUniformResourceIndex(g[0].tiles)][id.xy] = farthest;
}

// A culled froxel's extinction (fog_light_main): far below any real one, so that a history read
// blending it in at any weight comes out negative and is refused.
static const float FOG_CULLED = -10000.0;

// Each froxel column's last slice lit: two past the farthest surface its column and the two
// columns round it each way show (fog/apply reads a pixel's slice and the one before, across one
// column each way; a history read reaches one slice on and, the jitter shifting it, one column
// further). Every slice where any of them shows the sky or the volume's end.
[shader("compute")]
[numthreads(8, 8, 1)]
void fog_bound_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= FOG_FROXELS_X || id.y >= FOG_FROXELS_Y)
        return;
    Fog* g = fog_push.fog;
    Texture2D<float4> tiles = g_sampled_images[NonUniformResourceIndex(g[0].tiles_sampled)];
    float farthest = 0.0;
    for (int dy = -2; dy <= 2; dy++)
    {
        for (int dx = -2; dx <= 2; dx++)
        {
            int2 at = clamp(int2(id.xy) + int2(dx, dy), int2(0), int2(FOG_FROXELS_X - 1, FOG_FROXELS_Y - 1));
            farthest = max(farthest, tiles.Load(int3(at, 0)).r);
        }
    }
    float last = farthest < g[0].camera.w
        ? floor(float(FOG_SLICES) * sqrt(farthest / g[0].camera.w)) + 2.0
        : float(FOG_SLICES);
    g_storage_images_r32f[NonUniformResourceIndex(g[0].bound)][id.xy] = last;
}

// Whether froxel `id` lies past its column's last slice lit (fog_bound_main): no pixel's fog reads
// it, so it is not lit.
bool fog_culled(Fog* g, uint3 id)
{
    Texture2D<float4> bound = g_sampled_images[NonUniformResourceIndex(g[0].bound_sampled)];
    return float(id.z) > bound.Load(int3(int2(id.xy), 0)).r;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void fog_light_main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= FOG_FROXELS_X || id.y >= FOG_FROXELS_Y || id.z >= FOG_SLICES)
        return;
    Fog* g = fog_push.fog;
    Frame* f = g[0].frame;
    float2 middle_uv = (float2(id.xy) + 0.5) / float2(FOG_FROXELS_X, FOG_FROXELS_Y);
    float near = fog_slice_distance(g, float(id.z));
    float far = fog_slice_distance(g, float(id.z) + 1.0);
    // Behind every surface in view: not lit, marked (FOG_CULLED) so that the integration skips it and
    // a history read reaching it is refused (the froxel then lit afresh: no stale light fading in).
    if (g[0].cull != 0u && fog_culled(g, id))
    {
        g_storage_images_rgba16f[NonUniformResourceIndex(g[0].history_write)][uint2(id.z * FOG_FROXELS_X + id.x, id.y)] = float4(0.0, 0.0, 0.0, FOG_CULLED);
        return;
    }
    // The history: last frame's volume where this froxel's middle stood then.
    float4 previous;
    bool remembered = fog_history(g, f, middle_uv, 0.5 * (near + far), previous);
    // With it, a point jittered within the froxel (the frame's offset and the froxel's own), the
    // history averaging ten frames of them; without, its middle.
    bool jittered = remembered;
    float3 j = jittered ? frac(g[0].jitter.xyz + fog_froxel_offset(id)) - 0.5 : float3(0.0);
    float2 uv = jittered ? (float2(id.xy) + 0.5 + j.xy) / float2(FOG_FROXELS_X, FOG_FROXELS_Y) : middle_uv;
    float3 direction = fog_view_direction(g, uv);
    float t = jittered ? near + (far - near) * (0.5 + j.z) : 0.5 * (near + far);
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
        float sun_anisotropy = g[0].sun_color.a;
        // The lamps whose reach meets the froxel's column (every lamp beyond the grid), their
        // light as the light lists weigh it at a surface facing each (lux).
        float3 lamps = float3(0.0);
        // The strongest shadow-casting lamps so far, strongest first: their index, their light's
        // luminance and their light.
        uint shadowed = min(uint(g[0].phase.y), FOG_SHADOW_RAYS);
        uint top_index[FOG_SHADOW_RAYS];
        float top_y[FOG_SHADOW_RAYS];
        float3 top_light[FOG_SHADOW_RAYS];
        [unroll]
        for (uint q = 0u; q < FOG_SHADOW_RAYS; q++)
        {
            top_index[q] = 0u;
            top_y[q] = 0.0;
            top_light[q] = float3(0.0);
        }
        if (f[0].lamp_count > 0u && f[0].lamps != nullptr)
        {
            LampRuns runs = lamp_runs(f, REGION_NONE, REGION_NONE, world, true);
            for (uint r = 0u; r < 3u; r++)
            {
                for (uint e = runs.run[r].x; e < runs.run[r].y; e++)
                {
                    uint index = lamp_at(f, runs, e);
                    Lamp lamp = f[0].lamps[index];
                    float3 d = lamp.position_radius.xyz - world;
                    float dist2 = dot(d, d);
                    float reach = lamp.colour_reach.w;
                    if (dist2 >= reach * reach)
                        continue;
                    float dist = sqrt(dist2);
                    float3 to_lamp = d / max(dist, 1e-6);
                    float window = saturate(1.0 - pow(dist / reach, 4.0));
                    float least = max(lamp.position_radius.w, half_size);
                    // A tube's light along its line, its phase from its nearest point (#396).
                    if (lamp_tube(lamp))
                    {
                        float3 along = lamp.beam.xyz * clamp(dot(-d, lamp.beam.xyz), -lamp.beam_edge.x, lamp.beam_edge.x);
                        float3 nearest = d + along;
                        to_lamp = nearest / max(length(nearest), 1e-6);
                    }
                    float3 lux = lamp_tube(lamp)
                        ? lamp.colour_reach.rgb * (lamp_tube_fog(lamp, world, least) * window * window)
                        : lamp.colour_reach.rgb / max(dist2, least * least) * window * window * lamp_beam(lamp, to_lamp);
                    float share = 1.0 - asfloat(lamp.flags.y);
                    lamps += share * lux * fog_phase(dot(-direction, -to_lamp), g[0].phase.x);
                    // Kept among the strongest, carried down past the weaker ones.
                    if (shadowed > 0u && (lamp.flags.x & LAMP_SHADOWLESS) == 0u)
                    {
                        float3 light = share * lux * fog_phase(dot(-direction, -to_lamp), g[0].phase.x);
                        float y = dot(light, float3(0.2126, 0.7152, 0.0722));
                        [unroll]
                        for (uint q = 0u; q < FOG_SHADOW_RAYS; q++)
                        {
                            if (q < shadowed && y > top_y[q])
                            {
                                float ty = top_y[q];
                                uint ti = top_index[q];
                                float3 tl = top_light[q];
                                top_y[q] = y;
                                top_index[q] = index;
                                top_light[q] = light;
                                y = ty;
                                index = ti;
                                light = tl;
                            }
                        }
                    }
                }
            }
            // Their shadows: each strong lamp a ray to it, its light taken off where it is met. One
            // ray in a rolled loop (each inlined copy grows the pass, #348); the entry read by
            // constant indices, so the lists stay in registers.
            if (shadowed > 0u && f[0].tlas != 0)
            {
                float least_y = FOG_SHADOW_SHARE * dot(lamps, float3(0.2126, 0.7152, 0.0722));
                [loop]
                for (uint q = 0u; q < shadowed; q++)
                {
                    float y = 0.0;
                    uint index = 0u;
                    float3 light = float3(0.0);
                    [unroll]
                    for (uint p = 0u; p < FOG_SHADOW_RAYS; p++)
                    {
                        if (p == q)
                        {
                            y = top_y[p];
                            index = top_index[p];
                            light = top_light[p];
                        }
                    }
                    if (y <= 0.0 || y < least_y)
                        break;
                    Lamp lamp = f[0].lamps[index];
                    float3 d = lamp.position_radius.xyz - world;
                    // A tube's ray to its point nearest the froxel (#396), not its middle: one
                    // occluder before a long strip's middle took all of its light.
                    if (lamp_tube(lamp))
                        d += lamp.beam.xyz * clamp(dot(-d, lamp.beam.xyz), -lamp.beam_edge.x, lamp.beam_edge.x);
                    float dist = length(d);
                    RayDesc ray;
                    ray.Origin = world;
                    ray.Direction = d / max(dist, 1e-6);
                    ray.TMin = 0.0;
                    ray.TMax = max(dist - (lamp_flat(lamp) ? 0.02 : lamp.position_radius.w + FOG_SHADOW_CLEAR), 0.0);
                    if (ray.TMax > 0.0 && ray_blocked(f, ray))
                        lamps -= light;
                }
                lamps = max(lamps, float3(0.0));
            }
        }
        // The lamps of the fog alone (an authored scene's lights that light no surface).
        for (uint k = 0u; k < g[0].fog_lamp_count; k++)
        {
            Lamp lamp = g[0].fog_lamps[k];
            float3 d = lamp.position_radius.xyz - world;
            float dist2 = dot(d, d);
            float reach = lamp.colour_reach.w;
            if (dist2 >= reach * reach)
                continue;
            float dist = sqrt(dist2);
            float3 to_lamp = d / max(dist, 1e-6);
            float window = saturate(1.0 - pow(dist / reach, 4.0));
            float least = max(lamp.position_radius.w, half_size);
            // A tube's light along its line, its phase from its nearest point (#396).
            if (lamp_tube(lamp))
            {
                float3 nearest = d + lamp.beam.xyz * clamp(dot(-d, lamp.beam.xyz), -lamp.beam_edge.x, lamp.beam_edge.x);
                to_lamp = nearest / max(length(nearest), 1e-6);
            }
            float3 lux = lamp_tube(lamp)
                ? lamp.colour_reach.rgb * (lamp_tube_fog(lamp, world, least) * window * window)
                : lamp.colour_reach.rgb / max(dist2, least * least) * window * window * lamp_beam(lamp, to_lamp);
            lamps += (1.0 - asfloat(lamp.flags.y)) * lux * fog_phase(dot(-direction, -to_lamp), g[0].phase.x);
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
        float3 sunlight = g[0].sun.w * g[0].sun_color.rgb * fog_phase(dot(-direction, -sun), sun_anisotropy) * visible;
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
            // The lookup in a branch of its own: chosen per direction inside the loop, it cost the
            // bar's night 1 ms.
            if (g[0].probes != nullptr)
            {
                [loop]
                for (uint k = 0u; k < 2u; k++)
                {
                    float3 n = float3(0.0, k == 0u ? 1.0 : -1.0, 0.0);
                    ambient += probe_irradiance_hit(g[0].probes, world, n, -direction, sh_irradiance(g[0].sky, n), false);
                }
            }
            else
                ambient = sh_irradiance(g[0].sky, float3(0.0, 1.0, 0.0)) + sh_irradiance(g[0].sky, float3(0.0, -1.0, 0.0));
            ambient *= ambient_extinction * f[0].sun_illuminance * f[0].exposure / 6.28318531;
        }
        scattered = albedo * tint * (extinction * (lamps * (f[0].exposure * g[0].scales.y) + sunlight * g[0].scales.x) + ambient);
    }
    float4 lit = float4(scattered, extinction);
    // The history keeps its weight of the light: the jittered samples of about ten frames average
    // out the froxels' blocks and their crawl as the camera moves.
    uint2 texel = uint2(id.z * FOG_FROXELS_X + id.x, id.y);
    if (remembered)
        lit = lerp(lit, previous, g[0].prev_camera.w);
    // This frame's history, which the integration reads (`light` is its storage index too).
    g_storage_images_rgba16f[NonUniformResourceIndex(g[0].history_write)][texel] = lit;
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
        // A culled froxel (behind every surface in view) adds nothing.
        if (l.a < 0.0)
            l = float4(0.0);
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

// The distance fog's optical depth (D-101) along `direction` from the camera, `s0` to `s1` metres:
// each layer's extinction, density * exp(-falloff * (y - height)), integrated in closed form (the
// mirror of `DistanceFog::optical_depth`). Each end's exponent held within -80 and 20 (deep under a
// layer's height, it no longer thickens); a ray crossing less than a thousandth of an e-fold taken
// as level.
float distance_fog_depth(Fog* g, float3 direction, float s0, float s1)
{
    float y0 = g[0].camera.y - g[0].distance_range.z;
    float span = max(s1 - s0, 0.0);
    float tau = 0.0;
    [unroll]
    for (uint k = 0u; k < 2u; k++)
    {
        float density = k == 0u ? g[0].distance_layers.x : g[0].distance_layers.z;
        float falloff = max(k == 0u ? g[0].distance_layers.y : g[0].distance_layers.w, 0.0);
        if (density <= 0.0)
            continue;
        float a = falloff * direction.y;
        float near = clamp(-falloff * y0 - a * s0, -80.0, 20.0);
        if (abs(a * span) < 1e-3)
            tau += density * exp(near) * span;
        else
            tau += density * max((exp(near) - exp(clamp(-falloff * y0 - a * s1, -80.0, 20.0))) / a, 0.0);
    }
    return tau;
}

// How far a sky pixel's ray runs through the distance fog, metres: as far as its layers reach
// (`DistanceFog`'s test integrates to it).
static const float DISTANCE_FOG_SKY = 1e6;

// The colour `c` seen at `s1` metres along `direction` (the sky's: `sky`) through the distance fog
// (D-101): dimmed by it and lit by what it scatters, the sun's light through its phase and the
// sky's every way alike (the mean of its irradiance up and down over 2π, as the froxels' without
// probes), unshadowed, its colour from the near one at its start to the far one at `far`. On the
// sky, its cover fades from the horizon to the elevation `distance_range.w`.
float3 distance_fog(Fog* g, float3 c, float3 direction, float s1, bool sky)
{
    float s0 = g[0].distance_range.x;
    if (s1 <= s0)
        return c;
    float cover = 1.0 - exp(-distance_fog_depth(g, direction, s0, s1));
    if (sky && g[0].distance_range.w > 0.0)
        cover *= 1.0 - smoothstep(0.0, g[0].distance_range.w, asin(clamp(direction.y, -1.0, 1.0)));
    float3 sun = normalize(g[0].sun.xyz);
    // The sun's light where it stands above the horizon (as the froxels', which also trace it).
    float3 light = sun.y > -0.1
        ? g[0].sun.w * g[0].sun_color.rgb * fog_phase(dot(direction, sun), g[0].distance_phase.x) * g[0].distance_near.a
        : float3(0.0);
    if (g[0].sky != nullptr && g[0].distance_far.a > 0.0)
        light += (sh_irradiance(g[0].sky, float3(0.0, 1.0, 0.0)) + sh_irradiance(g[0].sky, float3(0.0, -1.0, 0.0))) * g[0].distance_far.a;
    float far = g[0].distance_range.y;
    float3 colour = lerp(g[0].distance_near.rgb, g[0].distance_far.rgb, saturate((min(s1, far) - s0) / max(far - s0, 1e-3)));
    return lerp(c, colour * light, cover);
}

// A thread per pixel: the colour dimmed by the fog in front of it and brightened by what the fog
// scatters; empty pixels (the sky) see the whole volume. Beyond the froxels, the distance fog
// (D-101) first: the froxels' fog lies in front of it.
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
    if (g[0].distance_phase.y > 0.0)
        c.rgb = distance_fog(g, c.rgb, fog_view_direction(g, uv), depth > 0.0 ? distance : DISTANCE_FOG_SKY, depth <= 0.0);
    color[id.xy] = float4(c.rgb * fog.a + fog.rgb, c.a);
}
