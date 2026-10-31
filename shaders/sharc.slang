// D-088's surface cache through NVIDIA's SHaRC 1.8.3 (Spatially Hashed Radiance Cache, the NVIDIA
// RTX SDKs licence): the light that reaches the scene's surfaces round the camera, measured by a
// sparse path tracer into a hash grid of voxels and read where the diffuse bounce's rays meet a
// surface (`docs/research/sharc-and-nrc.md`, D-088's design).
//
// SHaRC stays outside Forge's code (its licence, as RTXDI's in `restir.slang`): this file is
// Forge's side, the compiler finds SHaRC's headers in `sharc-sdk/src/include`
// (crates/forge-gpu/src/shader.rs, `SDK_INCLUDES`), and they are included unmodified. Without that
// directory this file is never compiled and `r.gi.cache sharc` falls back to the probes.
//
// - `sharc_update_main` (`gi/sharc update`): one pixel in each 5 × 5 block, a different one each
//   update, starts a path at its surface; each hit adds what it sends back to the voxels before
//   it on the path (`SharcUpdateHit`, `SharcUpdateMiss`), and stops where SHaRC reads its voxel
//   instead (its resampling).
// - `sharc_resolve_main` (`gi/sharc resolve`): per slot, the update's sums blended into the light
//   kept over the last frames, and the voxels no path met for a while emptied (`SharcResolveEntry`).
// - `diffuse_trace_main` built here (`gi/trace` with `r.gi.cache sharc`): meshlet.slang's bounce,
//   its hit lit by `sharc_bounce`.
//
// SHaRC's options, Forge's choices:
// - SHARC_SEPARATE_EMISSIVE: a hit's own sun (its shadow ray) and lamps are its "emissive" term,
//   left out of its own voxel and added back where it is read, so the voxels hold only the light
//   that reaches a surface and the bounce keeps its hits' exact sun shadows, as without the cache.
// - SHARC_MATERIAL_DEMODULATION: the albedo, divided out where a voxel is written and multiplied
//   back where it is read: a voxel holds the light that reaches it (irradiance, as the probes),
//   and a textured surface keeps its texture.
// - The buffers by device address, Forge's convention: SHaRC's structured buffers as pointers
//   (`RW_STRUCTURED_BUFFER`, `BUFFER_AT_OFFSET`, which its headers leave to the application).
// - 64-bit atomics for the keys (a device without them keeps the probes).
// - SHARC_PROPAGATION_DEPTH 4, not SHaRC's 2: each hit's light goes back to the four vertices
//   before it, and a path reads a voxel (its resampling) at a random one of its second to fifth
//   hits. SHaRC's guide raises it where light needs longer paths, as a room lit through its
//   windows: with 2, the landing's whitewashed walls (albedo near 0.9) took their many bounces
//   from voxels that settle over hundreds of frames, and frame 300 was a third of a stop under
//   the path tracer (16 × 9 tiles, mean |log2| 0.376, ×0.85); with 4, 0.089 and ×0.99, for
//   0.03 ms more in the update (D-088).
// Every entry of this file is compiled with SHARC_UPDATE, which only the update's functions read.

#define FORGE_SHARC 1
#include "meshlet.slang"

#define SHARC_UPDATE 1
#define SHARC_PROPAGATION_DEPTH 4
#define SHARC_ENABLE_RESPONSIVE_LIGHTING 0
#define SHARC_ENABLE_GLSL 0
#define SHARC_SEPARATE_EMISSIVE 1
#define SHARC_MATERIAL_DEMODULATION 1
#define HASH_GRID_ENABLE_64_BIT_ATOMICS 1
#define RW_STRUCTURED_BUFFER(name, type) type* name
#define BUFFER_AT_OFFSET(name, offset) name[offset]
#include "SharcCommon.h"

// A path's most hits in the update (the RTXGI sample's own default, 8): SHaRC's resampling ends
// most of them between the second and the fifth.
static const uint SHARC_MOST_BOUNCES = 8u;
// The segments `sharc_bounce` adds after the bounce's own where a segment is too short to read.
static const uint SHARC_MORE_SEGMENTS = 2u;
// The side of the blocks the update takes one pixel of (the guide's 5 × 5, 4 % of the pixels).
static const uint SHARC_BLOCK = 5u;

// SHaRC's parameters for `field`'s cache this frame.
SharcParameters sharc_parameters(SharcField* field)
{
    SharcParameters p;
    p.hashGridParameters.cameraPosition = field[0].camera;
    p.hashGridParameters.sceneScale = field[0].scene_scale;
    p.hashGridParameters.logarithmBase = SHARC_GRID_LOGARITHM_BASE;
    p.hashGridParameters.levelBias = SHARC_GRID_LEVEL_BIAS;
    p.hashGridData.capacity = field[0].capacity;
    p.hashGridData.hashEntriesBuffer = field[0].keys;
    p.radianceScale = field[0].radiance_scale;
    p.accumulationBuffer = (SharcAccumulationData*)field[0].accumulation;
    p.resolvedBuffer = (SharcPackedData*)field[0].resolved;
    return p;
}

// The factor a hit's voxel is demodulated by: its albedo, kept off 0 (a black surface still
// counts its path, with no light).
float3 sharc_demodulation(float3 albedo)
{
    return max(albedo, float3(0.01));
}

// The light that reaches `hit` from the probes, or the sky's where there are none: what the
// bounce reads where no voxel answers, as without the cache (`hit_radiance`).
float3 sharc_probe_light(float4* sky, ProbeField* probes, RayHit hit, float3 direction)
{
    float3 ambient = sh_irradiance(sky, hit.n);
    if (probes != nullptr)
        ambient = probe_irradiance_hit(probes, hit.position, hit.n, -direction, ambient, false);
    return ambient;
}

// The light the bounce's hit `hit` (met as `c` along `direction`) sends back, in the resolve's
// units: its own sun through a shadow ray and its lamps, exact, as without the cache, and its
// albedo times the light that reaches it, from the voxel it is in. As SHaRC's guide reads it,
// only where the segment that met the hit is longer than the voxel's diagonal, so that a voxel
// never stands for the light of its own corner; a shorter one goes on along one more cosine-
// weighted ray from the hit, at most SHARC_MORE_SEGMENTS, and reads where that ends. Where no
// voxel answers (a surface no path met yet, a leaf, whose rays start inside its crown's proxy,
// D-075), the probes' light stands in. `seed`: the pixel's own number, as `hit_lamps` takes it.
// The probes in place of the extra rays cost about 1 ms less at the Bistro's Exterior_1 (whose far
// voxels are 30 cm to 1 m across) and measured nearer the path tracer, but only as the probes'
// excess offset SHaRC's few per cent of deficit: the classroom's sky alone went from 0.097 to
// 0.196 (D-088).
float3 sharc_bounce(Frame* f, float4* sky, ProbeField* probes, SharcField* field, RayHit hit, ClosestHit c, float3 direction, uint seed)
{
    SharcParameters sharc = sharc_parameters(field);
    float3 sum = float3(0.0);
    float3 weight = float3(1.0);
    for (uint k = 0u; k <= SHARC_MORE_SEGMENTS; k++)
    {
        float3 own = hit_sunlight(f, hit) + hit_lamps(f, hit, REGION_NONE, seed + k * 0x68E31DA4u, false);
        uint level = HashGridGetLevel(hit.position, sharc.hashGridParameters);
        float voxel = HashGridGetVoxelSize(level, sharc.hashGridParameters);
        bool leaf = hit_foliage(f, c.instance, c.primitive);
        bool long_enough = c.t > voxel * 1.7320508;
        if (!leaf && long_enough)
        {
            SharcHitData h;
            h.positionWorld = hit.position;
            h.normalWorld = hit.n;
            h.materialDemodulation = sharc_demodulation(hit.albedo);
            h.emissive = own;
            float3 cached;
            if (SharcGetCachedRadiance(sharc, h, cached, false))
                return sum + weight * cached;
        }
        if (leaf || long_enough || k == SHARC_MORE_SEGMENTS)
            return sum + weight * (own + hit.albedo * sharc_probe_light(sky, probes, hit, direction));
        sum += weight * own;
        weight *= hit.albedo;
        uint turn = (seed + k * 0x9E3779B9u) ^ (f[0].noise_frame * 0x632BE5ABu);
        direction = cosine_direction(hit.n, hash_unit(turn), hash_unit(turn ^ 0x5BD1E995u));
        RayDesc ray;
        ray.Origin = hit.position + hit.n * DIFFUSE_START;
        ray.Direction = direction;
        ray.TMin = 0.0;
        ray.TMax = PATH_RANGE;
        c = trace_past_glass(f, ray, true, seed);
        if (!c.hit)
            return sum + weight * sky_along(sky, direction);
        hit = ray_hit_lit(f, c, ray.Origin, direction);
        if (hit.back)
            return sum;
    }
    return sum;
}

// ------------------------------------------------------------------------- the update ---

struct SharcUpdatePush
{
    Frame* frame;
    float4* sky;          // the sky's irradiance coefficients, as the resolve has them
    ProbeField* probes;   // the probes, or null: the sky's irradiance where a path ends early
    SharcField* sharc;
    uint vis_image;       // sampled (u32): the visibility buffer
    uint width;           // the frame's
    uint height;
    uint pad;
};

[[vk::push_constant]]
SharcUpdatePush sharc_update_push;

// One path for each SHARC_BLOCK × SHARC_BLOCK block of pixels, from the surface of a pixel of it
// picked anew each update, on along cosine-weighted rays as the bounce's (`path_from`'s Lambert,
// past a house's clear glass, the crowns as the probes see them). Each hit's own sun and lamps
// (the lamps as the probes' rays take them, `hit_lamp_weight`) go to the voxels before it on the
// path, through each segment's albedo. The pixel's own surface is the path's first voxel, its
// light to come counted at 1 (its albedo divided out and in again, as SHaRC's demodulation does
// it at every hit). A path that meets a leaf, or its last hit, ends on the probes' light there; a
// path that meets a surface's back ends there, as `path_from`'s.
[shader("compute")]
[numthreads(8, 8, 1)]
void sharc_update_main(uint3 id : SV_DispatchThreadID)
{
    SharcUpdatePush p = sharc_update_push;
    Frame* f = p.frame;
    SharcField* field = p.sharc;
    uint2 corner = id.xy * SHARC_BLOCK;
    if (corner.x >= p.width || corner.y >= p.height || f[0].tlas == 0)
        return;
    uint frame = field[0].frame;
    uint pick = (id.x * 0x8DA6B343u) ^ (id.y * 0xD8163841u) ^ (frame * 0x9E3779B9u);
    uint at = uint(hash_unit(pick) * float(SHARC_BLOCK * SHARC_BLOCK));
    uint2 pixel = min(corner + uint2(at % SHARC_BLOCK, at / SHARC_BLOCK), uint2(p.width - 1u, p.height - 1u));
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(p.vis_image)].Load(int3(pixel, 0));
    if (vis == VISIBILITY_EMPTY)
        return;
    PixelSurface s = pixel_surface(f, vis, pixel, p.width, p.height);
    // A crown's leaf starts no path: its rays start inside its crown's proxy (D-075).
    if (s.foliage)
        return;
    g_ray_mask = shadow_world(f, s.relative, pixel);
    SharcParameters sharc = sharc_parameters(field);
    SharcState state;
    SharcInit(state);
    uint seed = (pixel.x * 0x8DA6B343u) ^ (pixel.y * 0xD8163841u) ^ (frame * 0x632BE5ABu);
    SharcHitData root;
    root.positionWorld = to_scene(f, s.relative);
    root.normalWorld = s.g;
    root.materialDemodulation = float3(1.0);
    root.emissive = float3(0.0);
    if (!SharcUpdateHit(sharc, state, root, float3(0.0), hash_unit(seed)))
        return;
    SharcSetThroughput(state, float3(1.0));
    // Round the vertices' normal, never under the triangle's own face, as the bounce's.
    float3 d = cosine_direction(s.n, hash_unit(seed + 1u), hash_unit(seed + 2u));
    if (dot(d, s.g) < 0.0)
        d -= 2.0 * dot(d, s.g) * s.g;
    RayDesc ray;
    ray.Origin = root.positionWorld + s.n * diffuse_start(f, vis, s);
    ray.Direction = d;
    ray.TMin = 0.0;
    ray.TMax = PATH_RANGE;
    for (uint k = 0u; k < SHARC_MOST_BOUNCES; k++)
    {
        // The first ray leaves the drawn surface: past the back of its own mesh's traced cut
        // where that stands in front of it, as the bounce's (#367).
        ClosestHit c = k == 0u ? trace_from_pixel(f, vis, s, ray, seed) : trace_past_glass(f, ray, true, seed);
        if (!c.hit)
        {
            SharcUpdateMiss(sharc, state, sky_along(p.sky, d));
            return;
        }
        RayHit hit = ray_hit_lit(f, c, ray.Origin, d);
        if (hit.back)
            return;
        uint turn = seed + (k + 1u) * 0x9E3779B9u;
        float3 own = hit_sunlight(f, hit) + hit_lamps(f, hit, REGION_NONE, turn, true);
        if (k + 1u == SHARC_MOST_BOUNCES || hit_foliage(f, c.instance, c.primitive))
        {
            SharcUpdateMiss(sharc, state, own + hit.albedo * sharc_probe_light(p.sky, p.probes, hit, d));
            return;
        }
        SharcHitData h;
        h.positionWorld = hit.position;
        h.normalWorld = hit.n;
        h.materialDemodulation = sharc_demodulation(hit.albedo);
        h.emissive = own;
        if (!SharcUpdateHit(sharc, state, h, float3(0.0), hash_unit(turn)))
            return;
        // Lambert's, cosine-weighted: the segment's throughput is the albedo.
        d = cosine_direction(hit.n, hash_unit(turn + 1u), hash_unit(turn + 2u));
        SharcSetThroughput(state, hit.albedo);
        ray.Origin = hit.position + hit.n * DIFFUSE_START;
        ray.Direction = d;
    }
}

// ------------------------------------------------------------------------ the resolve ---

struct SharcResolvePush
{
    SharcField* sharc;
};

[[vk::push_constant]]
SharcResolvePush sharc_resolve_push;

// SHaRC's resolve for every slot (`SharcResolveEntry`), and the slots still in use counted.
[shader("compute")]
[numthreads(256, 1, 1)]
void sharc_resolve_main(uint3 id : SV_DispatchThreadID)
{
    SharcField* field = sharc_resolve_push.sharc;
    SharcResolveParameters r;
    r.cameraPositionPrev = field[0].camera_prev;
    r.accumulationFrameNum = field[0].accumulation_frames;
    r.responsiveFrameNum = field[0].accumulation_frames; // without SHARC_ENABLE_RESPONSIVE_LIGHTING, unread
    r.staleFrameNumMax = field[0].stale_frames;
    r.frameIndex = field[0].frame;
    SharcResolveEntry(id.x, sharc_parameters(field), r);
    bool live = id.x < field[0].capacity && field[0].keys[id.x] != 0ull;
    uint count = WaveActiveCountBits(live);
    if (WaveIsFirstLane() && count > 0u)
        InterlockedAdd(field[0].live[0], count);
}
