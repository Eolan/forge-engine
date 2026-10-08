// Splashes (#107): the spray thrown up where things meet the water, as ballistic particles
// (`docs/research/water.md` §7; Chentanez & Müller 2010's spray, without the solver). Driven by
// crates/forge-render/src/splashes.rs.
//
// The particles live in a ring of slots the CPU hands out in blocks, in order, so the draw's
// order is the same from frame to frame and nothing flickers where they overlap. Each frame:
// - `splashes/emit` (compute queue): the frame's new particles, from the blocks the CPU wrote;
//   each particle's random numbers come from its emitter's seed and its index in the emitter's
//   stream, so the same stream gives the same drops whatever the frame rate;
// - `splashes/advance` (compute queue): gravity and a quadratic drag towards the wind, to this
//   frame's time; a drop dies where it falls back into its water;
// - `splashes/draw`: each live particle a soft sprite, stretched along its velocity over half
//   a frame, at least a pixel wide with its alpha scaled down by the area it lacks (Persson's
//   phone-wire AA), lit by the sun through a shadow ray and by the sky, hazed by the aerial
//   perspective, faded where it meets the scene; it also writes the reactive mask TAA reads
//   (`taa.slang`), so the spray relies less on the history it would smear.
// Visual only: nothing reads the particles back.

import bindless;
import skyframe;
import sh;

static const float PI = 3.14159265358979;
static const float GRAVITY = 9.81;
// The kinds of block: drops on a ring round the origin (an impact's crown), or in a cone round
// an axis from a line through the origin (a fall's foot, a bow, a jet, drips). MIST and STILL
// are flags; STILL: its drops land on still water, where they make rings.
static const uint SPLASH_RING = 0u;
static const uint SPLASH_CONE = 1u;
static const uint SPLASH_MIST = 2u;
static const uint SPLASH_STILL = 4u;
// A flag: grains of the ground a slipping wheel throws (#192), their colour in their flags'
// bits 16–31 (5:6:5, of its square root); lit as solid lumps, gone where they land.
static const uint SPLASH_GRAIN = 8u;
// A grain's terminal speed, m/s: a clump of sand or a clod of mud falls nearly as in a vacuum
// over the metre or two it flies.
static const float GRAIN_TERMINAL = 15.0;
// Terminal speeds, m/s: the drag k = g / v_t² pulls a particle towards the air's velocity.
static const float DROP_TERMINAL = 8.0;
static const float MIST_TERMINAL = 0.6;
// Mist rises with the air the fall drags down and lets go, m/s.
static const float MIST_LIFT = 0.8;
// The longest step of the integration, s, and the longest span it covers in a frame.
static const float MAX_STEP = 1.0 / 60.0;
static const float MAX_SPAN = 0.1;
// Seconds a particle takes to fade in; it fades out over the last quarter of its life.
static const float FADE_IN = 0.04;
// The most a splash asks of TAA's current frame (FSR 2's cap for its reactive mask).
static const float REACTIVE = 0.9;
// The foam a landing drop leaves in its cell, in the field's units (`FOAM_FULL` in water.slang
// whitens a cell fully).
static const uint FOAM_PER_DROP = 128;
// How far over its water a drop that ends still lands there, metres.
static const float FOAM_REACH = 0.3;
// The impulse a drop landing on still water leaves in its cell, for the rings the wakes draw
// from it (`rings_main` in wakes.slang): its radius cubed times its speed, in these units.
static const float IMPULSE_UNIT = 1e-8;
// A drop's scattering: Henyey–Greenstein forwards, plus an even part for the light it
// scatters more than once inside a splash.
static const float FORWARD = 0.75;
static const float FORWARD_SHARE = 0.7;

struct SplashParticle
{
    float3 position; // the sea's frame, metres
    float birth;     // the sea's seconds
    float3 velocity; // m/s
    float life;      // seconds; 0 once it is dead
    float radius;    // drawn radius, metres
    float level;     // the water it falls back into, metres
    uint seed;
    uint flags;      // bit 1: mist; bit 2: still water; bits 8–15: its alpha (unorm8)
};

// Must match GpuSplashBlock in splashes.rs.
struct SplashBlock
{
    float4 origin;   // xyz: where its drops start; w: the water's level they fall back to
    float4 spread;   // xyz (cone): drops start along origin ± this; w (ring): the ring's radius
    float4 axis;     // xyz (cone): the launch direction; w: the cone's half-angle (cone) or the
                     // highest elevation (ring), radians
    float4 speed;    // xy: launch speeds, m/s; zw: drawn radii, metres
    float4 life;     // xy: lives, s; z: the first birth, the sea's seconds; w: seconds between births
    float4 carry;    // xyz: velocity added to every drop; w: the lowest elevation (ring), radians
    uint first;      // its first slot
    uint count;
    uint seed;       // its emitter's
    uint kind;       // SPLASH_RING or SPLASH_CONE, | SPLASH_MIST, | SPLASH_STILL
    uint start;      // its first particle among the frame's new ones
    uint index;      // its first particle's index in its emitter's stream
    float alpha;
    uint albedo;     // a grain's colour, 5:6:5 bits (SPLASH_GRAIN, #192)
};

// Must match GpuSplashFrame in splashes.rs.
struct SplashFrame
{
    float4x4 view_proj;  // camera-relative, jitter included
    float4 camera;       // xyz: the camera in the sea's frame; w: the near plane, metres
    float4 sun;          // xyz: towards the sun; w: the most pre-exposed luminance a drop shows
    float4 sun_radiance; // rgb: the sun's pre-exposed illuminance; w: the sky's scale
    float4 wind;         // xyz: the air's velocity, m/s; w: the shutter, s
    float now;           // the sea's seconds
    float previous;      // the last update's
    float focal;         // pixels a metre spans one metre away
    uint depth;          // the frame's depth (sampled)
    float width;
    float height;
    uint base;           // the first live slot
    uint live;           // live slots from it
    uint fresh;          // particles new this frame
    uint block_count;
    uint capacity;
    uint pad;
    uint64_t tlas;       // the scene's structure, for the sun's shadow (0: none)
    Sky* sky;
    SplashParticle* ring;
    SplashBlock* blocks;
    uint* foam;          // the foam field (#107's polish, splash_foam.slang), then its
                         // impulses, or null
    int2 foam_origin;    // its window's first world cell
    float foam_cell;     // metres a cell
    uint foam_cells;     // cells a side
    uint2 pad_foam;
};

struct SplashPush
{
    SplashFrame* frame;
};

[[vk::push_constant]]
SplashPush splash_push;

// Jarzynski & Olano 2020, "Hash Functions for GPU Rendering": three hashes of three integers.
uint3 pcg3d(uint3 v)
{
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> 16u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

float3 random3(uint3 v)
{
    return float3(pcg3d(v) >> 8u) * (1.0 / 16777216.0);
}

// A direction within `half_angle` of `axis`, uniform over the cap, from two numbers in 0..1.
float3 in_cone(float3 axis, float half_angle, float u, float v)
{
    float cos_theta = 1.0 - u * (1.0 - cos(half_angle));
    float sin_theta = sqrt(saturate(1.0 - cos_theta * cos_theta));
    float phi = 2.0 * PI * v;
    float3 t = normalize(cross(abs(axis.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0), axis));
    float3 b = cross(axis, t);
    return axis * cos_theta + (t * cos(phi) + b * sin(phi)) * sin_theta;
}

[shader("compute")]
[numthreads(64, 1, 1)]
void emit_main(uint3 id : SV_DispatchThreadID)
{
    SplashFrame* f = splash_push.frame;
    if (id.x >= f[0].fresh)
        return;
    // Its block: the last whose first particle is at or before it.
    uint lo = 0;
    uint hi = f[0].block_count;
    while (hi - lo > 1u)
    {
        uint mid = (lo + hi) / 2u;
        if (f[0].blocks[mid].start <= id.x)
            lo = mid;
        else
            hi = mid;
    }
    SplashBlock b = f[0].blocks[lo];
    uint j = id.x - b.start;
    uint k = b.index + j;
    float3 u = random3(uint3(b.seed, k, 0u));
    float3 w = random3(uint3(b.seed, k, 1u));
    float3 position;
    float3 direction;
    if ((b.kind & 1u) == SPLASH_RING)
    {
        float angle = 2.0 * PI * u.x;
        float3 radial = float3(cos(angle), 0.0, sin(angle));
        float elevation = lerp(b.carry.w, b.axis.w, u.y);
        position = b.origin.xyz + radial * b.spread.w;
        direction = radial * cos(elevation) + float3(0.0, 1.0, 0.0) * sin(elevation);
    }
    else
    {
        position = b.origin.xyz + b.spread.xyz * (2.0 * u.x - 1.0);
        direction = in_cone(b.axis.xyz, b.axis.w, u.y, u.z);
    }
    SplashParticle q;
    q.position = position;
    q.birth = b.life.z + b.life.w * float(j);
    q.velocity = direction * lerp(b.speed.x, b.speed.y, w.x) + b.carry.xyz;
    q.life = lerp(b.life.x, b.life.y, w.z);
    // More small drops than large ones.
    q.radius = lerp(b.speed.z, b.speed.w, w.y * w.y);
    q.level = b.origin.w;
    q.seed = pcg3d(uint3(b.seed, k, 2u)).x;
    q.flags = (b.kind & (SPLASH_MIST | SPLASH_STILL | SPLASH_GRAIN)) | (uint(saturate(b.alpha) * 255.0 + 0.5) << 8u)
        | ((b.albedo & 0xFFFFu) << 16u);
    f[0].ring[(b.first + j) % f[0].capacity] = q;
}

// A drop's foam where it lands (#107's polish): a whole number of units into its cell of the
// foam field, the field's slot the world cell modulo its size. On still water, its impulse
// too, into the same cell of the field's second half.
void deposit_foam(SplashFrame* f, SplashParticle q)
{
    if (f[0].foam == nullptr)
        return;
    int n = int(f[0].foam_cells);
    int2 cell = int2(floor(q.position.xz / f[0].foam_cell));
    int2 within = cell - f[0].foam_origin;
    if (any(within < 0) || any(within >= n))
        return;
    int2 slot = int2(((cell.x % n) + n) % n, ((cell.y % n) + n) % n);
    uint i = uint(slot.y * n + slot.x);
    uint ignored;
    InterlockedAdd(f[0].foam[i], FOAM_PER_DROP, ignored);
    if ((q.flags & SPLASH_STILL) != 0u)
    {
        uint impulse = uint(round(q.radius * q.radius * q.radius * length(q.velocity) / IMPULSE_UNIT));
        if (impulse > 0u)
            InterlockedAdd(f[0].foam[uint(n * n) + i], impulse, ignored);
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
void advance_main(uint3 id : SV_DispatchThreadID)
{
    SplashFrame* f = splash_push.frame;
    if (id.x >= f[0].live)
        return;
    uint slot = (f[0].base + id.x) % f[0].capacity;
    SplashParticle q = f[0].ring[slot];
    if (q.life <= 0.0)
        return;
    float now = f[0].now;
    if (now - q.birth > q.life)
    {
        // A drop ending just over its water lands there (a grain leaves no foam).
        if ((q.flags & (SPLASH_MIST | SPLASH_GRAIN)) == 0u && q.position.y < q.level + FOAM_REACH)
            deposit_foam(f, q);
        f[0].ring[slot].life = 0.0;
        return;
    }
    float span = min(now - max(q.birth, f[0].previous), MAX_SPAN);
    if (span <= 0.0)
        return;
    bool mist = (q.flags & SPLASH_MIST) != 0u;
    bool grain = (q.flags & SPLASH_GRAIN) != 0u;
    float terminal = mist ? MIST_TERMINAL : grain ? GRAIN_TERMINAL : DROP_TERMINAL;
    float k = GRAVITY / (terminal * terminal);
    float3 air = f[0].wind.xyz + (mist ? float3(0.0, MIST_LIFT, 0.0) : float3(0.0));
    uint steps = uint(ceil(span / MAX_STEP));
    float h = span / float(steps);
    for (uint i = 0u; i < steps; i++)
    {
        // Gravity, then the drag as a relaxation towards the air: stable at any step.
        float3 v = q.velocity + float3(0.0, -GRAVITY * h, 0.0);
        float3 slip = v - air;
        q.velocity = air + slip * exp(-k * length(slip) * h);
        q.position += q.velocity * h;
    }
    if (mist)
    {
        // Mist rides over its water, never into it.
        if (q.position.y < q.level)
        {
            q.position.y = q.level;
            q.velocity.y = max(q.velocity.y, 0.0);
        }
    }
    else if (q.position.y < q.level && q.velocity.y < 0.0)
    {
        // Back in its water, or a grain back on the ground.
        q.life = 0.0;
        if (!grain)
            deposit_foam(f, q);
    }
    f[0].ring[slot] = q;
}

// ---- The draw ---------------------------------------------------------------------------------

struct SplashVaryings
{
    float4 position : SV_Position;
    float3 color : COLOR;              // lit, pre-exposed and hazed
    float4 shape : TEXCOORD0;          // xy: the pixel from the streak's middle, along it and
                                       // across, in pixels; z: half its length; w: half its width
    nointerpolation float3 fade : TEXCOORD1; // x: its alpha before its shape; y: its distance,
                                             // metres; z: over how far it fades into the scene
};

// Henyey–Greenstein's phase at `cos_theta` between the light's way and the view's.
float henyey_greenstein(float cos_theta, float g)
{
    float d = 1.0 + g * g - 2.0 * g * cos_theta;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

// Whether the sun reaches `p` (the sea's frame, which is the scene's).
float sun_visibility(SplashFrame* f, float3 p)
{
    if (f[0].tlas == 0)
        return 1.0;
    RayDesc ray;
    ray.Origin = p;
    ray.Direction = f[0].sun.xyz;
    ray.TMin = 0.05;
    ray.TMax = 1e4;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER | RAY_FLAG_FORCE_OPAQUE> q;
    q.TraceRayInline(RaytracingAccelerationStructure(f[0].tlas), RAY_FLAG_NONE, 0xFF, ray);
    while (q.Proceed())
    {
    }
    return q.CommittedStatus() == COMMITTED_NOTHING ? 1.0 : 0.0;
}

[shader("vertex")]
SplashVaryings vert_main(uint vertex : SV_VertexID)
{
    SplashFrame* f = splash_push.frame;
    SplashVaryings o;
    // A dead slot: six times the same point, outside the view.
    o.position = float4(2.0, 2.0, 0.5, 1.0);
    o.color = float3(0.0);
    o.shape = float4(0.0);
    o.fade = float3(0.0);
    uint i = vertex / 6u;
    if (i >= f[0].live)
        return o;
    SplashParticle q = f[0].ring[(f[0].base + i) % f[0].capacity];
    float age = f[0].now - q.birth;
    if (q.life <= 0.0 || age < 0.0 || age > q.life)
        return o;
    float3 rel = q.position - f[0].camera.xyz;
    float4x4 view_proj = f[0].view_proj;
    float4 clip = mul(view_proj, float4(rel, 1.0));
    float near = f[0].camera.w;
    if (clip.w < near)
        return o;
    float2 half_size = 0.5 * float2(f[0].width, f[0].height);
    // The streak: where the drop is half the shutter either way.
    float3 travel = q.velocity * (0.5 * f[0].wind.w);
    float4 c0 = mul(view_proj, float4(rel - travel, 1.0));
    float4 c1 = mul(view_proj, float4(rel + travel, 1.0));
    float2 middle = clip.xy / clip.w * half_size;
    float2 p0 = middle;
    float2 p1 = middle;
    if (c0.w >= near && c1.w >= near)
    {
        p0 = c0.xy / c0.w * half_size;
        p1 = c1.xy / c1.w * half_size;
        middle = 0.5 * (p0 + p1);
    }
    float2 way = p1 - p0;
    float length_px = length(way);
    float2 along = length_px > 1e-3 ? way / length_px : float2(1.0, 0.0);
    float2 across = float2(-along.y, along.x);
    // At least a pixel across (a streak a pixel and a half), the alpha scaled by the area lost.
    float radius = q.radius * f[0].focal / clip.w;
    float width = max(radius, length_px > 1.0 ? 0.75 : 0.5);
    float coverage = (PI * radius * radius + 2.0 * radius * length_px) /
                     (PI * width * width + 2.0 * width * length_px);
    uint corner = vertex % 6u;
    float2 c = float2(corner == 1u || corner == 2u || corner == 4u ? 1.0 : -1.0,
                      corner == 2u || corner == 4u || corner == 5u ? 1.0 : -1.0);
    float half_length = 0.5 * length_px;
    float2 offset = c.x * (half_length + width) * along + c.y * width * across;
    o.position = float4((middle + offset) / half_size * clip.w, clip.z, clip.w);
    o.shape = float4(c.x * (half_length + width), c.y * width, half_length, width);

    bool mist = (q.flags & SPLASH_MIST) != 0u;
    float alpha = float((q.flags >> 8u) & 255u) / 255.0;
    float fade_in = saturate(age / FADE_IN);
    float fade_out = saturate((q.life - age) / (0.25 * q.life));
    o.fade = float3(alpha * coverage * fade_in * fade_out, clip.w, max(4.0 * q.radius, mist ? 0.5 : 0.05));

    // The light: the sun's, scattered towards the camera, and the sky's from all round.
    float3 view = rel / max(length(rel), 1e-4);
    float cos_theta = dot(f[0].sun.xyz, view);
    float phase = FORWARD_SHARE * henyey_greenstein(cos_theta, FORWARD) + (1.0 - FORWARD_SHARE) / (4.0 * PI);
    float sun = sun_visibility(f, q.position);
    Sky* sky = f[0].sky;
    float scale = f[0].sun_radiance.w;
    float3 sky_light = 0.5 * (sh_irradiance(sky[0].irradiance, float3(0.0, 1.0, 0.0)) +
                              sh_irradiance(sky[0].irradiance, float3(0.0, -1.0, 0.0))) * scale / PI;
    float3 color = f[0].sun_radiance.rgb * (4.0 * phase * sun) + sky_light;
    if ((q.flags & SPLASH_GRAIN) != 0u)
    {
        // A grain (#192): a Lambertian lump, its disc's mean radiance two thirds of a surface
        // facing the sun's times the Lambert sphere's phase law at the angle between the sun and
        // the camera seen from it (full towards the sun, dark against it), and the sky's.
        uint c = q.flags >> 16u;
        float3 root = float3(float((c >> 11u) & 31u) / 31.0, float((c >> 5u) & 63u) / 63.0, float(c & 31u) / 31.0);
        float3 albedo = root * root;
        float a = acos(clamp(-cos_theta, -1.0, 1.0));
        float law = (sin(a) + (PI - a) * cos(a)) / PI;
        color = albedo * (f[0].sun_radiance.rgb * (2.0 / (3.0 * PI)) * law * sun + sky_light);
    }
    float luminance = dot(color, float3(0.2126, 0.7152, 0.0722));
    color *= min(1.0, f[0].sun.w / max(luminance, 1e-6));
    // The air between the camera and the drop, as the compose hazes everything else.
    float2 ndc = clip.xy / clip.w;
    float2 uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    float4 air = aerial_at(sky, saturate(uv), clip.w * 0.001);
    o.color = color * air.a + air.rgb * scale;
    return o;
}

struct SplashTargets
{
    float4 color : SV_Target0;
    float4 reactive : SV_Target1;
};

[shader("fragment")]
SplashTargets frag_main(SplashVaryings input)
{
    SplashFrame* f = splash_push.frame;
    // A soft disc round the streak's segment, in half widths.
    float beyond = max(abs(input.shape.x) - input.shape.z, 0.0);
    float d2 = (beyond * beyond + input.shape.y * input.shape.y) / (input.shape.w * input.shape.w);
    float shape = saturate(1.0 - d2);
    shape *= shape;
    // Faded where it meets the scene: the water's surface, the banks.
    float depth = g_sampled_images_r[NonUniformResourceIndex(f[0].depth)].Load(int3(int2(input.position.xy), 0));
    float scene = depth > 0.0 ? f[0].camera.w / depth : 1e30;
    float soft = smoothstep(0.0, 1.0, saturate((scene - input.fade.y) / input.fade.z));
    float alpha = input.fade.x * shape * soft;
    if (alpha < 1.0 / 512.0)
        discard;
    SplashTargets o;
    o.color = float4(input.color, alpha);
    o.reactive = float4(REACTIVE, 0.0, 0.0, alpha);
    return o;
}
