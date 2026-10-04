// The cloud layer (Phase 4, issue #145), after Schneider's 2015 cloudscapes for Horizon Zero
// Dawn (docs/research/lighting-gi.md §6): a layer between two shells over the planet, its
// coverage from a weather map that drifts with the wind, its shapes from a Perlin–Worley noise
// eroded by a finer Worley noise, both tiling volumes laid out as 2-D atlases (the bindless set
// holds 2-D images). Ray-marched at half resolution, each pixel's start jittered by a
// spatio-temporal blue noise, lit by the sun through the cloud above each sample (Beer's law
// with the powder term) with a two-lobed phase, and by the sky's irradiance; accumulated over
// frames against the clouds as they were (reprojected by the camera's turn: they are far), so
// the jitter settles instead of shimmering. `sky/compose` lays the result over the sky.

import bindless;
import atmosphere;
import skyframe;
import noise;
import sh;
import skyview;

// Mirrors `GpuClouds` in clouds.rs.
struct Clouds
{
    float4x4 previous;     // last frame's camera-relative view-projection (its turn only: the clouds are far)
    float4 camera;         // the camera's world x and z (metres), the weather's drift x and z (metres)
    float4 layer;          // the layer's bottom and top over the ground (m), the coverage (0..1), the density (m⁻¹)
    uint shape;            // sampled: the shape noise's atlas
    uint detail;           // sampled: the detail noise's atlas
    uint weather;          // sampled: the weather map
    uint history;          // sampled: last frame's clouds (or ~0: none yet)
    uint target;           // storage: this frame's clouds
    uint width;            // the clouds' size (half the frame's)
    uint height;
    uint frame;
    Sky* sky;
    uint table;            // storage: the sky-view table with the clouds over it (#163; its pass only)
    uint pad;
    float4 shadow;         // the shadow map's first corner (world x, z), metres a texel, 0 (its pass only)
};

struct CloudsPush
{
    Clouds* clouds;
};

[[vk::push_constant]]
CloudsPush clouds_push;

// The noises' atlases: SIDE³ texels in TILES × TILES tiles, each with a texel of border
// wrapped from the opposite side, so a tile filters without seams (`noise_atlas` in clouds.rs).
static const uint SHAPE_SIDE = 64;
static const uint SHAPE_TILES = 8;
static const uint DETAIL_SIDE = 32;
static const uint DETAIL_TILES = 8;
// Metres one repeat of each noise spans, and the weather map's.
static const float SHAPE_PERIOD = 4000.0;
static const float DETAIL_PERIOD = 700.0;
static const float WEATHER_PERIOD = 48000.0;
// The march: steps through the layer at most, how far it looks (m), the light's steps.
static const uint STEPS = 48;
static const float FARTHEST = 60000.0;
static const uint LIGHT_STEPS = 6;
static const float LIGHT_FIRST = 40.0; // metres, each step twice the last
static const float PI = 3.14159265;
// The share of the history kept a frame (the rest the new march).
static const float KEEP = 0.9;
// The light scattered many times, as a multiple of the once scattered: clouds are as bright as
// they are for the light that bounces inside them (a stand-in for Wrenninge's octaves).
static const float MULTIPLE = 2.0;
// Octaves of the light scattered many times.
static const uint OCTAVES = 4;
// Fair weather (the owner, 2026-10-03: "whiter if no bad weather"): under a fair-weather cover
// the clouds are bright cumulus, their shaded sides and bases lit by the light the clouds round
// them and the ground send back. `fair` is 1 at a coverage of FAIR_COVER and under, 0 from
// OVERCAST; in between, and beyond it, the octaves and the ambient go back towards the overcast's
// grey. With it each octave sees less of the extinction and keeps more of the light, and the
// sky's light reaches the bases nearly whole and stronger.
static const float FAIR_COVER = 0.45;
static const float OVERCAST = 0.75;
static const float FAIR_EXTINCTION = 0.2;
static const float FAIR_OCTAVE = 0.7;
static const float FAIR_BASE = 0.8;
static const float FAIR_AMBIENT = 1.6;

// A tiling volume in an atlas at `p` (in its repeats), trilinear: two slices, each bilinear
// within its tile.
float4 sample_atlas(uint image, uint side, uint tiles, float3 p)
{
    float3 q = frac(p) * float(side);
    float s = q.z - 0.5;
    float z0 = floor(s);
    float t = s - z0;
    uint a = uint(z0 + float(side)) % side;
    uint b = (a + 1u) % side;
    float bordered = float(side + 2u);
    float atlas = bordered * float(tiles);
    float2 within = q.xy + 1.0; // past the border texel
    float2 ua = (float2(float(a % tiles), float(a / tiles)) * bordered + within) / atlas;
    float2 ub = (float2(float(b % tiles), float(b / tiles)) * bordered + within) / atlas;
    float4 va = sample_image_level(image, SAMPLER_LINEAR_CLAMP, ua, 0.0);
    float4 vb = sample_image_level(image, SAMPLER_LINEAR_CLAMP, ub, 0.0);
    return lerp(va, vb, t);
}

float remap(float v, float lo, float hi, float new_lo, float new_hi)
{
    return new_lo + (v - lo) / max(hi - lo, 1e-5) * (new_hi - new_lo);
}

// The cloud's density at `world` (metres; y over the ground), `h` its height in the layer
// (0..1); `detailed` erodes its edges with the fine noise.
float density_at(Clouds* c, float3 world, float h, bool detailed)
{
    float2 drift = c[0].camera.zw;
    float2 w = sample_image_level(c[0].weather, SAMPLER_LINEAR_REPEAT, (world.xz + drift) / WEATHER_PERIOD, 0.0).rg;
    float coverage = saturate(remap(w.r, 1.0 - c[0].layer.z, 1.0, 0.0, 1.0));
    if (coverage <= 0.0)
        return 0.0;
    // Cumulus: rounded bottoms, thinning tops, taller where the map says (its second channel).
    float top = lerp(0.55, 1.0, w.g);
    float gradient = smoothstep(0.0, 0.08, h) * (1.0 - smoothstep(top * 0.6, top, h));
    float4 n = sample_atlas(c[0].shape, SHAPE_SIDE, SHAPE_TILES, (world + float3(drift.x, 0.0, drift.y) * 0.5) / SHAPE_PERIOD);
    float fbm = n.g * 0.625 + n.b * 0.25 + n.a * 0.125;
    float base = remap(n.r, fbm - 1.0, 1.0, 0.0, 1.0) * gradient;
    base = saturate(remap(base, 1.0 - coverage, 1.0, 0.0, 1.0)) * coverage;
    if (base <= 0.0 || !detailed)
        return base;
    float3 d = sample_atlas(c[0].detail, DETAIL_SIDE, DETAIL_TILES, world / DETAIL_PERIOD).rgb;
    float fine = d.r * 0.625 + d.g * 0.25 + d.b * 0.125;
    // Wispy at the bottom, billowing higher up.
    float erosion = lerp(fine, 1.0 - fine, saturate(h * 4.0));
    return saturate(remap(base, erosion * 0.45, 1.0, 0.0, 1.0));
}

// Henyey–Greenstein's phase function.
float hg(float cos_theta, float g)
{
    float g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5));
}

// Where a ray from `o` (relative to the planet's centre, metres) along `d` leaves the sphere of
// radius `r` (−1 when it does not meet it), and where it first enters it (may be behind).
float2 sphere(float3 o, float3 d, float r)
{
    float b = dot(o, d);
    float c = dot(o, o) - r * r;
    float disc = b * b - c;
    if (disc < 0.0)
        return float2(-1.0, -1.0);
    float s = sqrt(disc);
    return float2(-b - s, -b + s);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void march_main(uint3 id : SV_DispatchThreadID)
{
    Clouds* c = clouds_push.clouds;
    if (id.x >= c[0].width || id.y >= c[0].height)
        return;
    Sky* s = c[0].sky;
    float2 uv = (float2(id.xy) + 0.5) / float2(c[0].width, c[0].height);
    float3 d = view_direction(s, uv);
    float4 result = march(c, d, spatio_temporal_noise(id.xy, c[0].frame));
    // With the clouds as they were where this direction looked last frame.
    if (c[0].history != 0xFFFFFFFFu)
    {
        float4 clip = mul(c[0].previous, float4(d, 0.0));
        float2 last = float2(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        if (clip.w > 0.0 && all(last >= 0.0) && all(last <= 1.0))
        {
            float4 before = sample_image_level(c[0].history, SAMPLER_LINEAR_CLAMP, last, 0.0);
            result = lerp(result, before, KEEP);
        }
    }
    g_storage_images_rgba16f[NonUniformResourceIndex(c[0].target)][id.xy] = result;
}

// The clouds over the sky-view table (#163): along each of its directions, the clear table's
// light through the clouds plus their own, in the table's units (per unit of sun illuminance):
// what `irradiance_main` projects for the sky's light with the clouds in it. Marched once from
// each texel's middle, with no history: the projection smooths what a jitter would.
[shader("compute")]
[numthreads(8, 8, 1)]
void sky_table_main(uint3 id : SV_DispatchThreadID)
{
    Clouds* c = clouds_push.clouds;
    if (id.x >= SKY_VIEW_WIDTH || id.y >= SKY_VIEW_HEIGHT)
        return;
    Sky* s = c[0].sky;
    float3 up = normalize(s[0].planet[0].view_position.xyz);
    float3 forward, right;
    sky_view_frame(up, s[0].sun.xyz, forward, right);
    float2 uv = (float2(id.xy) + 0.5) / float2(SKY_VIEW_WIDTH, SKY_VIEW_HEIGHT);
    float4 clouds = march(c, sky_view_direction(uv, up, forward, right), float2(0.5, 0.5));
    float3 sky = g_sampled_images[NonUniformResourceIndex(s[0].sky_view)].Load(int3(id.xy, 0)).rgb;
    float3 seen = sky * clouds.a + clouds.rgb / max(s[0].scale.x, 1e-20);
    g_storage_images_rgba16f[NonUniformResourceIndex(c[0].table)][id.xy] = float4(seen, 1.0);
}

// The clouds' shadow (#145): for the ground under each texel of a square round the camera, the
// sun's share through the layer along its direction, the clouds sampled without their fine
// detail. Softened as the octaves soften the clouds' own light, and never quite dark (light
// still reaches the ground round and through a cloud).
static const uint SHADOW_STEPS = 16;
static const float SHADOW_SOFTEN = 0.3;
static const float SHADOW_STRENGTH = 0.85;

[shader("compute")]
[numthreads(8, 8, 1)]
void shadow_main(uint3 id : SV_DispatchThreadID)
{
    Clouds* c = clouds_push.clouds;
    if (id.x >= c[0].width || id.y >= c[0].height)
        return;
    float3 sun = c[0].sky[0].sun.xyz;
    float lit = 1.0;
    if (sun.y > 0.03)
    {
        float2 ground = c[0].shadow.xy + (float2(id.xy) + 0.5) * c[0].shadow.z;
        float t0 = c[0].layer.x / sun.y;
        float t1 = c[0].layer.y / sun.y;
        float step = (t1 - t0) / float(SHADOW_STEPS);
        float optical = 0.0;
        for (uint i = 0; i < SHADOW_STEPS; i++)
        {
            float t = t0 + (float(i) + 0.5) * step;
            float3 p = float3(ground.x + sun.x * t, sun.y * t, ground.y + sun.z * t);
            float h = saturate((p.y - c[0].layer.x) / (c[0].layer.y - c[0].layer.x));
            optical += density_at(c, p, h, false) * step;
        }
        lit = lerp(1.0, exp(-c[0].layer.w * optical * SHADOW_SOFTEN), SHADOW_STRENGTH);
    }
    g_storage_images_r32f[NonUniformResourceIndex(c[0].target)][id.xy] = lit;
}

// The clouds along `d` from the camera: their light (pre-exposed: the sky's units times its
// `scale.x`) and the share of what lies behind them that gets through, the march's start
// jittered by `jitter` (in [0, 1)).
float4 march(Clouds* c, float3 d, float2 jitter)
{
    Sky* s = c[0].sky;
    Planet p = s[0].planet[0];
    float3 sun = s[0].sun.xyz;
    // The planet's frame in metres: the camera over its centre.
    float ground = p.radii.x * 1000.0;
    float3 up = normalize(p.view_position.xyz);
    float altitude = length(p.view_position.xyz) * 1000.0 - ground;
    float3 o = float3(0.0, ground + altitude, 0.0);
    // The local frame's y is the planet's up at the camera: world y.
    float bottom = ground + c[0].layer.x;
    float top = ground + c[0].layer.y;
    float4 result = float4(0.0, 0.0, 0.0, 1.0);
    float2 ground_hit = sphere(o, d, ground);
    float2 inner = sphere(o, d, bottom);
    float2 outer = sphere(o, d, top);
    float start = altitude < c[0].layer.x ? inner.y : max(outer.x, 0.0);
    float end = altitude < c[0].layer.x ? outer.y : (inner.x > 0.0 ? inner.x : outer.y);
    bool blocked = ground_hit.x > 0.0 && ground_hit.x < start;
    end = min(end, start + FARTHEST);
    if (!blocked && end > start && start >= 0.0)
    {
        float step = (end - start) / float(STEPS);
        float cos_theta = dot(d, sun);
        // The light on the layer: the key light (the sun, or at night the Moon, D-046) through
        // the air at its middle, the sky's from above.
        float3 middle_km = p.view_position.xyz + up * (0.5 * (c[0].layer.x + c[0].layer.y) - altitude) * 0.001;
        float3 sunlight = sunlight_at(p, s[0].transmittance, middle_km, sun) * (s[0].scale.x * s[0].scale.w);
        float3 ambient = sh_irradiance(s[0].irradiance, up) * (s[0].scale.x / PI);
        float sigma = c[0].layer.w;
        float fair = saturate((OVERCAST - c[0].layer.z) / (OVERCAST - FAIR_COVER));
        float t = start + jitter.x * step;
        float3 luminance = 0.0;
        float transmittance = 1.0;
        for (uint i = 0; i < STEPS && transmittance > 0.01; i++)
        {
            float3 at = o + d * t;
            float r = length(at);
            float h = saturate((r - bottom) / (top - bottom));
            // World x and z: the camera's, and the ray's along them (the local frame is flat
            // over the few tens of kilometres the march sees).
            float3 world = float3(c[0].camera.x + d.x * t, r - ground, c[0].camera.y + d.z * t);
            float rho = density_at(c, world, h, true);
            if (rho > 0.0)
            {
                // The cloud between the sample and the sun.
                float optical = 0.0;
                float along = LIGHT_FIRST;
                float reach = 0.0;
                for (uint k = 0; k < LIGHT_STEPS; k++)
                {
                    reach += along;
                    float3 lit = at + sun * reach;
                    float hl = saturate((length(lit) - bottom) / (top - bottom));
                    float3 lw = float3(c[0].camera.x + lit.x, length(lit) - ground, c[0].camera.y + lit.z);
                    optical += density_at(c, lw, hl, false) * along;
                    along *= 2.0;
                }
                // The sun through the cloud above, scattered many times on the way (Wrenninge's
                // octaves, as Hillaire's Frostbite clouds sum them: each octave sees less of
                // the extinction and a rounder phase, so light reaches deep into the cloud),
                // darkened a little where the cloud is thin towards the sun (the powder term).
                float lit = 0.0;
                float a = 1.0, b = 1.0, g = 1.0;
                for (uint octave = 0; octave < OCTAVES; octave++)
                {
                    float ph = lerp(hg(cos_theta, -0.25 * g), hg(cos_theta, 0.75 * g), 0.7);
                    lit += b * exp(-a * sigma * optical) * ph;
                    a *= lerp(0.3, FAIR_EXTINCTION, fair);
                    b *= lerp(0.6, FAIR_OCTAVE, fair);
                    g *= 0.5;
                }
                float powder = 1.0 - exp(-2.0 * sigma * optical - 0.05);
                float3 source = sunlight * (lit * lerp(1.0, powder, 0.3) * MULTIPLE)
                    + ambient * lerp(lerp(0.45, FAIR_BASE, fair), 1.0, h) * lerp(1.0, FAIR_AMBIENT, fair);
                float extinction = sigma * rho;
                float segment = exp(-extinction * step);
                luminance += transmittance * source * (1.0 - segment);
                transmittance *= segment;
            }
            t += step;
        }
        // Far clouds fade into the sky's haze.
        float haze = exp(-start / 30000.0);
        result = float4(luminance * haze, lerp(1.0, transmittance, haze));
    }
    return result;
}
