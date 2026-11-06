// D-088's trial of NVIDIA's NRC 0.16 (Neural Radiance Cache, the NVIDIA RTX SDKs licence) under the
// diffuse bounce: a small network, trained every frame on sparse paths, asked for the light that
// leaves the points where the bounce's rays meet a surface (`docs/research/sharc-and-nrc.md`).
//
// NRC stays outside Forge's code, as SHaRC in `sharc.slang`: this file is Forge's side, the compiler
// finds NRC's headers in `nrc-sdk/src/Include` (crates/forge-gpu/src/shader.rs, `SDK_INCLUDES`), and
// they are included unmodified. Without that directory this file is never compiled and
// `r.gi.cache nrc` falls back to the probes. NRC's network runs in its own library
// (`crates/forge-gpu/src/nrc.rs`), between these passes.
//
// - `nrc_update_main` (`gi/nrc update`): NRC's training paths, at its training size (197 × 111 for
//   1080p), each from the surface of a pixel picked anew each frame along a cosine-weighted ray
//   as the bounce's, on through up to eight hits. Each hit is a vertex of NRC's (`NrcUpdateOnHit`)
//   with the light it sends back itself: its sun through a shadow ray, its glow and one lamp
//   (`hit_sunlight`, `hit_lamps`); NRC carries each vertex's light back to the ones before it.
// - `diffuse_trace_main` built here (`gi/trace` with `r.gi.cache nrc`): meshlet.slang's bounce,
//   whose hit makes NRC's query (`nrc_query`) and sends back its own light, exact.
// - `nrc_resolve_main` (`gi/nrc resolve`), after NRC's inference: each pixel's answer, the light
//   after its hit, added to the bounce's light before REBLUR.
//
// NRC's options, Forge's choices:
// - The cache without the vertices' own light (`includeDirectLighting` off, `FORGE_NRC_DIRECT=1`
//   for NRC's default): the bounce's hit keeps its exact sun shadows, as under SHaRC, and the
//   network learns only the light after a hit. Holding the hits' own light in the cache blurred
//   their sunlit patches: the classroom 0.066 → 0.114 from Forge's path, Exterior_1 in clay
//   0.028 → 0.085 (D-090).
// - The light reaching a point learnt, the albedo divided out (`learnIrradiance`): a textured
//   surface keeps its texture where it is queried, as SHaRC's demodulation does.
// - The surfaces diffuse, as the bounce has them: roughness 1, no specular.
// - The query at the bounce's hit itself (NRC's termination heuristic off for the queries): the
//   paths' first vertex is that hit, not the pixel's surface, whose albedo the bounce never has.
// - The counters in Forge's own buffer, copied into NRC's before its inference (`nrc_count_main`):
//   NRC keeps its own where the host reads them back, and each wave's increment of it cost the
//   bounce 1.4 ms at 1600 × 900 (the classroom's `gi/trace`, 0.52 ms without the queries).
// - The buffers by device address, Forge's convention: NRC's structured buffers as pointers
//   (`NRC_RW_STRUCTURED_BUFFER`). Slang lays a pointer's target out naturally, as DXC's
//   `-fvk-use-dx-layout` that NRC expects (the device has `scalarBlockLayout`).

#define FORGE_NRC 1
#include "meshlet.slang"

#define ENABLE_NRC 1
#define NRC_RW_STRUCTURED_BUFFER(T) T*
#include "Nrc.hlsli"

// NRC's constants for this frame (`NrcField::constants`).
NrcConstants nrc_constants(NrcField* field)
{
    return ((NrcConstants*)field)[0];
}

NrcBuffers nrc_buffers(NrcField* field)
{
    NrcBuffers b;
    b.queryPathInfo = (NrcPackedQueryPathInfo*)field[0].query_path_info;
    b.trainingPathInfo = (NrcPackedTrainingPathInfo*)field[0].training_path_info;
    b.trainingPathVertices = (NrcPackedPathVertex*)field[0].training_path_vertices;
    b.queryRadianceParams = (NrcRadianceParams*)field[0].query_radiance_params;
    b.countersData = field[0].counters;
    return b;
}

// A hit as NRC takes it, seen from `toward` (unit, back along the ray): diffuse, its albedo with the
// bumps' share (the light it sends back) and its shading normal.
NrcSurfaceAttributes nrc_surface(NrcConstants k, RayHit hit, float3 toward)
{
    NrcSurfaceAttributes a;
    a.encodedPosition = NrcEncodePosition(hit.position, k);
    a.roughness = 1.0;
    a.specularF0 = float3(0.0);
    a.diffuseReflectance = hit.albedo;
    a.shadingNormal = hit.n;
    a.viewVector = toward;
    a.isDeltaLobe = false;
    return a;
}

// Whether NRC takes a point at `position` (metres in the scene frame): inside its bounds, where
// its encoding resolves it (NRC's rule; a point outside would alias into the cells it holds).
bool nrc_inside(NrcConstants k, float3 position)
{
    float3 e = NrcEncodePosition(position, k);
    return all(e >= 0.0) && all(e <= 1.0);
}

bool nrc_takes(NrcField* field, float3 position)
{
    return nrc_inside(nrc_constants(field), position);
}

// NRC's query at the bounce's hit `hit` (met at `t` along `direction`), whose answer
// `nrc_resolve_main` adds, and the light the hit sends back itself besides it: none where the cache
// holds the hits' own light too (`includeDirectLighting`, NRC's default, FORGE_NRC_DIRECT), else
// its sun through a shadow ray, its glow and one lamp, exact, as the bounce lights it without a
// cache. `pixel`'s record names the query.
float3 nrc_query(Frame* f, NrcField* field, RayHit hit, float t, float3 direction, uint2 pixel, uint seed)
{
    g_nrcMode = NrcMode::Query;
    NrcConstants k = nrc_constants(field);
    // The query at this first vertex (`NrcUpdateOnHit` queries at vertex 0 without the heuristic).
    k.enableTerminationHeuristic = 0u;
    NrcContext context = NrcCreateContext(k, nrc_buffers(field), pixel);
    NrcPathState state = NrcCreatePathState(k, 0.5);
    float3 throughput = float3(1.0);
    float3 radiance = float3(0.0);
    NrcUpdateOnHit(context, state, nrc_surface(k, hit, -direction), t, 0u, throughput, radiance);
    NrcWriteFinalPathInfo(context, state, throughput, radiance);
    if (k.radianceCacheDirect != 0u)
        return float3(0.0);
    return hit_sunlight(f, hit) + hit_lamps(f, hit, REGION_NONE, seed, false);
}

// `pixel`'s record without a query: its bounce met the sky, a leaf or a surface's back.
void nrc_no_query(NrcField* field, uint2 pixel)
{
    NrcConstants k = nrc_constants(field);
    NrcPackedQueryPathInfo none;
    none.prefixThroughput = 0u;
    none.queryBufferIndex = 0xFFFFFFFFu;
    uint index = NrcCalculateQueryPathIndex(k.frameDimensions, pixel, 0u, k.samplesPerPixel);
    ((NrcPackedQueryPathInfo*)field[0].query_path_info)[index] = none;
}

// ------------------------------------------------------------------------- the update ---

struct NrcUpdatePush
{
    Frame* frame;
    float4* sky;          // the sky's irradiance coefficients, as the resolve has them
    ProbeField* probes;   // the probes, or null: the sky's irradiance where a path meets a leaf
    NrcField* nrc;
    uint vis_image;       // sampled (u32): the visibility buffer
    uint width;           // the frame's
    uint height;
    uint pad;
};

[[vk::push_constant]]
NrcUpdatePush nrc_update_push;

// One of NRC's training paths, `id` of its training size: from the surface of a pixel of the
// frame's matching block, picked anew each frame, on along cosine-weighted rays as the bounce's
// (past a house's clear glass, the crowns as the probes see them), each hit a vertex with its own
// light. NRC ends a path by its spread (or after its most vertices), with its own answer for the
// light after it (self-training), or traces it whole (its unbiased share). A path that meets the sky
// ends on the sky along the ray; one that meets a leaf, or a point outside NRC's bounds, ends on the
// light that point sends back (its own, and the probes' after it, as SHaRC's), with no vertex there;
// one that meets a surface's back ends dark there. Every path writes its record, even one that
// starts on the sky.
[shader("compute")]
[numthreads(8, 8, 1)]
void nrc_update_main(uint3 id : SV_DispatchThreadID)
{
    NrcUpdatePush p = nrc_update_push;
    Frame* f = p.frame;
    NrcField* field = p.nrc;
    NrcConstants k = nrc_constants(field);
    uint2 path = id.xy;
    if (path.x >= k.trainingDimensions.x || path.y >= k.trainingDimensions.y)
        return;
    g_nrcMode = NrcMode::Update;
    NrcContext context = NrcCreateContext(k, nrc_buffers(field), path);
    uint frame = field[0].frame;
    uint seed = (path.x * 0x8DA6B343u) ^ (path.y * 0xD8163841u) ^ (frame * 0x9E3779B9u);
    NrcPathState state = NrcCreatePathState(k, hash_unit(seed));
    float3 throughput = float3(1.0);
    float3 radiance = float3(0.0);
    float2 block = float2(float(p.width), float(p.height)) / float2(k.trainingDimensions);
    float2 jitter = float2(hash_unit(seed ^ 0x68E31DA4u), hash_unit(seed ^ 0xB5297A4Du));
    uint2 pixel = min(uint2((float2(path) + jitter) * block), uint2(p.width - 1u, p.height - 1u));
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(p.vis_image)].Load(int3(pixel, 0));
    PixelSurface s = {};
    bool start = vis != VISIBILITY_EMPTY && f[0].tlas != 0;
    if (start)
    {
        s = pixel_surface(f, vis, pixel, p.width, p.height);
        // A crown's leaf starts no path: its rays start inside its crown's proxy (D-075).
        start = !s.foliage;
    }
    if (!start)
    {
        NrcUpdateOnMiss(state);
        NrcWriteFinalPathInfo(context, state, throughput, radiance);
        return;
    }
    g_ray_mask = shadow_world(f, s.relative, pixel);
    // Round the vertices' normal, never under the triangle's own face, as the bounce's.
    float3 d = cosine_direction(s.n, hash_unit(seed + 1u), hash_unit(seed + 2u));
    if (dot(d, s.g) < 0.0)
        d -= 2.0 * dot(d, s.g) * s.g;
    RayDesc ray;
    ray.Origin = to_scene(f, s.relative) + s.n * diffuse_start(f, vis, s);
    rays_leave(p.sky, p.probes, to_scene(f, s.relative), s.g, normalize(-s.relative));
    ray.Direction = d;
    ray.TMin = 0.0;
    ray.TMax = PATH_RANGE;
    [loop]
    for (uint bounce = 0u; bounce < k.maxPathVertices; bounce++)
    {
        // The first ray leaves the drawn surface: past the back of its own mesh's traced cut
        // where that stands in front of it, as the bounce's (#367).
        ClosestHit c = bounce == 0u ? trace_from_pixel(f, vis, s, ray, seed) : trace_past_glass(f, ray, true, seed);
        if (!c.hit)
        {
            radiance += throughput * sky_light_along(p.sky, d);
            NrcUpdateOnMiss(state);
            break;
        }
        RayHit hit = ray_hit_lit(f, c, ray.Origin, d);
        if (hit.back)
        {
            // Nothing after it: no answer of the cache's either.
            NrcUpdateOnMiss(state);
            break;
        }
        uint turn = seed + (bounce + 1u) * 0x9E3779B9u;
        // No vertex at a leaf (its rays start inside its crown's proxy, D-075) or outside NRC's
        // bounds: the light it sends back ends the path, as the sky does. A vertex there whose
        // path ended at once would teach the cache no light after it.
        if (hit_foliage(f, c.instance, c.primitive) || !nrc_inside(k, hit.position))
        {
            float3 ambient = sh_irradiance(p.sky, hit.n);
            if (p.probes != nullptr)
                ambient = probe_irradiance_hit(p.probes, hit.position, hit.n, -d, ambient, false);
            radiance += throughput * (hit_sunlight(f, hit) + hit_lamps(f, hit, REGION_NONE, turn, false) + hit.albedo * ambient);
            NrcUpdateOnMiss(state);
            break;
        }
        NrcProgressState progress = NrcUpdateOnHit(context, state, nrc_surface(k, hit, -d), c.t, bounce, throughput, radiance);
        if (progress == NrcProgressState::TerminateImmediately)
            break;
        radiance += throughput * (hit_sunlight(f, hit) + hit_lamps(f, hit, REGION_NONE, turn, false));
        if (progress == NrcProgressState::TerminateAfterDirectLighting)
            break;
        // Lambert's, cosine-weighted: the segment's throughput is the albedo, its density cos/π.
        d = cosine_direction(hit.n, hash_unit(turn + 1u), hash_unit(turn + 2u));
        throughput *= hit.albedo;
        NrcSetBrdfPdf(state, max(dot(hit.n, d), 1e-4) * 0.31830988);
        ray.Origin = hit.position + hit.n * DIFFUSE_START;
        ray.Direction = d;
    }
    NrcWriteFinalPathInfo(context, state, throughput, radiance);
}

// ------------------------------------------------------------------------- the counts ---

[[vk::push_constant]]
NrcResolvePush nrc_count_push;

// The queries and training records this frame counted, into NRC's own counters before its
// inference.
[shader("compute")]
[numthreads(1, 1, 1)]
void nrc_count_main(uint3 id : SV_DispatchThreadID)
{
    NrcField* field = nrc_count_push.nrc;
    field[0].nrc_counters[0] = field[0].counters[0];
    field[0].nrc_counters[1] = field[0].counters[1];
}

// ------------------------------------------------------------------------ the resolve ---

struct NrcResolvePush
{
    NrcField* nrc;
    uint bounce_image;    // storage (rgba16f): the bounce's light in YCoCg (rgb), its normalised ray length (a)
    uint width;
    uint height;
    uint pad;
};

[[vk::push_constant]]
NrcResolvePush nrc_resolve_push;

// After NRC's inference: each pixel's answer (the light after its bounce's hit, times the weight
// before the query: the hit's albedo, as the network learnt the light reaching it) added to the
// light `gi/trace` wrote, in REBLUR's YCoCg.
[shader("compute")]
[numthreads(8, 8, 1)]
void nrc_resolve_main(uint3 id : SV_DispatchThreadID)
{
    NrcResolvePush p = nrc_resolve_push;
    uint2 pixel = id.xy;
    if (pixel.x >= p.width || pixel.y >= p.height)
        return;
    NrcField* field = p.nrc;
    NrcConstants k = nrc_constants(field);
    uint index = NrcCalculateQueryPathIndex(k.frameDimensions, pixel, 0u, k.samplesPerPixel);
    NrcQueryPathInfo query = NrcUnpackQueryPathInfo(((NrcPackedQueryPathInfo*)field[0].query_path_info)[index]);
    if (query.queryBufferIndex == 0xFFFFFFFFu)
        return;
    float3 answer = NrcUnpackQueryRadiance(k, field[0].query_radiance[query.queryBufferIndex]) * query.prefixThroughput;
    answer = any(isnan(answer)) || any(isinf(answer)) ? float3(0.0) : clamp(answer, 0.0, 65504.0);
    RWTexture2D<float4> bounce = g_storage_images_rgba16f[NonUniformResourceIndex(p.bounce_image)];
    float4 light = bounce[pixel];
    bounce[pixel] = float4(linear_to_ycocg(ycocg_to_linear(light.rgb) + answer), light.a);
}
