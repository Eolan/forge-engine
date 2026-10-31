// The lamps by ReSTIR DI (D-065): NVIDIA's RTXDI library resamples, per pixel, one light sample
// of the lamps over this frame's candidates, the pixel's last frame and its neighbours' (Bitterli
// et al. 2020), and the resolve lights the pixel with it (`lamps_at`). With `--street-restir` the
// street lamps alone (`RestirFrame::outside_only`), every one however far, the light lists the
// houses' (the owner's option 2 of 2026-10-18: a far street keeps its light; not the default
// until its light has a denoiser, it sparkles); with `--lamp-restir` every lamp, D-065's
// comparison with the light lists.
//
// RTXDI stays outside Forge's code (its licence, D-065): this file is Forge's side of its
// application bridge (the `RAB_*` functions it calls), and the compiler finds its headers in
// `rtxdi-sdk/Include` (crates/forge-gpu/src/shader.rs, `SDK_INCLUDES`). Without that directory
// this file is never compiled and the switch is absent.
//
// - The candidates are `RestirFrame::candidates` points of the lamps of the pixel's place, its
//   light lists (`lamp_place`): of lamps picked uniformly, or of every one in turn when there are
//   no more.
// - A lamp is a sphere sampled by solid angle, or a flat piece's rectangle sampled by area
//   (`lamp_flat`); its sample's light is the lamp's luminance by the cosine, let go at its reach
//   as `lamp_add` lets it go, and none where the lamp does not light the surface's place (its
//   room, its storey band).
// - One shadow ray to the chosen candidate, one to the sample the resampling keeps: the latter's
//   light, times its reservoir's weight, is the pixel's lamps' light.
// - The buffers are by device address: the reservoirs, the neighbours' offsets and the surfaces
//   of this frame and the last, which the temporal and spatial reuse read back.

#include "meshlet.slang"

#define RTXDI_ENABLE_PRESAMPLING 0
#include "Rtxdi/RtxdiParameters.h"
#include "Rtxdi/Utils/RandomSamplerState.hlsli"
#include "Rtxdi/DI/Reservoir.hlsli"
#include "Rtxdi/DI/ReSTIRDIParameters.h"

// Mirrors `GpuRestirFrame` in crates/forge-render/src/restir.rs.
struct RestirFrame
{
    float4x4 clip_from_relative;     // this frame's view-projection, unjittered, camera-relative
    float4x4 previous_from_current;  // the previous frame's unjittered clip from this frame's (TAA's)
    RTXDI_RuntimeParameters runtime;
    RTXDI_ReservoirBufferParameters reservoir_params;
    RTXDI_DISpatioTemporalResamplingParameters spatiotemporal;
    RTXDI_PackedDIReservoir* reservoirs; // two arrays, `current` and the other
    float2* neighbor_offsets;        // the spatial reuse's, in the unit disc
    uint4* surfaces;                 // per pixel of the two frames: the scene's frame (xyz, a float's bits), the normal (w, octahedral, 16 bits each)
    uint* places;                    // per pixel of the two frames: region | other << 16 (`RESTIR_NO_REGION`), or ~0 where nothing was drawn
    uint* to_previous;               // per lamp of this frame, its index the frame before, or ~0
    uint* to_current;                // per lamp of the frame before, its index in this one, or ~0
    uint surface_width;              // pixels a row of `surfaces` holds
    uint surface_pitch;              // pixels a frame of `surfaces` holds
    uint current;                    // the array this frame writes, 0 or 1: the other is the last frame's
    uint candidates;                 // the initial candidates a pixel takes
    uint history;                    // 1: the last frame's arrays hold what it wrote (not the first frame, a cut or a new size)
    uint outside_only;               // 1: the street lamps alone (`lamp_outside`), the light lists the rest; 0: every lamp
    uint pad[2];
};

struct RestirPush
{
    Frame* frame;
    ProbeField* probes;  // the probes' regions (D-068), as the resolve has them, or null: no places
    RestirFrame* restir;
    uint vis_image;      // sampled (u32): the visibility buffer
    uint light_image;    // storage (rgba16f): the lamps' light on the pixel in lux (rgb), the lamp it came from + 1 (a, 0: none)
    uint width;
    uint height;
    uint diffuse_image;  // storage (rgba16f): the diffuse light for REBLUR (#372), by the shading normal, lux over RESTIR_DENOISE_SCALE in YCoCg (rgb), the distance to the sample's point normalised as REBLUR reads it (a)
    uint specular_image; // storage (rgba16f): the highlight over the surface's directional albedo (`environment_specular`), likewise, the distance normalised for its roughness (a)
    uint guide_image;    // storage (rgba16f): REBLUR's guide, the shading normal (xyz) and the linear roughness (w) both were lit with
    uint pad0;
    uint pad1;
    uint pad2;
    float4 hit_distance; // REBLUR's normalisation of that distance (A, B, C: `diffuse_bounce::HIT_DISTANCE`); w unused
};

[[vk::push_constant]]
RestirPush restir_push;

// RTXDI's reservoirs, read and written as D3D12 reads and writes a buffer: past the two arrays a
// read gives an empty reservoir and a write is dropped. RTXDI's pairwise reuse loads the reservoir
// of the pixel it reprojected to even where it found no surface there, which can lie off the
// screen: by device address, Vulkan faulted and lost the device in the Bistro and the rooms
// (#372), whose surfaces reach the screen's edges. A negative position's index wraps round and
// can land in the arrays, on some other pixel's reservoir: harmless, as that load only feeds
// RTXDI's test of naive samples, but not always empty.
struct RestirReservoirs
{
    __subscript(uint i) -> RTXDI_PackedDIReservoir
    {
        get
        {
            RestirFrame* r = restir_push.restir;
            if (i < 2u * r->reservoir_params.reservoirArrayPitch)
                return r->reservoirs[i];
            RTXDI_PackedDIReservoir none;
            none.lightData = 0u;
            none.uvData = 0u;
            none.mVisibility = 0u;
            none.distanceAge = 0u;
            none.targetPdf = 0.0;
            none.weight = 0.0;
            return none;
        }
        set
        {
            RestirFrame* r = restir_push.restir;
            if (i < 2u * r->reservoir_params.reservoirArrayPitch)
                r->reservoirs[i] = newValue;
        }
    }
};
static RestirReservoirs g_restir_reservoirs;

#define RTXDI_LIGHT_RESERVOIR_BUFFER g_restir_reservoirs
#define RTXDI_NEIGHBOR_OFFSETS_BUFFER (restir_push.restir->neighbor_offsets)

// `RestirFrame::places`' code for REGION_NONE, and for a pixel where nothing was drawn.
static const uint RESTIR_NO_REGION = 0xFFFEu;
static const uint RESTIR_EMPTY = 0xFFFFFFFFu;

// ---------------------------------------------------------------- the bridge's surfaces ---

struct RAB_Surface
{
    float3 world;   // the scene's frame
    float depth;    // view depth in its frame, metres
    float3 normal;
    float bias;     // where its shadow rays start off it
    uint region;    // its place (`lamp_place`)
    uint other;
    uint valid;
};

struct RAB_Material
{
    uint pad;
};

RAB_Surface RAB_EmptySurface()
{
    RAB_Surface s;
    s.world = float3(0.0);
    s.depth = 0.0;
    s.normal = float3(0.0, 1.0, 0.0);
    s.bias = 0.02;
    s.region = REGION_NONE;
    s.other = REGION_NONE;
    s.valid = 0u;
    return s;
}

bool RAB_IsSurfaceValid(RAB_Surface s) { return s.valid != 0u; }
float3 RAB_GetSurfaceWorldPos(RAB_Surface s) { return s.world; }
float3 RAB_GetSurfaceNormal(RAB_Surface s) { return s.normal; }
float RAB_GetSurfaceLinearDepth(RAB_Surface s) { return s.depth; }

// No material test (`enableMaterialSimilarityTest` is off): every surface is alike.
RAB_Material RAB_GetMaterial(RAB_Surface s)
{
    RAB_Material m;
    m.pad = 0u;
    return m;
}

bool RAB_AreMaterialsSimilar(RAB_Material a, RAB_Material b) { return true; }

// This frame's clip coordinates of a point of the scene's frame, unjittered, and the last
// frame's.
float4 restir_clip(Frame* f, RestirFrame* r, float3 world)
{
    return mul(r->clip_from_relative, float4(world - f[0].camera_in_scene.xyz, 1.0));
}

// The pixel (fractional, its centre at .5) clip coordinates `clip` fall on.
float2 restir_pixel(float4 clip, uint width, uint height)
{
    float2 ndc = clip.xy / clip.w;
    return float2((ndc.x + 1.0) * 0.5 * float(width), (1.0 - ndc.y) * 0.5 * float(height));
}

// The surface stored at `pixel` this frame (`previous` false) or the last.
RAB_Surface RAB_GetGBufferSurface(int2 pixel, bool previous)
{
    RestirFrame* r = restir_push.restir;
    RAB_Surface s = RAB_EmptySurface();
    if (any(pixel < 0) || pixel.x >= int(restir_push.width) || pixel.y >= int(restir_push.height))
        return s;
    if (previous && r->history == 0u)
        return s;
    uint array = previous ? 1u - r->current : r->current;
    uint i = array * r->surface_pitch + uint(pixel.y) * r->surface_width + uint(pixel.x);
    uint place = r->places[i];
    if (place == RESTIR_EMPTY)
        return s;
    uint4 stored = r->surfaces[i];
    s.world = asfloat(stored.xyz);
    s.normal = oct_decode(float2(float(stored.w & 0xFFFFu), float(stored.w >> 16)) / 65535.0 * 2.0 - 1.0);
    uint region = place & 0xFFFFu;
    uint other = place >> 16;
    s.region = region == RESTIR_NO_REGION ? REGION_NONE : region;
    s.other = other == RESTIR_NO_REGION ? REGION_NONE : other;
    Frame* f = restir_push.frame;
    float4 clip = restir_clip(f, r, s.world);
    s.depth = previous ? mul(r->previous_from_current, clip).w : clip.w;
    s.valid = 1u;
    return s;
}

int2 RAB_ClampSamplePositionIntoView(int2 pixel, bool previous)
{
    return clamp(pixel, int2(0), int2(int(restir_push.width) - 1, int(restir_push.height) - 1));
}

// ---------------------------------------------------------------- the bridge's lights ---

typedef Lamp RAB_LightInfo;

struct RAB_LightSample
{
    float3 position;      // on the lamp's sphere (a flat piece's rectangle), the scene's frame
    float solid_angle_pdf; // of its direction: over the cone the sphere fills, or the rectangle's
    float3 radiance;      // cd/m² per channel, let go at the lamp's reach; 0 where it does not light the surface
    float short_of;       // how far before `position` its shadow ray ends, besides the 2 cm (`RAB_GetConservativeVisibility`)
};

RAB_LightInfo RAB_EmptyLightInfo()
{
    Lamp l;
    l.position_radius = float4(0.0);
    l.colour_reach = float4(0.0);
    l.regions_a = uint4(REGION_NONE);
    l.regions_b = uint4(REGION_NONE);
    return l;
}

RAB_LightSample RAB_EmptyLightSample()
{
    RAB_LightSample s;
    s.position = float3(0.0);
    s.solid_angle_pdf = 0.0;
    s.radiance = float3(0.0);
    s.short_of = 0.0;
    return s;
}

static const uint MAX_RESTIR_LAMPS = 512u; // `lamps::LAMP_CAPACITY`

// Lamp `index` of this frame, or of the frame before (`previous`) by its index then: the lamps
// do not move between frames, so the last frame's lamp is this frame's under its new index.
RAB_LightInfo RAB_LoadLightInfo(uint index, bool previous)
{
    Frame* f = restir_push.frame;
    RestirFrame* r = restir_push.restir;
    uint i = index;
    if (previous)
        i = index < MAX_RESTIR_LAMPS ? r->to_current[index] : 0xFFFFFFFFu;
    if (i >= f[0].lamp_count)
        return RAB_EmptyLightInfo();
    return f[0].lamps[i];
}

// The presampling is off: no compact copies.
RAB_LightInfo RAB_LoadCompactLightInfo(uint linear_index) { return RAB_EmptyLightInfo(); }
bool RAB_StoreCompactLightInfo(uint linear_index, RAB_LightInfo info) { return false; }

int RAB_TranslateLightIndex(uint index, bool current_to_previous)
{
    RestirFrame* r = restir_push.restir;
    if (index >= MAX_RESTIR_LAMPS)
        return -1;
    uint i = current_to_previous ? r->to_previous[index] : r->to_current[index];
    return i == 0xFFFFFFFFu ? -1 : int(i);
}

// Whether `lamp` lights a surface of place (`region`, `other`) at height `y`: the place's light
// lists, and a house lamp's storey band (`lamp_weight_in`).
bool restir_lights(Lamp lamp, float y, uint region, uint other)
{
    if (!lamp_lights(lamp, region, other))
        return false;
    if (lamp_storeyed(lamp) && region != REGION_NONE)
    {
        float dy = y - lamp.position_radius.y;
        if (dy < -2.7 || dy > 0.8)
            return false;
    }
    return true;
}

// A point of flat lamp `lamp`'s rectangle (`lamp_rect_point`) by `uv`, uniform in area, seen from
// `surface` (#372): its density over solid angle is the distance squared over the rectangle's area
// and the cosine at the lamp, and its luminance the lamp's peak intensity over that area
// (Lambert's emitter), none behind the piece. Its light over the density is then the lamp's
// intensity by the two cosines over the distance squared, which over the rectangle sums to
// `lamp_rect_share`'s exact light. Taken as a sphere round the rectangle's middle (before #372),
// a flat lamp kept the light and shadows #369 and #370 corrected in the light lists.
RAB_LightSample restir_flat_sample(Lamp lamp, RAB_Surface surface, float2 uv)
{
    RAB_LightSample s = RAB_EmptyLightSample();
    float3 d = lamp.position_radius.xyz - surface.world;
    float reach = lamp.colour_reach.w;
    float dist2 = dot(d, d);
    if (dist2 >= reach * reach)
        return s;
    s.position = lamp.position_radius.xyz + lamp_rect_point(lamp, uv * 2.0 - 1.0);
    float3 to = s.position - surface.world;
    float to2 = dot(to, to);
    float facing = -dot(to, lamp.beam.xyz) / sqrt(max(to2, 1e-12));
    if (facing <= 0.0)
        return s;
    float area = 4.0 * lamp.beam_edge.z * lamp.beam_edge.w;
    s.solid_angle_pdf = to2 / (area * facing);
    if (!restir_lights(lamp, surface.world.y, surface.region, surface.other))
        return s;
    // Let go between 80 % and all of its reach from its middle, as the light lists let it go.
    float window = saturate(1.0 - pow(sqrt(dist2) / reach, 4.0));
    s.radiance = lamp.colour_reach.rgb / area * (window * window);
    return s;
}

// A point of `lamp` seen from `surface`, by `uv`: of its sphere, uniform over the cone the sphere
// fills; of a flat piece's rectangle, uniform in area (`restir_flat_sample`).
RAB_LightSample RAB_SamplePolymorphicLight(RAB_LightInfo lamp, RAB_Surface surface, float2 uv)
{
    if (lamp_flat(lamp))
        return restir_flat_sample(lamp, surface, uv);
    RAB_LightSample s = RAB_EmptyLightSample();
    float radius = lamp.position_radius.w;
    float3 d = lamp.position_radius.xyz - surface.world;
    float dist2 = dot(d, d);
    float reach = lamp.colour_reach.w;
    if (radius <= 0.0 || dist2 <= radius * radius * 1.0001 || dist2 >= reach * reach)
        return s;
    float dist = sqrt(dist2);
    float3 w = d / dist;
    float sin2 = radius * radius / dist2;
    float cos_max = sqrt(1.0 - sin2);
    // 1 − cos θmax without the cancellation of two numbers near 1.
    float cap = sin2 / (1.0 + cos_max);
    float cos_t = 1.0 - uv.x * cap;
    float sin_t = sqrt(max(0.0, 1.0 - cos_t * cos_t));
    float phi = 6.28318530718 * uv.y;
    float3 t;
    float3 b;
    lamp_disc_axes(w, t, b);
    float3 dir = (t * cos(phi) + b * sin(phi)) * sin_t + w * cos_t;
    // Where the direction enters the sphere.
    float along = dot(dir, d);
    float into = along - sqrt(max(radius * radius - (dist2 - along * along), 0.0));
    s.position = surface.world + dir * into;
    // Its ray ends where the sphere is nearest, as the light lists' do (`lamp_ray`): run on to a
    // point by the sphere's edge, it went into the fitting round the bulb (a lantern's frame and
    // glass) and met it, and the Bistro's night stood 8 % under the lists without the reuse.
    s.short_of = into - (dist - radius);
    s.solid_angle_pdf = 1.0 / (6.28318530718 * cap);
    if (!restir_lights(lamp, surface.world.y, surface.region, surface.other))
        return s;
    // A bulb of intensity I (cd) and radius r has the luminance I / (π r²); let go between 80 %
    // and all of its reach (`lamp_add`).
    float window = saturate(1.0 - pow(dist / reach, 4.0));
    s.radiance = lamp.colour_reach.rgb / (3.14159265359 * radius * radius) * (window * window * lamp_beam(lamp, w));
    return s;
}

void RAB_GetLightDirDistance(RAB_Surface surface, RAB_LightSample s, out float3 dir, out float dist)
{
    float3 to = s.position - surface.world;
    dist = length(to);
    dir = to / max(dist, 1e-6);
}

float RAB_LightSampleSolidAnglePdf(RAB_LightSample s) { return s.solid_angle_pdf; }
bool RAB_IsAnalyticLightSample(RAB_LightSample s) { return true; }

// The lamp's light on the surface over the sample's density: its illuminance there, in lux, if
// every sample were this one. Its luminance is the resampling's target.
float3 restir_light(RAB_LightSample s, RAB_Surface surface)
{
    if (s.solid_angle_pdf <= 0.0)
        return float3(0.0);
    float3 dir;
    float dist;
    RAB_GetLightDirDistance(surface, s, dir, dist);
    return s.radiance * (max(dot(surface.normal, dir), 0.0) / s.solid_angle_pdf);
}

float RAB_GetLightSampleTargetPdfForSurface(RAB_LightSample s, RAB_Surface surface)
{
    return dot(restir_light(s, surface), float3(0.2126, 0.7152, 0.0722));
}

// The lamps' light picks no candidate by BRDF and no environment: these are never asked for.
float RAB_GetLightTargetPdfForVolume(RAB_LightInfo light, float3 center, float radius) { return 0.0; }
float RAB_EvaluateLocalLightSourcePdf(uint index) { return 0.0; }
float RAB_SurfaceEvaluateBrdfPdf(RAB_Surface s, float3 dir) { return 0.0; }
bool RAB_SurfaceImportanceSampleBrdf(RAB_Surface s, inout RTXDI_RandomSamplerState rng, out float3 dir)
{
    dir = s.normal;
    return false;
}
bool RAB_TraceRayForLocalLight(float3 origin, float3 dir, float t_min, float t_max, out uint light_index, out float2 rand_xy)
{
    light_index = RTXDI_InvalidLightIndex;
    rand_xy = float2(0.0);
    return false;
}
float2 RAB_GetEnvironmentMapRandXYFromDir(float3 dir) { return float2(0.0); }
float RAB_EvaluateEnvironmentMapSamplingPdf(float3 dir) { return 0.0; }

// Whether the sample's light reaches the surface: one shadow ray, ending short of the bulb (where
// its sphere is nearest) or 2 cm short of a flat piece's rectangle.
bool RAB_GetConservativeVisibility(RAB_Surface surface, RAB_LightSample s)
{
    Frame* f = restir_push.frame;
    if (f[0].tlas == 0)
        return true;
    RayDesc ray;
    ray.Origin = surface.world + surface.normal * surface.bias;
    float3 to = s.position - ray.Origin;
    float length_to = length(to);
    ray.Direction = to / max(length_to, 1e-6);
    ray.TMin = 0.0;
    ray.TMax = max(length_to - s.short_of - 0.02, 0.0);
    return !ray_blocked(f, ray);
}

// Whether a sample lights a neighbour of the last frame, for the reuse's `RayTraced` bias
// correction: RTXDI passes the neighbour's surface first (the pixel's own for its reprojected
// one), whose shadow ray starts 2 cm off it (`RAB_GetGBufferSurface` keeps no bias).
bool RAB_GetTemporalConservativeVisibility(RAB_Surface current, RAB_Surface previous, RAB_LightSample s)
{
    return RAB_GetConservativeVisibility(current, s);
}

#include "Rtxdi/DI/SpatioTemporalResampling.hlsli"

// ---------------------------------------------------------------- the pass ---

// The pixel's initial reservoir: `candidates` points of the lamps of its place, one kept by its
// light (RIS), its shadow ray traced. Where the place has more lamps, each candidate a lamp picked
// uniformly; where it has no more, the lamps in turn, each its share of the candidates (#372): with
// one candidate a lamp, a room's single lamp (the Cornell box's) gave each pixel one point of it a
// frame, the reuse held a few points over a region, and its shadows showed their edges and a
// wall lit inside its block's shadow.
RTXDI_DIReservoir restir_initial(Frame* f, RestirFrame* r, LampPlace place, RAB_Surface surface, inout RTXDI_RandomSamplerState rng, out RAB_LightSample chosen)
{
    RTXDI_DIReservoir state = RTXDI_EmptyDIReservoir();
    chosen = RAB_EmptyLightSample();
    uint total = 0u;
    for (uint k = 0u; k < 3u; k++)
        total += place.runs.run[k].y - place.runs.run[k].x;
    if (total == 0u)
        return state;
    bool every = total <= r->candidates;
    uint count = r->candidates;
    for (uint c = 0u; c < count; c++)
    {
        // Entry `e` of the place's three runs, laid end to end; in turn, the entries before
        // `count % total` take one candidate more, and a candidate's density is its lamp's share.
        uint e = every ? c % total : min(uint(RTXDI_GetNextRandom(rng) * float(total)), total - 1u);
        float inverse_density = every ? float(count) / float(count / total + (e < count % total ? 1u : 0u)) : float(total);
        uint run = 0u;
        while (e >= place.runs.run[run].y - place.runs.run[run].x)
        {
            e -= place.runs.run[run].y - place.runs.run[run].x;
            run++;
        }
        uint i = lamp_at(f, place.runs, place.runs.run[run].x + e);
        float2 uv = float2(RTXDI_GetNextRandom(rng), RTXDI_GetNextRandom(rng));
        float target = 0.0;
        RAB_LightSample s = RAB_EmptyLightSample();
        // A lamp counted under the room is not again under the doorway's other room.
        if (lamp_counted(f, place.runs, run, i))
        {
            s = RAB_SamplePolymorphicLight(f[0].lamps[i], surface, uv);
            target = RAB_GetLightSampleTargetPdfForSurface(s, surface);
        }
        if (RTXDI_StreamSample(state, i, uv, RTXDI_GetNextRandom(rng), target, inverse_density))
            chosen = s;
    }
    RTXDI_FinalizeResampling(state, 1.0, float(count));
    state.M = 1.0;
    if (RTXDI_IsValidDIReservoir(state) && !RAB_GetConservativeVisibility(surface, chosen))
        RTXDI_StoreVisibilityInDIReservoir(state, float3(0.0), true);
    return state;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void lamp_restir_main(uint3 id : SV_DispatchThreadID)
{
    RestirPush p = restir_push;
    if (id.x >= p.width || id.y >= p.height)
        return;
    uint2 pixel = id.xy;
    Frame* f = p.frame;
    RestirFrame* r = p.restir;
    RWTexture2D<float4> light = g_storage_images_rgba16f[NonUniformResourceIndex(p.light_image)];
    RWTexture2D<float4> diffuse = g_storage_images_rgba16f[NonUniformResourceIndex(p.diffuse_image)];
    RWTexture2D<float4> specular = g_storage_images_rgba16f[NonUniformResourceIndex(p.specular_image)];
    RWTexture2D<float4> guide = g_storage_images_rgba16f[NonUniformResourceIndex(p.guide_image)];
    uint at = r->current * r->surface_pitch + pixel.y * r->surface_width + pixel.x;
    uint vis = g_sampled_images_u32[NonUniformResourceIndex(p.vis_image)].Load(int3(pixel, 0));
    if (vis == VISIBILITY_EMPTY || f[0].lamp_count == 0u)
    {
        r->places[at] = RESTIR_EMPTY;
        RTXDI_StoreDIReservoir(RTXDI_EmptyDIReservoir(), r->reservoir_params, pixel, r->current);
        light[pixel] = float4(0.0);
        diffuse[pixel] = float4(0.0, 0.0, 0.0, 1.0);
        specular[pixel] = float4(0.0, 0.0, 0.0, 1.0);
        guide[pixel] = float4(0.0, 1.0, 0.0, 1.0);
        return;
    }
    PixelSurface ps = pixel_surface(f, vis, pixel, p.width, p.height);
    float3 world = to_scene(f, ps.relative);
    LampPlace place = lamp_place(f, p.probes, world, ps.n, normalize(-ps.relative));
    RAB_Surface surface;
    surface.world = world;
    surface.normal = ps.n;
    surface.bias = is_grass(vis) ? GRASS_SHADOW_BIAS : shadow_bias(f, ps.inst, ps.m);
    surface.region = place.region;
    surface.other = place.other;
    surface.valid = 1u;
    float4 clip = restir_clip(f, r, world);
    surface.depth = clip.w;
    // This frame's surface, for the next frame's reuse.
    float2 packed = round(saturate(oct_encode(ps.n) * 0.5 + 0.5) * 65535.0);
    r->surfaces[at] = uint4(asuint(world), uint(packed.x) | (uint(packed.y) << 16));
    uint region = place.region == REGION_NONE ? RESTIR_NO_REGION : min(place.region, RESTIR_NO_REGION - 1u);
    uint other = place.other == REGION_NONE ? RESTIR_NO_REGION : min(place.other, RESTIR_NO_REGION - 1u);
    r->places[at] = region | (other << 16);
    // The pixel's material as the resolve lights it (#372; `pixel_shading`, `standard_inputs`, the
    // lamps' inputs in `shade_pixel`): the diffuse light by its mapped normal, the highlight by its
    // lobe and reflectance, as the lists' lamps take them (D-086). Only where every lamp is
    // ReSTIR's, whose light REBLUR denoises: the street lamps' (`outside_only`) is not.
    float3 shading_n = ps.n;
    float3 view = normalize(-ps.relative);
    float power = 1.0;
    float3 f0 = float3(0.04);
    float roughness = 1.0;
    if (r->outside_only == 0u)
    {
        PixelShading shading = pixel_shading(f, MATERIAL_CLASS_ANY, pixel, vis, float2(float(p.width), float(p.height)));
        shading.s.ao = -1.0;
        StandardInputs si = shading.mat.class_index == MATERIAL_CLASS_STANDARD ? standard_inputs(f, shading.mat, shading.s) : row_inputs(shading.mat, shading.s);
        shading_n = si.li.n;
        view = shading.s.view;
        power = si.li.power;
        f0 = si.li.reflectance;
        roughness = sqrt(sqrt(2.0 / (power + 2.0))); // RenderLayer::roughness_for_power, inverted
    }
    guide[pixel] = float4(shading_n, roughness);

    // The street lamps alone: their run of the light lists (the outside's region) for every
    // pixel, and none for a room's, which no street lamp lights (`lamp_lights`).
    LampPlace candidates = place;
    if (r->outside_only != 0u)
    {
        bool reached = place.region == REGION_NONE || place.region == REGION_OUTSIDE || place.other == REGION_OUTSIDE;
        if (!reached || f[0].lamp_lists == nullptr)
        {
            // No neighbour of the next frame's: an empty reservoir counts among the pairwise
            // reuse's neighbours but gives it nothing, which darkens the pixels it lit (#372).
            r->places[at] = RESTIR_EMPTY;
            RTXDI_StoreDIReservoir(RTXDI_EmptyDIReservoir(), r->reservoir_params, pixel, r->current);
            light[pixel] = float4(0.0);
            diffuse[pixel] = float4(0.0, 0.0, 0.0, 1.0);
            specular[pixel] = float4(0.0, 0.0, 0.0, 1.0);
            return;
        }
        candidates.runs.run[0] = lamp_run(f, REGION_OUTSIDE);
        candidates.runs.run[1] = uint2(0u, 0u);
        candidates.runs.run[2] = uint2(0u, 0u);
        candidates.runs.direct = false;
    }

    RTXDI_RandomSamplerState rng = RTXDI_InitRandomSampler(pixel, r->runtime.frameIndex, 1u);
    RAB_LightSample chosen;
    RTXDI_DIReservoir state = restir_initial(f, r, candidates, surface, rng, chosen);
    if (r->history != 0u)
    {
        // Where the pixel was a frame ago (TAA's reprojection), and how much nearer.
        float4 before = mul(r->previous_from_current, clip);
        float3 motion = float3(restir_pixel(before, p.width, p.height) - restir_pixel(clip, p.width, p.height), before.w - clip.w);
        int2 temporal_pixel;
        RTXDI_DIReservoir initial = state;
        RAB_LightSample initial_chosen = chosen;
        state = RTXDI_DISpatioTemporalResampling(pixel, surface, state, rng, motion, 1u - r->current, r->runtime, r->reservoir_params, r->spatiotemporal, temporal_pixel, chosen);
        // Where the pairwise reuse (once the bias correction) found no neighbour (a disocclusion, the screen's edge as the
        // camera turns), RTXDI's spatiotemporal pass leaves the pixel's own sample a weight of 0
        // (its spatial pass sets it to 1): the pixel's own reservoir then, not none, which went
        // black and darkened the frames after (#372's review).
        if (!RTXDI_IsValidDIReservoir(state) && RTXDI_IsValidDIReservoir(initial))
        {
            state = initial;
            chosen = initial_chosen;
        }
    }
    float3 lux = float3(0.0);
    float3 diffuse_lux = float3(0.0);
    float3 specular_lux = float3(0.0);
    float lamp = 0.0;
    float reach = 1.0;
    float specular_reach = 1.0;
    if (RTXDI_IsValidDIReservoir(state))
    {
        bool seen = RAB_GetConservativeVisibility(surface, chosen);
        lux = seen ? restir_light(chosen, surface) * RTXDI_GetDIReservoirInvPdf(state) : float3(0.0);
        // An occluded sample stays in its reservoir (#372): emptied, the reservoir kept its count and
        // gave its neighbours nothing for it, though its other candidates might have lit them, and
        // the reuse over three neighbours darkened the Bistro's night 6 %.
        RTXDI_StoreVisibilityInDIReservoir(state, seen ? float3(1.0) : float3(0.0), false);
        uint index = RTXDI_GetDIReservoirLightIndex(state);
        lamp = float(index + 1u);
        float3 dir;
        float dist;
        RAB_GetLightDirDistance(surface, chosen, dir, dist);
        if (seen && chosen.solid_angle_pdf > 0.0)
        {
            // The sample's light at normal incidence, lux, and by the shading normal its diffuse
            // light; its highlight as `lamp_add`'s, towards the lamp's middle with the lobe widened
            // by its size (towards the sampled point as well, the size counted twice: broader and
            // dimmer than the lists'), bounded as the light is, then over the surface's
            // directional albedo (the resolve multiplies it back, `lamps_at`).
            float3 normal_light = chosen.radiance / chosen.solid_angle_pdf * RTXDI_GetDIReservoirInvPdf(state);
            diffuse_lux = normal_light * saturate(dot(shading_n, dir));
            Lamp lamp = f[0].lamps[index];
            float3 to_middle = lamp.position_radius.xyz - world;
            float middle = length(to_middle);
            float size = 0.5 * min(lamp.position_radius.w / max(middle, 1e-3), 1.0);
            float3 highlight = ggx_light(shading_n, view, to_middle / max(middle, 1e-6), ggx_alpha2(power, size * size), f0) * normal_light;
            specular_lux = min(highlight, float3(65000.0)) / restir_specular_albedo(f, f0, power, shading_n, view);
        }
        // The distance to the light's point, as NRD asks for direct light.
        reach = reblur_hit_distance(dist, clip.w, p.hit_distance.xyz);
        specular_reach = reblur_specular_hit_distance(dist, clip.w, p.hit_distance.xyz, roughness);
    }
    // A reservoir of no sample (a place without lamps) is no neighbour of the next frame's: the
    // pairwise reuse counts it and takes nothing from it.
    if (state.M <= 0.0)
        r->places[at] = RESTIR_EMPTY;
    RTXDI_StoreDIReservoir(state, r->reservoir_params, pixel, r->current);
    // Half precision's largest value bounds it: a pixel by a bulb is lit by thousands of lux.
    lux = min(lux, float3(65000.0));
    light[pixel] = float4(lux, lamp);
    diffuse[pixel] = float4(linear_to_ycocg(min(diffuse_lux, float3(65000.0)) / RESTIR_DENOISE_SCALE), reach);
    // Half precision bounds the divided highlight too (0.02's albedo: 50 times the light's bound).
    specular[pixel] = float4(linear_to_ycocg(min(specular_lux / RESTIR_DENOISE_SCALE, float3(65000.0))), specular_reach);
}

// A distance as REBLUR's specular reads it (NRD's `REBLUR_FrontEnd_GetNormHitDist` for a roughness):
// over its normalisation, which grows towards `params.z` times as the lobe narrows.
float reblur_specular_hit_distance(float length, float view_z, float3 params, float roughness)
{
    float curve = (1.0 - exp2(-200.0 * roughness * roughness)) * sqrt(saturate(roughness));
    return saturate(length / ((params.x + abs(view_z) * params.y) * lerp(params.z, 1.0, curve)));
}
