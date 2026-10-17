// Particles (D-064, #271): smoke, dust and embers from emitters as data. Driven by
// crates/forge-render/src/particles.rs, the effects in crates/forge-render/data/particles.toml.
//
// Grown from the splashes (#107, `splashes.slang`): the particles live in a ring of slots the CPU
// hands out in blocks, in order, so the draw's order holds from frame to frame. Each frame:
// - `particles/emit` (compute queue): the frame's new particles, from the blocks the CPU wrote.
//   Each one's random numbers come from its emitter's seed and its index in the emitter's
//   stream, so an effect is the same whatever the frame rate.
// - `particles/advance` (compute queue): gravity or buoyancy, a drag towards the air (the
//   scene's wind plus a curl-noise swirl, divergence-free), to this frame's time. Then its
//   light, once a particle (D-064's point 4): one ray towards the sun against the scene's
//   structure, and the sky's light from above and below.
// - `particles/draw`: each live particle a soft puff facing the camera, turned by its own
//   angle, its edge broken by noise; lit smoke alpha-blended, glowing embers added (one
//   premultiplied blend does both), faded where it meets the scene, hazed by the aerial
//   perspective; it also writes the reactive mask TAA reads.
// Visual only: nothing reads the particles back.

import bindless;
import skyframe;
import sh;

static const float PI = 3.14159265358979;
static const float GRAVITY = 9.81;
// The longest step of the integration, s, and the longest span it covers in a frame.
static const float MAX_STEP = 1.0 / 30.0;
static const float MAX_SPAN = 0.1;
// The share of a particle's life over which it fades in.
static const float FADE_IN = 0.08;
// The most a particle asks of TAA's current frame (FSR 2's cap for its reactive mask).
static const float REACTIVE = 0.9;
// Smoke scatters a little forwards (Henyey–Greenstein), with an even part for the light it
// scatters more than once.
static const float FORWARD = 0.35;
static const float FORWARD_SHARE = 0.6;
// The ways an effect is drawn (`DrawKind` in particles.rs).
static const uint DRAW_SMOKE = 0u;
static const uint DRAW_GLOW = 1u;
// A glow's colour is in thousands of candelas a square metre.
static const float GLOW_UNIT = 1000.0;

struct Particle
{
    float3 position; // the scene's frame, metres
    float birth;     // the scene's seconds
    float3 velocity; // m/s
    float life;      // seconds; 0 once it is dead
    float3 light;    // pre-exposed radiance a white particle sends the camera from the sky (smoke)
    uint seed;
    uint effect;     // its effect's index
    float spin;      // its puff's turn, radians
    float sun;       // the sun's share: its phase towards the camera times its visibility
    float pad;
};

// Must match GpuEffect in particles.rs.
struct Effect
{
    float4 colour0;  // rgba at birth
    float4 colour1;  // rgba at death
    float2 radius;   // at birth, at death
    float gravity;   // times g, downwards
    float drag;      // 1/s
    float wind;      // the share of the scene's wind
    float curl;      // m/s
    float curl_size; // metres
    uint draw;       // DRAW_SMOKE or DRAW_GLOW
};

// Must match GpuBlock in particles.rs.
struct Block
{
    float4 origin;    // xyz: its emitter's point; w: the spread, metres
    float4 axis;      // xyz: the launch direction; w: the cone's half-angle, radians
    float4 speed;     // xy: launch speeds, m/s; zw: lives, s
    float4 carry;     // xyz: velocity added to every particle (the emitter's own); w: unused
    float birth;      // its first particle's birth, the scene's seconds
    float interval;   // seconds between births (0: a burst)
    uint first;       // its first slot
    uint count;
    uint seed;        // its emitter's
    uint effect;
    uint start;       // its first particle among the frame's new ones
    uint index;       // its first particle's index in its emitter's stream
};

// Must match GpuFrame in particles.rs.
struct ParticleFrame
{
    float4x4 view_proj;  // camera-relative, jitter included
    float4 camera;       // xyz: the camera in the scene's frame; w: the near plane, metres
    float4 sun;          // xyz: towards the sun; w: the exposure (pre-exposed per cd/m²)
    float4 sun_radiance; // rgb: the sun's pre-exposed illuminance; w: the sky's scale
    float4 wind;         // xyz: the air's velocity, m/s; w: the scene's seconds, for the swirl
    float now;           // the scene's seconds
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
    Particle* ring;
    Block* blocks;
    Effect* effects;
};

struct ParticlePush
{
    ParticleFrame* frame;
};

[[vk::push_constant]]
ParticlePush particle_push;

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

// Two directions across `axis`.
void across(float3 axis, out float3 t, out float3 b)
{
    t = normalize(cross(abs(axis.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0), axis));
    b = cross(axis, t);
}

// A direction within `half_angle` of `axis`, uniform over the cap, from two numbers in 0..1.
float3 in_cone(float3 axis, float half_angle, float u, float v)
{
    float cos_theta = 1.0 - u * (1.0 - cos(half_angle));
    float sin_theta = sqrt(saturate(1.0 - cos_theta * cos_theta));
    float phi = 2.0 * PI * v;
    float3 t;
    float3 b;
    across(axis, t, b);
    return axis * cos_theta + (t * cos(phi) + b * sin(phi)) * sin_theta;
}

[shader("compute")]
[numthreads(64, 1, 1)]
void emit_main(uint3 id : SV_DispatchThreadID)
{
    ParticleFrame* f = particle_push.frame;
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
    Block b = f[0].blocks[lo];
    uint j = id.x - b.start;
    uint k = b.index + j;
    float3 u = random3(uint3(b.seed, k, 0u));
    float3 w = random3(uint3(b.seed, k, 1u));
    float3 t;
    float3 bt;
    across(b.axis.xyz, t, bt);
    // Born within a disc across its direction, uniform over the disc.
    float r = b.origin.w * sqrt(u.x);
    float a = 2.0 * PI * u.y;
    Particle q;
    q.position = b.origin.xyz + (t * cos(a) + bt * sin(a)) * r;
    q.birth = b.birth + b.interval * float(j);
    q.velocity = in_cone(b.axis.xyz, b.axis.w, u.z, w.x) * lerp(b.speed.x, b.speed.y, w.y) + b.carry.xyz;
    q.life = lerp(b.speed.z, b.speed.w, w.z);
    q.light = float3(0.0);
    q.seed = pcg3d(uint3(b.seed, k, 2u)).x;
    q.effect = b.effect;
    q.spin = 2.0 * PI * random3(uint3(b.seed, k, 3u)).x;
    q.sun = 0.0;
    q.pad = 0.0;
    f[0].ring[(b.first + j) % f[0].capacity] = q;
}

// A divergence-free swirl (Bridson et al. 2007's curl noise, from three sums of sines in
// place of their noise): the curl of a potential, so the smoke turns without gathering or
// thinning. `size` is the eddies', metres; `time` drifts them.
float3 curl_swirl(float3 p, float size, float time)
{
    float3 x = p / max(size, 1e-3);
    float s = 0.31 * time;
    // ψ = (sin(y + s) + sin(z·1.3), sin(z − s) + sin(x·1.7), sin(x + s·0.7) + sin(y·1.1)),
    // and the swirl its curl: (∂ψz/∂y − ∂ψy/∂z, ∂ψx/∂z − ∂ψz/∂x, ∂ψy/∂x − ∂ψx/∂y).
    float dzdy = 1.1 * cos(x.y * 1.1);
    float dydz = cos(x.z - s);
    float dxdz = 1.3 * cos(x.z * 1.3);
    float dzdx = cos(x.x + 0.7 * s);
    float dydx = 1.7 * cos(x.x * 1.7);
    float dxdy = cos(x.y + s);
    return float3(dzdy - dydz, dxdz - dzdx, dydx - dxdy) * 0.5;
}

// Henyey–Greenstein's phase at `cos_theta` between the light's way and the view's.
float henyey_greenstein(float cos_theta, float g)
{
    float d = 1.0 + g * g - 2.0 * g * cos_theta;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

// Whether the sun reaches `p` (the scene's frame).
float sun_visibility(ParticleFrame* f, float3 p)
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

[shader("compute")]
[numthreads(64, 1, 1)]
void advance_main(uint3 id : SV_DispatchThreadID)
{
    ParticleFrame* f = particle_push.frame;
    if (id.x >= f[0].live)
        return;
    uint slot = (f[0].base + id.x) % f[0].capacity;
    Particle q = f[0].ring[slot];
    if (q.life <= 0.0)
        return;
    float now = f[0].now;
    if (now - q.birth > q.life)
    {
        f[0].ring[slot].life = 0.0;
        return;
    }
    Effect e = f[0].effects[q.effect];
    float span = min(now - max(q.birth, f[0].previous), MAX_SPAN);
    if (span > 0.0)
    {
        uint steps = uint(ceil(span / MAX_STEP));
        float h = span / float(steps);
        for (uint i = 0u; i < steps; i++)
        {
            float3 air = f[0].wind.xyz * e.wind + curl_swirl(q.position, e.curl_size, f[0].wind.w) * e.curl;
            // Gravity (or buoyancy), then the drag as a relaxation towards the air: stable at
            // any step.
            float3 v = q.velocity + float3(0.0, -GRAVITY * e.gravity * h, 0.0);
            q.velocity = air + (v - air) * exp(-e.drag * h);
            q.position += q.velocity * h;
        }
    }
    // Its light, once a particle a frame: the sun's, scattered towards the camera, and the
    // sky's from above and below. A glow makes its own.
    if (e.draw == DRAW_SMOKE)
    {
        float3 view = normalize(q.position - f[0].camera.xyz);
        float cos_theta = dot(f[0].sun.xyz, view);
        float phase = FORWARD_SHARE * henyey_greenstein(cos_theta, FORWARD) + (1.0 - FORWARD_SHARE) / (4.0 * PI);
        Sky* sky = f[0].sky;
        float3 sky_light = 0.5 * (sh_irradiance(sky[0].irradiance, float3(0.0, 1.0, 0.0)) +
                                  sh_irradiance(sky[0].irradiance, float3(0.0, -1.0, 0.0))) * f[0].sun_radiance.w / PI;
        q.light = sky_light;
        q.sun = 4.0 * phase * sun_visibility(f, q.position);
    }
    f[0].ring[slot] = q;
}

// ---- The draw ---------------------------------------------------------------------------------

struct ParticleVaryings
{
    float4 position : SV_Position;
    float4 color : COLOR;              // rgb: pre-exposed and hazed, premultiplied by a; a: its
                                       // opacity (0 for a glow, which adds). Smoke: the sky's
                                       // light and the haze
    float3 sun : COLOR1;               // smoke: the sun's light, premultiplied, before the puff's
                                       // shading
    nointerpolation float3 towards_sun : TEXCOORD2; // the sun in the puff's frame: x right, y
                                                    // down the screen, z towards the camera
    float3 corner : TEXCOORD0;         // xy: where in its puff, -1..1 each way; z: its age over
                                       // its life
    nointerpolation float4 fade : TEXCOORD1; // x: its distance, metres; y: over how far it fades
                                             // into the scene; z: its seed's noise offset; w: its
                                             // spin
};

// Value noise in 2D: a smooth random field, about 0..1.
float value_noise(float2 p, uint seed)
{
    float2 i = floor(p);
    float2 t = p - i;
    t = t * t * (3.0 - 2.0 * t);
    uint2 c = uint2(int2(i) + 4096);
    float a = random3(uint3(c, seed)).x;
    float b = random3(uint3(c + uint2(1, 0), seed)).x;
    float d = random3(uint3(c + uint2(0, 1), seed)).x;
    float g = random3(uint3(c + uint2(1, 1), seed)).x;
    return lerp(lerp(a, b, t.x), lerp(d, g, t.x), t.y);
}

[shader("vertex")]
ParticleVaryings vert_main(uint vertex : SV_VertexID)
{
    ParticleFrame* f = particle_push.frame;
    ParticleVaryings o;
    // A dead slot: six times the same point, outside the view.
    o.position = float4(2.0, 2.0, 0.5, 1.0);
    o.color = float4(0.0);
    o.sun = float3(0.0);
    o.towards_sun = float3(0.0, 0.0, 1.0);
    o.corner = float3(0.0);
    o.fade = float4(0.0);
    uint i = vertex / 6u;
    if (i >= f[0].live)
        return o;
    Particle q = f[0].ring[(f[0].base + i) % f[0].capacity];
    float age = f[0].now - q.birth;
    if (q.life <= 0.0 || age < 0.0 || age > q.life)
        return o;
    Effect e = f[0].effects[q.effect];
    float s = saturate(age / q.life);
    float3 rel = q.position - f[0].camera.xyz;
    float4 clip = mul(f[0].view_proj, float4(rel, 1.0));
    float near = f[0].camera.w;
    if (clip.w < near)
        return o;
    float2 half_size = 0.5 * float2(f[0].width, f[0].height);
    float radius = lerp(e.radius.x, e.radius.y, s);
    // At least a pixel across, its opacity scaled by the area it lacks (Persson's phone-wire AA).
    float radius_px = radius * f[0].focal / clip.w;
    float drawn_px = max(radius_px, 0.75);
    float coverage = (radius_px * radius_px) / (drawn_px * drawn_px);
    uint corner = vertex % 6u;
    float2 c = float2(corner == 1u || corner == 2u || corner == 4u ? 1.0 : -1.0,
                      corner == 2u || corner == 4u || corner == 5u ? 1.0 : -1.0);
    float2 middle = clip.xy / clip.w * half_size;
    o.position = float4((middle + c * drawn_px) / half_size * clip.w, clip.z, clip.w);
    o.corner = float3(c, s);
    o.fade = float4(clip.w, max(radius, 0.05), float(q.seed & 1023u), q.spin);

    float4 colour = lerp(e.colour0, e.colour1, s);
    float fade_in = saturate(s / FADE_IN);
    float alpha = colour.a * fade_in * coverage;
    Sky* sky = f[0].sky;
    float scale = f[0].sun_radiance.w;
    float2 ndc = clip.xy / clip.w;
    float2 uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    float4 air = aerial_at(sky, saturate(uv), clip.w * 0.001);
    if (e.draw == DRAW_GLOW)
    {
        // Added: its own light, thinned by the air, over what is behind it.
        float3 glow = colour.rgb * GLOW_UNIT * f[0].sun.w * alpha;
        o.color = float4(glow * air.a, 0.0);
    }
    else
    {
        // Over what is behind it, by its opacity: lit, then hazed as the compose hazes the rest.
        // The sky's light and the haze are even over the puff; the sun's is shaded across it.
        o.color = float4((colour.rgb * q.light * air.a + air.rgb * scale) * alpha, alpha);
        o.sun = colour.rgb * f[0].sun_radiance.rgb * (q.sun * air.a * alpha);
        // The sun's way on the screen: where a point a little towards it lands, from the
        // particle's own place.
        float3 view = rel / max(length(rel), 1e-4);
        float toward_camera = -dot(f[0].sun.xyz, view);
        float4 lit_clip = mul(f[0].view_proj, float4(rel + f[0].sun.xyz * (0.05 * clip.w), 1.0));
        float2 way = lit_clip.w > near ? lit_clip.xy / lit_clip.w - ndc : float2(0.0);
        float2 across = length(way) > 1e-6 ? normalize(way) : float2(0.0);
        o.towards_sun = float3(across * sqrt(saturate(1.0 - toward_camera * toward_camera)), toward_camera);
    }
    return o;
}

struct ParticleTargets
{
    float4 color : SV_Target0;
    float4 reactive : SV_Target1;
};

[shader("fragment")]
ParticleTargets frag_main(ParticleVaryings input)
{
    ParticleFrame* f = particle_push.frame;
    // A billowing puff: three octaves of noise turned with the particle and churning over its
    // life, eroded from the edge in (denser in the middle), so the puffs show lumps and do not
    // read as discs.
    float2 p = input.corner.xy;
    float age = input.corner.z;
    float spin = input.fade.w;
    float2 turned = float2(p.x * cos(spin) - p.y * sin(spin), p.x * sin(spin) + p.y * cos(spin));
    uint seed = uint(input.fade.z);
    float2 churn = float2(1.3, -0.9) * age;
    float n = 0.55 * value_noise(turned * 1.8 + 7.0 + churn, seed)
        + 0.3 * value_noise(turned * 4.2 + 3.0 - churn, seed + 1u)
        + 0.15 * value_noise(turned * 9.0 + 11.0 + 2.0 * churn, seed + 2u);
    float d2 = dot(p, p);
    float shape = saturate(1.0 - d2) * smoothstep(0.0, 0.45, n + 0.35 - 0.9 * d2);
    // Faded where it meets the scene: the ground, a wall.
    float depth = g_sampled_images[NonUniformResourceIndex(f[0].depth)].Load(int3(int2(input.position.xy), 0)).r;
    float scene = depth > 0.0 ? f[0].camera.w / depth : 1e30;
    float soft = smoothstep(0.0, 1.0, saturate((scene - input.fade.x) / input.fade.y));
    float k = shape * soft;
    // The puff shaded as a lumpy ball in the sun: its normal from where in the disc, bent by
    // the noise, lit with a wrap (light reaches round a puff of smoke), so a plume shows its
    // billows' lit and shaded sides. About as bright on average as unshaded.
    float3 normal = normalize(float3(p + (n - 0.5) * float2(0.8, -0.6), sqrt(saturate(1.0 - d2)) + 0.2));
    float wrap = saturate((dot(normal, input.towards_sun) + 0.5) / 1.5);
    float shade = 0.35 + 1.1 * wrap;
    float4 color = float4(input.color.rgb + input.sun * shade, input.color.a) * k;
    if (max(color.a, max(color.r, max(color.g, color.b))) < 1.0 / 4096.0)
        discard;
    ParticleTargets o;
    o.color = color;
    // The reactive mask, over what the splashes wrote: premultiplied too, so it stays under
    // REACTIVE however many puffs overlap.
    float r = saturate(max(color.a, 0.25 * k));
    o.reactive = float4(REACTIVE * r, 0.0, 0.0, r);
    return o;
}
