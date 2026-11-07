// Particles lit by the lamps (D-064): for an effect with a share of them (`Effect::lamps`), each live
// puff takes the frame's lamps at its middle, as a froxel of the fog does (`fog.slang`; unshadowed,
// the same reach and beams), towards the camera by the smoke's phase, added to the light
// `particles/advance` gave it this frame. Its own pass and file: it needs the frame's lamps
// (`meshlet.slang`), which `particles.slang` does not. Driven by crates/forge-render/src/particles.rs.
#include "meshlet.slang"

// Smoke scatters a little forwards, with an even part for the light it scatters more than once
// (`FORWARD`, `FORWARD_SHARE` in particles.slang).
static const float LAMP_FORWARD = 0.35;
static const float LAMP_FORWARD_SHARE = 0.6;
// A lamp lights a puff as if no nearer than this, metres: a puff is about 2 m wide and a lamp inside it
// lights it through, not at a point. At half the puff's radius the strongest lamps lit white blobs.
static const float LAMP_LEAST_DISTANCE = 1.0;

// Must match `Particle` in particles.slang.
struct LampedParticle
{
    float3 position;
    float birth;
    float3 velocity;
    float life;
    float3 light;
    uint seed;
    uint effect;
    float spin;
    float sun;
    float lit;
};

// Must match `Effect` in particles.slang.
struct LampedEffect
{
    float4 colour0;
    float4 colour1;
    float2 radius;
    float gravity;
    float drag;
    float wind;
    float curl;
    float curl_size;
    uint draw;
    float sun;
    float lamps;
    float pad1;
    float pad2;
};

// Must match `LampPush` in particles.rs.
struct ParticleLampPush
{
    float4 camera;           // xyz: the camera in the scene's frame; w: the pre-exposure
    LampedParticle* ring;
    LampedEffect* effects;
    Frame* frame;            // the resolve's frame: its lamps, their grid
    uint base;               // the first live slot
    uint live;               // live slots from it
    uint capacity;
    float now;               // the scene's seconds
};

[[vk::push_constant]]
ParticleLampPush particle_lamp_push;

float particle_lamp_phase(float cos_theta)
{
    float g2 = LAMP_FORWARD * LAMP_FORWARD;
    float forward = (1.0 - g2) / (4.0 * 3.14159265 * pow(max(1.0 + g2 - 2.0 * LAMP_FORWARD * cos_theta, 1e-4), 1.5));
    return LAMP_FORWARD_SHARE * forward + (1.0 - LAMP_FORWARD_SHARE) / (4.0 * 3.14159265);
}

[shader("compute")]
[numthreads(64, 1, 1)]
void lamps_main(uint3 id : SV_DispatchThreadID)
{
    ParticleLampPush p = particle_lamp_push;
    if (id.x >= p.live)
        return;
    uint slot = (p.base + id.x) % p.capacity;
    LampedParticle q = p.ring[slot];
    if (q.life <= 0.0 || p.now < q.birth || p.now > q.birth + q.life)
        return;
    LampedEffect e = p.effects[q.effect];
    if (e.lamps <= 0.0)
        return;
    Frame* f = p.frame;
    if (f[0].lamp_count == 0u || f[0].lamps == nullptr)
        return;
    float3 direction = normalize(q.position - p.camera.xyz);
    float3 light = float3(0.0);
    LampRuns runs = lamp_runs(f, REGION_NONE, REGION_NONE, q.position, true);
    for (uint r = 0u; r < 3u; r++)
    {
        for (uint k = runs.run[r].x; k < runs.run[r].y; k++)
        {
            Lamp lamp = f[0].lamps[lamp_at(f, runs, k)];
            float3 d = lamp.position_radius.xyz - q.position;
            float dist2 = dot(d, d);
            float reach = lamp.colour_reach.w;
            if (dist2 >= reach * reach)
                continue;
            float dist = sqrt(dist2);
            float3 to_lamp = d / max(dist, 1e-6);
            float window = lamp_tube(lamp) ? lamp_tube_window(lamp, d) : saturate(1.0 - pow(dist / reach, 4.0));
            float least = max(lamp.position_radius.w, LAMP_LEAST_DISTANCE);
            if (lamp_tube(lamp))
            {
                float3 nearest = d + lamp.beam.xyz * clamp(dot(-d, lamp.beam.xyz), -lamp.beam_edge.x, lamp.beam_edge.x);
                to_lamp = nearest / max(length(nearest), 1e-6);
            }
            float3 lux = lamp_tube(lamp)
                ? lamp.colour_reach.rgb * (lamp_tube_fog(lamp, q.position, least) * window * window)
                : lamp.colour_reach.rgb / max(dist2, least * least) * window * window * lamp_beam(lamp, to_lamp);
            // By the lamp's share in the air (`Lamp::fog`); 4 × the phase, as `q.sun` takes the sun's.
            light += (1.0 - asfloat(lamp.flags.y)) * lux * (4.0 * particle_lamp_phase(dot(-direction, -to_lamp)));
        }
    }
    p.ring[slot].light = q.light + light * (e.lamps * p.camera.w);
}
