// Wakes on still water and the sea (#107): wave particles (Yuksel, House & Keyser 2007, "Wave
// Particles"; `docs/research/water.md`), each carrying a short packet of waves rather than a
// single bump, so a wake's arms show several crests.
//
// A particle is a piece of a wave front moving out from where it set out at the speed of its
// waves. It carries a packet: a cosine envelope of the kernel's radius over waves of a fixed
// length running along its heading, their phase the world's (where and when), so the packets of
// one front add up instead of cancelling. When the gap to its neighbours on the spreading front
// grows past half a radius it splits in three: a third of the height and of the front's angle
// each. Each frame:
// - `wakes/advance` moves last frame's particles on, splitting and dropping them;
// - `wakes/emit` adds new ones around the outline of each thing moving through the water: a
//   crest where it pushes the water out, a trough where it leaves it;
// - `wakes/rings` adds a small ring where the splashes' drops landed on still water last frame
//   (`docs/research/water.md` §7.5): a cell whose drops brought enough impulse sends one out,
//   so a shower of spray becomes a field of ripples without a ring a drop;
// - each particle adds its packet to a field of heights around the camera as it is written;
// - `wakes/slopes` turns the field into the slopes the water's shading adds (`wake_slope` in
//   `water.slang`).
// Visual only: the wakes never feed gameplay (`docs/research/water.md`).

import bindless;

static const float PI = 3.14159265358979;

// A packet's radius and its waves' length, metres, and their speed in deep water
// (c = sqrt(g λ / 2π)): something faster than that leaves a V of half-angle asin(c / v), about
// Kelvin's 19.5° at 2.5 m/s.
static const float WAKE_RADIUS = 1.0;
static const float WAKE_LENGTH = 0.5;
static const float WAKE_SPEED = 0.88;
// A particle's height falls by e every this many seconds; under this height it is dropped.
static const float WAKE_DECAY = 3.0;
static const float WAKE_LEAST = 2e-4;
// Metres of height a metre a second of the outline's push makes in a second of emission.
static const float WAKE_GAIN = 0.05;
// The field's heights are whole numbers of this many metres, added atomically.
static const float WAKE_UNIT = 1e-6;
// The drops' rings: a cell rings once its impulse (`IMPULSE_UNIT` in splashes.slang: a drop's
// radius cubed times its speed) reaches RING_LEAST, as a 1 cm drop at 2 m/s brings; under that
// it keeps half of it a frame. A ring's height is RING_GAIN metres a unit, at most RING_MOST,
// on RING_AROUND particles starting RING_START out from a point in the cell.
static const uint RING_LEAST = 200u;
static const float RING_GAIN = 0.6e-6;
static const float RING_MOST = 1e-3;
static const uint RING_AROUND = 8u;
static const float RING_START = 0.05;

struct WakeParticle
{
    float2 origin;   // where its front set out from, world x, z (the sea's frame)
    float birth;     // when, the sea's seconds
    float angle;     // its heading, radians from +x towards +z
    float spread;    // the front's angle it stands for, radians
    float height;    // metres, at its birth
    float2 pad;
};

struct WakeEmitter
{
    float4 a; // world x, z, the radius of its outline at the water's level, its speed upwards
    float4 b; // its velocity through the water (world x, z, m/s), 0, 0
};

struct WakePush
{
    WakeParticle* from;   // last frame's particles
    WakeParticle* to;     // this frame's
    uint* counts;         // the particles in each of the two buffers, then this frame's rings
    int* heights;         // the field, row-major, `cells` a side
    WakeEmitter* emitters;
    float time;           // the sea's seconds
    uint from_index;      // which of `counts` is `from`'s (0 or 1); `to`'s is the other
    float2 origin;        // the field's corner, world x, z
    float cell;           // metres a cell
    uint cells;           // cells a side
    uint capacity;        // particles a buffer holds
    uint emitter_count;
    uint around;          // particles around each emitter's outline
    float step;           // seconds of emission: since the last, or 0 for none this frame
    uint slopes;          // storage index (rg16f): the field's slopes
    // The drops' impulses (splash_foam.slang's second half):
    uint ring_cells;      // cells a side
    uint* impulses;       // the cells, a world cell at its index modulo ring_cells
    int2 ring_origin;     // this frame's window: its first world cell
    float ring_cell;      // metres a cell
    uint ring_most;       // the most cells that ring in a frame
};

[[vk::push_constant]]
WakePush wake_push;

float2 heading(float angle)
{
    return float2(cos(angle), sin(angle));
}

// Adds a packet of `height` at `at`, its waves running along `dir`, to the field.
void splat(WakePush p, float2 at, float2 dir, float height)
{
    float2 g = (at - p.origin) / p.cell;
    int reach = int(ceil(WAKE_RADIUS / p.cell));
    int2 centre = int2(floor(g));
    // The waves' phase at the field's corner: where along `dir` and when, in wavelengths.
    float phase = frac((dot(p.origin, dir) - WAKE_SPEED * p.time) / WAKE_LENGTH);
    for (int y = -reach; y <= reach; y++)
    {
        for (int x = -reach; x <= reach; x++)
        {
            int2 c = centre + int2(x, y);
            if (any(c < 0) || any(c >= int(p.cells)))
                continue;
            float2 cell = float2(c) + 0.5;
            float d = length(cell - g) * p.cell;
            if (d >= WAKE_RADIUS)
                continue;
            float wave = cos(2.0 * PI * (phase + dot(cell, dir) * p.cell / WAKE_LENGTH));
            float h = height * 0.5 * (cos(PI * d / WAKE_RADIUS) + 1.0) * wave;
            int units = int(round(h / WAKE_UNIT));
            if (units != 0)
                InterlockedAdd(p.heights[uint(c.y) * p.cells + uint(c.x)], units);
        }
    }
}

// Writes particle `q` to this frame's buffer (dropped when it is full) and splats it, its height
// now `height`, at `at`.
void keep(WakePush p, WakeParticle q, float2 at, float height)
{
    uint slot;
    InterlockedAdd(p.counts[1u - p.from_index], 1u, slot);
    if (slot >= p.capacity)
        return;
    p.to[slot] = q;
    splat(p, at, heading(q.angle), height);
}

[shader("compute")]
[numthreads(64, 1, 1)]
void advance_main(uint3 id : SV_DispatchThreadID)
{
    WakePush p = wake_push;
    if (id.x >= min(p.counts[p.from_index], p.capacity))
        return;
    WakeParticle q = p.from[id.x];
    float age = p.time - q.birth;
    float now = q.height * exp(-age / WAKE_DECAY);
    if (age < 0.0 || abs(now) < WAKE_LEAST)
        return;
    float travelled = WAKE_SPEED * age;
    if (q.spread * travelled <= 0.5 * WAKE_RADIUS)
    {
        keep(p, q, q.origin + heading(q.angle) * travelled, now);
        return;
    }
    // Its front has spread past half a radius to its neighbours: three particles from the same
    // place and time, a third of the angle and of the height each.
    float third = q.spread / 3.0;
    for (int k = -1; k <= 1; k++)
    {
        WakeParticle c = q;
        c.angle = q.angle + float(k) * third;
        c.spread = third;
        c.height = q.height / 3.0;
        keep(p, c, c.origin + heading(c.angle) * travelled, now / 3.0);
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
void emit_main(uint3 id : SV_DispatchThreadID)
{
    WakePush p = wake_push;
    uint e = id.x / p.around;
    if (e >= p.emitter_count)
        return;
    WakeEmitter m = p.emitters[e];
    float spread = 2.0 * PI / float(p.around);
    float angle = (float(id.x % p.around) + 0.5) * spread;
    float2 n = heading(angle);
    // The water its outline pushed out since the last emission: ahead of it a crest, behind it a
    // trough; sinking it pushes the water out all round, rising it draws it in.
    float height = WAKE_GAIN * (dot(n, m.b.xy) - m.a.w) * p.step;
    if (abs(height) < WAKE_LEAST)
        return;
    WakeParticle q;
    q.origin = m.a.xy + n * m.a.z;
    q.birth = p.time;
    q.angle = angle;
    q.spread = spread;
    q.height = height;
    q.pad = 0.0;
    keep(p, q, q.origin, height);
}

// Two numbers in 0..1 from a world cell (a PCG step each way).
float2 cell_random(int2 c)
{
    uint2 v = uint2(c) * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v ^= v >> 16u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v ^= v >> 16u;
    return float2(v >> 8u) * (1.0 / 16777216.0);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void rings_main(uint3 id : SV_DispatchThreadID)
{
    WakePush p = wake_push;
    int n = int(p.ring_cells);
    if (int(id.x) >= n || int(id.y) >= n)
        return;
    uint i = id.y * uint(n) + id.x;
    uint impulse = p.impulses[i];
    if (impulse == 0u)
        return;
    uint ring = p.ring_most;
    if (impulse >= RING_LEAST)
        InterlockedAdd(p.counts[2], 1u, ring);
    if (ring >= p.ring_most)
    {
        // Too little yet, or too many rings this frame: it keeps half for the next.
        p.impulses[i] = impulse / 2u;
        return;
    }
    p.impulses[i] = 0u;
    // The world cell this slot holds, and a point in it of its own.
    int2 slot = int2(id.xy);
    int2 rel = slot - p.ring_origin;
    int2 cell = p.ring_origin + int2(((rel.x % n) + n) % n, ((rel.y % n) + n) % n);
    float2 r = cell_random(cell);
    float2 centre = (float2(cell) + 0.2 + 0.6 * r) * p.ring_cell;
    float height = min(RING_GAIN * float(impulse), RING_MOST);
    float spread = 2.0 * PI / float(RING_AROUND);
    for (uint k = 0u; k < RING_AROUND; k++)
    {
        float angle = (float(k) + r.y) * spread;
        WakeParticle q;
        q.origin = centre + heading(angle) * RING_START;
        q.birth = p.time;
        q.angle = angle;
        q.spread = spread;
        q.height = height;
        q.pad = 0.0;
        keep(p, q, q.origin, height);
    }
}

float field_height(WakePush p, int2 c)
{
    c = clamp(c, 0, int(p.cells) - 1);
    return float(p.heights[uint(c.y) * p.cells + uint(c.x)]) * WAKE_UNIT;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void slopes_main(uint3 id : SV_DispatchThreadID)
{
    WakePush p = wake_push;
    if (any(id.xy >= p.cells))
        return;
    int2 c = int2(id.xy);
    float2 slope = float2(
        field_height(p, c + int2(1, 0)) - field_height(p, c - int2(1, 0)),
        field_height(p, c + int2(0, 1)) - field_height(p, c - int2(0, 1)));
    g_storage_images_rg16f[p.slopes][id.xy] = slope / (2.0 * p.cell);
}
